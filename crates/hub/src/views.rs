//! The instance tree as whole values (design §6.3).
//!
//! Four of the hub's own topics, each also answered once by a method of the
//! same shape: every instance; one instance's top-level sessions; one
//! session's tasks; and what every session is doing right now.
//!
//! A value is computed when somebody subscribes and again when something it
//! reads changes, at most once per short window however many changes arrive
//! — a turn's lifecycle touches three of these at once. Nobody subscribed,
//! nothing is computed.

use std::sync::Arc;
use std::time::Duration;

use harnix_contract::RpcError;
use serde_json::{json, Value};

use crate::{Hub, SessionKey};

/// How long changes are gathered before a value is recomputed.
const COALESCE: Duration = Duration::from_millis(40);

impl Hub {
    /// Recompute a hub topic soon, if anyone is subscribed to it.
    pub(crate) fn refresh(self: &Arc<Self>, topic: &str, params: Value) {
        let marker = (topic.to_string(), params.to_string());
        {
            let mut pending = self.refreshing.lock().unwrap_or_else(|e| e.into_inner());
            if !pending.insert(marker.clone()) {
                return;
            }
        }
        let hub = self.clone();
        let topic = topic.to_string();
        tokio::spawn(async move {
            tokio::time::sleep(COALESCE).await;
            hub.refreshing
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(&marker);
            let watched = {
                let guard = hub.inner.lock().await;
                guard
                    .topics
                    .key(&topic, &params)
                    .is_ok_and(|key| guard.topics.has_subscribers(&key))
            };
            if !watched {
                return;
            }
            match hub.compute(&topic, &params).await {
                Ok(value) => hub.publish(&topic, params, value).await,
                Err(e) => tracing::debug!(topic, error = %e.message, "topic not recomputed"),
            }
        });
    }

    /// The current value of one of the hub's topics; `None` for a topic
    /// somebody else publishes.
    pub(crate) async fn compute(&self, topic: &str, params: &Value) -> Result<Value, RpcError> {
        match topic {
            "harnix.instances" => Ok(self.instances_view()),
            "harnix.instances.sessions" => {
                let instance = self.instance_of(params)?;
                self.sessions_view(&instance).await
            }
            "harnix.sessions.tasks" => {
                let key = self.key_of(params)?;
                self.tasks_view(&key).await
            }
            "harnix.sessions.status" => Ok(self.status_view().await),
            _ => Err(RpcError::not_found(format!("`{topic}` is not the hub's"))),
        }
    }

    pub(crate) fn instances_view(&self) -> Value {
        let list: Vec<Value> = self.slots().iter().map(|s| self.instance_view(s)).collect();
        json!({"instances": list})
    }

    /// One instance's top-level sessions — sub-agents are their parent's
    /// tasks and are not listed here — with what each is doing.
    pub(crate) async fn sessions_view(&self, instance: &str) -> Result<Value, RpcError> {
        let listed = self.call(instance, "session.list", json!({})).await?;
        let rows: Vec<Value> = listed
            .get("sessions")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let guard = self.inner.lock().await;
        let rows: Vec<Value> = rows
            .into_iter()
            .map(|mut row| {
                let sid = row
                    .get("session_id")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let state = guard.sessions.get(&(instance.to_string(), sid.clone()));
                let running = state.is_some_and(|s| s.running_turn.is_some());
                let queued = state.map(|s| s.queue.len()).unwrap_or(0);
                let space = self.space_of(&(instance.to_string(), sid.clone()));
                if let Some(obj) = row.as_object_mut() {
                    obj.insert("instance".into(), json!(instance));
                    obj.insert("space".into(), json!(space));
                    obj.insert("running".into(), json!(running));
                    obj.insert("queued".into(), json!(queued));
                    obj.insert(
                        "state".into(),
                        json!(match (running, queued) {
                            (false, _) => "idle",
                            (true, 0) => "running",
                            (true, _) => "queued",
                        }),
                    );
                }
                row
            })
            .collect();
        Ok(json!({"instance": instance, "sessions": rows}))
    }

    /// A session's tasks, as Agix reports them. Watching them keeps the hub
    /// subscribed to the session, which is how it hears a task start or end.
    pub(crate) async fn tasks_view(&self, key: &SessionKey) -> Result<Value, RpcError> {
        let result = self
            .call(&key.0, "session.tasks", json!({"session_id": key.1}))
            .await;
        let mut value = match result {
            Ok(v) => v,
            // Not in memory: nothing of it is running.
            Err(e) if e.code == daemon::rpc::codes::SESSION_NOT_FOUND => {
                json!({"active": false, "tasks": []})
            }
            Err(e) => return Err(e),
        };
        if let Some(obj) = value.as_object_mut() {
            obj.insert("instance".into(), json!(key.0));
            obj.insert("session_id".into(), json!(key.1));
        }
        Ok(value)
    }

    /// Every session that is not idle, across instances (IPDrun R-11): what
    /// a session list needs to draw a spinner without asking each one.
    pub(crate) async fn status_view(&self) -> Value {
        let guard = self.inner.lock().await;
        let mut rows: Vec<Value> = guard
            .sessions
            .iter()
            .filter(|(_, s)| s.running_turn.is_some() || !s.queue.is_empty())
            .map(|((instance, session_id), s)| {
                json!({
                    "instance": instance,
                    "session_id": session_id,
                    "state": if s.running_turn.is_some() { "running" } else { "queued" },
                    "turn_id": s.running_turn,
                    "queued": s.queue.len(),
                    "external": s.external,
                })
            })
            .collect();
        rows.sort_by_key(|row| row.to_string());
        json!({"sessions": rows})
    }
}
