//! What an instance tells the hub without being asked, and how the hub
//! recovers when it stops being told (concept §3.1.1).
//!
//! One router per instance reads the link's event stream in order. Session
//! frames go to the session's buffer and watchers; `daemon.event` updates
//! what the hub knows about turns it did not start (a scheduled turn says
//! `initiator: "schedule"`) and goes to every client as a host event.
//!
//! Two things make the hub's view of a session stale, and both end in the
//! same resync: the link reconnected (every subscription belonged to the old
//! connection), or the engine dropped one subscription because the hub fell
//! behind. Resync is the path a refreshing client already takes —
//! subscribe for the watermark, read up to it, take live frames after — and
//! ends by telling the session's watchers to attach again.

use std::sync::Arc;

use harnix_contract::{Frame, RpcError};
use harnix_engine_link::events::{DaemonEventKind, EngineEvent, SessionEventKind};
use harnix_engine_link::EventReceiver;
use serde_json::{json, Value};

use crate::{notification, Hub, SessionKey};

impl Hub {
    pub(crate) async fn route(self: Arc<Self>, instance: String, mut events: EventReceiver) {
        while let Some(event) = events.recv().await {
            match event {
                EngineEvent::Session { session, turn_id, kind, event } => {
                    self.session_event(&instance, session, turn_id, kind, event).await
                }
                EngineEvent::Daemon { kind, event } => self.daemon_event(&instance, kind, event).await,
                EngineEvent::Barrier { id } => self.barrier_reached(id),
                EngineEvent::Reconnected => {
                    tracing::info!(instance, "instance reconnected; resyncing its sessions");
                    tokio::spawn(self.clone().resync_instance(instance.clone()));
                }
                EngineEvent::SubscriptionDropped { session } => {
                    tracing::warn!(instance, session, "the engine dropped the hub's subscription; resyncing");
                    let (hub, key) = (self.clone(), (instance.clone(), session));
                    tokio::spawn(async move { hub.resync_session(key).await });
                }
                other => tracing::debug!(event = ?other, instance, "unrouted engine event"),
            }
        }
    }

    async fn session_event(
        self: &Arc<Self>,
        instance: &str,
        session_id: String,
        turn_id: String,
        kind: SessionEventKind,
        event: Value,
    ) {
        let ends_turn = matches!(kind, SessionEventKind::TurnComplete(_));
        if !self.bounded(instance, &session_id, event.to_string().len(), ends_turn) {
            return;
        }
        let key: SessionKey = (instance.to_string(), session_id.clone());
        let tasks = || json!({"instance": instance, "session_id": session_id});
        // What a session waits on is on its row.
        let relisted = matches!(
            kind,
            SessionEventKind::Asked(_)
                | SessionEventKind::AskResolved { .. }
                | SessionEventKind::ToolResult { .. }
                | SessionEventKind::TurnComplete(_)
                | SessionEventKind::GoalChanged { .. }
        );
        let task = match &kind {
            SessionEventKind::Task { task_id, started, session, live } => {
                // A sub-agent's session is the engine's making: it is its parent's space's.
                if let (true, Some(child)) = (*started, session) {
                    self.child_session(&key, child);
                }
                Some((task_id.clone(), *live))
            }
            SessionEventKind::TaskChanged { task_id, live } => Some((task_id.clone(), *live)),
            _ => None,
        };
        let teams = matches!(kind, SessionEventKind::TeamProgress { .. });
        crate::session::route_event(&self.inner, instance, &session_id, &turn_id, &kind, event, self.is_extension(instance)).await;
        if relisted {
            self.refresh("harnix.instances.sessions", json!({"instance": instance}));
        }
        if teams {
            self.refresh("harnix.session.tasks", tasks());
        }
        if let Some((task_id, live_now)) = task {
            {
                let mut guard = self.inner.lock().await;
                let live = &mut guard.sessions.entry(key.clone()).or_default().live_tasks;
                if live_now {
                    live.insert(task_id);
                } else {
                    live.remove(&task_id);
                }
            }
            self.refresh("harnix.session.tasks", tasks());
            self.refresh("harnix.instances.sessions", json!({"instance": instance}));
        }
    }

    async fn daemon_event(self: &Arc<Self>, instance: &str, kind: DaemonEventKind, mut event: Value) {
        // An extension's engine-wide events are the hub's own account of its turns, and nobody
        // else's: what it says of itself is not broadcast to every client. That a turn of one of
        // its sessions started or ended is, as for the product's engines — rebuilt by the hub,
        // marked as an extension's, and within its engine's bounds.
        let extension = self.is_extension(instance);
        let told = if extension { extension_turn(&kind, &event) } else { None };
        let ends_turn = matches!(kind, DaemonEventKind::TurnCompleted { .. });
        match kind {
            DaemonEventKind::TurnStarted { session, turn_id } => {
                self.turn_started_elsewhere(instance, &session, &turn_id).await;
            }
            DaemonEventKind::TurnCompleted { session, turn_id, initiator } => {
                self.turn_completed_elsewhere(instance, &session, &turn_id, initiator).await;
            }
            DaemonEventKind::SessionsChanged { session, ended } => {
                // An engine that is gone runs no tasks, and says nothing more of those it ran.
                if let (Some(sid), true) = (session, ended) {
                    let key: SessionKey = (instance.to_string(), sid);
                    if let Some(state) = self.inner.lock().await.sessions.get_mut(&key) {
                        state.live_tasks.clear();
                    }
                }
                self.refresh("harnix.instances.sessions", json!({"instance": instance}));
                self.refresh("harnix.instances", json!({}));
            }
            DaemonEventKind::Lagged => {
                tokio::spawn(self.clone().reconcile(instance.to_string()));
            }
            DaemonEventKind::McpServer { ref server, failed } if !extension => {
                let key = (instance.to_string(), server.clone());
                let mut told = event.clone();
                if let Some(obj) = told.as_object_mut() {
                    obj.insert("instance".into(), json!(instance));
                }
                let mut guard = self.inner.lock().await;
                if failed {
                    guard.failed_servers.insert(key, told);
                } else {
                    guard.failed_servers.remove(&key);
                }
            }
            _ => {}
        }
        if extension {
            match told {
                Some((session, told)) if self.bounded(instance, &session, told.to_string().len(), ends_turn) => event = told,
                _ => return,
            }
        }
        // Which instance said it: a host event about a session is about
        // `(instance, session)` like every other frame.
        if let Some(obj) = event.as_object_mut() {
            obj.insert("instance".into(), json!(instance));
        }
        let out = Frame::host(notification("harnix.host_event", event));
        let guard = self.inner.lock().await;
        let out = guard.clients.to_all(out);
        crate::send_after(guard, [out]).await;
    }

    /// A turn the hub did not start — a scheduled one, or one the hub lost
    /// track of — is a running turn like any other: sends queue behind it,
    /// and its end settles it.
    async fn turn_started_elsewhere(self: &Arc<Self>, instance: &str, session_id: &str, turn_id: &str) {
        let key: SessionKey = (instance.to_string(), session_id.to_string());
        {
            let mut guard = self.inner.lock().await;
            let state = guard.sessions.entry(key.clone()).or_default();
            if state.running_turn.as_deref() == Some(turn_id) {
                return;
            }
            state.running_turn = Some(turn_id.to_string());
            // On behalf of no call this process made.
            self.turns.start(instance, session_id, Default::default());
            state.turn_started_ms = Some(crate::now_ms());
            state.external = true;
            state.compacted = false;
            // What it says, calls and writes is its own, not the turn's before it.
            state.answer = Default::default();
            state.replay.clear();
        }
        self.refresh("harnix.sessions.status", json!({}));
        self.refresh("harnix.instances.sessions", json!({"instance": instance}));
        // Where history ends for an attach, read off the router so the
        // router never waits on its own link.
        let hub = self.clone();
        let turn = turn_id.to_string();
        tokio::spawn(async move {
            let total = hub.history_total(&key).await;
            let compactions = hub.compactions(&key).await;
            let mut guard = hub.inner.lock().await;
            if let Some(state) = guard.sessions.get_mut(&key) {
                if state.running_turn.as_deref() == Some(turn.as_str()) {
                    state.history_total_before_turn = total;
                    state.compactions_before_turn = compactions;
                }
            }
        });
    }

    async fn turn_completed_elsewhere(
        self: &Arc<Self>,
        instance: &str,
        session_id: &str,
        turn_id: &str,
        initiator: Value,
    ) {
        let key: SessionKey = (instance.to_string(), session_id.to_string());
        let ours_to_settle = {
            let guard = self.inner.lock().await;
            guard.sessions.get(&key).is_some_and(|s| {
                s.external && s.running_turn.as_deref() == Some(turn_id)
            })
        };
        if ours_to_settle {
            let outcome = Ok(json!({"initiator": initiator}));
            let hub = self.clone();
            let turn = turn_id.to_string();
            tokio::spawn(async move { hub.settle_external(key, turn, outcome).await });
        }
    }

    /// Settle a turn no loop of ours is driving, and start what was queued
    /// behind it.
    pub(crate) async fn settle_external(
        self: Arc<Self>,
        key: SessionKey,
        turn_id: String,
        outcome: Result<Value, RpcError>,
    ) {
        self.barrier(&key.0).await;
        if let Some(next) = self.settle_turn(&key, &turn_id, outcome).await {
            if let Err(e) = self.start_turn(&key, next).await {
                tracing::warn!(error = %e.message, "queued send not started");
            }
        }
    }

    /// The link came back: every subscription it had is gone.
    pub(crate) async fn resync_instance(self: Arc<Self>, instance: String) {
        let keys: Vec<SessionKey> = {
            let mut guard = self.inner.lock().await;
            guard
                .sessions
                .iter_mut()
                .filter(|(key, s)| {
                    key.0 == instance
                        && (s.subscribed || s.running_turn.is_some() || !s.queue.is_empty())
                })
                .map(|(key, s)| {
                    s.subscribed = false;
                    key.clone()
                })
                .collect()
        };
        for key in keys {
            self.resync_session(key).await;
        }
        self.refresh("harnix.instances", json!({}));
        self.refresh("harnix.instances.sessions", json!({"instance": instance}));
    }

    /// Bring one session back in step with its engine, then tell its
    /// watchers to attach again.
    pub(crate) async fn resync_session(self: &Arc<Self>, key: SessionKey) {
        let (instance, session_id) = (key.0.clone(), key.1.clone());
        let got = match self.engine(&instance) {
            Ok(linked) => linked.ops().get(&session_id).await,
            Err(e) => Err(e),
        };
        let info = match got {
            Ok(info) => info,
            Err(e) => {
                tracing::warn!(instance, session_id, error = %e.message, "resync: engine.session.get failed");
                return;
            }
        };
        // Cleared before subscribing again: what arrives after the subscribe
        // is past the watermark it returns, and belongs in the buffer.
        let had_watchers = {
            let mut guard = self.inner.lock().await;
            let state = guard.sessions.entry(key.clone()).or_default();
            state.subscribed = false;
            state.replay.clear();
            !state.watchers.is_empty()
        };
        // Only sessions somebody was watching are subscribed again; the rest
        // only needed their turn state checked.
        let watermark = if had_watchers {
            match self.ensure_subscribed(&key, &info).await {
                Ok(last_seq) => last_seq,
                Err(e) => {
                    tracing::warn!(instance, session_id, error = %e.message, "resync: not subscribed");
                    None
                }
            }
        } else {
            None
        };
        let total = match watermark {
            Some(seq) => seq,
            None => self.history_total(&key).await,
        };
        // Task events missed while the subscription was gone: read what is live now. A session
        // not subscribed again hears of no task ending, so it lists none.
        let live_tasks = if had_watchers { self.live_tasks(&key).await } else { Default::default() };
        let running = info.running();
        let held = info.held();
        let current = info.current_turn().map(str::to_string);
        let (watchers, settle, start_next) = {
            let mut guard = self.inner.lock().await;
            let state = guard.sessions.entry(key.clone()).or_default();
            state.history_total_before_turn = total;
            state.compactions_before_turn = info.compactions();
            state.live_tasks = live_tasks;
            let mut settle = None;
            if running {
                if state.running_turn != current {
                    state.running_turn = current;
                    state.external = true;
                }
            } else if held {
                // Neither running nor free: a turn of the hub's is not over for that.
            } else if let Some(turn) = state.running_turn.clone() {
                // Finished while nobody was listening. There is no result to
                // report; the transcript has it.
                settle = Some(turn);
            }
            let start_next = !running && !held && settle.is_none() && !state.queue.is_empty();
            (state.watchers(), settle, start_next)
        };
        self.fan_out(
            &watchers,
            Frame::session(
                &session_id,
                notification(
                    "harnix.session_resync",
                    json!({"instance": instance, "session_id": session_id}),
                ),
            ),
        )
        .await;
        if let Some(turn) = settle {
            self.clone()
                .settle_external(key, turn, Ok(json!({"recovered": true})))
                .await;
        } else if start_next {
            let next = {
                let mut guard = self.inner.lock().await;
                guard.sessions.get_mut(&key).and_then(|s| s.queue.pop_front())
            };
            if let Some(next) = next {
                if let Err(e) = self.start_turn(&key, next).await {
                    tracing::warn!(error = %e.message, "queued send not started after resync");
                }
            }
        }
        self.refresh("harnix.sessions.status", json!({}));
    }

    /// Host events were dropped: check every turn the hub thinks is running
    /// elsewhere against the engine.
    async fn reconcile(self: Arc<Self>, instance: String) {
        let keys: Vec<SessionKey> = {
            let guard = self.inner.lock().await;
            guard
                .sessions
                .iter()
                .filter(|(key, s)| key.0 == instance && s.external && s.running_turn.is_some())
                .map(|(key, _)| key.clone())
                .collect()
        };
        for key in keys {
            let idle = match self.engine(&key.0) {
                Ok(linked) => linked.ops().get(&key.1).await.is_ok_and(|info| info.idle()),
                Err(_) => false,
            };
            if idle {
                let turn = {
                    let guard = self.inner.lock().await;
                    guard.sessions.get(&key).and_then(|s| s.running_turn.clone())
                };
                if let Some(turn) = turn {
                    self.clone()
                        .settle_external(key, turn, Ok(json!({"recovered": true})))
                        .await;
                }
            }
        }
        self.refresh("harnix.instances.sessions", json!({"instance": instance}));
    }
}

/// A turn of an extension's engine starting or ending, as the hub tells every client of it: only
/// what the hub reads off the event, never the event itself. `None` for anything else it says.
fn extension_turn(kind: &DaemonEventKind, event: &Value) -> Option<(String, Value)> {
    let name = kind.told_as()?;
    let (session, turn_id) = match kind {
        DaemonEventKind::TurnStarted { session, turn_id } => (session, turn_id),
        DaemonEventKind::TurnCompleted { session, turn_id, .. } => (session, turn_id),
        _ => return None,
    };
    // Who started it, when it is one of the engine's own words for that.
    let initiator = event
        .get("initiator")
        .and_then(Value::as_str)
        .filter(|i| !i.is_empty() && i.len() <= 32 && i.bytes().all(|b| b.is_ascii_lowercase() || b == b'_'));
    let told = json!({
        "kind": name,
        "session_id": session,
        "turn_id": turn_id,
        "initiator": initiator,
        "trust": "extension",
    });
    Some((session.clone(), told))
}
