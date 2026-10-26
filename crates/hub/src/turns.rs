//! Sending, the turn loop, the queue, and interruption.
//!
//! `run_turn` is started **here**, not by a client connection. Directly:
//! closing or refreshing a tab does not end the turn and does not lose its
//! final response, because the response comes back to the hub and the hub
//! broadcasts it. A client that talks to the engine directly loses that
//! response with its connection and has to guess from `turn_complete`.
//!
//! The engine allows one turn per session at a time and refuses a second
//! rather than queueing. That constraint is the engine's business and should
//! not reach the user unchanged, so the queue is here (§3.2, point 4).

use std::sync::Arc;

use daemon::rpc::codes as engine_codes;
use harnix_contract::{Frame, RpcError, Subject};
use serde_json::{json, Value};

use crate::session::QueueItem;
use crate::{new_id, notification, without_instance, Hub, SessionKey};

/// What a send does when the session already has a turn running.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OnBusy {
    /// Wait behind it. The default: queueing is product behavior.
    Queue,
    Reject,
    /// Stop it, drop what was queued, and run this next.
    Interrupt,
    /// Hand it to the running turn, which reads it at its next step
    /// boundary (Agix §5.3).
    Steer,
}

impl OnBusy {
    fn parse(params: &Value) -> Result<Self, RpcError> {
        match params.get("on_busy").and_then(Value::as_str) {
            None | Some("queue") => Ok(OnBusy::Queue),
            Some("reject") => Ok(OnBusy::Reject),
            Some("interrupt") => Ok(OnBusy::Interrupt),
            Some("steer") => Ok(OnBusy::Steer),
            Some(other) => Err(RpcError::invalid_params(format!(
                "on_busy is queue, reject, interrupt or steer, not `{other}`"
            ))),
        }
    }
}

impl Hub {
    pub(crate) async fn send(
        self: &Arc<Self>,
        _subject: &Subject,
        params: Value,
    ) -> Result<Value, RpcError> {
        let key = self.key_of(&params)?;
        let session_id = key.1.clone();
        let on_busy = OnBusy::parse(&params)?;
        let message = params
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        if message.trim().is_empty() {
            return Err(RpcError::invalid_params("empty message"));
        }
        // `@session` references: a copy of what was said in each, as text the turn can read.
        let mut attachments = match params.get("attachments") {
            Some(Value::Array(list)) => list.clone(),
            _ => Vec::new(),
        };
        for reference in params.get("references").and_then(Value::as_array).into_iter().flatten() {
            attachments.push(self.reference(reference).await?);
        }
        let item = QueueItem {
            item_id: new_id("q"),
            message,
            attachments: json!(attachments),
        };
        // Sending implies watching. A client that skipped `harnix.attach` would
        // otherwise get neither the turn's events nor its settlement — frames
        // go to watchers, and it would not be one.
        if let Some(client) = params
            .get("__client")
            .and_then(Value::as_u64)
            .map(crate::ClientId)
        {
            let mut guard = self.inner.lock().await;
            guard
                .sessions
                .entry(key.clone())
                .or_default()
                .watchers
                .insert(client);
        }

        // An instance that is reconnecting takes the send into the queue: it
        // runs once the instance is back and its sessions are resynced. A
        // send is not refused for a hiccup the user cannot see.
        let ready = self
            .slot(&key.0)
            .is_some_and(|slot| slot.link.status().borrow().is_ready());
        if !ready {
            if on_busy == OnBusy::Reject {
                let status = self.slot(&key.0).map(|s| s.status());
                return Err(harnix_instance::link::unavailable(
                    &status.unwrap_or(harnix_instance::LinkStatus::Starting),
                ));
            }
            return Ok(self.enqueue(&key, item, false).await);
        }

        // The session has to exist before a turn is opened on it.
        //
        // `session.run_turn` does not require one — handed an id it does not
        // know, the engine makes a session and runs the turn there. That is
        // reasonable for the engine and wrong for a client: a mistyped id
        // came back as a settled turn, with the transcript in a session under
        // an id the caller had never seen and could not ask for again, and a
        // model call spent on it. The hub owns turns (§3.2), so refusing here
        // is the hub's to do.
        let info = self
            .call(&key.0, "session.get", json!({"session_id": &session_id}))
            .await?;
        self.ensure_subscribed(&key, &info).await?;

        let busy = {
            let guard = self.inner.lock().await;
            guard
                .sessions
                .get(&key)
                .is_some_and(|s| s.running_turn.is_some())
        };
        if busy {
            return match on_busy {
                OnBusy::Reject => Err(RpcError::new(engine_codes::SESSION_BUSY, "session is busy")),
                OnBusy::Queue => Ok(self.enqueue(&key, item, false).await),
                OnBusy::Interrupt => {
                    self.call(&key.0, "session.interrupt", json!({"session_id": session_id}))
                        .await?;
                    {
                        let mut guard = self.inner.lock().await;
                        guard.sessions.entry(key.clone()).or_default().queue.clear();
                    }
                    Ok(self.enqueue(&key, item, true).await)
                }
                // Attachments do not steer (Agix §5.3); they queue.
                OnBusy::Steer if !item.attachments.as_array().is_some_and(|a| !a.is_empty()) => {
                    self.steer(&key, item).await
                }
                OnBusy::Steer => Ok(self.enqueue(&key, item, false).await),
            };
        }

        let turn_id = self.start_turn(&key, item).await?;
        Ok(json!({"turn_id": turn_id}))
    }

    async fn enqueue(&self, key: &SessionKey, item: QueueItem, front: bool) -> Value {
        let (view, snapshot, watchers) = {
            let mut guard = self.inner.lock().await;
            let state = guard.sessions.entry(key.clone()).or_default();
            let view = item.view();
            if front {
                state.queue.push_front(item);
            } else {
                state.queue.push_back(item);
            }
            (view, state.queue_view(), state.watchers())
        };
        self.push_queue(key, &watchers, snapshot).await;
        json!({"queued": true, "item": view})
    }

    /// Give a message to the running turn. The engine answers at once when
    /// it took it; a turn already past its last step boundary turns the
    /// steer into a queued turn of its own, whose call then lasts that
    /// turn — so the answer is waited for briefly and the call left to run.
    async fn steer(self: &Arc<Self>, key: &SessionKey, item: QueueItem) -> Result<Value, RpcError> {
        let turn_id = new_id("t");
        let hub = self.clone();
        let instance = key.0.clone();
        let params = json!({
            "session_id": key.1,
            "turn_id": turn_id,
            "message": item.message,
            "on_busy": "steer",
        });
        let call = tokio::spawn(async move { hub.call(&instance, "session.run_turn", params).await });
        match tokio::time::timeout(std::time::Duration::from_secs(5), call).await {
            Ok(Ok(result)) => result,
            Ok(Err(e)) => Err(RpcError::new(harnix_contract::codes::INTERNAL_ERROR, e.to_string())),
            Err(_) => Ok(json!({"turn_id": turn_id, "delivery": "queued_by_engine"})),
        }
    }

    /// Begin a turn and hand it to a task, so `harnix.send` returns at once and
    /// the turn outlives the client that asked for it.
    ///
    /// The task is a **loop**, not a chain of recursive calls: a settled turn
    /// that finds a queued send starts it in the same task. Written
    /// recursively, `start_turn → settle → start_turn` does not even prove
    /// `Send` in Rust.
    pub(crate) async fn start_turn(
        self: &Arc<Self>,
        key: &SessionKey,
        item: QueueItem,
    ) -> Result<String, RpcError> {
        let turn_id = self.open_turn(key, &item).await;
        let hub = self.clone();
        let key = key.clone();
        let first = (turn_id.clone(), item);
        tokio::spawn(async move {
            let mut current = first;
            loop {
                let (turn_id, item) = current;
                let outcome = hub
                    .call(
                        &key.0,
                        "session.run_turn",
                        json!({
                            "session_id": key.1,
                            "turn_id": turn_id,
                            "message": item.message,
                            "attachments": item.attachments,
                        }),
                    )
                    .await;
                // The link went away under the turn. The turn did not: it
                // runs on, and either reports its end on the link once it is
                // back or is found finished by the resync.
                if crate::is_code(&outcome, harnix_contract::codes::INSTANCE_UNAVAILABLE) {
                    let mut guard = hub.inner.lock().await;
                    if let Some(state) = guard.sessions.get_mut(&key) {
                        if state.running_turn.as_deref() == Some(turn_id.as_str()) {
                            state.external = true;
                        }
                    }
                    break;
                }
                hub.barrier(&key.0).await;
                match hub.settle_turn(&key, &turn_id, outcome).await {
                    Some(next) => {
                        let next_id = hub.open_turn(&key, &next).await;
                        current = (next_id, next);
                    }
                    None => break,
                }
            }
        });
        Ok(turn_id)
    }

    /// Record the transcript depth this turn starts from, drop the previous
    /// turn's buffer, and put the user's own message into the event stream —
    /// the engine emits no event for it, and every other client needs to see
    /// it.
    async fn open_turn(self: &Arc<Self>, key: &SessionKey, item: &QueueItem) -> String {
        let (instance, session_id) = (&key.0, key.1.as_str());
        let total = self.history_total(key).await;
        let turn_id = new_id("t");

        let (seq, event, watchers) = {
            let mut guard = self.inner.lock().await;
            let state = guard.sessions.entry(key.clone()).or_default();
            state.running_turn = Some(turn_id.clone());
            state.external = false;
            state.history_total_before_turn = total;
            state.replay.clear();
            state.seq += 1;
            let seq = state.seq;
            let event = json!({"kind": "user_message", "text": item.message});
            state.replay.push(seq, &turn_id, &event);
            (seq, event, state.watchers())
        };
        self.fan_out(
            &watchers,
            Frame::session(
                session_id,
                notification(
                    "harnix.event",
                    json!({"instance": instance, "session_id": session_id, "seq": seq, "turn_id": turn_id, "event": event}),
                ),
            ),
        )
        .await;
        self.refresh("harnix.sessions.status", json!({}));
        self.refresh("harnix.instances.sessions", json!({"instance": instance}));
        turn_id
    }

    /// A turn ended — completed, failed or interrupted. Tell every watcher,
    /// including clients that were not open when it started; drop the buffer
    /// only once the transcript has caught up; report what to run next.
    pub(crate) async fn settle_turn(
        self: &Arc<Self>,
        key: &SessionKey,
        turn_id: &str,
        outcome: Result<Value, RpcError>,
    ) -> Option<QueueItem> {
        let (instance, session_id) = (&key.0, key.1.as_str());
        let total_after = self.history_total(key).await;
        let (watchers, next) = {
            let mut guard = self.inner.lock().await;
            let state = guard.sessions.entry(key.clone()).or_default();
            if state.running_turn.as_deref() != Some(turn_id) {
                // Settled already, by whichever of the loop, `turn_completed`
                // and a resync got here first.
                return None;
            }
            state.running_turn = None;
            state.external = false;
            state.pending_prompts.clear();
            // Only once the transcript has grown. A turn that finished but is
            // not yet on disk would otherwise be in neither place.
            if total_after > state.history_total_before_turn {
                state.replay.clear();
            }
            (state.watchers(), state.queue.pop_front())
        };

        let params = match &outcome {
            Ok(result) => json!({"instance": instance, "session_id": session_id, "turn_id": turn_id, "result": result}),
            Err(error) => json!({
                "instance": instance, "session_id": session_id, "turn_id": turn_id,
                "error": {"code": error.code, "message": error.message},
            }),
        };
        self.fan_out(
            &watchers,
            Frame::session(session_id, notification("harnix.turn_settled", params)),
        )
        .await;

        if next.is_some() {
            let snapshot = {
                let guard = self.inner.lock().await;
                guard
                    .sessions
                    .get(key)
                    .map(|s| s.queue_view())
                    .unwrap_or_default()
            };
            self.push_queue(key, &watchers, snapshot).await;
        }
        self.refresh("harnix.sessions.status", json!({}));
        self.refresh("harnix.instances.sessions", json!({"instance": instance}));
        next
    }

    /// Interrupting is a decision about the whole session, not just the
    /// running turn: the queue behind it is abandoned too. Otherwise a user
    /// presses stop and the next queued message starts immediately.
    pub(crate) async fn interrupt(self: &Arc<Self>, params: Value) -> Result<Value, RpcError> {
        let key = self.key_of(&params)?;
        let result = self
            .call(&key.0, "session.interrupt", json!({"session_id": key.1}))
            .await?;
        let watchers = {
            let mut guard = self.inner.lock().await;
            let state = guard.sessions.entry(key.clone()).or_default();
            state.queue.clear();
            state.watchers()
        };
        self.push_queue(&key, &watchers, Vec::new()).await;
        Ok(result)
    }

    pub(crate) async fn close_or_delete(
        &self,
        method: &str,
        params: Value,
    ) -> Result<Value, RpcError> {
        let key = self.key_of(&params)?;
        let session_id = key.1.clone();
        let result = self.call(&key.0, method, without_instance(params.clone())).await?;
        // A dry run only asks what would go.
        if params.get("dry_run").and_then(Value::as_bool) == Some(true) {
            return Ok(result);
        }
        self.inner.lock().await.sessions.remove(&key);
        if method == "session.delete" {
            self.unbind_space(&key);
        }
        // The recorder keeps no retention policy and says so — a recording is
        // a self-contained directory for whoever owns the session to remove,
        // and deleting the session is that moment. Closing is not: the
        // conversation can be reopened, and what the model saw should still
        // be there (§6.4).
        let recordings = self.slot(&key.0).and_then(|s| s.facts.recordings_root.clone());
        if let (true, Some(root)) = (method == "session.delete", recordings) {
            let dir = root.join(&session_id);
            if dir.is_dir() {
                if let Err(e) = std::fs::remove_dir_all(&dir) {
                    tracing::warn!(dir = %dir.display(), error = %e, "recording not removed");
                }
            }
        }
        Ok(result)
    }

    pub(crate) async fn queue_remove(self: &Arc<Self>, params: Value) -> Result<Value, RpcError> {
        let key = self.key_of(&params)?;
        let item_id = params
            .get("item_id")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let (watchers, snapshot) = {
            let mut guard = self.inner.lock().await;
            let state = guard.sessions.entry(key.clone()).or_default();
            state.queue.retain(|i| i.item_id != item_id);
            (state.watchers(), state.queue_view())
        };
        self.push_queue(&key, &watchers, snapshot.clone())
            .await;
        Ok(json!({"items": snapshot}))
    }

    /// The queue is pushed **whole**, never as a delta. A merge bug only shows
    /// up in one order of arrival, and that order is not one tests reliably
    /// produce (§3.2).
    async fn push_queue(&self, key: &SessionKey, watchers: &[crate::ClientId], items: Vec<Value>) {
        let (instance, session_id) = (&key.0, key.1.as_str());
        self.fan_out(
            watchers,
            Frame::session(
                session_id,
                notification(
                    "harnix.queue",
                    json!({"instance": instance, "session_id": session_id, "items": items}),
                ),
            ),
        )
        .await;
    }
}
