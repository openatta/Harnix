//! What the extension's methods, scenes, topics and host share: the runner once started, the
//! engine, the context, the sandbox, and what is Harnix's alone — each authoring conversation's
//! flow and project, each flow's conversation, the instances made for runs.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

use harnix_ext::{ExtensionContext, RpcError};

use super::lock;
use crate::author::AuthorTools;
use crate::dryrun::Sandbox;
use crate::kernel::engine::Engine;
use crate::kernel::host::Packs;
use crate::packs::ProjectTrust;
use crate::runner::Runner;

pub(crate) type ConversationRecordingFn = Arc<dyn Fn(&str) -> Option<String> + Send + Sync>;

/// How the extension is set up, beyond what the runner takes (`RunnerConfig`).
pub(crate) struct Config {
    pub space: String,
    pub instance: String,
    /// The wrapped scene ids (`flow-<id>`).
    pub scenes: Vec<String>,
    /// The wrapped scene a run's main session is opened on when the flow's `main` names none.
    pub main_scene: Option<String>,
    /// The wrapped scenes whose sessions need a project.
    pub project_scenes: std::collections::BTreeSet<String>,
    /// The recording a flow's conversation replays (`""`: the general one).
    pub conversation_recording: Option<ConversationRecordingFn>,
    /// How long a station's template instance may take to be ready.
    pub ensure_ms: u64,
}

pub(crate) struct Shared {
    pub cfg: Config,
    pub engine: OnceLock<Arc<Engine>>,
    pub runner: OnceLock<Arc<Runner>>,
    /// The authoring tools `flow-author` offers, once the runner is there.
    pub author: OnceLock<Arc<AuthorTools>>,
    pub cx: OnceLock<ExtensionContext>,
    /// Topic publishes in order: each reads the newest snapshot under it.
    pub publish: tokio::sync::Mutex<()>,
    /// Accepting proposals one at a time, so one is never accepted twice.
    pub proposals: Arc<tokio::sync::Mutex<()>>,
    /// Dry runs.
    pub sandbox: OnceLock<Arc<Sandbox>>,
    /// The flow each authoring conversation works on: `(instance, session_id)` → (flow, when
    /// set). Bounded by [`FOCUS_KEPT`]: the oldest goes first.
    focus: Mutex<HashMap<(String, String), (String, u64)>>,
    focus_seq: std::sync::atomic::AtomicU64,
    /// Instances the extension made for runs under its own names: instance → run. Kept in the
    /// extension's data directory, so an instance a run left behind is stopped after a restart too.
    instances: Mutex<BTreeMap<String, String>>,
    pub ledger: OnceLock<PathBuf>,
    /// Each flow's conversation, and the general one under `""`: flow → `<instance>/<session_id>`.
    /// Kept in the extension's data directory (`sessions.json`).
    flow_sessions: Mutex<BTreeMap<String, String>>,
    flow_sessions_path: OnceLock<PathBuf>,
    /// One conversation made at a time, so a flow never gets two.
    pub making_session: tokio::sync::Mutex<()>,
    /// Whether projects are trusted, as last heard from the engine: shared with the runner and
    /// the packs, so a project's own packs are read only while it is.
    pub projects: Arc<ProjectTrust>,
    /// The project each authoring conversation works in, when it has one: `(instance,
    /// session_id)` → project root. Bounded by [`FOCUS_KEPT`].
    conversation_projects: Mutex<HashMap<(String, String), String>>,
}

/// How many conversations' focus is remembered.
pub const FOCUS_KEPT: usize = 1024;

impl Shared {
    pub fn new(cfg: Config) -> Self {
        Self {
            cfg,
            engine: OnceLock::new(),
            runner: OnceLock::new(),
            author: OnceLock::new(),
            cx: OnceLock::new(),
            publish: tokio::sync::Mutex::new(()),
            proposals: Arc::default(),
            sandbox: OnceLock::new(),
            focus: Mutex::new(HashMap::new()),
            focus_seq: std::sync::atomic::AtomicU64::new(0),
            instances: Mutex::new(BTreeMap::new()),
            ledger: OnceLock::new(),
            flow_sessions: Mutex::new(BTreeMap::new()),
            flow_sessions_path: OnceLock::new(),
            making_session: tokio::sync::Mutex::new(()),
            projects: Arc::default(),
            conversation_projects: Mutex::new(HashMap::new()),
        }
    }

    fn not_started() -> RpcError {
        RpcError::new(harnix_ext::codes::INSTANCE_UNAVAILABLE, "the flow extension has not started")
    }

    pub fn engine(&self) -> Result<Arc<Engine>, RpcError> {
        self.engine.get().cloned().ok_or_else(Self::not_started)
    }

    pub fn runner(&self) -> Result<Arc<Runner>, RpcError> {
        self.runner.get().cloned().ok_or_else(Self::not_started)
    }

    pub fn sandbox(&self) -> Result<Arc<Sandbox>, RpcError> {
        self.sandbox.get().cloned().ok_or_else(Self::not_started)
    }

    /// Whether `root` is trusted: asked of the engine through the runner, when there is one; as
    /// last heard otherwise.
    pub async fn hear(&self, root: &str) -> bool {
        match self.runner.get() {
            Some(rn) => rn.hear(root).await,
            None => self.projects.trusted(root),
        }
    }

    /// The packs as a project sees them, its trust heard first; the application's without one.
    pub async fn packs_in(&self, project: Option<&str>) -> Result<Arc<dyn Packs>, RpcError> {
        let engine = self.engine()?;
        let Some(root) = project.filter(|r| !r.trim().is_empty()) else {
            return Ok(engine.packs().clone());
        };
        let root = project_root(root).map_err(RpcError::invalid_params)?;
        self.hear(&root).await;
        Ok(engine.packs_for(Some(&root)))
    }

    /// The project an authoring conversation works in: its flows and parts are read and saved
    /// there.
    pub fn conversation_project(&self, instance: &str, session_id: &str) -> Option<String> {
        lock(&self.conversation_projects).get(&(instance.to_string(), session_id.to_string())).cloned()
    }

    /// Set (or, with `None`, clear) the project a conversation works in.
    pub fn set_conversation_project(&self, instance: &str, session_id: &str, root: Option<String>) {
        let key = (instance.to_string(), session_id.to_string());
        let mut g = lock(&self.conversation_projects);
        match root {
            Some(r) => {
                if g.len() >= FOCUS_KEPT && !g.contains_key(&key) {
                    if let Some(k) = g.keys().next().cloned() {
                        g.remove(&k);
                    }
                }
                g.insert(key, r);
            }
            None => {
                g.remove(&key);
            }
        }
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
                tracing::warn!(path = %path.display(), error = %e, "conversations not saved");
            }
        }
    }

    /// The flow whose conversation `<instance>/<session_id>` is, if it is one (`""`: the general one).
    pub fn flow_of_session(&self, instance: &str, session_id: &str) -> Option<String> {
        let name = format!("{instance}/{session_id}");
        lock(&self.flow_sessions).iter().find(|(_, s)| **s == name).map(|(f, _)| f.clone())
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
            tracing::warn!(path = %path.display(), error = %e, "instance ledger not saved");
        }
    }

    /// Record an instance the extension makes for `run` under its own name.
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
        let names: Vec<String> = g.iter().filter(|(_, r)| *r == run).map(|(i, _)| i.clone()).collect();
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

    /// The flow a conversation works on: a flow's own conversation works on that flow, for
    /// good; another one on what was set for it.
    pub fn focus(&self, instance: &str, session_id: &str) -> Option<String> {
        if let Some(f) = self.flow_of_session(instance, session_id) {
            return (!f.is_empty()).then_some(f);
        }
        lock(&self.focus).get(&(instance.to_string(), session_id.to_string())).map(|(f, _)| f.clone())
    }

    /// Set (or, with `None`, clear) the flow a conversation works on. Past [`FOCUS_KEPT`]
    /// conversations the one set longest ago is forgotten.
    pub fn set_focus(&self, instance: &str, session_id: &str, flow: Option<String>) {
        let key = (instance.to_string(), session_id.to_string());
        let mut g = lock(&self.focus);
        match flow {
            Some(f) => {
                let seq = self.focus_seq.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                g.insert(key, (f, seq));
                while g.len() > FOCUS_KEPT {
                    let Some(oldest) = g.iter().min_by_key(|(_, (_, s))| *s).map(|(k, _)| k.clone()) else {
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
        lock(&self.focus).iter().filter(|(_, (f, _))| *f == flow).map(|(k, _)| k.clone()).collect()
    }

    #[cfg(test)]
    pub fn focus_len(&self) -> usize {
        lock(&self.focus).len()
    }
}

/// A project root a device or a conversation gives: an existing absolute directory, as given
/// but for surrounding space and a trailing `/`.
pub(crate) use crate::author::project_root;

/// How a session is written in a run record: `<instance>/<session_id>`.
pub fn session_name(s: &harnix_ext::SessionRef) -> String {
    format!("{}/{}", s.instance, s.session_id)
}

/// The session a run record names. A bare id is on `main`.
pub fn session_ref(name: &str) -> harnix_ext::SessionRef {
    match name.split_once('/') {
        Some((instance, id)) if !instance.is_empty() && !id.is_empty() => harnix_ext::SessionRef::new(instance, id),
        _ => harnix_ext::SessionRef::new(super::FLOW_INSTANCE, name),
    }
}

#[cfg(test)]
impl Shared {
    /// What the extension shares before it starts, given nothing. A test sets the engine; there is
    /// no context and no runner.
    pub(crate) fn for_tests() -> Self {
        Shared::new(Config {
            space: "flow".into(),
            instance: "main".into(),
            scenes: vec![],
            main_scene: None,
            project_scenes: Default::default(),
            conversation_recording: None,
            ensure_ms: 60_000,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shared() -> Shared {
        Shared::for_tests()
    }

    #[test]
    fn a_conversation_remembers_its_flow_until_it_is_cleared() {
        let sh = shared();
        assert_eq!(sh.focus("main", "s1"), None);
        sh.set_focus("main", "s1", Some("demo".into()));
        sh.set_focus("main", "s2", Some("other".into()));
        assert_eq!(sh.focus("main", "s1").as_deref(), Some("demo"));
        assert_eq!(sh.focus("side", "s1"), None, "per instance and session");
        assert_eq!(sh.focused_on("demo"), [("main".to_string(), "s1".to_string())]);
        sh.set_focus("main", "s1", None);
        assert_eq!(sh.focus("main", "s1"), None);
        assert!(super::super::NO_FOCUS.contains("select"));
        for i in 0..FOCUS_KEPT + 10 {
            sh.set_focus("main", &format!("many-{i}"), Some("demo".into()));
        }
        assert_eq!(sh.focus_len(), FOCUS_KEPT, "bounded");
        assert_eq!(sh.focus("main", "s2"), None, "the oldest went first");
        assert_eq!(sh.focus("main", &format!("many-{}", FOCUS_KEPT + 9)).as_deref(), Some("demo"));
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
        assert_eq!(again.instance_runs(), ["r-1", "r-2"], "it survives a restart");
        assert_eq!(again.take_instances("r-1"), ["flow-r-1-aaaa", "flow-r-1-bbbb"]);
        let third = shared();
        third.load_ledger(path);
        assert_eq!(third.instance_runs(), ["r-2"]);
        assert!(third.take_instances("r-1").is_empty());
    }

    #[test]
    fn session_names_round_trip() {
        let s = harnix_ext::SessionRef::new("main", "abc");
        assert_eq!(session_name(&s), "main/abc");
        assert_eq!(session_ref("main/abc"), s);
        assert_eq!(session_ref("abc"), s, "a bare id is on main");
    }

    #[test]
    fn a_project_root_is_an_existing_absolute_directory() {
        let root = tempfile::tempdir().unwrap();
        let at = root.path().to_str().unwrap();
        assert_eq!(project_root(&format!(" {at}/ ")).unwrap(), at);
        assert!(project_root("relative").is_err());
        assert!(project_root(&format!("{at}/missing")).is_err());
    }
}
