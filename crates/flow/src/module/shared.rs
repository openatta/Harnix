//! What the module, its native tools, its middleware and its dispatcher share.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, OnceLock};

use harnix_module::{ModuleContext, RpcError, SessionRef};
use tokio::sync::broadcast;
use tokio::task::JoinHandle;

use crate::dryrun::Sandbox;
use crate::kernel::engine::Engine;
use crate::kernel::host::{Event, Gates, Programs, WorkItem};
use crate::kernel::parts::Granted;
use crate::kernel::run::RunRecord;

pub(crate) type RecordingFn = Arc<dyn Fn(&WorkItem) -> Option<String> + Send + Sync>;
pub(crate) type MainRecordingFn = Arc<dyn Fn(&RunRecord) -> Option<String> + Send + Sync>;
pub(crate) type ConversationRecordingFn = Arc<dyn Fn(&str) -> Option<String> + Send + Sync>;

pub(crate) struct Config {
    pub space: String,
    pub instance: String,
    /// The wrapped scene ids (`flow-<id>`).
    pub scenes: Vec<String>,
    pub programs: Option<Arc<dyn Programs>>,
    pub gates: Option<Arc<dyn Gates>>,
    /// Whether `command` gates may run (xops-04 §4).
    pub command_gates: bool,
    pub guarded: Vec<String>,
    pub recording: Option<RecordingFn>,
    /// The recording a run's main session replays (xops-03 §1).
    pub main_recording: Option<MainRecordingFn>,
    /// The recording a flow's conversation replays (`""`: the general one) — xops-04 §8.
    pub conversation_recording: Option<ConversationRecordingFn>,
    /// The wrapped scene a run's main session is opened on when the flow's `main` names none.
    pub main_scene: Option<String>,
    /// The wrapped scenes whose sessions need a project: a main session on one works in a
    /// directory of its run's (`workspaces/<run>` in the module's data directory).
    pub project_scenes: std::collections::BTreeSet<String>,
    pub lease_ms: u64,
    pub tick_ms: u64,
    /// How long a station's template instance may take to be ready.
    pub ensure_ms: u64,
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
    /// What the task works under, references resolved (xops-04 §3).
    pub permissions: Vec<Granted>,
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
    /// Dry runs (xops-02 §5).
    pub sandbox: OnceLock<Arc<Sandbox>>,
    /// The flow each authoring conversation works on: `(instance, session_id)` → (flow, when
    /// set). Bounded by [`FOCUS_KEPT`]: the oldest goes first.
    focus: Mutex<HashMap<(String, String), (String, u64)>>,
    focus_seq: std::sync::atomic::AtomicU64,
    /// Instances the module made for runs under its own names: instance → run. Kept in the
    /// module's data directory, so an instance a run left behind is stopped after a restart too.
    instances: Mutex<BTreeMap<String, String>>,
    pub ledger: OnceLock<PathBuf>,
    /// Main sessions being watched (xops-03 §1): session id → run.
    watched: Mutex<HashMap<String, String>>,
    /// Each flow's conversation, and the general one under `""` (xops-03 §5): flow →
    /// `<instance>/<session_id>`. Kept in the module's data directory (`sessions.json`).
    flow_sessions: Mutex<BTreeMap<String, String>>,
    flow_sessions_path: OnceLock<PathBuf>,
    /// One conversation made at a time, so a flow never gets two.
    pub making_session: tokio::sync::Mutex<()>,
    /// The sub-agents each run's main session started, as its watch saw them (xops-03 §2):
    /// run → sub-agents, oldest first. Not kept over a restart.
    agents: Mutex<HashMap<String, (u64, Vec<serde_json::Value>)>>,
    agents_seq: std::sync::atomic::AtomicU64,
    /// What the dispatcher sent a main session for a task and has not seen start yet: session id
    /// → (the message's text, the task's binding). The turn whose `user_message` is that text is
    /// the task's; every other turn of the session is a person's (xops-03 review #2).
    sent: Mutex<HashMap<String, Vec<(String, Binding)>>>,
    /// Each hook's last delivery (xops-04 §5): hook → `{at, event, status, error?}`.
    pub hook_status: Mutex<BTreeMap<String, serde_json::Value>>,
    /// Each hook's delivery queue.
    pub hook_queues: super::hooks::Queues,
}

/// How many runs' sub-agents are remembered.
pub const AGENTS_KEPT: usize = 256;

/// How many conversations' focus is remembered.
pub const FOCUS_KEPT: usize = 1024;

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
            sandbox: OnceLock::new(),
            focus: Mutex::new(HashMap::new()),
            focus_seq: std::sync::atomic::AtomicU64::new(0),
            instances: Mutex::new(BTreeMap::new()),
            ledger: OnceLock::new(),
            watched: Mutex::new(HashMap::new()),
            flow_sessions: Mutex::new(BTreeMap::new()),
            flow_sessions_path: OnceLock::new(),
            making_session: tokio::sync::Mutex::new(()),
            agents: Mutex::new(HashMap::new()),
            agents_seq: std::sync::atomic::AtomicU64::new(0),
            sent: Mutex::new(HashMap::new()),
            hook_status: Mutex::new(BTreeMap::new()),
            hook_queues: Default::default(),
        }
    }

    /// Note a message sent to a main session for the task `b` (before it is sent).
    pub fn note_sent(&self, session_id: &str, text: &str, b: Binding) {
        lock(&self.sent)
            .entry(session_id.to_string())
            .or_default()
            .push((text.to_string(), b));
    }

    /// The task a turn that began with `text` was started for, taken off the list; `None`: a
    /// person's turn.
    pub fn take_sent(&self, session_id: &str, text: &str) -> Option<Binding> {
        let mut g = lock(&self.sent);
        let list = g.get_mut(session_id)?;
        let i = list.iter().position(|(t, _)| t == text)?;
        let (_, b) = list.remove(i);
        if list.is_empty() {
            g.remove(session_id);
        }
        Some(b)
    }

    /// Note a sub-agent of `run`'s main session, or what more is known of it (by
    /// `agent_session_id`): the fields of `seen` go over the ones noted.
    pub fn note_agent(&self, run: &str, seen: serde_json::Value) {
        let Some(id) = seen.get("agent_session_id").and_then(|v| v.as_str()).map(String::from) else {
            return;
        };
        let seq = self
            .agents_seq
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let mut g = lock(&self.agents);
        // The runs whose sub-agents were noted longest ago go first, past AGENTS_KEPT.
        if !g.contains_key(run) && g.len() >= AGENTS_KEPT {
            if let Some(oldest) = g.iter().min_by_key(|(_, (s, _))| *s).map(|(k, _)| k.clone()) {
                g.remove(&oldest);
            }
        }
        let (at, list) = g.entry(run.to_string()).or_insert((seq, vec![]));
        *at = seq;
        match list
            .iter_mut()
            .find(|a| a.get("agent_session_id").and_then(|v| v.as_str()) == Some(id.as_str()))
        {
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
    pub fn agents(&self, run: &str) -> Vec<serde_json::Value> {
        lock(&self.agents)
            .get(run)
            .map(|(_, l)| l.clone())
            .unwrap_or_default()
    }

    /// Read the flows' conversations from `path`, and save to it from now on.
    pub fn load_flow_sessions(&self, path: PathBuf) {
        if let Ok(text) = std::fs::read_to_string(&path) {
            if let Ok(m) = serde_json::from_str::<BTreeMap<String, String>>(&text) {
                *lock(&self.flow_sessions) = m;
            }
        }
        let _ = self.flow_sessions_path.set(path);
    }

    /// The conversation of `flow` (`""`: the general one), as recorded.
    pub fn flow_session(&self, flow: &str) -> Option<String> {
        lock(&self.flow_sessions).get(flow).cloned()
    }

    /// Record (or, with `None`, forget) the conversation of `flow`.
    pub fn set_flow_session(&self, flow: &str, session: Option<String>) {
        let mut g = lock(&self.flow_sessions);
        let changed = match session {
            Some(s) => g.insert(flow.to_string(), s.clone()).as_ref() != Some(&s),
            None => g.remove(flow).is_some(),
        };
        if !changed {
            return;
        }
        if let Some(path) = self.flow_sessions_path.get() {
            let tmp = path.with_extension("json.tmp");
            let ok = std::fs::write(&tmp, serde_json::to_vec_pretty(&*g).unwrap_or_default())
                .and_then(|_| std::fs::rename(&tmp, path));
            if let Err(e) = ok {
                tracing::warn!(path = %path.display(), error = %e, "flow: conversations not saved");
            }
        }
    }

    /// The flow whose conversation `<instance>/<session_id>` is, if it is one (`""`: the general one).
    pub fn flow_of_session(&self, instance: &str, session_id: &str) -> Option<String> {
        let name = format!("{instance}/{session_id}");
        lock(&self.flow_sessions)
            .iter()
            .find(|(_, s)| **s == name)
            .map(|(f, _)| f.clone())
    }

    /// Read the instance ledger from `path`, and save to it from now on.
    pub fn load_ledger(&self, path: PathBuf) {
        if let Ok(text) = std::fs::read_to_string(&path) {
            if let Ok(m) = serde_json::from_str::<BTreeMap<String, String>>(&text) {
                *lock(&self.instances) = m;
            }
        }
        let _ = self.ledger.set(path);
    }

    fn save_ledger(&self, m: &BTreeMap<String, String>) {
        let Some(path) = self.ledger.get() else {
            return;
        };
        let tmp = path.with_extension("json.tmp");
        let ok = std::fs::write(&tmp, serde_json::to_vec_pretty(m).unwrap_or_default())
            .and_then(|_| std::fs::rename(&tmp, path));
        if let Err(e) = ok {
            tracing::warn!(path = %path.display(), error = %e, "flow: instance ledger not saved");
        }
    }

    /// Record an instance the module makes for `run` under its own name.
    pub fn own_instance(&self, instance: &str, run: &str) {
        let mut g = lock(&self.instances);
        if g.get(instance).map(String::as_str) != Some(run) {
            g.insert(instance.to_string(), run.to_string());
            self.save_ledger(&g);
        }
    }

    /// Forget the instances made for `run`; their names.
    pub fn take_instances(&self, run: &str) -> Vec<String> {
        let mut g = lock(&self.instances);
        let names: Vec<String> = g
            .iter()
            .filter(|(_, r)| *r == run)
            .map(|(i, _)| i.clone())
            .collect();
        if !names.is_empty() {
            g.retain(|_, r| r != run);
            self.save_ledger(&g);
        }
        names
    }

    /// Every run some recorded instance was made for.
    pub fn instance_runs(&self) -> Vec<String> {
        let mut runs: Vec<String> = lock(&self.instances).values().cloned().collect();
        runs.sort();
        runs.dedup();
        runs
    }

    pub fn sandbox(&self) -> Result<Arc<Sandbox>, RpcError> {
        self.sandbox.get().cloned().ok_or_else(|| {
            RpcError::new(
                harnix_contract::codes::INSTANCE_UNAVAILABLE,
                "the flow module has not started",
            )
        })
    }

    /// The flow a conversation works on: a flow's own conversation works on that flow, for
    /// good (xops-03 §5); another one on what was set for it.
    pub fn focus(&self, instance: &str, session_id: &str) -> Option<String> {
        if let Some(f) = self.flow_of_session(instance, session_id) {
            return (!f.is_empty()).then_some(f);
        }
        lock(&self.focus)
            .get(&(instance.to_string(), session_id.to_string()))
            .map(|(f, _)| f.clone())
    }

    /// Set (or, with `None`, clear) the flow a conversation works on. Past [`FOCUS_KEPT`]
    /// conversations the one set longest ago is forgotten.
    pub fn set_focus(&self, instance: &str, session_id: &str, flow: Option<String>) {
        let key = (instance.to_string(), session_id.to_string());
        let mut g = lock(&self.focus);
        match flow {
            Some(f) => {
                let seq = self
                    .focus_seq
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                g.insert(key, (f, seq));
                while g.len() > FOCUS_KEPT {
                    let Some(oldest) = g
                        .iter()
                        .min_by_key(|(_, (_, s))| *s)
                        .map(|(k, _)| k.clone())
                    else {
                        break;
                    };
                    g.remove(&oldest);
                }
            }
            None => {
                g.remove(&key);
            }
        }
    }

    /// Every conversation whose current flow is `flow`.
    pub fn focused_on(&self, flow: &str) -> Vec<(String, String)> {
        lock(&self.focus)
            .iter()
            .filter(|(_, (f, _))| *f == flow)
            .map(|(k, _)| k.clone())
            .collect()
    }

    #[cfg(test)]
    pub fn focus_len(&self) -> usize {
        lock(&self.focus).len()
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

    /// Start watching a main session. `false`: it is watched already.
    pub fn watch(&self, session_id: &str, run: &str) -> bool {
        let mut g = lock(&self.watched);
        if g.contains_key(session_id) {
            return false;
        }
        g.insert(session_id.to_string(), run.to_string());
        true
    }

    pub fn unwatch(&self, session_id: &str) {
        lock(&self.watched).remove(session_id);
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
                command_gates: false,
                guarded: vec![],
                recording: None,
                main_recording: None,
                conversation_recording: None,
                main_scene: None,
                project_scenes: Default::default(),
                lease_ms: 60_000,
                tick_ms: 1000,
                ensure_ms: 60_000,
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
            permissions: vec![],
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
    fn a_conversation_remembers_its_flow_until_it_is_cleared() {
        let sh = shared();
        assert_eq!(sh.focus("system", "s1"), None);
        sh.set_focus("system", "s1", Some("demo".into()));
        sh.set_focus("system", "s2", Some("other".into()));
        assert_eq!(sh.focus("system", "s1").as_deref(), Some("demo"));
        assert_eq!(sh.focus("side", "s1"), None, "per instance and session");
        assert_eq!(
            sh.focused_on("demo"),
            [("system".to_string(), "s1".to_string())]
        );
        sh.set_focus("system", "s1", None);
        assert_eq!(sh.focus("system", "s1"), None);
        assert!(super::super::NO_FOCUS.contains("select"));
        for i in 0..FOCUS_KEPT + 10 {
            sh.set_focus("system", &format!("many-{i}"), Some("demo".into()));
        }
        assert_eq!(sh.focus_len(), FOCUS_KEPT, "bounded");
        assert_eq!(sh.focus("system", "s2"), None, "the oldest went first");
        assert_eq!(
            sh.focus("system", &format!("many-{}", FOCUS_KEPT + 9))
                .as_deref(),
            Some("demo")
        );
    }

    #[test]
    fn the_instances_made_for_a_run_are_remembered_on_disk_until_taken() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("instances.json");
        let sh = shared();
        sh.load_ledger(path.clone());
        sh.own_instance("flow-r-1-aaaa", "r-1");
        sh.own_instance("flow-r-1-bbbb", "r-1");
        sh.own_instance("flow-r-2-cccc", "r-2");
        let again = shared();
        again.load_ledger(path.clone());
        assert_eq!(
            again.instance_runs(),
            ["r-1", "r-2"],
            "it survives a restart"
        );
        assert_eq!(
            again.take_instances("r-1"),
            ["flow-r-1-aaaa", "flow-r-1-bbbb"]
        );
        let third = shared();
        third.load_ledger(path);
        assert_eq!(third.instance_runs(), ["r-2"]);
        assert!(third.take_instances("r-1").is_empty());
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
