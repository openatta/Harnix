//! The instance tree as whole values (concept §3.2).
//!
//! Four of the hub's own topics, each also answered once by a method of the
//! same shape: every instance; one instance's top-level sessions; one
//! session's tasks; and what every session is doing right now.
//!
//! A value is computed when somebody subscribes and again when something it
//! reads changes, at most once per short window however many changes arrive
//! — a turn's lifecycle touches three of these at once. Nobody subscribed,
//! nothing is computed.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use harnix_contract::sync::lock;
use harnix_contract::RpcError;
use serde_json::{json, Value};

use crate::{Hub, SessionKey};

/// How long changes are gathered before a value is recomputed.
const COALESCE: Duration = Duration::from_millis(40);

/// A topic value (topic, params) being recomputed.
pub(crate) type Marker = (String, String);

/// The recomputations of each topic value, so a burst of changes costs one and an older one
/// never lands after a newer one.
///
/// Two recomputations of one value can overlap: the second change arrives after the first's
/// window closed and while its computation (an RPC into the engine) is still out. The two then
/// race to publish, and the one that read the engine first could publish last — a deleted
/// session listed again, and nothing to correct it until the next change. So each computation is
/// numbered when it starts, and publishes only if no later one has started since.
#[derive(Default)]
pub(crate) struct Refreshes {
    by_value: HashMap<Marker, Refresh>,
}

#[derive(Default)]
struct Refresh {
    /// A recomputation is waiting out its window: another change joins it.
    waiting: bool,
    /// The number of the newest computation that has started.
    started: u64,
    /// How many have started and not finished.
    running: u32,
}

impl Refreshes {
    /// A change arrived. True when it opens a window; false when one is already open and the
    /// change joins it.
    pub(crate) fn want(&mut self, marker: &Marker) -> bool {
        let r = self.by_value.entry(marker.clone()).or_default();
        !std::mem::replace(&mut r.waiting, true)
    }

    /// The window closed and the computation starts: its number.
    pub(crate) fn start(&mut self, marker: &Marker) -> u64 {
        let r = self.by_value.entry(marker.clone()).or_default();
        r.waiting = false;
        r.started += 1;
        r.running += 1;
        r.started
    }

    /// The computation numbered `generation` is done: whether it may publish — no later one has
    /// started. A value nothing is recomputing any more is forgotten.
    pub(crate) fn finish(&mut self, marker: &Marker, generation: u64) -> bool {
        let Some(r) = self.by_value.get_mut(marker) else {
            return true;
        };
        let newest = r.started == generation;
        r.running = r.running.saturating_sub(1);
        if r.running == 0 && !r.waiting {
            self.by_value.remove(marker);
        }
        newest
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.by_value.len()
    }
}

impl Hub {
    /// Recompute a hub topic soon, if anyone is subscribed to it.
    pub(crate) fn refresh(self: &Arc<Self>, topic: &str, params: Value) {
        let marker: Marker = (topic.to_string(), params.to_string());
        if !lock(&self.refreshing).want(&marker) {
            return;
        }
        let hub = self.clone();
        let topic = topic.to_string();
        tokio::spawn(async move {
            tokio::time::sleep(COALESCE).await;
            let generation = lock(&hub.refreshing).start(&marker);
            let watched = {
                let guard = hub.inner.lock().await;
                guard
                    .topics
                    .key(&topic, &params)
                    .is_ok_and(|key| guard.topics.has_subscribers(&key))
            };
            let value = if watched {
                Some(hub.compute(&topic, &params).await)
            } else {
                None
            };
            // Decided and published under the hub's lock, which every publish takes: a later
            // computation that started meanwhile publishes after this one or instead of it.
            let guard = hub.inner.lock().await;
            if !lock(&hub.refreshing).finish(&marker, generation) {
                tracing::debug!(topic, "a later recomputation is out; this one is dropped");
                return;
            }
            match value {
                Some(Ok(value)) => hub.publish_locked(guard, &topic, params, value).await,
                Some(Err(e)) => tracing::debug!(topic, error = %e.message, "topic not recomputed"),
                None => {}
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
            "harnix.process" => Ok(self.process()),
            crate::SPACES_TOPIC => Ok(self.spaces_view()),
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
                let tasks = state.map(|s| s.live_tasks.len()).unwrap_or(0);
                let waiting = state.and_then(|s| s.pending_prompts.first()).map(|p| {
                    match (
                        p.get("prompt_type").and_then(Value::as_str),
                        p.get("tool_name").and_then(Value::as_str),
                    ) {
                        (Some("question"), _) => "answer",
                        // Leaving plan mode is asked as a permission: the plan waits on a review.
                        (_, Some("ExitPlanMode")) => "plan",
                        _ => "approval",
                    }
                });
                let key = (instance.to_string(), sid.clone());
                let space = self.space_of(&key);
                if let Some(obj) = row.as_object_mut() {
                    obj.insert("instance".into(), json!(instance));
                    obj.insert("space".into(), json!(space));
                    obj.insert("running".into(), json!(running));
                    obj.insert("queued".into(), json!(queued));
                    obj.insert("tasks".into(), json!(tasks));
                    obj.insert("waiting".into(), json!(waiting));
                    obj.insert(
                        "turn_started_ms".into(),
                        json!(state.and_then(|s| s.turn_started_ms)),
                    );
                    obj.insert("started_by".into(), json!(self.starter_of(&key)));
                    // What the hub last heard of its goal; absent when it has heard nothing.
                    if let Some(goal) = state.and_then(|s| s.goal.as_ref()) {
                        obj.insert("goal".into(), goal.clone());
                    }
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

    /// A session's tasks, as Agix reports them, and its teams as the hub last heard them
    /// (`team_progress`). Watching them keeps the hub subscribed to the session, which is how it
    /// hears a task start or end.
    pub(crate) async fn tasks_view(&self, key: &SessionKey) -> Result<Value, RpcError> {
        let result = self
            .call(&key.0, "session.tasks", json!({"session_id": key.1}))
            .await;
        let mut value = match result {
            Ok(v) => v,
            // Not in memory: nothing of it is running.
            Err(e) if e.code == harnix_instance::engine_codes::SESSION_NOT_FOUND => {
                json!({"active": false, "tasks": []})
            }
            Err(e) => return Err(e),
        };
        let teams: Vec<Value> = self
            .inner
            .lock()
            .await
            .sessions
            .get(key)
            .map(|s| s.teams.values().cloned().collect())
            .unwrap_or_default();
        if let Some(obj) = value.as_object_mut() {
            obj.insert("instance".into(), json!(key.0));
            obj.insert("session_id".into(), json!(key.1));
            obj.insert("teams".into(), json!(teams));
        }
        Ok(value)
    }

    /// The ids of a session's tasks that are running or waiting, as the
    /// engine reports them now; none if it cannot say.
    pub(crate) async fn live_tasks(&self, key: &SessionKey) -> std::collections::BTreeSet<String> {
        let Ok(view) = self.tasks_view(key).await else {
            return Default::default();
        };
        view.get("tasks")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|t| {
                matches!(
                    t.get("status").and_then(Value::as_str),
                    Some("running" | "waiting" | "pending")
                )
            })
            .filter_map(|t| t.get("task_id").and_then(Value::as_str).map(str::to_string))
            .collect()
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

#[cfg(test)]
mod tests {
    use super::*;

    fn marker() -> Marker {
        ("harnix.instances.sessions".into(), r#"{"instance":"system"}"#.into())
    }

    /// The race behind a deleted session that stayed listed: A starts, B starts while A's call
    /// is out, B publishes, then A finishes with what it read before the delete. A is dropped.
    #[test]
    fn an_older_recomputation_does_not_publish_over_a_newer_one() {
        let mut r = Refreshes::default();
        let m = marker();
        assert!(r.want(&m));
        let a = r.start(&m);
        assert!(r.want(&m), "A's window is closed, so a change opens another");
        let b = r.start(&m);
        assert!(r.finish(&m, b), "the newest publishes");
        assert!(!r.finish(&m, a), "the older one, finishing last, does not");
        assert_eq!(r.len(), 0, "nothing is recomputing it: forgotten");
    }

    #[test]
    fn changes_inside_one_window_cost_one_recomputation() {
        let mut r = Refreshes::default();
        let m = marker();
        assert!(r.want(&m));
        assert!(!r.want(&m));
        assert!(!r.want(&m));
        let a = r.start(&m);
        assert!(r.finish(&m, a));
        assert_eq!(r.len(), 0);
    }

    /// One that finished before the next started publishes, and the next publishes after it.
    #[test]
    fn recomputations_that_do_not_overlap_all_publish() {
        let mut r = Refreshes::default();
        let m = marker();
        r.want(&m);
        let a = r.start(&m);
        r.want(&m);
        assert!(r.finish(&m, a), "the next is still in its window, not started");
        let b = r.start(&m);
        assert!(r.finish(&m, b));
        assert_eq!(r.len(), 0);
    }
}
