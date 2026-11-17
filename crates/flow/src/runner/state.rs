//! What the runner's tasks share: which session serves which task, which claims are in hand,
//! the locks tasks sharing a session take turns on, the main sessions watched, what was sent to a
//! main session and not seen start yet, and the sub-agents a run's main session started.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde_json::Value;

use super::host::SessionId;
use super::lock;
use crate::kernel::parts::Granted;
use crate::kernel::run::{RunRecord, StationStatus, TaskStatus};

/// The task a session is working on right now, and under which claim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Binding {
    pub run: String,
    pub station: String,
    pub round: u32,
    pub task: String,
    pub token: String,
    /// What the task works under, references resolved.
    pub permissions: Vec<Granted>,
}

/// One task in one round: what is dispatched at most once at a time.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TaskKey {
    pub run: String,
    pub station: String,
    pub round: u32,
    pub task: String,
}

/// How many runs' sub-agents are remembered.
pub const AGENTS_KEPT: usize = 256;

#[derive(Default)]
pub(crate) struct State {
    /// Session → the task it serves.
    bindings: Mutex<HashMap<SessionId, Binding>>,
    /// Task → the token of the claim being carried out.
    active: Mutex<HashMap<TaskKey, String>>,
    /// One lock per session key: tasks sharing a session take turns.
    session_locks: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    /// Main sessions being watched: session → run.
    watched: Mutex<HashMap<SessionId, String>>,
    /// What the dispatcher sent a main session for a task and has not seen start yet: session →
    /// (the message's text, the task's binding). The turn that begins with that text is the
    /// task's; every other turn of the session is a person's.
    sent: Mutex<HashMap<SessionId, Vec<(String, Binding)>>>,
    /// The sub-agents each run's main session started, as its watch saw them: run →
    /// sub-agents, oldest first. Not kept over a restart.
    agents: Mutex<HashMap<String, (u64, Vec<Value>)>>,
    agents_seq: std::sync::atomic::AtomicU64,
}

impl State {
    pub fn binding(&self, id: &str) -> Option<Binding> {
        lock(&self.bindings).get(id).cloned()
    }

    pub fn bind(&self, id: &str, b: Binding) {
        lock(&self.bindings).insert(id.to_string(), b);
    }

    /// Forget a session's binding, if it is still this claim's.
    pub fn unbind(&self, id: &str, token: &str) {
        let mut g = lock(&self.bindings);
        if g.get(id).is_some_and(|b| b.token == token) {
            g.remove(id);
        }
    }

    pub fn unbind_session(&self, id: &str) {
        lock(&self.bindings).remove(id);
    }

    /// Drop the bindings of `run` that `keep` says are over.
    pub fn prune_bindings(&self, run: &str, keep: impl Fn(&Binding) -> bool) {
        lock(&self.bindings).retain(|_, b| b.run != run || keep(b));
    }

    /// Take a task for this claim. `false`: it is already being carried out under it.
    pub fn begin(&self, key: &TaskKey, token: &str) -> bool {
        let mut g = lock(&self.active);
        if g.get(key).is_some_and(|t| t == token) {
            return false;
        }
        g.insert(key.clone(), token.to_string());
        true
    }

    pub fn end(&self, key: &TaskKey, token: &str) {
        let mut g = lock(&self.active);
        if g.get(key).is_some_and(|t| t == token) {
            g.remove(key);
        }
    }

    pub fn session_lock(&self, key: &str) -> Arc<tokio::sync::Mutex<()>> {
        lock(&self.session_locks).entry(key.to_string()).or_default().clone()
    }

    /// Give back a session lock; the last one out removes it, so the table holds only the
    /// sessions some task is using or waiting for.
    pub fn release_lock(&self, key: &str, held: Arc<tokio::sync::Mutex<()>>) {
        let mut g = lock(&self.session_locks);
        drop(held);
        if g.get(key).is_some_and(|l| Arc::strong_count(l) == 1) {
            g.remove(key);
        }
    }

    #[cfg(test)]
    pub fn session_locks(&self) -> usize {
        lock(&self.session_locks).len()
    }

    /// Start watching a main session. `false`: it is watched already.
    pub fn watch(&self, id: &str, run: &str) -> bool {
        let mut g = lock(&self.watched);
        if g.contains_key(id) {
            return false;
        }
        g.insert(id.to_string(), run.to_string());
        true
    }

    pub fn unwatch(&self, id: &str) {
        lock(&self.watched).remove(id);
    }

    /// The run whose main session is being watched, when it is.
    pub fn watched_run(&self, id: &str) -> Option<String> {
        lock(&self.watched).get(id).cloned()
    }

    /// Note a message sent to a main session for the task `b` (before it is sent).
    pub fn note_sent(&self, id: &str, text: &str, b: Binding) {
        lock(&self.sent).entry(id.to_string()).or_default().push((text.to_string(), b));
    }

    /// The task a turn that began with `text` was started for, taken off the list; `None`: a
    /// person's turn.
    pub fn take_sent(&self, id: &str, text: &str) -> Option<Binding> {
        let mut g = lock(&self.sent);
        let list = g.get_mut(id)?;
        let i = list.iter().position(|(t, _)| t == text)?;
        let (_, b) = list.remove(i);
        if list.is_empty() {
            g.remove(id);
        }
        Some(b)
    }

    /// Note a sub-agent of `run`'s main session, or what more is known of it (by `agent`): the
    /// fields of `seen` go over the ones noted.
    pub fn note_agent(&self, run: &str, seen: Value) {
        let Some(id) = seen.get("agent").and_then(Value::as_str).map(String::from) else {
            return;
        };
        let seq = self.agents_seq.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let mut g = lock(&self.agents);
        // The runs whose sub-agents were noted longest ago go first, past AGENTS_KEPT.
        if !g.contains_key(run) && g.len() >= AGENTS_KEPT {
            if let Some(oldest) = g.iter().min_by_key(|(_, (s, _))| *s).map(|(k, _)| k.clone()) {
                g.remove(&oldest);
            }
        }
        let (at, list) = g.entry(run.to_string()).or_insert((seq, vec![]));
        *at = seq;
        match list.iter_mut().find(|a| a.get("agent").and_then(Value::as_str) == Some(id.as_str())) {
            Some(a) => {
                if let (Some(a), Some(n)) = (a.as_object_mut(), seen.as_object()) {
                    for (k, v) in n {
                        if !v.is_null() {
                            a.insert(k.clone(), v.clone());
                        }
                    }
                }
            }
            None => list.push(seen),
        }
    }

    /// The sub-agents noted for `run`.
    pub fn agents(&self, run: &str) -> Vec<Value> {
        lock(&self.agents).get(run).map(|(_, l)| l.clone()).unwrap_or_default()
    }
}

/// Whether a binding's claim still counts: its station open, its task claimed or handed in under
/// the binding's token. A task handed in keeps its session bound so it can hand in again until
/// the station passes.
pub(crate) fn binding_current(rec: &RunRecord, b: &Binding) -> bool {
    rec.is_active()
        && rec.stations.get(&b.station).is_some_and(|st| {
            matches!(st.status, StationStatus::Working | StationStatus::Unmet)
                && st.task(&b.task).is_some_and(|t| {
                    matches!(t.status, TaskStatus::Claimed | TaskStatus::Submitted)
                        && t.lease.as_ref().is_some_and(|l| l.token == b.token)
                })
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn key(round: u32) -> TaskKey {
        TaskKey { run: "r-1".into(), station: "analyze".into(), round, task: "core".into() }
    }

    fn binding(token: &str) -> Binding {
        Binding { run: "r-1".into(), station: "a".into(), round: 1, task: "a".into(), token: token.into(), permissions: vec![] }
    }

    #[test]
    fn a_claim_is_carried_out_once_and_a_new_one_replaces_it() {
        let st = State::default();
        assert!(st.begin(&key(1), "t1"));
        assert!(!st.begin(&key(1), "t1"), "the same claim twice: once");
        assert!(st.begin(&key(2), "t1"), "another round is another task");
        assert!(st.begin(&key(1), "t2"), "a new claim (retry, expiry) goes ahead");
        st.end(&key(1), "t1");
        assert!(!st.begin(&key(1), "t2"), "ending the old claim leaves the new one");
        st.end(&key(1), "t2");
        assert!(st.begin(&key(1), "t2"));
    }

    #[test]
    fn a_binding_is_dropped_only_by_its_own_claim() {
        let st = State::default();
        st.bind("s", binding("old"));
        st.bind("s", binding("new"));
        st.unbind("s", "old");
        assert_eq!(st.binding("s").map(|b| b.token), Some("new".into()));
        st.unbind("s", "new");
        assert!(st.binding("s").is_none());
    }

    #[tokio::test]
    async fn a_session_lock_is_removed_when_nobody_uses_it() {
        let st = State::default();
        let first = st.session_lock("r#run");
        let second = st.session_lock("r#run");
        let held = first.clone().lock_owned().await;
        st.release_lock("r#run", first);
        assert_eq!(st.session_locks(), 1, "another task still waits on it");
        drop(held);
        st.release_lock("r#run", second);
        assert_eq!(st.session_locks(), 0);
    }

    #[test]
    fn a_sub_agent_is_noted_once_and_what_is_learnt_later_goes_over_it() {
        let st = State::default();
        st.note_agent("r", json!({"agent": "s/a1", "call": "c1", "status": null}));
        st.note_agent("r", json!({"agent": "s/a1", "status": "done"}));
        st.note_agent("r", json!({"nothing": true}));
        let agents = st.agents("r");
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0], json!({"agent": "s/a1", "call": "c1", "status": "done"}));
        assert!(st.agents("other").is_empty());
    }

    #[test]
    fn a_binding_counts_until_its_station_passes_or_its_claim_is_superseded() {
        let mut rec: RunRecord = serde_json::from_value(json!({
            "id": "r", "flow": "f", "def_digest": "d", "subject": "s", "status": "active",
            "rev": 1, "created_at": 0, "created_by": {"kind": "x", "id": "y"},
            "def": {"name": "f", "stations": [{"id": "a", "exit": [{"kind": "confirm"}]}]},
            "stations": {"a": {"status": "working", "round": 1, "tasks": [
                {"id": "t", "status": "submitted", "attempt": 1,
                 "lease": {"by": {"kind": "flow", "id": "dispatch"}, "token": "tok", "until": 9}}
            ]}}
        }))
        .unwrap();
        let b = Binding { run: "r".into(), station: "a".into(), round: 1, task: "t".into(), token: "tok".into(), permissions: vec![] };
        assert!(binding_current(&rec, &b), "handed in, station open: it may hand in again");
        let other = Binding { token: "old".into(), ..b.clone() };
        assert!(!binding_current(&rec, &other));
        rec.stations.get_mut("a").unwrap().status = StationStatus::Passed;
        assert!(!binding_current(&rec, &b));
    }
}
