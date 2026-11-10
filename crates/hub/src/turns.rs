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
//! not reach the user unchanged, so the queue is here (concept §3.2, point 4).

use std::sync::Arc;

use harnix_contract::{Frame, RpcError};
use harnix_engine_api::events::Completion;
use harnix_engine_api::ops::{codes as engine_codes, TurnInput};
use serde_json::{json, Value};

use crate::session::{QueueItem, USER_MESSAGE};
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
        cx: &harnix_kernel::CallContext,
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
        // Sending implies watching. A client that skipped `harnix.session.attach` would
        // otherwise get neither the turn's events nor its settlement — frames
        // go to watchers, and it would not be one.
        if let Some(client) = cx.client {
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
        let linked = self.engine(&key.0)?;
        let ready = linked.link.status().borrow().is_ready();
        if !ready {
            if on_busy == OnBusy::Reject {
                let status = linked.link.status().borrow().clone();
                return Err(status.unavailable());
            }
            return Ok(self.enqueue(&key, item, false).await);
        }

        // The session has to exist before a turn is opened on it.
        //
        // Running a turn does not require one — handed an id it does not
        // know, the engine makes a session and runs the turn there. That is
        // reasonable for the engine and wrong for a client: a mistyped id
        // came back as a settled turn, with the transcript in a session under
        // an id the caller had never seen and could not ask for again, and a
        // model call spent on it. The hub owns turns (concept §3.2), so refusing here
        // is the hub's to do.
        let info = linked.ops().get(&session_id).await?;
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
                    linked.ops().interrupt(&session_id).await?;
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
        let linked = self.engine(&key.0)?;
        let (session, input) = (key.1.clone(), TurnInput::steer(turn_id.clone(), item.message));
        let call = tokio::spawn(async move { linked.ops().run_turn(&session, input).await });
        match tokio::time::timeout(std::time::Duration::from_secs(5), call).await {
            Ok(Ok(result)) => result,
            Ok(Err(e)) => Err(RpcError::new(harnix_contract::codes::INTERNAL_ERROR, e.to_string())),
            Err(_) => Ok(json!({"turn_id": turn_id, "delivery": "queued_by_engine"})),
        }
    }

    /// Begin a turn and hand it to a task, so `harnix.session.send` returns at once and
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
                let outcome = match hub.engine(&key.0) {
                    Ok(linked) => {
                        let input = TurnInput::new(turn_id.clone(), item.message.clone(), item.attachments.clone());
                        linked.ops().run_turn(&key.1, input).await
                    }
                    Err(e) => Err(e),
                };
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
        let compactions = self.compactions(key).await;
        let turn_id = new_id("t");

        let (seq, event, watchers) = {
            let mut guard = self.inner.lock().await;
            let state = guard.sessions.entry(key.clone()).or_default();
            state.running_turn = Some(turn_id.clone());
            state.turn_started_ms = Some(crate::now_ms());
            state.external = false;
            state.last_complete = None;
            state.answer = Default::default();
            state.history_total_before_turn = total;
            state.compactions_before_turn = compactions;
            state.compacted = false;
            state.replay.clear();
            state.seq += 1;
            let seq = state.seq;
            let event = json!({"kind": USER_MESSAGE, "text": item.message});
            state.replay.push(seq, &turn_id, &harnix_engine_api::SessionEventKind::Other, &event);
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
        let compactions_after = self.compactions(key).await;
        let (watchers, next, complete) = {
            let mut guard = self.inner.lock().await;
            let state = guard.sessions.entry(key.clone()).or_default();
            if state.running_turn.as_deref() != Some(turn_id) {
                // Settled already, by whichever of the loop, `turn_completed`
                // and a resync got here first.
                return None;
            }
            state.running_turn = None;
            state.turn_started_ms = None;
            state.external = false;
            state.pending_prompts.clear();
            // Only once the transcript has grown — or been compacted, which
            // writes it shorter. A turn that finished but is not yet on disk
            // would otherwise be in neither place.
            if total_after > state.history_total_before_turn
                || compactions_after != state.compactions_before_turn
                || state.compacted
            {
                state.replay.clear();
            }
            state.compacted = false;
            let answer = std::mem::take(&mut state.answer);
            (state.watchers(), state.queue.pop_front(), state.last_complete.take().map(|c| (c, answer)))
        };

        let mut params = match &outcome {
            Ok(result) => json!({"instance": instance, "session_id": session_id, "turn_id": turn_id, "result": result}),
            Err(error) => json!({
                "instance": instance, "session_id": session_id, "turn_id": turn_id,
                "error": {"code": error.code, "message": error.message},
            }),
        };
        // How the turn ended, as the engine said when it completed: here too, so a watcher that
        // missed that frame (a tap that fell behind) still has it (ipdrun-17).
        if let Some((complete, answer)) = complete {
            params["outcome"] = outcome_of(&complete, &answer);
        }
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
        let result = self.engine(&key.0)?.ops().interrupt(&key.1).await?;
        let watchers = {
            let mut guard = self.inner.lock().await;
            let state = guard.sessions.entry(key.clone()).or_default();
            state.queue.clear();
            state.watchers()
        };
        self.push_queue(&key, &watchers, Vec::new()).await;
        Ok(result)
    }

    /// `harnix.session.rename {instance?, session_id, title}`: the session's title is the
    /// engine's name for it, so every client lists the same one; nothing here keeps a title of
    /// its own.
    pub(crate) async fn rename(self: &Arc<Self>, params: Value) -> Result<Value, RpcError> {
        let key = self.key_of(&params)?;
        let title = params.get("title").and_then(Value::as_str).unwrap_or("").trim().to_string();
        if title.is_empty() {
            return Err(RpcError::invalid_params("a title is not empty"));
        }
        self.engine(&key.0)?.ops().rename(&key.1, &title).await?;
        self.refresh("harnix.instances.sessions", json!({"instance": key.0}));
        Ok(json!({"session_id": key.1, "title": title}))
    }

    /// `harnix.session.setModel {instance?, session_id, model, provider?, space?}`: the model of a
    /// session of the space the caller claims (the default space when it claims none). Never
    /// across spaces: a space reaching across is not where this is done.
    pub(crate) async fn set_model(self: &Arc<Self>, params: Value) -> Result<Value, RpcError> {
        let key = self.key_of(&params)?;
        let model = params
            .get("model")
            .and_then(Value::as_str)
            .filter(|m| !m.is_empty())
            .ok_or_else(|| RpcError::invalid_params("missing model"))?
            .to_string();
        let provider = params.get("provider").and_then(Value::as_str).map(str::to_string);
        let (claimed, crosses, bound) = {
            let spaces = harnix_contract::sync::lock(&self.spaces);
            let claimed = spaces.claimed_space(&params);
            let crosses = spaces.crosses(&claimed);
            (claimed, crosses, spaces.bound(&key).to_string())
        };
        if crosses {
            return Err(RpcError::new(
                harnix_contract::codes::SPACE_MISMATCH,
                format!("`{claimed}` reaches across spaces; a session's model is changed within its own"),
            ));
        }
        if bound != claimed {
            return Err(RpcError {
                code: harnix_contract::codes::SPACE_MISMATCH,
                message: format!("session `{}` on instance `{}` belongs to the space `{bound}`, not `{claimed}`", key.1, key.0),
                data: Some(json!({"space": bound, "instance": key.0, "session_id": key.1})),
            });
        }
        self.engine(&key.0)?.ops().set_model(&key.1, &model, provider.as_deref()).await
    }

    /// Close a session, or `delete` it. Deleting it deletes its recording too, which is the link's
    /// to do.
    pub(crate) async fn close_or_delete(
        self: &Arc<Self>,
        params: Value,
        delete: bool,
    ) -> Result<Value, RpcError> {
        let key = self.key_of(&params)?;
        let result = self.engine(&key.0)?.ops().close(without_instance(params.clone()), delete).await?;
        // A dry run only asks what would go.
        if params.get("dry_run").and_then(Value::as_bool) == Some(true) {
            return Ok(result);
        }
        self.inner.lock().await.sessions.remove(&key);
        if delete {
            self.unbind_space(&key);
        }
        // The engine's own events about it came while it was still being taken down: a view
        // computed for them may have listed it still. Now that the engine has answered, and what
        // the hub kept of it is gone, the views are computed once more.
        self.refresh("harnix.instances.sessions", json!({"instance": key.0}));
        self.refresh("harnix.instances", json!({}));
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

    /// `harnix.session.queue.edit {session_id, item_id, message}`: change what a queued message says before
    /// it runs. Its place and attachments stay. One that has started (or was withdrawn) is no longer
    /// in the queue, and is refused rather than quietly sent as it was.
    pub(crate) async fn queue_edit(self: &Arc<Self>, params: Value) -> Result<Value, RpcError> {
        let key = self.key_of(&params)?;
        let item_id = params
            .get("item_id")
            .and_then(Value::as_str)
            .ok_or_else(|| RpcError::invalid_params("missing item_id"))?
            .to_string();
        let message = params
            .get("message")
            .and_then(Value::as_str)
            .filter(|m| !m.trim().is_empty())
            .ok_or_else(|| RpcError::invalid_params("missing message: to take it out of the queue, harnix.session.queue.remove"))?
            .to_string();
        let (watchers, snapshot) = {
            let mut guard = self.inner.lock().await;
            let state = guard.sessions.entry(key.clone()).or_default();
            let Some(item) = state.queue.iter_mut().find(|i| i.item_id == item_id) else {
                return Err(RpcError::new(
                    harnix_contract::codes::REFUSED,
                    format!("`{item_id}` is not waiting in this session's queue: it has started or was withdrawn"),
                ));
            };
            item.message = message;
            (state.watchers(), state.queue_view())
        };
        self.push_queue(&key, &watchers, snapshot.clone()).await;
        Ok(json!({"items": snapshot}))
    }

    /// The queue is pushed **whole**, never as a delta. A merge bug only shows
    /// up in one order of arrival, and that order is not one tests reliably
    /// produce (concept §3.2).
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

/// What `harnix.turn_settled` carries as `outcome`: the points of how the engine said the turn
/// ended that a caller acts on, and what the turn said and called (ipdrun-41) — the answer's text
/// up to [`ANSWER_TEXT_MAX`](crate::session::ANSWER_TEXT_MAX) (`text_truncated` past it; the whole
/// of it is in the transcript), and each tool call's `id`, `name` and `is_error` once it has a
/// result.
fn outcome_of(complete: &Completion, answer: &crate::session::Answer) -> Value {
    let mut outcome = json!({
        "stop_reason": complete.stop_reason,
        "usage": complete.usage,
        "text": answer.text,
        "tool_calls": answer.tool_calls,
    });
    if answer.truncated {
        outcome["text_truncated"] = json!(true);
    }
    if let Some(output) = &complete.structured_output {
        outcome["structured_output"] = output.clone();
    }
    if let Some(message) = &complete.message {
        outcome["message"] = message.clone();
    }
    outcome
}

#[cfg(test)]
mod outcome_tests {
    use super::*;
    use harnix_engine_api::events::{Fragment, Joins, SessionEventKind};

    #[test]
    fn the_outcome_is_the_points_of_the_completion() {
        let complete = Completion::new(json!("stopped_by_hook"), json!({"input_tokens": 10, "output_tokens": 3}))
            .with_structured_output(Some(json!({"ok": true})))
            .with_message(Some(json!("the hook said no")));
        assert_eq!(
            outcome_of(&complete, &Default::default()),
            json!({
                "stop_reason": "stopped_by_hook",
                "usage": {"input_tokens": 10, "output_tokens": 3},
                "structured_output": {"ok": true},
                "message": "the hook said no",
                "text": "",
                "tool_calls": [],
            })
        );
        let plain = outcome_of(&Completion::new(json!("end_turn"), json!({})), &Default::default());
        assert!(plain.get("structured_output").is_none() && plain.get("message").is_none());
    }

    fn text(t: &str) -> SessionEventKind {
        SessionEventKind::Fragment(Fragment::new(Joins::Answer, "text", t))
    }

    #[test]
    fn the_outcome_has_what_the_turn_said_and_called() {
        let mut answer = crate::session::Answer::default();
        for kind in [
            text("Let me look. "),
            SessionEventKind::ToolCall { id: "tu1".into(), name: "Read".into() },
            SessionEventKind::ToolCall { id: "tu2".into(), name: "Bash".into() },
            SessionEventKind::ToolResult { id: "tu1".into(), is_error: false },
            SessionEventKind::ToolResult { id: "tu2".into(), is_error: true },
            SessionEventKind::Fragment(Fragment::new(Joins::Thinking, "text", "hmm")),
            text("Done."),
        ] {
            answer.take_event(&kind);
        }
        let outcome = outcome_of(&Completion::new(json!("end_turn"), json!({})), &answer);
        assert_eq!(outcome["text"], "Let me look. Done.");
        assert_eq!(
            outcome["tool_calls"],
            json!([
                {"id": "tu1", "name": "Read", "is_error": false},
                {"id": "tu2", "name": "Bash", "is_error": true},
            ])
        );
        assert!(outcome.get("text_truncated").is_none());
    }

    #[test]
    fn a_long_answer_is_cut_on_a_char_boundary_and_says_so() {
        let mut answer = crate::session::Answer::default();
        let chunk = "é".repeat(1000);
        while !answer.truncated {
            answer.take_event(&text(&chunk));
        }
        answer.take_event(&text("more"));
        assert!(answer.text.len() <= crate::session::ANSWER_TEXT_MAX);
        let outcome = outcome_of(&Completion::new(json!("end_turn"), json!({})), &answer);
        assert_eq!(outcome["text_truncated"], true);
    }
}
