//! What the module, its native tools, its middleware and its dispatcher share.

use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, OnceLock};

use harnix_module::{ModuleContext, RpcError, SessionRef};
use tokio::sync::broadcast;
use tokio::task::JoinHandle;

use crate::kernel::engine::Engine;
use crate::kernel::host::{Event, Gates, Hooks, Programs, WorkItem};

pub(crate) type RecordingFn = Arc<dyn Fn(&WorkItem) -> Option<String> + Send + Sync>;

pub(crate) struct Config {
    pub space: String,
    pub instance: String,
    /// The wrapped scene ids (`flow-<id>`).
    pub scenes: Vec<String>,
    pub programs: Option<Arc<dyn Programs>>,
    pub gates: Option<Arc<dyn Gates>>,
    pub hooks: Option<Arc<dyn Hooks>>,
    pub guarded: Vec<String>,
    pub recording: Option<RecordingFn>,
    pub lease_ms: u64,
    pub tick_ms: u64,
}

/// The task a session is working on right now, and under which claim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Binding {
    pub instance: String,
    pub run: String,
    pub station: String,
    pub round: u32,
    pub task: String,
    pub token: String,
    /// The station's `writes`.
    pub writes: Vec<String>,
}

/// One task in one round: what is dispatched at most once at a time.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct TaskKey {
    pub run: String,
    pub station: String,
    pub round: u32,
    pub task: String,
}

pub(crate) struct Shared {
    pub cfg: Config,
    pub engine: OnceLock<Arc<Engine>>,
    pub cx: OnceLock<ModuleContext>,
    pub events: broadcast::Sender<Event>,
    /// Session id (the engine's, bare) → the task it serves.
    bindings: Mutex<HashMap<String, Binding>>,
    /// Task → the token of the claim being carried out.
    active: Mutex<HashMap<TaskKey, String>>,
    /// One lock per session key: tasks sharing a session take turns.
    session_locks: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    /// Topic publishes in order: each reads the newest snapshot under it.
    pub publish: tokio::sync::Mutex<()>,
    pub stopping: AtomicBool,
    pub tick: Mutex<Option<JoinHandle<()>>>,
    /// Accepting proposals one at a time, so one is never accepted twice.
    pub proposals: Arc<tokio::sync::Mutex<()>>,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl Shared {
    pub fn new(cfg: Config, events: broadcast::Sender<Event>) -> Self {
        Self {
            cfg,
            engine: OnceLock::new(),
            cx: OnceLock::new(),
            events,
            bindings: Mutex::new(HashMap::new()),
            active: Mutex::new(HashMap::new()),
            session_locks: Mutex::new(HashMap::new()),
            publish: tokio::sync::Mutex::new(()),
            stopping: AtomicBool::new(false),
            tick: Mutex::new(None),
            proposals: Arc::default(),
        }
    }

    pub fn engine(&self) -> Result<Arc<Engine>, RpcError> {
        self.engine.get().cloned().ok_or_else(|| {
            RpcError::new(
                harnix_contract::codes::INSTANCE_UNAVAILABLE,
                "the flow module has not started",
            )
        })
    }

    pub fn stopping(&self) -> bool {
        self.stopping.load(std::sync::atomic::Ordering::SeqCst)
    }

    pub fn binding(&self, session_id: &str) -> Option<Binding> {
        lock(&self.bindings).get(session_id).cloned()
    }

    pub fn bind(&self, session_id: &str, b: Binding) {
        lock(&self.bindings).insert(session_id.to_string(), b);
    }

    /// Forget a session's binding, if it is still this claim's.
    pub fn unbind(&self, session_id: &str, token: &str) {
        let mut g = lock(&self.bindings);
        if g.get(session_id).is_some_and(|b| b.token == token) {
            g.remove(session_id);
        }
    }

    pub fn unbind_session(&self, session_id: &str) {
        lock(&self.bindings).remove(session_id);
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
        lock(&self.session_locks)
            .entry(key.to_string())
            .or_default()
            .clone()
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

    /// Drop the bindings of `run` that `keep` says are over.
    pub fn prune_bindings(&self, run: &str, keep: impl Fn(&Binding) -> bool) {
        lock(&self.bindings).retain(|_, b| b.run != run || keep(b));
    }
}

/// How a session is written in a run record: `<instance>/<session_id>`.
pub fn session_name(s: &SessionRef) -> String {
    format!("{}/{}", s.instance, s.session_id)
}

/// The session a run record names. A bare id is on `system`.
pub fn session_ref(name: &str) -> SessionRef {
    match name.split_once('/') {
        Some((instance, id)) if !instance.is_empty() && !id.is_empty() => SessionRef {
            instance: instance.to_string(),
            session_id: id.to_string(),
        },
        _ => SessionRef {
            instance: super::FLOW_INSTANCE.to_string(),
            session_id: name.to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shared() -> Shared {
        let (tx, _) = broadcast::channel(4);
        Shared::new(
            Config {
                space: "flow".into(),
                instance: "system".into(),
                scenes: vec![],
                programs: None,
                gates: None,
                hooks: None,
                guarded: vec![],
                recording: None,
                lease_ms: 60_000,
                tick_ms: 1000,
            },
            tx,
        )
    }

    fn key(round: u32) -> TaskKey {
        TaskKey {
            run: "r-1".into(),
            station: "analyze".into(),
            round,
            task: "core".into(),
        }
    }

    #[test]
    fn a_claim_is_carried_out_once_and_a_new_one_replaces_it() {
        let sh = shared();
        assert!(sh.begin(&key(1), "t1"));
        assert!(!sh.begin(&key(1), "t1"), "the same claim twice: once");
        assert!(sh.begin(&key(2), "t1"), "another round is another task");
        assert!(
            sh.begin(&key(1), "t2"),
            "a new claim (retry, expiry) goes ahead"
        );
        sh.end(&key(1), "t1");
        assert!(
            !sh.begin(&key(1), "t2"),
            "ending the old claim leaves the new one"
        );
        sh.end(&key(1), "t2");
        assert!(sh.begin(&key(1), "t2"));
    }

    #[test]
    fn a_binding_is_dropped_only_by_its_own_claim() {
        let sh = shared();
        let b = |token: &str| Binding {
            instance: "system".into(),
            run: "r-1".into(),
            station: "a".into(),
            round: 1,
            task: "a".into(),
            token: token.into(),
            writes: vec![],
        };
        sh.bind("s", b("old"));
        sh.bind("s", b("new"));
        sh.unbind("s", "old");
        assert_eq!(sh.binding("s").map(|b| b.token), Some("new".into()));
        sh.unbind("s", "new");
        assert!(sh.binding("s").is_none());
    }

    #[tokio::test]
    async fn a_session_lock_is_removed_when_nobody_uses_it() {
        let sh = shared();
        let first = sh.session_lock("r#run");
        let second = sh.session_lock("r#run");
        let held = first.clone().lock_owned().await;
        sh.release_lock("r#run", first);
        assert_eq!(sh.session_locks(), 1, "another task still waits on it");
        drop(held);
        sh.release_lock("r#run", second);
        assert_eq!(sh.session_locks(), 0);
    }

    #[test]
    fn session_names_round_trip() {
        let s = SessionRef {
            instance: "system".into(),
            session_id: "abc".into(),
        };
        assert_eq!(session_name(&s), "system/abc");
        assert_eq!(session_ref("system/abc"), s);
        assert_eq!(session_ref("abc"), s, "a bare id is on system");
    }
}
