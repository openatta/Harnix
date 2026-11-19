//! The runner: carrying out what the kernel hands back, in sessions a host provides.
//!
//! The kernel decides what a run needs next and says so in an `Outcome`'s work items; the runner
//! does it — opens a session where a station says, tells it the protocol layer, the rules and the
//! skill, takes what it hands in, answers its permission questions from the task's permissions,
//! runs gates, posts hooks, projects the run into its project, makes and ends worktrees — and
//! hands the result back to the kernel. It knows sessions only through [`SessionHost`]
//! (`host.rs`): the host opens them, sends to them, streams their frames and answers their
//! questions in the runner's words.
//!
//! ```text
//! host.rs      SessionHost and what goes over it: OpenSpec, Frame, TurnEnd, Decision, HostError
//! state.rs     what the runner's tasks share: bindings, claims in hand, session locks, watches
//! dispatch.rs  carrying out an Outcome: events, changes, hooks, and every work item handed on
//! session.rs   a session task: opening (or finding) and binding its session, carrying the task
//! turns.rs     a session task's turns: sending, judging, correcting, nudging, interrupting
//! main.rs      the run's main session, watched: questions answered, usage counted, sub-agents noted
//! gate.rs      a gate's work item; `command` gates
//! hooks.rs     webhooks: queues, retries, signatures; the transport a host may replace
//! project.rs   a run's output directory and process directory in its project; starting a run
//! worktree.rs  a run in a worktree of its project: made at start, merged, kept or discarded after
//! git.rs       the git command line worktree.rs works through
//! trust.rs     a project's own packs: whether the project is trusted, as the host says
//! tools.rs     the flow's three tools: flow_state, flow_submit, flow_propose
//! testing.rs   ScriptedHost: a host played from a script, for tests and flowctl
//! ```
//!
//! What a task may do is judged call by call: a flow session asks about every guarded tool and
//! every tool a permission names, and the runner answers from the permissions of the task the turn
//! works on. A station's output contract is the session's output schema when it hands in by
//! structured output.

pub mod host;
pub mod testing;

mod dispatch;
mod gate;
mod git;
mod hooks;
mod main;
mod project;
mod session;
mod state;
mod tools;
mod trust;
mod turns;
mod worktree;

use std::collections::BTreeSet;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::Duration;

use serde_json::Value;
use tokio::sync::broadcast;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use crate::kernel::engine::{CreateIn, Engine, DEFAULT_LEASE_MS};
use crate::kernel::host::{Catalog, Event, Gates, Origin, Outcome, Packs, Programs, WorkItem};
use crate::kernel::layout::ProjectLayout;
use crate::kernel::run::{Actor, RunRecord, WorktreeEnd};
use crate::packs::{MemPacks, ProjectPacks, ProjectTrust};

pub use host::{
    Decision, Frame, HostError, Note, OpenSpec, Opened, SessionHost, SessionId, SessionInfo, ToolSpec, TurnEnd, Watch,
};
pub use hooks::{HookTransport, QUEUE_KEPT};
pub use state::{Binding, TaskKey};
pub use testing::{ScriptedHost, Turn};
pub use hooks::redact_url;
pub use tools::{tool_specs, FlowTools, ToolReply, TOOL_PROPOSE, TOOL_STATE, TOOL_SUBMIT};
pub use trust::withheld;
pub use turns::{NUDGE, NUDGE_STRUCTURED};
pub use worktree::{info as worktree_info, EndError};

#[cfg(feature = "webhooks")]
pub use hooks::Webhooks;

/// The tools that change something outside the conversation, as the engine Harnix runs on names
/// them: the shell, the file editors, the notebook editor, the interpreters. What a flow task is
/// refused unless a permission allows it, when the host names nothing else.
pub const DEFAULT_GUARDED: &[&str] = &["Bash", "PowerShell", "REPL", "Write", "Edit", "NotebookEdit"];

/// The recording a session task's session replays, by its work item (hosts that replay).
pub type RecordingFn = Arc<dyn Fn(&WorkItem) -> Option<String> + Send + Sync>;
/// The recording a run's main session replays, by the run.
pub type MainRecordingFn = Arc<dyn Fn(&RunRecord) -> Option<String> + Send + Sync>;

/// A lock, taken whatever a thread that panicked holding it left: what is under these is written
/// whole or not at all.
pub(crate) fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// How a runner is set up: what the host offers sessions, and what it hands in.
pub struct RunnerConfig {
    /// The profiles (scene names) a station's session may be opened on: what definitions write
    /// (`flow-harnix`…), which the catalog offers.
    pub profiles: Vec<String>,
    /// Those of them whose sessions hand in with `flow_submit` rather than structured output.
    pub submit_profiles: BTreeSet<String>,
    /// The profile a run's main session is opened on when the flow's `main` names none.
    pub main_profile: Option<String>,
    /// The profiles whose sessions need a project: a session on one for a run of no project works
    /// in a directory of the run's own (`Engine::with_workspaces`).
    pub project_profiles: BTreeSet<String>,
    /// The tools refused to a task unless a permission allows them ([`DEFAULT_GUARDED`]).
    pub guarded: Vec<String>,
    /// Whether `command` gates may run: they run a command on this machine.
    pub command_gates: bool,
    /// The lease the runner holds on the tasks it carries out (renewed while it works).
    pub lease_ms: u64,
    /// How often expired leases are looked for.
    pub tick_ms: u64,
    /// The prefix of the HTTP headers a webhook delivery carries (`<prefix>-Event`,
    /// `-Delivery`, `-Timestamp`, `-Signature`); Harnix sends `X-Harnix`.
    pub hook_headers: String,
    pub recording: Option<RecordingFn>,
    pub main_recording: Option<MainRecordingFn>,
    /// The application's code `kind: program` tasks run.
    pub programs: Option<Arc<dyn Programs>>,
    /// The application's own checks `host` gates name.
    pub gates: Option<Arc<dyn Gates>>,
    /// How webhooks go out; [`Webhooks`] (feature `webhooks`) when none is given and the feature
    /// is on, else no hook is delivered.
    pub hook_transport: Option<Arc<dyn HookTransport>>,
    /// Whether projects are trusted, as last heard: shared with the packs the engine reads
    /// ([`CatalogPacks`]), so a project's own flows are read only while it is.
    pub trust: Arc<ProjectTrust>,
}

impl Default for RunnerConfig {
    fn default() -> Self {
        Self {
            profiles: vec![],
            submit_profiles: BTreeSet::new(),
            main_profile: None,
            project_profiles: BTreeSet::new(),
            guarded: DEFAULT_GUARDED.iter().map(|s| s.to_string()).collect(),
            command_gates: false,
            lease_ms: DEFAULT_LEASE_MS,
            tick_ms: 1000,
            hook_headers: "X-Flow".into(),
            recording: None,
            main_recording: None,
            programs: None,
            gates: None,
            hook_transport: None,
            trust: Arc::default(),
        }
    }
}

/// Runs whose records changed, for a host that publishes them: after every outcome carried out.
/// `substantive` is false after bookkeeping alone (a lease moved on), when no run's summary
/// changed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Changed {
    pub runs: BTreeSet<String>,
    pub substantive: bool,
}

/// How many events and changes a slow subscriber may fall behind by.
const EVENTS_KEPT: usize = 1024;

/// The runner. Build it with [`Runner::new`], then [`Runner::start`].
pub struct Runner {
    cfg: RunnerConfig,
    engine: Arc<Engine>,
    host: Arc<dyn SessionHost>,
    events: broadcast::Sender<Event>,
    changes: broadcast::Sender<Changed>,
    state: state::State,
    hooks: hooks::Hooks,
    tools: OnceLock<FlowTools>,
    /// Every task the runner starts, so stopping can wait for them; and what tells them to stop.
    /// A run is not cancelled by either — `recover` takes up its work at the next start — the
    /// tasks only stop waiting on it.
    tasks: TaskTracker,
    cancel: CancellationToken,
    tick: Mutex<Option<JoinHandle<()>>>,
}

impl Runner {
    /// A runner over `engine`, its sessions from `host`. Nothing runs until [`Runner::start`].
    pub fn new(engine: Arc<Engine>, host: Arc<dyn SessionHost>, cfg: RunnerConfig) -> Arc<Self> {
        let (events, _) = broadcast::channel(EVENTS_KEPT);
        let (changes, _) = broadcast::channel(EVENTS_KEPT);
        let hooks = hooks::Hooks::new(cfg.hook_transport.clone());
        let rn = Arc::new(Self {
            cfg,
            engine,
            host,
            events,
            changes,
            state: state::State::default(),
            hooks,
            tools: OnceLock::new(),
            tasks: TaskTracker::new(),
            cancel: CancellationToken::new(),
            tick: Mutex::new(None),
        });
        let _ = rn.tools.set(FlowTools::new(Arc::downgrade(&rn)));
        rn
    }

    /// Take up the work that was handed out and not answered (`recover`), open or watch again
    /// every active run's main session, and start looking for expired leases.
    pub async fn start(self: &Arc<Self>) {
        let rn = self.clone();
        self.spawn(async move {
            // Runs in projects read their project's packs as they go: its trust first.
            trust::hear_active(&rn).await;
            dispatch::recover(&rn).await;
        });
        let rn = self.clone();
        let tick = self.spawn(async move { dispatch::tick_loop(rn).await });
        *lock(&self.tick) = Some(tick);
    }

    /// Tell every task to stop waiting, and wait for them up to `grace`. True when all of them
    /// did. Runs are not cancelled; the next start recovers their work.
    pub async fn stop(&self, grace: Duration) -> bool {
        if let Some(t) = lock(&self.tick).take() {
            t.abort();
        }
        self.cancel.cancel();
        self.tasks.close();
        tokio::time::timeout(grace, self.tasks.wait()).await.is_ok()
    }

    /// Carry out what an operation produced: broadcast its events, project the runs it touched
    /// and tell the host they changed, notify the hooks, start its work.
    pub async fn apply(self: &Arc<Self>, outcome: Outcome) {
        dispatch::apply(self, None, outcome).await;
    }

    /// [`Runner::apply`], with `run` among the runs told changed whatever the outcome names (an
    /// operation whose value names no run: a claim, a heartbeat).
    pub async fn apply_for(self: &Arc<Self>, run: &str, outcome: Outcome) {
        dispatch::apply(self, Some(run), outcome).await;
    }

    /// [`Runner::apply`] for the result of an engine call, passing the value through.
    pub async fn settle(self: &Arc<Self>, result: crate::Result<Outcome>) -> crate::Result<Value> {
        let o = result?;
        let value = o.value.clone();
        self.apply(o).await;
        Ok(value)
    }

    /// Start a run: who may; the project's trust heard; its worktree made when it works in one;
    /// its output directory resolved and made; the run created; what a new run needs on disk.
    /// What was made for a run that is then not started is taken back. The outcome is not yet
    /// carried out: [`Runner::apply`] it (or [`Runner::settle`] the result).
    pub async fn start_run(self: &Arc<Self>, input: CreateIn, by: Actor) -> crate::Result<Outcome> {
        project::start(self, input, by).await
    }

    /// Decide a finished run's worktree: merged into the project's current branch, kept as a
    /// branch, or discarded; recorded on the run. The outcome is not yet carried out.
    pub async fn end_worktree(&self, run: &str, how: WorktreeEnd, by: Actor) -> Result<Outcome, EndError> {
        worktree::end(&self.engine, run, how, by).await
    }

    /// Open the run's main session if it has none (or the one it had is gone), and watch it.
    pub async fn ensure_main(self: &Arc<Self>, run: &str) -> Result<SessionId, String> {
        main::ensure_main(self, run).await
    }

    /// Every kernel event, as committed, from here on.
    pub fn events(&self) -> broadcast::Receiver<Event> {
        self.events.subscribe()
    }

    /// Which runs changed, after each outcome carried out, from here on.
    pub fn changes(&self) -> broadcast::Receiver<Changed> {
        self.changes.subscribe()
    }

    /// The flow's tools, for the host to register to its sessions and route calls to.
    pub fn tools(&self) -> &FlowTools {
        self.tools.get().expect("set when the runner was made")
    }

    /// The host heard the project at `root` became trusted or not: what runs there read next is
    /// the project's own packs, or not.
    pub fn set_trust(&self, root: &Path, trusted: bool) {
        self.cfg.trust.set(&root.to_string_lossy(), trusted);
    }

    /// Ask the host whether `root` is trusted, and keep the answer for the kernel's reads.
    pub async fn hear(&self, root: &str) -> bool {
        trust::hear(self, root).await
    }

    pub fn engine(&self) -> &Arc<Engine> {
        &self.engine
    }

    pub fn host(&self) -> &Arc<dyn SessionHost> {
        &self.host
    }

    pub fn config(&self) -> &RunnerConfig {
        &self.cfg
    }

    /// Whether projects are trusted, as last heard.
    pub fn trust(&self) -> &Arc<ProjectTrust> {
        &self.cfg.trust
    }

    /// The hooks of the pack with each one's last delivery.
    pub fn hook_status(&self) -> Vec<Value> {
        self.hooks.listing(&self.engine.packs().hooks())
    }

    /// The sub-agents `run`'s main session started, as its watch saw them: `{agent, agent_type,
    /// call, description, flow: {station, task}?, status}` each.
    pub fn agents_of(&self, run: &str) -> Vec<Value> {
        self.state.agents(run)
    }

    /// The run whose main session `id` is: the one watching it while it runs, else the newest
    /// run whose record names it; `None` for a session that is no run's main.
    pub async fn run_of_session(&self, id: &SessionId) -> Option<String> {
        main::run_of_main(self, id).await
    }

    /// The task `id` is working on, if it is.
    pub fn binding(&self, id: &SessionId) -> Option<Binding> {
        self.state.binding(id)
    }

    /// Whether the runner is stopping: its tasks stop waiting.
    pub fn stopping(&self) -> bool {
        self.cancel.is_cancelled()
    }

    /// What tells the runner's tasks to stop.
    pub(crate) fn cancelled(&self) -> &CancellationToken {
        &self.cancel
    }

    /// Start one of the runner's tasks: tracked, so [`Runner::stop`] waits for it.
    pub(crate) fn spawn<F>(&self, task: F) -> JoinHandle<()>
    where
        F: std::future::Future<Output = ()> + Send + 'static,
    {
        self.tasks.spawn(task)
    }

    /// How many tasks are still running.
    pub fn tasks_left(&self) -> usize {
        self.tasks.len()
    }

    /// Tell the host which runs changed, their bindings pruned and their process directories
    /// written (not after bookkeeping, which changes nothing shown there).
    pub(crate) async fn changed(&self, runs: BTreeSet<String>, substantive: bool) {
        for run in &runs {
            if let Ok(rec) = self.engine.record(run).await {
                self.state.prune_bindings(run, |b| state::binding_current(&rec, b));
                if substantive {
                    project::project_run(&self.engine, &rec);
                }
            }
        }
        if !runs.is_empty() {
            let _ = self.changes.send(Changed { runs, substantive });
        }
    }
}

#[cfg(test)]
impl Runner {
    /// A runner for a test: the engine and the host given, a minute's lease, nothing else.
    pub(crate) fn for_tests(engine: Arc<Engine>, host: Arc<dyn SessionHost>) -> Arc<Self> {
        Self::new(engine, host, RunnerConfig { lease_ms: 60_000, guarded: vec![], ..Default::default() })
    }

    pub(crate) fn for_tests_with(engine: Arc<Engine>, host: Arc<dyn SessionHost>, cfg: RunnerConfig) -> Arc<Self> {
        Self::new(engine, host, cfg)
    }
}

/// The host's packs with what the runner adds to the catalog: the profiles sessions may be opened
/// on (which hand in with `flow_submit`, which is the main one), the guarded tools, whether
/// command gates run, and — when the host lists templates at all — its own templates. A project's
/// view (`Packs::project`) is the project's own packs over these, read only while the project is
/// trusted as last heard.
pub struct CatalogPacks {
    inner: OnceLock<Arc<dyn Packs>>,
    profiles: Vec<String>,
    templates: Vec<String>,
    guarded: Vec<String>,
    main_profile: Option<String>,
    command_gates: bool,
    /// Whose trust a project's own packs wait on; `None` for a project's view itself.
    trust: Option<Arc<ProjectTrust>>,
    /// Where a project's own packs are, once known.
    layout: OnceLock<ProjectLayout>,
}

impl CatalogPacks {
    /// Over packs given later ([`CatalogPacks::set_inner`]): empty until then. `templates` are
    /// the host's own, which stations may name.
    pub fn new(cfg: &RunnerConfig, templates: Vec<String>) -> Self {
        Self {
            inner: OnceLock::new(),
            profiles: cfg.profiles.clone(),
            templates,
            guarded: cfg.guarded.clone(),
            main_profile: cfg.main_profile.clone(),
            command_gates: cfg.command_gates,
            trust: Some(cfg.trust.clone()),
            layout: OnceLock::new(),
        }
    }

    /// [`CatalogPacks::new`] over `packs`.
    pub fn over(cfg: &RunnerConfig, templates: Vec<String>, packs: Arc<dyn Packs>) -> Self {
        let c = Self::new(cfg, templates);
        let _ = c.inner.set(packs);
        c
    }

    /// The packs underneath, once; a second call changes nothing.
    pub fn set_inner(&self, packs: Arc<dyn Packs>) {
        let _ = self.inner.set(packs);
    }

    /// Where projects keep their own packs, once; without it no project has a view.
    pub fn set_layout(&self, layout: ProjectLayout) {
        let _ = self.layout.set(layout);
    }

    fn inner(&self) -> &dyn Packs {
        static EMPTY: OnceLock<MemPacks> = OnceLock::new();
        match self.inner.get() {
            Some(p) => p.as_ref(),
            None => EMPTY.get_or_init(MemPacks::new),
        }
    }
}

impl Packs for CatalogPacks {
    fn flows(&self) -> Vec<String> {
        self.inner().flows()
    }
    fn flow(&self, name: &str) -> Option<(crate::FlowDef, String)> {
        self.inner().flow(name)
    }
    fn part(&self, kind: crate::PartKind, name: &str) -> Option<(String, String)> {
        self.inner().part(kind, name)
    }
    fn parts(&self, kind: crate::PartKind) -> Vec<String> {
        self.inner().parts(kind)
    }
    fn catalog(&self) -> Catalog {
        let mut c = self.inner().catalog();
        c.command_gates = self.command_gates;
        c.scenes.extend(self.profiles.iter().cloned());
        if !c.templates.is_empty() {
            c.templates.extend(self.templates.iter().cloned());
        }
        c.guarded = self.guarded.clone();
        c.submit_scenes = self.profiles.iter().cloned().collect();
        if c.main_scene.is_none() {
            c.main_scene = self.main_profile.clone();
        }
        c
    }
    fn flow_text(&self, name: &str) -> Option<String> {
        self.inner().flow_text(name)
    }
    fn writable(&self) -> bool {
        self.inner().writable()
    }
    fn origin(&self, flow: &str) -> Origin {
        self.inner().origin(flow)
    }
    fn save_flow(&self, yaml: &str) -> Result<String, String> {
        self.inner().save_flow(yaml)
    }
    fn delete_flow(&self, name: &str) -> Result<(), String> {
        self.inner().delete_flow(name)
    }
    fn part_origin(&self, kind: crate::PartKind, name: &str) -> Origin {
        self.inner().part_origin(kind, name)
    }
    fn source(&self, flow: &str) -> Option<String> {
        self.inner().source(flow)
    }
    fn part_source(&self, kind: crate::PartKind, name: &str) -> Option<String> {
        self.inner().part_source(kind, name)
    }
    fn save_part(&self, kind: crate::PartKind, name: &str, text: &str) -> Result<String, String> {
        self.inner().save_part(kind, name, text)
    }
    fn delete_part(&self, kind: crate::PartKind, name: &str) -> Result<(), String> {
        self.inner().delete_part(kind, name)
    }
    /// The project's own packs over the host's, with the same additions to the catalog; read only
    /// while the project is trusted, as last heard.
    fn project(&self, root: &str) -> Option<Arc<dyn Packs>> {
        let trust = self.trust.as_ref()?;
        let base = self.inner.get()?.clone();
        let view = ProjectPacks::new(base, root, self.layout.get()?, trust.trusted(root));
        let inner = OnceLock::new();
        let _ = inner.set(Arc::new(view) as Arc<dyn Packs>);
        Some(Arc::new(CatalogPacks {
            inner,
            profiles: self.profiles.clone(),
            templates: self.templates.clone(),
            guarded: self.guarded.clone(),
            main_profile: self.main_profile.clone(),
            command_gates: self.command_gates,
            trust: None,
            layout: OnceLock::new(),
        }))
    }
    fn withheld_flows(&self) -> Vec<(String, crate::Withheld)> {
        self.inner().withheld_flows()
    }
}

/// A `Weak<Runner>` upgraded, for what the runner's tools hold.
pub(crate) fn upgrade(w: &Weak<Runner>) -> Option<Arc<Runner>> {
    w.upgrade()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MemStore;

    fn cfg(profiles: &[&str]) -> RunnerConfig {
        RunnerConfig { profiles: profiles.iter().map(|s| s.to_string()).collect(), guarded: vec![], ..Default::default() }
    }

    fn catalog_packs(inner: MemPacks, templates: &[&str]) -> CatalogPacks {
        CatalogPacks::over(&cfg(&["flow-harnix"]), templates.iter().map(|s| s.to_string()).collect(), Arc::new(inner))
    }

    #[test]
    fn the_catalog_has_the_hosts_scenes_and_the_profiles() {
        let packs = catalog_packs(MemPacks::new().scene("harnix").skill("work", "Do it."), &["flow.local"]);
        let yaml = |scene: &str| {
            format!(
                "name: f\nstations: [{{id: a, executor: {{kind: session, scene: {scene}}}, skill: work, output: {{type: object}}, exit: [{{kind: output}}]}}]"
            )
        };
        assert!(crate::validate_yaml(&yaml("flow-harnix"), &packs).is_empty());
        assert!(crate::validate_yaml(&yaml("harnix"), &packs).is_empty());
        let problems = crate::validate_yaml(&yaml("nowhere"), &packs);
        assert!(problems.iter().any(|p| p.message.contains("scene nowhere")), "{problems:?}");
        assert_eq!(packs.catalog().scenes.into_iter().collect::<Vec<_>>(), ["flow-harnix", "harnix"]);
        assert!(packs.catalog().templates.is_empty(), "templates the host does not list are not checked");
        let listed = catalog_packs(MemPacks::new().template("acme.box"), &["flow.local"]);
        assert_eq!(listed.catalog().templates.into_iter().collect::<Vec<_>>(), ["acme.box", "flow.local"]);
    }

    #[test]
    fn packs_given_later_are_empty_until_then_and_writable_after() {
        let packs = CatalogPacks::new(&cfg(&[]), vec![]);
        assert!(packs.flows().is_empty() && !packs.writable());
        packs.set_inner(Arc::new(MemPacks::new().writable()));
        assert!(packs.writable());
        packs.save_flow("name: x\nstations: [{id: a, exit: [{kind: confirm}]}]").unwrap();
        assert_eq!(packs.origin("x"), Origin::User);
    }

    /// Stopping waits for the runner's tasks, and each of them stops waiting on what it waited
    /// for — here a turn that never settles — rather than on its next timer.
    #[tokio::test]
    async fn stopping_ends_every_task_and_waits_for_them() {
        let engine = Arc::new(Engine::new(Arc::new(MemStore::new()), Arc::new(MemPacks::new())));
        let rn = Runner::for_tests(engine.clone(), ScriptedHost::new());
        for _ in 0..3 {
            let rn2 = rn.clone();
            rn.spawn(async move {
                tokio::select! {
                    _ = std::future::pending::<()>() => {}
                    _ = rn2.cancelled().cancelled() => {}
                }
            });
        }
        let started = std::time::Instant::now();
        assert!(rn.stop(Duration::from_secs(5)).await);
        assert!(started.elapsed() < Duration::from_secs(1));
        assert!(rn.stopping());

        // One that will not stop is waited for only so long.
        let stuck = Runner::for_tests(engine, ScriptedHost::new());
        stuck.spawn(std::future::pending());
        assert!(!stuck.stop(Duration::from_millis(100)).await);
        assert_eq!(stuck.tasks_left(), 1);
    }
}
