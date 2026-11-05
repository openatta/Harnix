//! The Harnix module: the kernel wired to sessions, the application's code, native tools,
//! methods and topics (xops-01 §6).
//!
//! ```text
//! mod.rs       Flow, its builder, FlowHandle; the Module impl (spaces, engine, events, start)
//! shared.rs    what the module, its tools and its dispatcher share
//! dispatch.rs  carrying out an Outcome: topics, events, and every work item
//! tools.rs     the scene wrapper and its native tools (flow_state, flow_submit, flow_propose)
//! guard.rs     `writes`: a tool middleware refusing writes a station does not allow
//! methods.rs   app.flow.* — the device methods
//! ```
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

mod dispatch;
mod guard;
mod methods;
mod shared;
mod tools;

use std::path::Path;
use std::sync::{Arc, Mutex};

use base::interface::scene::AgentScene;
use harnix_module::{
    EngineContrib, EventRegistry, Manifest, MethodRegistry, Module, ModuleContext, SpaceDecl,
    SpaceRegistry,
};
use serde_json::json;
use tokio::sync::broadcast;

use crate::kernel::engine::{Engine, DEFAULT_LEASE_MS};
use crate::kernel::host::{
    Authorizer, Catalog, Event, Gates, Hooks, Outcome, Packs, Programs, RunStore, Tools, WorkItem,
};
use crate::kernel::FlowError;
use crate::packs::MemPacks;
use crate::store::FileStore;

pub use guard::{matches_any, DEFAULT_GUARDED};
pub use shared::{session_name, session_ref};
pub use tools::{FlowScene, TOOL_PROPOSE, TOOL_STATE, TOOL_SUBMIT};

use shared::{Config, RecordingFn, Shared};

/// The module id: methods and topics are `app.flow.*`, entries `app.flow`.
pub const MODULE_ID: &str = "flow";
/// The instance flow sessions run on. `Module::engine` contributes to it alone, so the
/// wrapped scenes and the `writes` middleware exist only there.
pub const FLOW_INSTANCE: &str = "system";

type MakeStore = Box<dyn FnOnce(&Path) -> Arc<dyn RunStore> + Send>;

/// Where runs are stored: given, or made from the module's data directory at start.
enum StoreSpec {
    Ready(Arc<dyn RunStore>),
    Lazy(MakeStore),
}

/// The flow module. Build it with [`Flow::builder`], register it with the application.
pub struct Flow {
    sh: Arc<Shared>,
    store: Mutex<Option<StoreSpec>>,
    packs: Arc<dyn Packs>,
    tools: Option<Arc<dyn Tools>>,
    auth: Option<Arc<dyn Authorizer>>,
    scenes: Vec<Arc<dyn AgentScene>>,
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
    scenes: Vec<Arc<dyn AgentScene>>,
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
            scenes: vec![],
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

    /// A scene stations may run sessions in. It is offered as `flow-<its id>`: the same scene
    /// with the flow's native tools added. Repeatable.
    pub fn scene(mut self, scene: Arc<dyn AgentScene>) -> Self {
        self.scenes.push(scene);
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

    /// The tools that count as writes for a station's `writes` (`prefix*` matches a group):
    /// in a session working on a flow task, a call to one of them that the station does not
    /// list is refused, and the protocol layer names them. Only these are guarded — an
    /// application whose sessions reach other writing tools (its own MCP tools, `mcp__*`,
    /// plugin tools) lists them here. By default the engine's own: [`DEFAULT_GUARDED`].
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
            },
            tx,
        ));
        let scenes: Vec<Arc<dyn AgentScene>> = self
            .scenes
            .iter()
            .map(|s| Arc::new(FlowScene::new(s.clone(), sh.clone())) as Arc<dyn AgentScene>)
            .collect();
        let inner = self
            .packs
            .unwrap_or_else(|| Arc::new(MemPacks::new()) as Arc<dyn Packs>);
        let packs: Arc<dyn Packs> = Arc::new(CatalogPacks {
            inner,
            scenes: sh.cfg.scenes.clone(),
        });
        Flow {
            sh,
            store: Mutex::new(self.store),
            packs,
            tools: self.tools,
            auth: self.auth,
            scenes,
        }
    }
}

/// The application's packs, with the catalog's scenes replaced by the wrapped ones: a session
/// task runs only in a scene that carries `flow_submit` and the `writes` guard, so a
/// definition naming `chat` instead of `flow-chat` does not validate.
struct CatalogPacks {
    inner: Arc<dyn Packs>,
    scenes: Vec<String>,
}

impl Packs for CatalogPacks {
    fn flows(&self) -> Vec<String> {
        self.inner.flows()
    }
    fn flow(&self, name: &str) -> Option<(crate::FlowDef, String)> {
        self.inner.flow(name)
    }
    fn skill(&self, name: &str) -> Option<(String, String)> {
        self.inner.skill(name)
    }
    fn catalog(&self) -> Catalog {
        let mut c = self.inner.catalog();
        c.scenes = self.scenes.iter().cloned().collect();
        c
    }
    fn flow_text(&self, name: &str) -> Option<String> {
        self.inner.flow_text(name)
    }
    fn skills(&self) -> Vec<String> {
        self.inner.skills()
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
            "runs flows: headless sessions on `system` in its space, the application's programs, \
             gates and hooks; writes run records where the application said",
        )
    }

    fn engine(&self, e: &mut EngineContrib) {
        for s in &self.scenes {
            e.scene(s.clone());
        }
        let sh = self.sh.clone();
        e.customize_session(move |b, cx| {
            if sh.cfg.scenes.iter().any(|s| s == cx.scene) {
                b.tool_middleware(Arc::new(guard::Writes::new(sh.clone(), cx.session_id)))
            } else {
                b
            }
        });
    }

    fn spaces(&self, s: &mut SpaceRegistry) {
        let mut decl = SpaceDecl::new(&self.sh.cfg.space).instance(FLOW_INSTANCE);
        if let Some(first) = self.sh.cfg.scenes.first() {
            decl = decl.scene(first.clone());
        }
        s.add(decl);
    }

    fn methods(&self, m: &mut MethodRegistry) {
        methods::register(m, &self.sh, &self.packs);
    }

    fn events(&self, e: &mut EventRegistry) {
        e.topic("runs");
        e.topic("run").param("run");
    }

    async fn start(&self, cx: ModuleContext) -> anyhow::Result<()> {
        let spec = self.store.lock().unwrap_or_else(|e| e.into_inner()).take();
        let store: Arc<dyn RunStore> = match spec {
            Some(StoreSpec::Ready(s)) => s,
            Some(StoreSpec::Lazy(make)) => make(cx.data_dir()),
            None => Arc::new(FileStore::new(cx.data_dir().join("runs"))),
        };
        let mut engine = Engine::new(store, self.packs.clone())
            .with_lease_ms(self.sh.cfg.lease_ms)
            .with_guarded(self.sh.cfg.guarded.clone());
        if let Some(a) = &self.auth {
            engine = engine.with_authorizer(a.clone());
        }
        if let Some(t) = &self.tools {
            engine = engine.with_tools(t.clone());
        }
        if self.sh.engine.set(Arc::new(engine)).is_err() {
            anyhow::bail!("the flow module was started twice");
        }
        let _ = self.sh.cx.set(cx.clone());
        for scene in &self.sh.cfg.scenes {
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
        let sh = self.sh.clone();
        tokio::spawn(async move { dispatch::recover(&sh).await });
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

    #[test]
    fn a_session_task_must_name_a_wrapped_scene() {
        let packs = CatalogPacks {
            inner: Arc::new(MemPacks::new().scene("chat").skill("work", "Do it.")),
            scenes: vec!["flow-chat".into()],
        };
        let yaml = |scene: &str| {
            format!(
                "name: f\nstations: [{{id: a, executor: {{kind: session, scene: {scene}}}, skill: work, output: {{type: object}}, exit: [{{kind: output}}]}}]"
            )
        };
        assert!(crate::validate_yaml(&yaml("flow-chat"), &packs).is_empty());
        let problems = crate::validate_yaml(&yaml("chat"), &packs);
        assert!(
            problems.iter().any(|p| p.message.contains("scene chat")),
            "an unwrapped scene has no flow_submit and no writes guard: {problems:?}"
        );
        assert_eq!(
            packs.catalog().scenes.into_iter().collect::<Vec<_>>(),
            ["flow-chat"]
        );
    }
}
