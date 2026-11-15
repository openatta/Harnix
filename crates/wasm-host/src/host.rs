//! [`Host`]: the packages that are running, and the [`Control`] the product drives them by.
//!
//! # Starting, in order, and backwards on a failure
//!
//! 1. The manifest unpacked at [`Start::pkg`] is read, and checked to be the record's.
//! 2. Its signatures are the product's to verify, over its files as they are now, and it says how
//!    far they vouched ([`Start::trust`]) and the sha256 they gave its component
//!    ([`Start::cwasm_sha256`]). The host checks only that the bytes it loads are those.
//! 3. Its agent package and skills are switched on in the instances the install put them in.
//! 4. It is registered with the kernel — its methods, topics and spaces, under `<id>.*`,
//!    its `calls` capability, its tools and its manifest's digest — then its component loaded
//!    (refused if it was compiled for another engine, or its bytes are not the ones verified), its
//!    VM started under the product's [`Start::limits`] (a backend's `init` runs, serving the
//!    start, and may already publish, call and subscribe), its tools listed and offered.
//!
//! A step that fails undoes every step before it, and why is kept for [`Control::status`].

use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use async_trait::async_trait;
use harnix_package::acquire::{self, internal, refused};
use harnix_package::manifest::{self, toml_to_json, Manifest};
use harnix_package::signature::{sha256, Trust};
use harnix_wasm_port::{
    Agent, CallMeta, ChainLink, Control, HookCall, HookReply, Kernel, MethodCall, MethodDecl, Offer,
    Place, Placement, Progress, Registration, RpcError, SpaceDecl, Start, Started, Stop, ToolCall,
    ToolDef, ToolReply, TopicValue, TurnChain, TurnRef, Unplace,
};
use harnix_contract::sync::{lock, read, write};
use harnix_wasm::{
    AppSpec, AppVm, Capabilities, FsGrant, HookDecision, HttpFetcher, Limits, ProgressSink,
    VmState, WasmEngine, World,
};
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use crate::bridge::{failure_of, Bridge, Method};
use crate::idle::{self, Activity};
use crate::placement::{self, Placed};

/// What the product tells the host when it makes it: what holds for every package it will
/// run. What is each package's own — its trust, configuration and secrets — comes with its
/// [`Start`].
#[derive(Default)]
pub struct HostOptions {
    /// Where packages' HTTP goes out, when the product decides their `net` on its side too;
    /// this process's network otherwise.
    pub http: Option<Arc<dyn HttpFetcher>>,
}

/// One package that is running.
struct Running {
    version: String,
    /// How far its package was trusted when it started.
    trust: Trust,
    vm: Option<AppVm>,
    /// What its component listed, as the port carries it: converted once, when it started.
    tools: Vec<ToolDef>,
    bridge: Arc<Bridge>,
    methods: BTreeMap<String, Method>,
    /// Its calls in flight, for idling.
    activity: Arc<Activity>,
    /// `[backend] resident`, and how long it idles before it hibernates when that is `on-demand`.
    resident: String,
    idle: Option<std::time::Duration>,
    /// Ends its idle watch.
    watch: CancellationToken,
    /// What it declared, by full name: `<id>.<name>`.
    method_names: Vec<String>,
    topics: Vec<String>,
    spaces: Vec<String>,
    calls: Vec<String>,
    /// `[events] tools = "all"`: its `events` export is offered every tool call, not only its own
    /// tools' (`own_tools`, by the names a session knows them by).
    events_all: bool,
    own_tools: std::collections::BTreeSet<String>,
}

/// What starting a package's VM made.
struct Booted {
    vm: Option<AppVm>,
    tools: Vec<ToolDef>,
}

/// The packages of one product (or, in mode 3, with a process per package, of that
/// process): one VM each, every VM on one engine.
pub struct Host {
    engine: WasmEngine,
    kernel: Arc<dyn Kernel>,
    agent: Arc<dyn Agent>,
    http: Arc<dyn HttpFetcher>,
    running: RwLock<BTreeMap<String, Arc<Running>>>,
    /// The bridges of packages starting or running: where their subscriptions' values go,
    /// `init`'s included.
    bridges: Mutex<BTreeMap<String, Arc<Bridge>>>,
    /// Why a package that should be running is not.
    failed: Mutex<BTreeMap<String, String>>,
    /// Tool calls and hooks running, by their ids: what [`Control::cancel`] reaches.
    calls: Mutex<HashMap<String, CancellationToken>>,
    /// See [`Host::generation`].
    generation: Arc<AtomicU64>,
    /// The chains of turns running on an engine of the host's own, as the product told of them
    /// ([`Control::turn_started`]).
    turns: Mutex<HashMap<(String, String), Vec<ChainLink>>>,
}

/// A running call's entry in [`Host::calls`], taken out however the call ends (a caller that goes
/// away drops it).
struct CallEntry<'a> {
    host: &'a Host,
    id: String,
}

impl Drop for CallEntry<'_> {
    fn drop(&mut self) {
        lock(&self.host.calls).remove(&self.id);
    }
}

/// A tool as the manifest declares it.
fn declared_tool(t: &manifest::Tool) -> ToolDef {
    ToolDef {
        name: t.name.clone(),
        description: t.description.clone(),
        doc: t.doc.clone(),
        input_schema: t.input_schema_json(),
        read_only: t.read_only,
        concurrency_safe: t.concurrency_safe,
    }
}

/// How what a component lists is not what the manifest declares, if it is not: other names, or
/// another description, schema or scheduling flag for one of them.
fn tools_differ(declared: &[manifest::Tool], listed: &[harnix_wasm::ToolDef]) -> Option<String> {
    let schema = |text: &str| serde_json::from_str::<Value>(text).ok();
    for tool in declared {
        let Some(found) = listed.iter().find(|t| t.name == tool.name) else {
            return Some(format!("`{}` is not listed", tool.name));
        };
        let same = found.description == tool.description
            && found.doc == tool.doc
            && (found.read_only, found.concurrency_safe) == (tool.read_only, tool.concurrency_safe)
            && schema(&found.input_schema) == schema(&tool.input_schema_json());
        if !same {
            return Some(format!("`{}` is listed differently", tool.name));
        }
    }
    listed
        .iter()
        .find(|t| !declared.iter().any(|d| d.name == t.name))
        .map(|t| format!("`{}` is listed but not declared", t.name))
}

fn world_name(world: World) -> String {
    match world {
        World::Tools => "tools",
        World::Backend => "backend",
    }
    .to_string()
}

/// How far the product said a package's signatures vouched for it.
fn trust_of(trust: &str) -> Result<Trust, RpcError> {
    match trust {
        "store" => Ok(Trust::Store),
        "trusted" => Ok(Trust::Trusted),
        "unsigned" => Ok(Trust::Unsigned),
        other => Err(internal(format!("`{other}` is not a trust: store, trusted or unsigned"))),
    }
}

/// A tool call's `host.progress`, in order, to the kernel: the sink is called on the VM's side
/// and does not wait, so it queues for one task that does — boundedly, dropping (and counting)
/// what a tool says faster than it can be told.
struct ProgressQueue {
    tx: tokio::sync::mpsc::Sender<String>,
    stats: Arc<crate::Stats>,
}

impl ProgressSink for ProgressQueue {
    fn on_progress(&self, _call_id: &str, text: &str) {
        if let Err(tokio::sync::mpsc::error::TrySendError::Full(_)) =
            self.tx.try_send(text.to_string())
        {
            self.stats.dropped_progress();
        }
    }
}

impl Host {
    /// A host whose packages reach the product through `kernel`, and whose agent side goes to
    /// `agent`.
    pub fn new(options: HostOptions, kernel: Arc<dyn Kernel>, agent: Arc<dyn Agent>) -> anyhow::Result<Arc<Self>> {
        Ok(Arc::new(Self {
            engine: WasmEngine::new()?,
            kernel,
            agent,
            http: options.http.unwrap_or_else(|| Arc::new(crate::http::Http::new())),
            running: RwLock::new(BTreeMap::new()),
            bridges: Mutex::new(BTreeMap::new()),
            failed: Mutex::new(BTreeMap::new()),
            calls: Mutex::new(HashMap::new()),
            generation: Arc::default(),
            turns: Mutex::new(HashMap::new()),
        }))
    }

    /// The chain of the turn running in `session` on the engine of `instance`, as the product told
    /// of it; none when it has not (then ask it: [`Kernel::turn_chain`]).
    pub fn turn_chain(&self, instance: &str, session: &str) -> Option<Vec<ChainLink>> {
        lock(&self.turns).get(&(instance.to_string(), session.to_string())).cloned()
    }

    /// A number that moves whenever what [`Control::status`] answers may have: a package
    /// started, stopped or failed; a method call, tool call, hook or delivered value ended; a
    /// backend let go while idle; anything its bridge counts (what a backend does on its own,
    /// through its timers, included). A product that shows the status asks again only when this
    /// moved. What a VM changes without passing through any of these — the memory it holds — is
    /// seen at the next move.
    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::Relaxed)
    }

    fn moved(&self) {
        self.generation.fetch_add(1, Ordering::Relaxed);
    }

    /// The engine every VM runs on: what precompiled artifacts must have been compiled for
    /// ([`WasmEngine::fingerprint`]).
    pub fn engine(&self) -> &WasmEngine {
        &self.engine
    }

    fn running(&self, id: &str) -> Option<Arc<Running>> {
        read(&self.running).get(id).cloned()
    }

    /// The registration of `manifest`'s names (read from `root`), and its methods' schemas
    /// compiled.
    fn declare(manifest: &Manifest, root: &Path) -> Result<(Registration, BTreeMap<String, Method>), RpcError> {
        let id = &manifest.app.id;
        let mut methods = BTreeMap::new();
        let mut decls = Vec::new();
        for m in &manifest.methods {
            let schema = match &m.params {
                Some(params) => Some(jsonschema::validator_for(&toml_to_json(params)).map_err(|e| {
                    refused(format!("`{id}`'s method `{}` has a schema that does not compile: {e}", m.name))
                })?),
                None => None,
            };
            methods.insert(
                m.name.clone(),
                Method {
                    app: id.clone(),
                    name: m.name.clone(),
                    schema,
                },
            );
            decls.push(MethodDecl {
                name: m.name.clone(),
                description: m.description.clone().unwrap_or_default(),
                subjects: m.subject.names().iter().map(|s| s.to_string()).collect(),
            });
        }
        let spaces = manifest
            .spaces
            .iter()
            .map(|space| {
                let mut decl = SpaceDecl::new(space.id.clone());
                if let Some(instance) = &space.instance {
                    decl = decl.instance(instance);
                }
                if let Some(scene) = &space.scene {
                    decl = decl.scene(scene.clone());
                }
                decl
            })
            .collect();
        let text = std::fs::read(root.join(manifest::MANIFEST))
            .map_err(|e| internal(format!("`{id}`'s manifest: {e}")))?;
        let registration = Registration {
            app: id.clone(),
            methods: decls,
            topics: manifest.topics.iter().map(|t| t.name.clone()).collect(),
            spaces,
            calls: manifest.capabilities.calls.clone(),
            tools: manifest.tools.iter().map(declared_tool).collect(),
            digest: sha256(&text),
        };
        Ok((registration, methods))
    }

    /// Register the package, then start its VM: so what `init` does already reaches the
    /// product, and a method called before the VM is up is answered "not running". A VM that does
    /// not start takes the registration back with it.
    async fn boot(
        &self,
        meta: &CallMeta,
        manifest: &Manifest,
        start: &Start,
        (bytes, run): (Option<Vec<u8>>, &Path),
        registration: Registration,
        bridge: Arc<Bridge>,
    ) -> Result<Booted, RpcError> {
        let id = &manifest.app.id;
        self.kernel.register(meta.child(id, false), registration).await?;
        // `init` holds the backend as a method call does, serving the start.
        let serving = bridge.serve(meta.call_id(), meta, true);
        bridge.booting(Some(serving.id()));
        let started = self.start_vm(manifest, start, bytes, run, bridge.clone()).await;
        bridge.booting(None);
        drop(serving);
        match started {
            Ok((vm, tools)) => {
                if let Some(vm) = &vm {
                    bridge.started(vm);
                }
                Ok(Booted { vm, tools })
            }
            Err(e) => {
                if let Err(w) = self.kernel.withdraw(meta.child(id, false), id.clone()).await {
                    tracing::debug!(app = %id, error = %w.message, "a registration was not taken back");
                }
                Err(e)
            }
        }
    }

    /// The component's bytes for this engine, checked to be the ones the product verified — read
    /// once, and what is loaded, not merely a file of that name that was verified a moment ago.
    /// None for a package without one.
    fn component(manifest: &Manifest, start: &Start, root: &Path, fingerprint: &str) -> Result<Option<Vec<u8>>, RpcError> {
        let Some(component) = &manifest.component else {
            return Ok(None);
        };
        let id = &manifest.app.id;
        let artifact = root
            .join("wasm")
            .join(format!("{}.{fingerprint}.cwasm", component.name));
        let bytes = std::fs::read(&artifact).map_err(|_| {
            refused(format!(
                "`{id}` has no `{}` component compiled for this engine (`{fingerprint}`): it needs an \
                 artifact built for this Harnix release",
                component.name
            ))
        })?;
        match (&start.cwasm_sha256, trust_of(&start.trust)?) {
            (Some(expected), _) if *expected != sha256(&bytes) => {
                let mut e = refused(format!(
                    "`{id}`: its component is not the one that was verified (its sha256 differs)"
                ));
                // Marks it for a process that ends on it rather than waiting to be told what to do.
                e.data = Some(serde_json::json!({"setup": "cwasm_sha256"}));
                Err(e)
            }
            (Some(_), _) | (None, Trust::Unsigned) => Ok(Some(bytes)),
            (None, trust) => Err(refused(format!(
                "`{id}` is {} and its component was given no sha256 to be checked against",
                trust.as_str()
            ))),
        }
    }

    /// Start the VM from the component's checked `bytes` and list its tools; what it writes goes
    /// under `run`.
    async fn start_vm(
        &self,
        manifest: &Manifest,
        start: &Start,
        bytes: Option<Vec<u8>>,
        run: &Path,
        bridge: Arc<Bridge>,
    ) -> Result<(Option<AppVm>, Vec<ToolDef>), RpcError> {
        let (Some(component), Some(bytes)) = (&manifest.component, bytes) else {
            return Ok((None, Vec::new()));
        };
        let id = &manifest.app.id;
        let fingerprint = self.engine.fingerprint();
        // SAFETY: an artifact is machine code. Its bytes were just checked against the sha256 the
        // product took from the signatures it verified — the store's, or a publisher's the profile
        // trusts. With none, the product starts it as unsigned, which it does only where the
        // operator said unsigned code is theirs to trust (`[ext.packages] dev`): the developer's
        // own build, on the developer's own machine.
        let loaded = unsafe { self.engine.load(&bytes, fingerprint) }
            .map_err(|e| refused(format!("`{id}`: {e:#}")))?;
        let declared = if component.world == manifest::WORLDS[1] {
            World::Backend
        } else {
            World::Tools
        };
        if loaded.world() != declared {
            return Err(refused(format!(
                "`{id}`'s component implements {:?}, and its manifest says `{}`",
                loaded.world(),
                component.world
            )));
        }
        let caps = &manifest.capabilities;
        let spec = AppSpec {
            id: id.clone(),
            capabilities: Capabilities {
                net: caps.net.clone(),
                env: caps.env.clone(),
                storage_bytes: u64::from(start.limits.storage_mb) << 20,
                fs: caps
                    .fs
                    .iter()
                    .map(|path| FsGrant {
                        path: path.clone(),
                        writable: path.starts_with("${data}"),
                    })
                    .collect(),
            },
            limits: Limits {
                memory_bytes: u64::from(start.limits.memory_mb) << 20,
                timeout: std::time::Duration::from_millis(start.limits.timeout_ms),
            },
            dirs: harnix_wasm::AppDirs {
                data: run.join("data"),
            },
            config: manifest.config_with(Some(&start.config)),
        };
        let kv = harnix_wasm::FileKv::open(run.join("kv"))
            .map_err(|e| internal(format!("`{id}`'s storage: {e}")))?;
        let services = harnix_wasm::Services {
            host: bridge,
            kv: Arc::new(kv),
            http: self.http.clone(),
        };
        let vm = AppVm::start(&self.engine, &loaded, spec, services)
            .await
            .map_err(|e| refused(format!("`{id}` did not start: {e:#}")))?;
        // The manifest is what was shown and consented to, and what is registered: the component
        // is asked only to agree with it, never trusted to say what its tools are.
        let listed = match vm.list_tools(&CancellationToken::new()).await {
            Ok(tools) => tools,
            Err(e) => {
                vm.stop().await;
                return Err(refused(format!("`{id}` did not list its tools: {e}")));
            }
        };
        if let Some(why) = tools_differ(&manifest.tools, &listed) {
            vm.stop().await;
            return Err(refused(format!(
                "`{id}`'s component does not provide the tools its manifest declares: {why}"
            )));
        }
        // A row drawn for a tool it does not have is a manifest that does not match its component.
        let unknown = manifest.inline.unknown_tools(&manifest.tool_names());
        if !unknown.is_empty() {
            vm.stop().await;
            return Err(refused(format!(
                "`{id}`'s manifest draws a tool row for `{}`, which it does not declare as a `[[tool]]`",
                unknown.join("`, `")
            )));
        }
        let tools: Vec<ToolDef> = manifest.tools.iter().map(declared_tool).collect();
        Ok((Some(vm), tools))
    }

    /// Everything [`Control::start`] does but keeping why it failed.
    async fn start_app(&self, meta: &CallMeta, start: &Start) -> Result<Started, RpcError> {
        let id = &start.app;
        let (Some(root), Some(run)) = (start.pkg.clone(), start.run.clone()) else {
            return Err(internal(format!("`{id}` was started with no package or run directory")));
        };
        let trust = trust_of(&start.trust)?;
        let manifest = acquire::manifest_in(&root)?;
        if &manifest.app.id != id || manifest.app.version != start.version {
            return Err(internal(format!(
                "{} says it is `{}` {}, and the record `{id}` {}",
                root.display(),
                manifest.app.id,
                manifest.app.version,
                start.version
            )));
        }
        let (registration, methods) = Self::declare(&manifest, &root)?;
        let bytes = Self::component(&manifest, start, &root, self.engine.fingerprint())?;
        placement::switch(self.agent.as_ref(), id, &start.parts, true).await?;
        let bridge = Bridge::new(
            id,
            manifest.topics.iter().map(|t| t.name.clone()),
            start.secrets.clone(),
            self.kernel.clone(),
            self.generation.clone(),
        );
        lock(&self.bridges).insert(id.clone(), bridge.clone());
        let booted = match self
            .boot(meta, &manifest, start, (bytes, &run), registration, bridge.clone())
            .await
        {
            Ok(booted) => booted,
            Err(e) => {
                lock(&self.bridges).remove(id);
                placement::switch(self.agent.as_ref(), id, &start.parts, false)
                    .await
                    .ok();
                return Err(e);
            }
        };
        let full = |name: &String| format!("{id}.{name}");
        let world = booted.vm.as_ref().map(|vm| world_name(vm.world()));
        let backend = booted.vm.as_ref().filter(|vm| vm.world() == World::Backend);
        // Held to what the product keeps it to: an extension process's backend hibernates when
        // the product expects it to.
        let idle = backend.and(start.limits.on_demand().then(|| std::time::Duration::from_millis(start.limits.idle_ms)));
        let activity = Arc::new(Activity::default());
        let watch = CancellationToken::new();
        if let (Some(vm), Some(idle)) = (backend, idle) {
            idle::watch(vm, activity.clone(), idle, watch.clone(), self.generation.clone());
        }
        let running = Arc::new(Running {
            version: start.version.clone(),
            trust,
            vm: booted.vm,
            tools: booted.tools.clone(),
            bridge,
            methods,
            activity,
            resident: start.limits.resident.clone(),
            idle,
            watch,
            method_names: manifest.methods.iter().map(|m| full(&m.name)).collect(),
            topics: manifest.topics.iter().map(|t| full(&t.name)).collect(),
            spaces: manifest.spaces.iter().map(|s| s.id.clone()).collect(),
            calls: manifest.capabilities.calls.clone(),
            events_all: manifest.events.all(),
            own_tools: booted
                .tools
                .iter()
                .map(|t| harnix_package::tool_name(id, &t.name))
                .collect(),
        });
        let offer = Offer {
            app: id.clone(),
            title: manifest.app.title.clone(),
            version: start.version.clone(),
            component: running.vm.is_some(),
            tools: booted.tools.clone(),
        };
        write(&self.running).insert(id.clone(), running);
        if let Err(e) = self.agent.expose_tools(offer).await {
            tracing::warn!(app = %id, error = %e.message, "its tools were not offered");
        }
        tracing::info!(app = %id, tools = booted.tools.len(), "application started");
        Ok(Started {
            app: id.clone(),
            trust: trust.as_str().to_string(),
            world,
            tools: booted.tools,
        })
    }

    fn status_of(&self, id: &str) -> Value {
        if let Some(app) = self.running(id) {
            let health = app.vm.as_ref().map(|vm| vm.health());
            let (state, reason) = match health.as_ref() {
                Some(harnix_wasm::HealthSnapshot {
                    state: VmState::Disabled { reason },
                    ..
                }) => ("faulted", Some(reason.clone())),
                // Not a fault: its backend was let go while idle, and the next call starts it.
                Some(h) if h.hibernating => ("hibernating", None),
                _ => ("running", None),
            };
            return json!({
                "state": state,
                "reason": reason,
                "resident": app.resident,
                "idle_ms": app.idle.map(|d| d.as_millis() as u64),
                "world": app.vm.as_ref().map(|vm| vm.world()),
                "methods": app.method_names,
                "topics": app.topics,
                "spaces": app.spaces,
                "allowed_calls": app.calls,
                "calls": app.bridge.stats.report(),
                "tools": app.tools.iter().map(|t| json!({
                    "name": harnix_package::tool_name(id, &t.name),
                    "tool": t.name,
                    "description": t.description,
                })).collect::<Vec<_>>(),
                "health": health.map(|h| serde_json::to_value(h).unwrap_or(Value::Null)),
            });
        }
        match lock(&self.failed).get(id) {
            Some(reason) => json!({"state": "failed", "reason": reason}),
            None => json!({"state": "stopped"}),
        }
    }

    /// Whether a lifecycle event is about a call of one of the package's own tools.
    fn about_own_tool(app: &Running, call: &HookCall) -> bool {
        call.payload["tool"]
            .as_str()
            .is_some_and(|tool| app.own_tools.contains(tool))
    }

    /// A cancellation for the call `call_id`, registered for as long as the entry is held.
    fn cancellation(&self, call_id: &str) -> (CancellationToken, CallEntry<'_>) {
        let token = CancellationToken::new();
        lock(&self.calls).insert(call_id.to_string(), token.clone());
        (
            token,
            CallEntry {
                host: self,
                id: call_id.to_string(),
            },
        )
    }
}

#[async_trait]
impl Control for Host {
    async fn place(&self, _meta: CallMeta, place: Place) -> Result<Placement, RpcError> {
        let root = place.app_dir.join(&place.version);
        let mut placed = Placed::default();
        match placement::place(&self.agent, &root, place.instances.as_deref(), &mut placed).await {
            Ok(()) => Ok(Placement {
                parts: placed.parts(),
                disclosure: placed.disclosure,
            }),
            Err(e) => {
                tracing::warn!(app = %place.app, error = %e.message, "placing it failed; undoing it");
                placed.undo(self.agent.as_ref(), &place.app).await;
                Err(e)
            }
        }
    }

    async fn unplace(&self, _meta: CallMeta, unplace: Unplace) -> Result<Value, RpcError> {
        let placed = Placed::of(&unplace.parts);
        placed.undo(self.agent.as_ref(), &unplace.app).await;
        Ok(json!({
            "agent": placed.agent.as_ref().map(|(package, _, instances)| json!({"package": package, "instances": instances})),
        }))
    }

    async fn start(&self, meta: CallMeta, start: Start) -> Result<Started, RpcError> {
        if let Some(running) = self.running(&start.app) {
            let world = running.vm.as_ref().map(|vm| world_name(vm.world()));
            return Ok(Started {
                app: start.app.clone(),
                trust: running.trust.as_str().to_string(),
                world,
                tools: running.tools.clone(),
            });
        }
        let outcome = match self.start_app(&meta, &start).await {
            Ok(started) => {
                lock(&self.failed).remove(&start.app);
                Ok(started)
            }
            Err(e) => {
                lock(&self.failed).insert(start.app.clone(), e.message.clone());
                Err(e)
            }
        };
        self.moved();
        outcome
    }

    async fn stop(&self, meta: CallMeta, stop: Stop) -> Result<(), RpcError> {
        let id = &stop.app;
        lock(&self.failed).remove(id);
        self.moved();
        let Some(app) = write(&self.running).remove(id) else {
            return Ok(());
        };
        app.watch.cancel();
        // Its tools first: a session built from here on has none of them.
        let none = Offer {
            app: id.clone(),
            title: None,
            version: app.version.clone(),
            component: false,
            tools: Vec::new(),
        };
        let _ = self.agent.expose_tools(none).await;
        // Then its names: a call decided from here is unknown. (The product has usually withdrawn
        // them already; then there is nothing left to take.) Calls queued behind the backend are
        // answered that it stopped.
        if self.kernel.withdraw(meta.child(id, false), id.clone()).await.is_ok() {
            tracing::debug!(app = %id, "registration withdrawn");
        }
        lock(&self.bridges).remove(id);
        if let Some(vm) = &app.vm {
            vm.stop().await;
        }
        placement::switch(self.agent.as_ref(), id, &stop.parts, false)
            .await
            .ok();
        tracing::info!(app = %id, "application stopped");
        self.moved();
        Ok(())
    }

    async fn forget(&self, _meta: CallMeta, app: String) -> Result<(), RpcError> {
        lock(&self.failed).remove(&app);
        self.moved();
        Ok(())
    }

    async fn status(&self, _meta: CallMeta, app: String) -> Result<Value, RpcError> {
        Ok(self.status_of(&app))
    }

    async fn call_method(&self, meta: CallMeta, call: MethodCall) -> Result<Value, RpcError> {
        let full = format!("{}.{}", call.app, call.method);
        if meta.expired(crate::bridge::now_ms()) {
            return Err(RpcError::new(
                harnix_wasm_port::codes::DEADLINE_EXCEEDED,
                format!("`{full}`: its deadline passed before its backend was asked"),
            ));
        }
        let Some(app) = self.running(&call.app) else {
            return Err(crate::bridge::unavailable(
                &call.app,
                &full,
                "the package is not running",
                "stopped",
            ));
        };
        // Its backend is waiting on this very call, a hop or several back: the chain holds it on
        // a call it is still serving. Refused now rather than left to wait for itself until the
        // timeout recycles it — said by the call itself, so it holds whatever carried it here; the
        // VM's own check is for a call that stayed on one task. A call the chain holds it on that
        // was answered long ago (a turn queued by a method that has returned) is not waited on.
        if meta.links().iter().any(|l| l.ext() == call.app && app.bridge.holds_backend(l.call())) {
            return Err(crate::bridge::unavailable(
                &call.app,
                &full,
                &format!(
                    "would re-enter the backend of `{}`, which is waiting on this very call",
                    call.app
                ),
                "reentrant",
            ));
        }
        let Some(method) = app.methods.get(&call.method) else {
            return Err(RpcError::not_found(format!("`{full}` is not one of its methods")));
        };
        let outcome = {
            let _busy = app.activity.hold();
            let serving = app.bridge.serve(meta.call_id(), &meta, true);
            method.answer(app.vm.as_ref(), serving.id(), &call.caller, call.params).await
        };
        app.bridge.stats.method(&call.method, outcome.as_ref().err());
        self.moved();
        outcome
    }

    async fn deliver(&self, _meta: CallMeta, value: TopicValue) -> Result<(), RpcError> {
        let bridge = lock(&self.bridges).get(&value.app).cloned();
        if let Some(bridge) = bridge {
            bridge.deliver(value.topic, value.value);
            self.moved();
        }
        Ok(())
    }

    async fn call_tool(&self, meta: CallMeta, call: ToolCall) -> Result<ToolReply, RpcError> {
        let Some(app) = self.running(&call.app) else {
            return Ok(ToolReply::Failed {
                failure: harnix_wasm_port::Failure::Unavailable,
                message: harnix_wasm::CallFailure::Unavailable(format!("`{}` is stopped", call.app))
                    .to_string(),
            });
        };
        let Some(vm) = &app.vm else {
            return Err(RpcError::not_found(format!("`{}` has no component", call.app)));
        };
        let (cancel, _running) = self.cancellation(&call.call_id);
        let _busy = app.activity.hold();
        // The tool's own instance: what it calls is made for this call, holding no backend.
        let _serving = app.bridge.serve(&call.call_id, &meta, false);
        let (sink, forwarded) = if call.progress {
            let (tx, mut rx) = tokio::sync::mpsc::channel::<String>(crate::bridge::QUEUE);
            let (kernel, app_id, tool, call_id) = (
                self.kernel.clone(),
                call.app.clone(),
                call.tool.clone(),
                call.call_id.clone(),
            );
            let forwarded = tokio::spawn(async move {
                while let Some(text) = rx.recv().await {
                    let progress = Progress {
                        app: app_id.clone(),
                        tool: tool.clone(),
                        call_id: call_id.clone(),
                        text,
                    };
                    kernel.progress(progress);
                }
            });
            let queue = ProgressQueue {
                tx,
                stats: app.bridge.stats.clone(),
            };
            (
                Some(Arc::new(queue) as Arc<dyn ProgressSink>),
                Some(forwarded),
            )
        } else {
            (None, None)
        };
        let outcome = vm
            .call_tool(
                &call.tool,
                &call.input,
                &call.call_id,
                call.project.as_deref(),
                sink,
                &cancel,
            )
            .await;
        // Every progress told before the answer is.
        if let Some(forwarded) = forwarded {
            let _ = forwarded.await;
        }
        self.moved();
        Ok(match outcome {
            Ok(out) => ToolReply::Output {
                content: out.content,
                structured: out.structured,
                is_error: out.is_error,
            },
            Err(failure) => ToolReply::Failed {
                failure: failure_of(&failure),
                message: failure.to_string(),
            },
        })
    }

    async fn hook(&self, meta: CallMeta, call: HookCall) -> Result<HookReply, RpcError> {
        let Some(app) = self.running(&call.app) else {
            return Ok(HookReply::Failed(format!("`{}` is not running", call.app)));
        };
        let Some(vm) = &app.vm else {
            return Ok(HookReply::Proceed);
        };
        // `[events]`: what it was not granted, it is not shown — not even that it happened.
        if !app.events_all && !Self::about_own_tool(&app, &call) {
            return Ok(HookReply::Proceed);
        }
        let (cancel, _running) = self.cancellation(&call.call_id);
        let _busy = app.activity.hold();
        let _serving = app.bridge.serve(&call.call_id, &meta, false);
        let decided = vm.on_event_as(Some(&call.call_id), &call.event, &call.payload, &cancel).await;
        self.moved();
        Ok(match decided {
            Ok(HookDecision::Proceed) => HookReply::Proceed,
            Ok(HookDecision::Block(reason)) => HookReply::Block(reason),
            Ok(HookDecision::AddContext(note)) => HookReply::AddContext(note),
            Err(failure) => HookReply::Failed(failure.to_string()),
        })
    }

    async fn cancel(&self, _meta: CallMeta, call_id: String) -> Result<(), RpcError> {
        if let Some(token) = lock(&self.calls).get(&call_id) {
            token.cancel();
        }
        Ok(())
    }

    async fn shutdown(&self, _meta: CallMeta, reason: String) -> Result<(), RpcError> {
        let running: Vec<(String, Arc<Running>)> = std::mem::take(&mut *write(&self.running)).into_iter().collect();
        lock(&self.bridges).clear();
        for (id, app) in running {
            app.watch.cancel();
            if let Some(vm) = &app.vm {
                vm.stop().await;
            }
            tracing::debug!(app = %id, %reason, "application stopped");
        }
        self.moved();
        Ok(())
    }

    fn turn_started(&self, turn: TurnChain) {
        lock(&self.turns).insert((turn.instance, turn.session_id), turn.chain);
    }

    fn turn_settled(&self, turn: TurnRef) {
        lock(&self.turns).remove(&(turn.instance, turn.session_id));
    }
}
