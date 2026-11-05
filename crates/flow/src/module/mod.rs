//! The Harnix module: the kernel wired to sessions, the application's code, native tools,
//! methods and topics (xops-01 §6).
//!
//! ```text
//! mod.rs       Flow, its builder, FlowHandle; the Module impl (spaces, engine, events, start)
//! shared.rs    what the module, its tools and its dispatcher share
//! dispatch.rs  carrying out an Outcome: topics, events, and every work item
//! tools.rs     the scene wrappers: flow-* (flow_state, flow_submit, flow_propose), structured-*
//! author.rs    flow-author: writing flows in conversation (xops-02 §4)
//! methods.rs   app.flow.* — the device methods
//! ```
//!
//! A station's `writes` are carried by the session's permission rules (`deny` for each guarded
//! tool the station does not allow), its output contract by the session's `output_schema`:
//! the same on `system`, an in-process template's instance or a spawned daemon (xops-02 §2).
//!
//! An application registers it like any module:
//!
//! ```no_run
//! # use std::sync::Arc;
//! # use harnix_flow::{Flow, MemPacks};
//! # fn scene() -> Arc<dyn base::interface::scene::AgentScene> { unimplemented!() }
//! let flow = Flow::builder()
//!     .packs(MemPacks::new().flow("name: demo\nstations: [{id: a, exit: [{kind: confirm}]}]"))
//!     .scene(scene())          // offered to definitions as `flow-<id>`
//!     .build();
//! let handle = flow.handle();  // the kernel from Rust, once the module has started
//! # let _ = (flow, handle);
//! ```

mod author;
mod dispatch;
mod methods;
mod shared;
mod tools;

use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};

use base::interface::scene::AgentScene;
use harnix_module::{
    EngineContrib, EventRegistry, InstanceTemplate, InstanceTemplates, Manifest, MethodRegistry,
    Module, ModuleContext, SpaceDecl, SpaceRegistry,
};
use serde_json::json;
use tokio::sync::broadcast;

use crate::dryrun::{Sandbox, SandboxConfig};
use crate::kernel::engine::{Engine, DEFAULT_LEASE_MS};
use crate::kernel::host::{
    Authorizer, Catalog, Event, Gates, Hooks, Origin, Outcome, Packs, Programs, RunStore, Tools,
    WorkItem,
};
use crate::kernel::FlowError;
use crate::packs::MemPacks;
use crate::store::FileStore;

pub use crate::kernel::protocol::matches_any;
pub use author::{AuthorScene, AUTHOR_PROMPT, AUTHOR_SCENE, AUTHOR_TOOLS, NO_FOCUS};
pub use dispatch::permission_rules;
pub use shared::{session_name, session_ref};
pub use tools::{structured_id, FlowScene, StructuredScene, TOOL_PROPOSE, TOOL_STATE, TOOL_SUBMIT};

use shared::{Config, RecordingFn, Shared};

/// The module id: methods and topics are `app.flow.*`, entries `app.flow`.
pub const MODULE_ID: &str = "flow";
/// The instance the flow space's sessions run on, and where a station's session goes when it
/// names no instance or template and stays in the flow's space. `Module::engine` contributes to
/// it alone, so the wrapped scenes and `flow-author` exist only there.
pub const FLOW_INSTANCE: &str = "system";

/// The engine's tools that change something outside the conversation: the shell, the file
/// editors, the notebook editor, the interpreters.
pub const DEFAULT_GUARDED: &[&str] = &[
    "Bash",
    "PowerShell",
    "REPL",
    "Write",
    "Edit",
    "NotebookEdit",
];

type MakeStore = Box<dyn FnOnce(&Path) -> Arc<dyn RunStore> + Send>;
type MakePacks = Box<dyn FnOnce(&Path) -> Arc<dyn Packs> + Send>;

/// Where runs are stored: given, or made from the module's data directory at start.
enum StoreSpec {
    Ready(Arc<dyn RunStore>),
    Lazy(MakeStore),
}

/// The flow module. Build it with [`Flow::builder`], register it with the application.
pub struct Flow {
    sh: Arc<Shared>,
    store: Mutex<Option<StoreSpec>>,
    packs: Arc<CatalogPacks>,
    lazy_packs: Mutex<Option<MakePacks>>,
    tools: Option<Arc<dyn Tools>>,
    auth: Option<Arc<dyn Authorizer>>,
    scenes: Vec<Arc<dyn AgentScene>>,
    author: Arc<dyn AgentScene>,
    templates: Vec<InstanceTemplate>,
    /// Why the module must not start: a guarded-tool pattern no permission rule can say.
    config_error: Option<String>,
}

impl Flow {
    pub fn builder() -> FlowBuilder {
        FlowBuilder::default()
    }

    /// The kernel from Rust, for the application.
    pub fn handle(&self) -> FlowHandle {
        FlowHandle {
            sh: self.sh.clone(),
        }
    }
}

/// [`Flow`], from parts.
pub struct FlowBuilder {
    store: Option<StoreSpec>,
    packs: Option<Arc<dyn Packs>>,
    lazy_packs: Option<MakePacks>,
    scenes: Vec<Arc<dyn AgentScene>>,
    author: Option<Arc<dyn AgentScene>>,
    templates: Vec<InstanceTemplate>,
    ensure_ms: u64,
    programs: Option<Arc<dyn Programs>>,
    gates: Option<Arc<dyn Gates>>,
    tools: Option<Arc<dyn Tools>>,
    hooks: Option<Arc<dyn Hooks>>,
    auth: Option<Arc<dyn Authorizer>>,
    space: String,
    guarded: Vec<String>,
    recording: Option<RecordingFn>,
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
            hooks: None,
            auth: None,
            space: "flow".into(),
            guarded: DEFAULT_GUARDED.iter().map(|s| s.to_string()).collect(),
            recording: None,
            lease_ms: DEFAULT_LEASE_MS,
            tick_ms: 1000,
        }
    }
}

impl FlowBuilder {
    /// Where runs live. Without one, a [`FileStore`] in the module's data directory (`runs/`).
    pub fn store(mut self, store: impl RunStore + 'static) -> Self {
        self.store = Some(StoreSpec::Ready(Arc::new(store)));
        self
    }

    /// A store made at start from the module's data directory, which is not known before.
    pub fn store_with<S, F>(mut self, make: F) -> Self
    where
        S: RunStore + 'static,
        F: FnOnce(&Path) -> S + Send + 'static,
    {
        self.store = Some(StoreSpec::Lazy(Box::new(move |dir: &Path| {
            Arc::new(make(dir)) as Arc<dyn RunStore>
        })));
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

    /// Packs made at start from the module's data directory — a writable layer there, say:
    /// `DirPacks::writable(dir.join("packs")).with_builtin(…)`.
    pub fn packs_with<P, F>(mut self, make: F) -> Self
    where
        P: Packs + 'static,
        F: FnOnce(&Path) -> P + Send + 'static,
    {
        self.lazy_packs = Some(Box::new(move |dir: &Path| {
            Arc::new(make(dir)) as Arc<dyn Packs>
        }));
        self
    }

    /// A scene stations may run sessions in. It is offered as `flow-<id>`: the same scene
    /// with the flow's native tools added (its sessions hand in with `flow_submit`). Repeatable.
    pub fn scene(mut self, scene: Arc<dyn AgentScene>) -> Self {
        self.scenes.push(scene);
        self
    }

    /// The scene `flow-author` wraps — the flow space's conversations. Agix's chat scene when
    /// not given.
    pub fn author_scene(mut self, scene: Arc<dyn AgentScene>) -> Self {
        self.author = Some(scene);
        self
    }

    /// A template of the module's own, which stations may name as `flow.<id>` (the hub lets a
    /// module build only from its own templates and the profile's). Repeatable.
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

    pub fn gates(mut self, g: impl Gates + 'static) -> Self {
        self.gates = Some(Arc::new(g));
        self
    }

    /// Read-only application tools `tool` exits call.
    pub fn tools(mut self, t: impl Tools + 'static) -> Self {
        self.tools = Some(Arc::new(t));
        self
    }

    pub fn hooks(mut self, h: impl Hooks + 'static) -> Self {
        self.hooks = Some(Arc::new(h));
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

    /// The tools that count as writes for a station's `writes`: a session working on a flow
    /// task gets a `deny` permission rule for each of them the station does not list, and the
    /// protocol layer names them. Only these are guarded — an application whose sessions reach
    /// other writing tools (its own MCP tools, plugin tools) lists them here, by name or as a
    /// namespace group `ns__*`. By default the engine's own: [`DEFAULT_GUARDED`].
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
    pub fn recording(
        mut self,
        f: impl Fn(&WorkItem) -> Option<String> + Send + Sync + 'static,
    ) -> Self {
        self.recording = Some(Arc::new(f));
        self
    }

    /// The lease the dispatcher holds on the tasks it carries out (it renews it while working).
    pub fn lease_ms(mut self, ms: u64) -> Self {
        self.lease_ms = ms.max(1000);
        self
    }

    /// How often expired leases are looked for; 1 s by default.
    pub fn tick_ms(mut self, ms: u64) -> Self {
        self.tick_ms = ms.max(50);
        self
    }

    pub fn build(self) -> Flow {
        let (tx, _) = broadcast::channel(1024);
        let own_templates: Vec<String> = self
            .templates
            .iter()
            .map(|t| format!("{MODULE_ID}.{}", t.id))
            .collect();
        let sh = Arc::new(Shared::new(
            Config {
                space: self.space.clone(),
                instance: FLOW_INSTANCE.to_string(),
                scenes: self
                    .scenes
                    .iter()
                    .map(|s| tools::wrapped_id(s.id()))
                    .collect(),
                programs: self.programs.clone(),
                gates: self.gates.clone(),
                hooks: self.hooks.clone(),
                guarded: self.guarded.clone(),
                recording: self.recording.clone(),
                lease_ms: self.lease_ms,
                tick_ms: self.tick_ms,
                ensure_ms: self.ensure_ms,
            },
            tx,
        ));
        let scenes: Vec<Arc<dyn AgentScene>> = self
            .scenes
            .iter()
            .map(|s| Arc::new(FlowScene::new(s.clone(), sh.clone())) as Arc<dyn AgentScene>)
            .collect();
        let base = self
            .author
            .unwrap_or_else(|| Arc::new(scene::scene::chat::ChatScene) as Arc<dyn AgentScene>);
        let author: Arc<dyn AgentScene> = Arc::new(AuthorScene::new(base, sh.clone()));
        let packs = Arc::new(CatalogPacks {
            inner: OnceLock::new(),
            scenes: sh.cfg.scenes.clone(),
            templates: own_templates,
            guarded: sh.cfg.guarded.clone(),
        });
        let problems = crate::kernel::protocol::guard_problems(&self.guarded);
        let config_error = (!problems.is_empty()).then(|| {
            format!(
                "the flow module's guarded tools cannot all be refused by a session's permission \
                 rules: {}",
                problems.join("; ")
            )
        });
        if let Some(p) = self.packs {
            let _ = packs.inner.set(p);
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
            config_error,
        }
    }
}

/// The application's packs (given, or made at start), with what the module adds to the
/// catalog: its wrapped `flow-*` scenes, and — when the application lists templates at all —
/// its own templates.
struct CatalogPacks {
    inner: OnceLock<Arc<dyn Packs>>,
    scenes: Vec<String>,
    templates: Vec<String>,
    guarded: Vec<String>,
}

impl CatalogPacks {
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
    fn skill(&self, name: &str) -> Option<(String, String)> {
        self.inner().skill(name)
    }
    fn catalog(&self) -> Catalog {
        let mut c = self.inner().catalog();
        c.scenes.extend(self.scenes.iter().cloned());
        if !c.templates.is_empty() {
            c.templates.extend(self.templates.iter().cloned());
        }
        c.guarded = self.guarded.clone();
        c.submit_scenes = self.scenes.iter().cloned().collect();
        c
    }
    fn flow_text(&self, name: &str) -> Option<String> {
        self.inner().flow_text(name)
    }
    fn skills(&self) -> Vec<String> {
        self.inner().skills()
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
    fn save_skill(&self, name: &str, body: &str) -> Result<String, String> {
        self.inner().save_skill(name, body)
    }
    fn delete_skill(&self, name: &str) -> Result<(), String> {
        self.inner().delete_skill(name)
    }
}

/// The kernel from Rust. Clone freely.
///
/// The application fills in the [`crate::Actor`] itself. An operation's [`Outcome`] carries
/// work (sessions to start, programs to run) that only the module can carry out: hand it to
/// [`FlowHandle::dispatch`].
#[derive(Clone)]
pub struct FlowHandle {
    sh: Arc<Shared>,
}

impl FlowHandle {
    /// The kernel; `None` until the module has started.
    pub fn engine(&self) -> Option<Arc<Engine>> {
        self.sh.engine.get().cloned()
    }

    /// Every kernel event, as committed, from here on.
    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.sh.events.subscribe()
    }

    /// Carry out what an operation produced: publish the runs it touched, broadcast its
    /// events, start its work. The module does this itself for its methods and tools.
    pub async fn dispatch(&self, outcome: Outcome) {
        dispatch::apply(&self.sh, None, outcome).await;
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
impl Module for Flow {
    const API_VERSION: u32 = harnix_module::MODULE_API_VERSION;

    fn manifest(&self) -> Manifest {
        Manifest::new(MODULE_ID, env!("CARGO_PKG_VERSION")).disclosure(
            "runs flows: headless sessions in its space (or where a station places them, on \
             instances of its templates), the application's programs, gates and hooks; writes \
             run records and saved flows where the application said",
        )
    }

    /// On `system`: the wrapped scenes and `flow-author`.
    fn engine(&self, e: &mut EngineContrib) {
        for s in &self.scenes {
            e.scene(s.clone());
        }
        e.scene(self.author.clone());
    }

    fn templates(&self, t: &mut InstanceTemplates) {
        for template in &self.templates {
            t.add(template.clone());
        }
    }

    /// The flow space: its conversations write flows (`flow-author`) on `system`.
    fn spaces(&self, s: &mut SpaceRegistry) {
        s.add(
            SpaceDecl::new(&self.sh.cfg.space)
                .instance(FLOW_INSTANCE)
                .scene(AUTHOR_SCENE),
        );
    }

    fn methods(&self, m: &mut MethodRegistry) {
        let packs: Arc<dyn Packs> = self.packs.clone();
        methods::register(m, &self.sh, &packs);
    }

    fn events(&self, e: &mut EventRegistry) {
        e.topic("runs");
        e.topic("run").param("run");
        e.topic("defs");
        e.topic("focus").param("instance").param("session_id");
        e.topic("dry_runs");
    }

    async fn start(&self, cx: ModuleContext) -> anyhow::Result<()> {
        let spec = self.store.lock().unwrap_or_else(|e| e.into_inner()).take();
        let store: Arc<dyn RunStore> = match spec {
            Some(StoreSpec::Ready(s)) => s,
            Some(StoreSpec::Lazy(make)) => make(cx.data_dir()),
            None => Arc::new(FileStore::new(cx.data_dir().join("runs"))),
        };
        if let Some(why) = &self.config_error {
            anyhow::bail!("{why}");
        }
        self.sh.load_ledger(cx.data_dir().join("instances.json"));
        if let Some(make) = self
            .lazy_packs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
        {
            let _ = self.packs.inner.set(make(cx.data_dir()));
        }
        let packs: Arc<dyn Packs> = self.packs.clone();
        let mut engine = Engine::new(store, packs.clone())
            .with_lease_ms(self.sh.cfg.lease_ms)
            .with_guarded(self.sh.cfg.guarded.clone())
            .with_submit_scenes(self.sh.cfg.scenes.clone());
        if let Some(a) = &self.auth {
            engine = engine.with_authorizer(a.clone());
        }
        if let Some(t) = &self.tools {
            engine = engine.with_tools(t.clone());
        }
        if self.sh.engine.set(Arc::new(engine)).is_err() {
            anyhow::bail!("the flow module was started twice");
        }
        let _ = self.sh.sandbox.set(Arc::new(Sandbox::new(
            packs,
            SandboxConfig {
                guarded: self.sh.cfg.guarded.clone(),
                submit_scenes: self.sh.cfg.scenes.clone(),
            },
        )));
        let _ = self.sh.cx.set(cx.clone());
        for scene in self
            .sh
            .cfg
            .scenes
            .iter()
            .map(String::as_str)
            .chain([AUTHOR_SCENE])
        {
            if let Err(e) = cx
                .call(
                    "scene.activate",
                    json!({"instance": FLOW_INSTANCE, "scene": scene}),
                )
                .await
            {
                anyhow::bail!("scene {scene} could not be activated: {}", e.message);
            }
        }
        dispatch::publish_all(&self.sh).await;
        dispatch::publish_defs(&self.sh).await;
        dispatch::publish_dry(&self.sh, None).await;
        let sh = self.sh.clone();
        tokio::spawn(async move {
            dispatch::stop_stale_instances(&sh).await;
            dispatch::recover(&sh).await
        });
        let sh = self.sh.clone();
        let tick = tokio::spawn(async move { dispatch::tick_loop(sh).await });
        *self.sh.tick.lock().unwrap_or_else(|e| e.into_inner()) = Some(tick);
        Ok(())
    }

    async fn stop(&self) {
        self.sh
            .stopping
            .store(true, std::sync::atomic::Ordering::SeqCst);
        if let Some(t) = self
            .sh
            .tick
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
        {
            t.abort();
        }
    }
}

/// A kernel error as a method answers it: `data.flow` is the kernel's code
/// (`not_found`, `invalid`, `denied`, `lease`, `conflict`, `store`).
pub fn rpc_error(e: FlowError) -> harnix_module::RpcError {
    use harnix_contract::codes;
    let code = match &e {
        FlowError::Denied(_) => codes::REFUSED,
        FlowError::NotFound(_)
        | FlowError::Invalid(_)
        | FlowError::Contract(_)
        | FlowError::Lease(_) => codes::INVALID_PARAMS,
        FlowError::Conflict(_) => codes::BUSY,
        FlowError::Store(_) => codes::INTERNAL_ERROR,
    };
    let mut err = harnix_module::RpcError::new(code, e.to_string());
    err.data = Some(json!({"flow": e.code()}));
    err
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalog_packs(inner: MemPacks, templates: &[&str]) -> CatalogPacks {
        let packs = CatalogPacks {
            inner: OnceLock::new(),
            scenes: vec!["flow-chat".into()],
            templates: templates.iter().map(|s| s.to_string()).collect(),
            guarded: vec![],
        };
        let _ = packs.inner.set(Arc::new(inner));
        packs
    }

    #[test]
    fn the_catalog_has_the_applications_scenes_and_the_wrapped_ones() {
        let packs = catalog_packs(
            MemPacks::new()
                .scene("structured-chat")
                .skill("work", "Do it."),
            &["flow.local"],
        );
        let yaml = |scene: &str| {
            format!(
                "name: f\nstations: [{{id: a, executor: {{kind: session, scene: {scene}}}, skill: work, output: {{type: object}}, exit: [{{kind: output}}]}}]"
            )
        };
        assert!(crate::validate_yaml(&yaml("flow-chat"), &packs).is_empty());
        assert!(crate::validate_yaml(&yaml("structured-chat"), &packs).is_empty());
        let problems = crate::validate_yaml(&yaml("nowhere"), &packs);
        assert!(
            problems.iter().any(|p| p.message.contains("scene nowhere")),
            "{problems:?}"
        );
        assert_eq!(
            packs.catalog().scenes.into_iter().collect::<Vec<_>>(),
            ["flow-chat", "structured-chat"]
        );
        assert!(
            packs.catalog().templates.is_empty(),
            "templates the application does not list are not checked"
        );
        let listed = catalog_packs(MemPacks::new().template("app.box"), &["flow.local"]);
        assert_eq!(
            listed.catalog().templates.into_iter().collect::<Vec<_>>(),
            ["app.box", "flow.local"]
        );
    }

    #[test]
    fn a_guarded_pattern_no_permission_rule_can_say_keeps_the_module_from_starting() {
        let ok = Flow::builder()
            .guarded_tools(["Bash", "mcp__db__*"])
            .build();
        assert!(ok.config_error.is_none());
        let bad = Flow::builder().guarded_tools(["Bash", "Note*"]).build();
        let why = bad.config_error.unwrap();
        assert!(why.contains("Note*") && why.contains("ns__*"), "{why}");
        // The catalog carries what is guarded, for validation.
        assert_eq!(ok.packs.catalog().guarded, ["Bash", "mcp__db__*"]);
    }

    #[test]
    fn packs_made_at_start_are_empty_until_then_and_writable_after() {
        let packs = CatalogPacks {
            inner: OnceLock::new(),
            scenes: vec![],
            templates: vec![],
            guarded: vec![],
        };
        assert!(packs.flows().is_empty() && !packs.writable());
        let _ = packs.inner.set(Arc::new(MemPacks::new().writable()));
        assert!(packs.writable());
        packs
            .save_flow("name: x\nstations: [{id: a, exit: [{kind: confirm}]}]")
            .unwrap();
        assert_eq!(packs.origin("x"), Origin::User);
    }
}
