//! What an instance tells the hub without being asked, and how the hub
//! recovers when it stops being told (design §5.7).
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
use harnix_instance::link::{BARRIER, RECONNECTED};
use harnix_instance::link::EventReceiver;
use serde_json::{json, Value};

use crate::{notification, Hub, SessionKey};

impl Hub {
    pub(crate) async fn route(self: Arc<Self>, instance: String, mut events: EventReceiver) {
        while let Some(frame) = events.recv().await {
            let params = frame.get("params").cloned().unwrap_or(Value::Null);
            match frame.get("method").and_then(Value::as_str) {
                Some("session.event") => self.session_event(&instance, params).await,
                Some("daemon.event") => self.daemon_event(&instance, params).await,
                Some(BARRIER) => {
                    if let Some(id) = params.get("id").and_then(Value::as_u64) {
                        self.barrier_reached(id);
                    }
                }
                Some(RECONNECTED) => {
                    tracing::info!(instance, "instance reconnected; resyncing its sessions");
                    tokio::spawn(self.clone().resync_instance(instance.clone()));
                }
                other => tracing::debug!(method = ?other, instance, "unrouted engine frame"),
            }
        }
    }

    async fn session_event(self: &Arc<Self>, instance: &str, params: Value) {
        let kind = params
            .get("event")
            .and_then(|e| e.get("kind"))
            .and_then(Value::as_str)
            .unwrap_or("");
        let Some(session_id) = params.get("session_id").and_then(Value::as_str) else {
            return;
        };
        let key: SessionKey = (instance.to_string(), session_id.to_string());
        if kind == "subscription_dropped" {
            tracing::warn!(instance, session_id, "the engine dropped the hub's subscription; resyncing");
            let hub = self.clone();
            tokio::spawn(async move { hub.resync_session(key).await });
            return;
        }
        crate::session::route_event(&self.inner, instance, params.clone()).await;
        if matches!(kind, "task_started" | "task_finished") {
            self.refresh(
                "harnix.sessions.tasks",
                json!({"instance": instance, "session_id": session_id}),
            );
            self.refresh("harnix.instances.sessions", json!({"instance": instance}));
        }
    }

    async fn daemon_event(self: &Arc<Self>, instance: &str, mut params: Value) {
        let kind = params.get("kind").and_then(Value::as_str).unwrap_or("").to_string();
        let session_id = params
            .get("session_id")
            .and_then(Value::as_str)
            .map(str::to_string);
        match (kind.as_str(), session_id) {
            ("turn_started", Some(sid)) => {
                let turn_id = params.get("turn_id").and_then(Value::as_str).unwrap_or("");
                self.turn_started_elsewhere(instance, &sid, turn_id).await;
            }
            ("turn_completed", Some(sid)) => {
                let turn_id = params.get("turn_id").and_then(Value::as_str).unwrap_or("");
                self.turn_completed_elsewhere(instance, &sid, turn_id, &params).await;
            }
            (
                "session_created" | "session_resumed" | "session_evicted" | "session_closed"
                | "session_deleted",
                _,
            ) => {
                self.refresh("harnix.instances.sessions", json!({"instance": instance}));
                self.refresh("harnix.instances", json!({}));
            }
            ("events_lagged", _) => {
                tokio::spawn(self.clone().reconcile(instance.to_string()));
            }
            _ => {}
        }
        // Which instance said it: a host event about a session is about
        // `(instance, session)` like every other frame.
        if let Some(obj) = params.as_object_mut() {
            obj.insert("instance".into(), json!(instance));
        }
        let out = Frame::host(notification("harnix.host_event", params));
        let guard = self.inner.lock().await;
        guard.clients.broadcast(&out).await;
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
            state.external = true;
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
            let mut guard = hub.inner.lock().await;
            if let Some(state) = guard.sessions.get_mut(&key) {
                if state.running_turn.as_deref() == Some(turn.as_str()) {
                    state.history_total_before_turn = total;
                }
            }
        });
    }

    async fn turn_completed_elsewhere(
        self: &Arc<Self>,
        instance: &str,
        session_id: &str,
        turn_id: &str,
        params: &Value,
    ) {
        let key: SessionKey = (instance.to_string(), session_id.to_string());
        let ours_to_settle = {
            let guard = self.inner.lock().await;
            guard.sessions.get(&key).is_some_and(|s| {
                s.external && s.running_turn.as_deref() == Some(turn_id)
            })
        };
        if ours_to_settle {
            let outcome = Ok(json!({"initiator": params.get("initiator")}));
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
        let info = match self
            .call(&instance, "session.get", json!({"session_id": session_id}))
            .await
        {
            Ok(info) => info,
            Err(e) => {
                tracing::warn!(instance, session_id, error = %e.message, "resync: session.get failed");
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
        let running = info.get("turn_state").and_then(Value::as_str) == Some("running");
        let current = info
            .get("current_turn_id")
            .and_then(Value::as_str)
            .map(str::to_string);
        let (watchers, settle, start_next) = {
            let mut guard = self.inner.lock().await;
            let state = guard.sessions.entry(key.clone()).or_default();
            state.history_total_before_turn = total;
            let mut settle = None;
            if running {
                if state.running_turn != current {
                    state.running_turn = current;
                    state.external = true;
                }
            } else if let Some(turn) = state.running_turn.clone() {
                // Finished while nobody was listening. There is no result to
                // report; the transcript has it.
                settle = Some(turn);
            }
            let start_next = !running && settle.is_none() && !state.queue.is_empty();
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
            let idle = self
                .call(&key.0, "session.get", json!({"session_id": key.1}))
                .await
                .ok()
                .is_some_and(|info| info.get("turn_state").and_then(Value::as_str) == Some("idle"));
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
