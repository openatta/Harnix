//! The Harnix extension: the flow library's runner wired to Harnix's sessions, the application's
//! code, native tools, methods and topics.
//!
//! ```text
//! mod.rs       Flow, its builder, FlowHandle; the Extension impl (spaces, engine, start, stop)
//! state.rs     what the extension's methods, scenes, topics and host share
//! host.rs      HarnixSessionHost: the runner's sessions, through the hub
//! scenes.rs    the scene wrappers: flow-* (flow_state, flow_submit, flow_propose)
//! author.rs    flow-author: writing flows in conversation
//! methods.rs   flow.* — the device methods
//! publish.rs   the topics: `run`, `runs`, `defs`, `focus`, `dry_runs`
//! ```
//!
//! What runs a run is the library's `Runner` (`harnix_flow::runner`): it opens sessions where a
//! station places them, judges each tool call of a task against its permissions, takes the hand-in,
//! runs gates, posts hooks, projects the run into its project. This extension gives it Harnix's
//! sessions (`host.rs`), the application's packs, programs and gates, and tells Harnix what changed.
//!
//! An application registers it like any extension:
//!
//! ```no_run
//! # use std::sync::Arc;
//! # use harnix_ext_flow::{Flow, MemPacks};
//! # use harnix_ext::engine::SceneBase;
//! let flow = Flow::builder()
//!     .packs(MemPacks::new().flow("name: demo\nstations: [{id: a, exit: [{kind: confirm}]}]"))
//!     .scene(SceneBase::builtin("harnix"))   // offered to definitions as `flow-harnix`
//!     .build();
//! let handle = flow.handle();  // the kernel from Rust, once the extension has started
//! # let _ = (flow, handle);
//! ```

mod author;
mod host;
mod methods;
mod publish;
mod scenes;
mod state;

use std::path::Path;
use std::sync::{Arc, Mutex};

use harnix_ext::engine::{InstanceTemplate, SceneBase, SceneLayer};
use harnix_ext::{caps, catalog, Extension, ExtensionContext, Manifest, Registrar, SpaceDecl};
use serde_json::json;
use tokio::sync::broadcast;

use crate::dryrun::{Sandbox, SandboxConfig};
use crate::kernel::engine::{Engine, DEFAULT_LEASE_MS};
use crate::kernel::host::{Authorizer, Event, Gates, Outcome, Packs, Programs, RunStore, Tools, WorkItem};
use crate::kernel::FlowError;
use crate::runner::{CatalogPacks, EndError, MainRecordingFn, RecordingFn, Runner, RunnerConfig};
use crate::store::FileStore;

pub use crate::kernel::protocol::matches_any;
pub use crate::runner::DEFAULT_GUARDED;
pub use author::{AUTHOR_PROMPT, AUTHOR_SCENE, AUTHOR_TOOLS, NO_FOCUS};
pub use scenes::{TOOL_PROPOSE, TOOL_STATE, TOOL_SUBMIT};
pub use state::{session_name, session_ref};

use state::{Config, ConversationRecordingFn, Shared};

/// The extension's id: methods and topics are `flow.*`, its templates `flow.<id>`.
pub const MODULE_ID: &str = "flow";

/// What a run's worktree branch is named under in Harnix: `harnix/flow/<run>`, apart from the
/// engine's own `harnix/worktree-*`. A branded product's is `<its lowercase name>/flow`.
pub const BRANCH_PREFIX: &str = "harnix/flow";

/// The directories of a project's data directory (`.harnix/`) the engine's sandbox lets a session
/// write: a task's own there, where the rest of the directory is the application's.
pub const AGENT_DIRS: &[&str] = &["scratchpad", "memory", "worktrees"];

/// The prefix of the HTTP headers a webhook delivery carries: `X-Harnix-Event`, `-Delivery`,
/// `-Timestamp`, `-Signature`.
pub const HOOK_HEADERS: &str = "X-Harnix";

/// Where the extension keeps what it writes for a project, from the product's names and its data
/// directory: the project's data directory (`.harnix/`, a branded product's `.<product_id>/`,
/// shared with the engine), runs' worktrees in `<data>/worktrees/<run>` on `<product>/flow/<run>`
/// ([`BRANCH_PREFIX`] in Harnix). An extension is handed only its own directory, so the worktrees
/// are there rather than beside the engine's state.
pub(crate) fn project_layout(names: &harnix_ext::engine::ProductNames, data_dir: &Path) -> crate::ProjectLayout {
    crate::ProjectLayout::new(names.project_dir(), data_dir.join("worktrees"), format!("{}/flow", names.product_id()))
        .with_agent_dirs(AGENT_DIRS.iter().copied())
}

/// Harnix's own layout, with no brand: what the tests take.
#[cfg(test)]
pub(crate) fn layout(data_dir: &Path) -> crate::ProjectLayout {
    project_layout(&Default::default(), data_dir)
}

/// `spec`, with `set` applied when there is a value to set.
pub(crate) fn spec_with<T>(
    spec: harnix_ext::SessionSpec,
    value: Option<T>,
    set: impl FnOnce(harnix_ext::SessionSpec, T) -> harnix_ext::SessionSpec,
) -> harnix_ext::SessionSpec {
    match value {
        Some(value) => set(spec, value),
        None => spec,
    }
}

/// A lock, taken whatever a thread that panicked holding it left: what is under these is written
/// whole or not at all.
pub(crate) fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// The instance the flow space's sessions run on, and where a station's session goes when it
/// names no instance or template and stays in the flow's space. `Extension::register` contributes
/// to it alone, so the wrapped scenes and `flow-author` exist only there.
pub const FLOW_INSTANCE: &str = "main";

type MakeStore = Box<dyn FnOnce(&Path) -> Arc<dyn RunStore> + Send>;
type MakePacks = Box<dyn FnOnce(&Path) -> Arc<dyn Packs> + Send>;

/// Where runs are stored: given, or made from the extension's data directory at start.
enum StoreSpec {
    Ready(Arc<dyn RunStore>),
    Lazy(MakeStore),
}

/// What the runner is built with at start, kept from the builder until then.
struct RunnerParts {
    programs: Option<Arc<dyn Programs>>,
    gates: Option<Arc<dyn Gates>>,
    command_gates: bool,
    guarded: Vec<String>,
    recording: Option<RecordingFn>,
    main_recording: Option<MainRecordingFn>,
    lease_ms: u64,
    tick_ms: u64,
}

impl RunnerParts {
    fn config(&self, sh: &Shared) -> RunnerConfig {
        RunnerConfig {
            profiles: sh.cfg.scenes.clone(),
            submit_profiles: sh.cfg.scenes.iter().cloned().collect(),
            main_profile: sh.cfg.main_scene.clone(),
            project_profiles: sh.cfg.project_scenes.clone(),
            guarded: self.guarded.clone(),
            command_gates: self.command_gates,
            lease_ms: self.lease_ms,
            tick_ms: self.tick_ms,
            hook_headers: HOOK_HEADERS.into(),
            recording: self.recording.clone(),
            main_recording: self.main_recording.clone(),
            programs: self.programs.clone(),
            gates: self.gates.clone(),
            hook_transport: None,
            trust: sh.projects.clone(),
        }
    }
}

/// The flow extension. Build it with [`Flow::builder`], register it with the application.
pub struct Flow {
    sh: Arc<Shared>,
    store: Mutex<Option<StoreSpec>>,
    packs: Arc<CatalogPacks>,
    lazy_packs: Mutex<Option<MakePacks>>,
    tools: Option<Arc<dyn Tools>>,
    auth: Option<Arc<dyn Authorizer>>,
    scenes: Vec<SceneLayer>,
    author: SceneLayer,
    templates: Vec<InstanceTemplate>,
    runner: Mutex<Option<RunnerParts>>,
    /// Every kernel event, for [`FlowHandle::subscribe`] before and after the runner starts.
    events: broadcast::Sender<Event>,
    /// Why the extension must not start: a guarded-tool pattern no permission rule can say.
    config_error: Option<String>,
}

impl Flow {
    pub fn builder() -> FlowBuilder {
        FlowBuilder::default()
    }

    /// The kernel from Rust, for the application.
    pub fn handle(&self) -> FlowHandle {
        FlowHandle { sh: self.sh.clone(), events: self.events.clone() }
    }
}

/// [`Flow`], from parts.
pub struct FlowBuilder {
    store: Option<StoreSpec>,
    packs: Option<Arc<dyn Packs>>,
    lazy_packs: Option<MakePacks>,
    /// Each base, and whether its sessions need a project (`None`: as the base says).
    scenes: Vec<(SceneBase, Option<bool>)>,
    author: Option<SceneBase>,
    templates: Vec<InstanceTemplate>,
    ensure_ms: u64,
    programs: Option<Arc<dyn Programs>>,
    gates: Option<Arc<dyn Gates>>,
    tools: Option<Arc<dyn Tools>>,
    command_gates: bool,
    auth: Option<Arc<dyn Authorizer>>,
    space: String,
    guarded: Vec<String>,
    recording: Option<RecordingFn>,
    main_recording: Option<MainRecordingFn>,
    conversation_recording: Option<ConversationRecordingFn>,
    main_scene: Option<String>,
    lease_ms: u64,
    tick_ms: u64,
}

impl Default for FlowBuilder {
    fn default() -> Self {
        Self {
            store: None,
            packs: None,
            lazy_packs: None,
            scenes: vec![],
            author: None,
            templates: vec![],
            ensure_ms: 60_000,
            programs: None,
            gates: None,
            tools: None,
            command_gates: false,
            auth: None,
            space: "flow".into(),
            guarded: DEFAULT_GUARDED.iter().map(|s| s.to_string()).collect(),
            recording: None,
            main_recording: None,
            conversation_recording: None,
            main_scene: None,
            lease_ms: DEFAULT_LEASE_MS,
            tick_ms: 1000,
        }
    }
}

/// The engine's own scenes, the ones a flow offers when the application gives none: `harnix` (the
/// main scene), `code` and `mini`, each with whether its sessions need a project (`code`'s do).
/// Attended, as the wrapper is: the runner answers a flow session's asks in place of a person, so
/// they must reach the host. A flow session lists no questions to put, so it has no
/// `AskUserQuestion`.
pub fn builtin_scenes() -> Vec<(SceneBase, bool)> {
    vec![
        (SceneBase::builtin("harnix"), false),
        (SceneBase::builtin("code"), true),
        (SceneBase::builtin("mini"), false),
    ]
}

impl FlowBuilder {
    /// Where runs live. Without one, a [`FileStore`] in the extension's data directory (`runs/`).
    pub fn store(mut self, store: impl RunStore + 'static) -> Self {
        self.store = Some(StoreSpec::Ready(Arc::new(store)));
        self
    }

    /// A store made at start from the extension's data directory, which is not known before.
    pub fn store_with<S, F>(mut self, make: F) -> Self
    where
        S: RunStore + 'static,
        F: FnOnce(&Path) -> S + Send + 'static,
    {
        self.store = Some(StoreSpec::Lazy(Box::new(move |dir: &Path| Arc::new(make(dir)) as Arc<dyn RunStore>)));
        self
    }

    /// Definitions, skills and the catalog.
    pub fn packs(mut self, packs: impl Packs + 'static) -> Self {
        self.packs = Some(Arc::new(packs));
        self
    }

    pub fn packs_arc(mut self, packs: Arc<dyn Packs>) -> Self {
        self.packs = Some(packs);
        self
    }

    /// Packs made at start from the extension's data directory — a writable layer there, say:
    /// `DirPacks::writable(dir.join("packs")).with_builtin(…)`.
    pub fn packs_with<P, F>(mut self, make: F) -> Self
    where
        P: Packs + 'static,
        F: FnOnce(&Path) -> P + Send + 'static,
    {
        self.lazy_packs = Some(Box::new(move |dir: &Path| Arc::new(make(dir)) as Arc<dyn Packs>));
        self
    }

    /// A scene stations may run sessions in. It is offered as `flow-<id>`: the same scene
    /// with the flow's native tools added (its sessions hand in with `flow_submit`). Repeatable.
    /// Given none, the extension offers the engine's own ones ([`builtin_scenes`]): `flow-harnix`,
    /// `flow-code` and `flow-mini`, with `flow-harnix` the main scene.
    pub fn scene(mut self, base: SceneBase) -> Self {
        self.scenes.push((base, None));
        self
    }

    /// [`FlowBuilder::scene`], for a scene whose sessions work in a project: a run's session in it
    /// gets a workspace directory of the run's own.
    pub fn project_scene(mut self, base: SceneBase) -> Self {
        self.scenes.push((base, Some(true)));
        self
    }

    /// The scene `flow-author` wraps — the flow space's conversations. The engine's harnix scene
    /// when not given.
    pub fn author_scene(mut self, base: SceneBase) -> Self {
        self.author = Some(base);
        self
    }

    /// A template of the extension's own, which stations may name as `flow.<id>` (the hub lets an
    /// extension build only from its own templates and the profile's). Repeatable.
    pub fn template(mut self, template: InstanceTemplate) -> Self {
        self.templates.push(template);
        self
    }

    /// How long a station's template instance may take to be ready; a minute by default.
    pub fn ensure_ms(mut self, ms: u64) -> Self {
        self.ensure_ms = ms.max(1000);
        self
    }

    pub fn programs(mut self, p: impl Programs + 'static) -> Self {
        self.programs = Some(Arc::new(p));
        self
    }

    /// The application's own checks, which `host` gates name (each name also in the catalog's
    /// `gates`).
    pub fn gates(mut self, g: impl Gates + 'static) -> Self {
        self.gates = Some(Arc::new(g));
        self
    }

    /// Read-only application tools `tool` gates call.
    pub fn tools(mut self, t: impl Tools + 'static) -> Self {
        self.tools = Some(Arc::new(t));
        self
    }

    /// Let `command` gates run: a gate of the pack then runs a command on this host, in the run's
    /// workspace by default. Off unless turned on.
    pub fn command_gates(mut self, on: bool) -> Self {
        self.command_gates = on;
        self
    }

    pub fn authorizer(mut self, a: impl Authorizer + 'static) -> Self {
        self.auth = Some(Arc::new(a));
        self
    }

    /// The space flow sessions belong to; `flow` by default.
    pub fn space(mut self, id: impl Into<String>) -> Self {
        self.space = id.into();
        self
    }

    /// The tools refused to a flow task unless one of its permissions allows them: a flow
    /// session asks about each, the runner answers, and the protocol layer names them. Only
    /// these are refused by default — an application whose sessions reach other writing tools
    /// (its own MCP tools, plugin tools) lists them here, by name or as a namespace group
    /// `ns__*`. By default the engine's own: [`DEFAULT_GUARDED`].
    pub fn guarded_tools<I, S>(mut self, patterns: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.guarded = patterns.into_iter().map(Into::into).collect();
        self
    }

    /// The recording a session task's session replays (tests and demos): ignored unless the
    /// process runs in replay.
    pub fn recording(mut self, f: impl Fn(&WorkItem) -> Option<String> + Send + Sync + 'static) -> Self {
        self.recording = Some(Arc::new(f));
        self
    }

    /// The recording a run's main session replays (tests and demos): ignored unless the process
    /// runs in replay. Every task in the main session takes its turns in that one recording.
    pub fn main_recording(mut self, f: impl Fn(&crate::RunRecord) -> Option<String> + Send + Sync + 'static) -> Self {
        self.main_recording = Some(Arc::new(f));
        self
    }

    /// The recording a flow's conversation replays (the flow's name; `""` for the general
    /// conversation) — tests and demos: ignored unless the process runs in replay.
    pub fn conversation_recording(mut self, f: impl Fn(&str) -> Option<String> + Send + Sync + 'static) -> Self {
        self.conversation_recording = Some(Arc::new(f));
        self
    }

    /// The scene (an id given to [`FlowBuilder::scene`], or `harnix`, `code` or `mini` when none
    /// is) a run's main session is opened on when its flow's `main` names none; it is wrapped
    /// (`flow-<id>`). The first scene by default: `harnix` when none is given.
    pub fn main_scene(mut self, id: impl Into<String>) -> Self {
        self.main_scene = Some(id.into());
        self
    }

    /// The lease the runner holds on the tasks it carries out (it renews it while working).
    pub fn lease_ms(mut self, ms: u64) -> Self {
        self.lease_ms = ms.max(1000);
        self
    }

    /// How often expired leases are looked for; 1 s by default.
    pub fn tick_ms(mut self, ms: u64) -> Self {
        self.tick_ms = ms.max(50);
        self
    }

    pub fn build(mut self) -> Flow {
        if self.scenes.is_empty() {
            self.scenes = builtin_scenes().into_iter().map(|(base, project)| (base, Some(project))).collect();
        }
        let (events, _) = broadcast::channel(1024);
        let own_templates: Vec<String> = self.templates.iter().map(|t| format!("{MODULE_ID}.{}", t.id)).collect();
        let sh = Arc::new(Shared::new(Config {
            space: self.space.clone(),
            instance: FLOW_INSTANCE.to_string(),
            scenes: self.scenes.iter().map(|(s, _)| scenes::wrapped_id(s.id())).collect(),
            conversation_recording: self.conversation_recording.clone(),
            main_scene: self
                .main_scene
                .clone()
                .or_else(|| self.scenes.first().map(|(s, _)| s.id().to_string()))
                .map(|id| scenes::wrapped_id(&id)),
            project_scenes: self
                .scenes
                .iter()
                .filter(|(_, project)| *project == Some(true))
                .map(|(s, _)| scenes::wrapped_id(s.id()))
                .collect(),
            ensure_ms: self.ensure_ms,
        }));
        let runner = RunnerParts {
            programs: self.programs.clone(),
            gates: self.gates.clone(),
            command_gates: self.command_gates,
            guarded: self.guarded.clone(),
            recording: self.recording.clone(),
            main_recording: self.main_recording.clone(),
            lease_ms: self.lease_ms,
            tick_ms: self.tick_ms,
        };
        let tools = scenes::FlowTools::new(sh.clone());
        let scenes: Vec<SceneLayer> = self
            .scenes
            .iter()
            .map(|(base, project)| scenes::flow_scene(base.clone(), &tools, *project))
            .collect();
        let base = self.author.unwrap_or_else(|| SceneBase::builtin("harnix"));
        let author = author::author_scene(base, sh.clone());
        let packs = Arc::new(CatalogPacks::new(&runner.config(&sh), own_templates));
        let problems = crate::kernel::protocol::guard_problems(&self.guarded);
        let config_error = (!problems.is_empty()).then(|| {
            format!(
                "the flow extension's guarded tools cannot all be refused by a session's permission rules: {}",
                problems.join("; ")
            )
        });
        if let Some(p) = self.packs {
            packs.set_inner(p);
        }
        Flow {
            sh,
            store: Mutex::new(self.store),
            packs,
            lazy_packs: Mutex::new(self.lazy_packs),
            tools: self.tools,
            auth: self.auth,
            scenes,
            author,
            templates: self.templates,
            runner: Mutex::new(Some(runner)),
            events,
            config_error,
        }
    }
}

/// The kernel from Rust. Clone freely.
///
/// The application fills in the [`crate::Actor`] itself. An operation's [`Outcome`] carries
/// work (sessions to start, programs to run) that only the runner can carry out: hand it to
/// [`FlowHandle::dispatch`].
#[derive(Clone)]
pub struct FlowHandle {
    sh: Arc<Shared>,
    events: broadcast::Sender<Event>,
}

impl FlowHandle {
    /// The kernel; `None` until the extension has started.
    pub fn engine(&self) -> Option<Arc<Engine>> {
        self.sh.engine.get().cloned()
    }

    /// Every kernel event, as committed, from here on.
    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.events.subscribe()
    }

    /// Carry out what an operation produced: publish the runs it touched, broadcast its
    /// events, start its work. The extension does this itself for its methods and tools. Before
    /// the extension has started nothing can be carried out, and the outcome is dropped.
    pub async fn dispatch(&self, outcome: Outcome) {
        match self.sh.runner.get() {
            Some(rn) => rn.apply(outcome).await,
            None => tracing::warn!("an outcome handed to the flow extension before it started is dropped"),
        }
    }

    /// [`FlowHandle::dispatch`] for the result of an engine call, passing the value through.
    pub async fn settle(&self, result: crate::Result<Outcome>) -> crate::Result<serde_json::Value> {
        let o = result?;
        let value = o.value.clone();
        self.dispatch(o).await;
        Ok(value)
    }
}

#[async_trait::async_trait]
impl Extension for Flow {
    type Config = ();

    fn manifest(&self) -> Manifest {
        Manifest::new(MODULE_ID, env!("CARGO_PKG_VERSION"))
            .title("Flows")
            .disclosure(
                "runs flows: headless sessions in its space (or where a station places them, on \
                 instances of its templates), the application's programs and gates (command gates \
                 only when the application turned them on); answers its sessions' permission \
                 questions from the flow's permissions; posts run events to the hooks' URLs; \
                 writes run records and saved flows where the application said; names the sessions \
                 it opens and lists a run's sub-agents",
            )
            // Naming a run's main session and a flow's conversation; a main session's sub-agents
            // and other tasks.
            .requires([caps::ENGINE_CONTROL, caps::ENGINE_INSPECT])
    }

    fn register(&self, r: &mut Registrar<'_, ()>) {
        // On the main instance: the wrapped scenes and `flow-author`.
        let engine = r.engine();
        for s in &self.scenes {
            engine.scene(s.clone());
        }
        engine.scene(self.author.clone());
        for template in &self.templates {
            r.template(template.clone());
        }
        // The flow space: its conversations write flows (`flow-author`) on the main instance.
        r.space(SpaceDecl::new(&self.sh.cfg.space).instance(FLOW_INSTANCE).scene(AUTHOR_SCENE));
        let packs: Arc<dyn Packs> = self.packs.clone();
        methods::register(r, &self.sh, &packs);
        r.topic("runs");
        r.topic("run").param("run");
        r.topic("defs");
        r.topic("focus").param("instance").param("session_id");
        r.topic("dry_runs");
        // Its UI package defines a task row's badges and the other ways to start a flow, for others to fill.
        for point in catalog::points_of(MODULE_ID) {
            r.ui().point(point.decl());
        }
    }

    async fn start(&self, cx: ExtensionContext) -> anyhow::Result<()> {
        let spec = lock(&self.store).take();
        let store: Arc<dyn RunStore> = match spec {
            Some(StoreSpec::Ready(s)) => s,
            Some(StoreSpec::Lazy(make)) => make(cx.data_dir()),
            None => Arc::new(FileStore::new(cx.data_dir().join("runs"))),
        };
        if let Some(why) = &self.config_error {
            anyhow::bail!("{why}");
        }
        let Some(parts) = lock(&self.runner).take() else {
            anyhow::bail!("the flow extension was started twice");
        };
        let cfg = parts.config(&self.sh);
        self.sh.load_ledger(cx.data_dir().join("instances.json"));
        self.sh.load_flow_sessions(cx.data_dir().join("sessions.json"));
        if let Some(make) = lock(&self.lazy_packs).take() {
            self.packs.set_inner(make(cx.data_dir()));
        }
        let layout = project_layout(cx.product(), cx.data_dir());
        self.packs.set_layout(layout.clone());
        let packs: Arc<dyn Packs> = self.packs.clone();
        let mut engine = Engine::new(store, packs.clone())
            .with_lease_ms(cfg.lease_ms)
            .with_guarded(cfg.guarded.clone())
            .with_submit_scenes(cfg.profiles.clone())
            .with_workspaces(cx.data_dir().join("workspaces"))
            .with_layout(layout);
        if let Some(m) = &cfg.main_profile {
            engine = engine.with_main_scene(m.clone());
        }
        if let Some(a) = &self.auth {
            engine = engine.with_authorizer(a.clone());
        }
        if let Some(t) = &self.tools {
            engine = engine.with_tools(t.clone());
        }
        let engine = Arc::new(engine);
        if self.sh.engine.set(engine.clone()).is_err() {
            anyhow::bail!("the flow extension was started twice");
        }
        let _ = self.sh.sandbox.set(Arc::new(Sandbox::new(
            packs,
            SandboxConfig {
                guarded: cfg.guarded.clone(),
                submit_scenes: cfg.profiles.clone(),
                main_scene: cfg.main_profile.clone(),
            },
        )));
        let _ = self.sh.cx.set(cx.clone());
        let host = host::HarnixSessionHost::new(self.sh.clone());
        let runner = Runner::new(engine, host.clone(), cfg);
        let _ = self.sh.runner.set(runner.clone());
        for scene in self.sh.cfg.scenes.iter().map(String::as_str).chain([AUTHOR_SCENE]) {
            if let Err(e) = cx.call("engine.scene.activate", json!({"instance": FLOW_INSTANCE, "scene": scene})).await {
                anyhow::bail!("scene {scene} could not be activated: {}", e.message);
            }
        }
        publish::publish_all(&self.sh).await;
        publish::publish_defs(&self.sh).await;
        publish::publish_dry(&self.sh, None).await;
        // What the runner changes goes out on the topics; its events to the handle's subscribers.
        tokio::spawn(publish::follow(self.sh.clone(), runner.changes()));
        let events = self.events.clone();
        let mut from_runner = runner.events();
        tokio::spawn(async move {
            loop {
                match from_runner.recv().await {
                    Ok(e) => {
                        let _ = events.send(e);
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => return,
                }
            }
        });
        host.stop_stale_instances().await;
        runner.start().await;
        Ok(())
    }

    /// Every task told to stop, and waited for a little: the extension can say when it is quiet.
    /// Runs are not cancelled; the next start recovers their work.
    async fn stop(&self) {
        if let Some(rn) = self.sh.runner.get() {
            if !rn.stop(STOP_GRACE).await {
                tracing::warn!(left = rn.tasks_left(), "tasks still running after the stop's grace");
            }
        }
    }
}

/// How long stopping waits for the runner's tasks to notice.
const STOP_GRACE: std::time::Duration = std::time::Duration::from_secs(5);

/// A kernel error as a method answers it: the code by kind — `NOT_FOUND` for no such run, station,
/// task or flow; `INVALID_PARAMS` for a request that does not fit the run or the contract;
/// `REFUSED`; `CONFLICT` for a lease that is not the caller's or revisions that moved on;
/// `INTERNAL_ERROR` for the store — and `data.flow` the kernel's own (`not_found`, `invalid`,
/// `contract`, `denied`, `lease`, `conflict`, `store`).
pub fn rpc_error(e: FlowError) -> harnix_ext::RpcError {
    use harnix_ext::codes;
    let code = match &e {
        FlowError::Denied(_) => codes::REFUSED,
        FlowError::NotFound(_) => codes::NOT_FOUND,
        FlowError::Invalid(_) | FlowError::Contract(_) => codes::INVALID_PARAMS,
        // Someone else's claim, or a run that moved on between the read and the write.
        FlowError::Lease(_) | FlowError::Conflict(_) => codes::CONFLICT,
        FlowError::Store(_) => codes::INTERNAL_ERROR,
    };
    let mut err = harnix_ext::RpcError::new(code, e.to_string());
    err.data = Some(json!({"flow": e.code()}));
    err
}

/// Why a worktree could not be ended, as a method answers it: a merge that conflicts is
/// `CONFLICT` with the files (`data: {flow: "conflict", files, branch, worktree}`), git's refusal
/// `CONFLICT` too (`data: {flow: "git", branch}`), the kernel's as [`rpc_error`] says.
pub fn worktree_error(e: EndError) -> harnix_ext::RpcError {
    use harnix_ext::codes;
    match e {
        EndError::Flow(e) => rpc_error(e),
        EndError::Conflict { files, branch, worktree } => {
            let message = EndError::Conflict { files: files.clone(), branch: branch.clone(), worktree: worktree.clone() }.to_string();
            harnix_ext::RpcError::with_data(
                codes::CONFLICT,
                message,
                json!({"flow": "conflict", "files": files, "branch": branch, "worktree": worktree}),
            )
        }
        EndError::Git { message, branch } => {
            harnix_ext::RpcError::with_data(codes::CONFLICT, message, json!({"flow": "git", "branch": branch}))
        }
        EndError::Lost(m) => harnix_ext::RpcError::new(codes::INTERNAL_ERROR, m),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A client tells "no such run" from "that run cannot do this now" from "the lease is not
    /// yours" by the code, not by reading `data.flow`.
    #[test]
    fn kernel_errors_answer_codes_by_kind() {
        use harnix_ext::codes;
        let code = |e| rpc_error(e).code;
        assert_eq!(code(FlowError::NotFound("run r".into())), codes::NOT_FOUND);
        assert_eq!(code(FlowError::Invalid("paused".into())), codes::INVALID_PARAMS);
        assert_eq!(code(FlowError::Contract("output".into())), codes::INVALID_PARAMS);
        assert_eq!(code(FlowError::Lease("not yours".into())), codes::CONFLICT);
        assert_eq!(code(FlowError::Conflict("revs".into())), codes::CONFLICT);
        assert_eq!(code(FlowError::Denied("no".into())), codes::REFUSED);
        assert_eq!(rpc_error(FlowError::Lease("x".into())).data, Some(json!({"flow": "lease"})));
        let conflict = worktree_error(EndError::Conflict { files: vec!["a.txt".into()], branch: "b".into(), worktree: "/w".into() });
        assert_eq!(conflict.code, codes::CONFLICT);
        assert_eq!(conflict.data.unwrap()["files"], json!(["a.txt"]));
        assert_eq!(worktree_error(EndError::Flow(FlowError::Invalid("x".into()))).code, codes::INVALID_PARAMS);
    }

    #[test]
    fn a_guarded_pattern_no_permission_rule_can_say_keeps_the_extension_from_starting() {
        let ok = Flow::builder().guarded_tools(["Bash", "mcp__db__*"]).build();
        assert!(ok.config_error.is_none());
        let bad = Flow::builder().guarded_tools(["Bash", "Note*"]).build();
        let why = bad.config_error.unwrap();
        assert!(why.contains("Note*") && why.contains("ns__*"), "{why}");
        // The catalog carries what is guarded, for validation.
        assert_eq!(ok.packs.catalog().guarded, ["Bash", "mcp__db__*"]);
    }

    #[test]
    fn with_no_scene_given_the_built_in_ones_are_offered_on_harnix() {
        let flow = Flow::builder().build();
        assert_eq!(flow.sh.cfg.scenes, ["flow-harnix", "flow-code", "flow-mini"]);
        assert_eq!(flow.sh.cfg.main_scene.as_deref(), Some("flow-harnix"));
        assert!(flow.scenes.iter().all(|s| s.parts().interactive == Some(true)));
        assert_eq!(flow.sh.cfg.project_scenes.iter().collect::<Vec<_>>(), ["flow-code"], "code's sessions work in a project");
        let code = Flow::builder().main_scene("code").build();
        assert_eq!(code.sh.cfg.main_scene.as_deref(), Some("flow-code"));
    }

    #[test]
    fn the_layout_is_the_projects_data_directory_shared_with_the_engine() {
        let l = layout(Path::new("/state/ext/flow/data"));
        assert_eq!(l.dir, harnix_ext::engine::PROJECT_DIR);
        assert_eq!(l.worktree_dir("f-1"), Path::new("/state/ext/flow/data/worktrees/f-1"));
        assert_eq!(l.worktree_branch("f-1"), format!("{BRANCH_PREFIX}/f-1"));
        assert_eq!(l.agent_dirs_in("/p"), ["/p/.harnix/scratchpad", "/p/.harnix/memory", "/p/.harnix/worktrees"]);
    }

    /// A branded product's projects keep its own directory, and its runs' branches its own name.
    #[test]
    fn a_branded_products_layout_is_its_own() {
        let names = harnix_ext::engine::ProductNames::new(".acme", "Acme").unwrap();
        let l = project_layout(&names, Path::new("/state/ext/flow/data"));
        assert_eq!(l.dir, ".acme");
        assert_eq!(l.worktree_branch("f-1"), "acme/flow/f-1");
        assert_eq!(l.runs_dir(Path::new("/p")), Path::new("/p/.acme/runs"));
        assert_eq!(l.agent_dirs_in("/p"), ["/p/.acme/scratchpad", "/p/.acme/memory", "/p/.acme/worktrees"]);
    }
}
