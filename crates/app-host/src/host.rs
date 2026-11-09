//! [`Host`]: the applications that are running, and the [`Control`] the product drives them by.
//!
//! # Starting, in order, and backwards on a failure
//!
//! 1. The manifest unpacked at `<app_dir>/<version>/` is read, and checked to be the record's.
//! 2. Its signatures are verified over its files as they are now: what is on disk may not be what
//!    was installed. Unsigned, it starts only where the product runs unsigned code.
//! 3. Its agent package and skills are switched on in the instances the install put them in.
//! 4. It is registered with the platform — its methods, topics and spaces, under `app.<id>.*`,
//!    and its `hub` capability — then its component loaded (refused if it was compiled for another
//!    engine, or its bytes are not the ones signed), its VM started (a backend's `init` runs, and
//!    may already publish, call and subscribe), its tools listed and offered.
//!
//! A step that fails undoes every step before it, and why is kept for [`Control::status`].

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use async_trait::async_trait;
use harnix_app_package::acquire::{self, internal, refused, Admission};
use harnix_app_package::manifest::{self, toml_to_json, Manifest};
use harnix_app_package::signature::{Trust, Verified};
use harnix_app_port::{
    Control, HookCall, HookReply, MethodCall, MethodDecl, Offer, Place, Placement, Platform,
    Progress, Registration, RpcError, SpaceDecl, Start, Started, Stop, ToolCall, ToolDef,
    ToolReply, TopicValue, Unplace,
};
use harnix_wasm::{
    AppSpec, AppVm, Capabilities, FsGrant, HookDecision, HttpFetcher, Limits, ProgressSink,
    VmState, WasmEngine, World,
};
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use crate::bridge::{failure_of, lock, Bridge, Method};
use crate::idle::{self, Activity};
use crate::placement::{self, Placed};

/// What the product tells the host when it makes it.
#[derive(Default)]
pub struct HostOptions {
    /// The keys packages are verified with, and whether unsigned code runs (`[apps] dev`).
    pub admission: Admission,
    /// `[apps.config]`, by application id: set over each one's `[config]`.
    pub config: BTreeMap<String, toml::Value>,
    /// `[apps.secrets]`, by application id: what `host.secret` gives each one, by name, for the
    /// names its `env` declares. Nothing else is: not this process's environment.
    pub secrets: BTreeMap<String, BTreeMap<String, String>>,
    /// How long every on-demand backend idles before it hibernates, in place of its manifest's
    /// `idle_secs` (which is never under a minute). For a test application that needs to see one
    /// hibernate; a product has no reason to, and a profile cannot.
    #[doc(hidden)]
    pub idle: Option<std::time::Duration>,
}

/// One application that is running.
struct Running {
    version: String,
    /// The unpacked package.
    root: PathBuf,
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
    /// What it declared, by full name: `app.<id>.<name>`.
    method_names: Vec<String>,
    topics: Vec<String>,
    spaces: Vec<String>,
    hub: Vec<String>,
    /// `[events] tools = "all"`: its `events` export is offered every tool call, not only its own
    /// tools' (`own_tools`, by the names a session knows them by).
    events_all: bool,
    own_tools: std::collections::BTreeSet<String>,
}

/// What starting an application's VM made.
struct Booted {
    vm: Option<AppVm>,
    tools: Vec<ToolDef>,
}

/// The applications of one product (or, in an edition with a process per application, of that
/// process): one VM each, every VM on one engine.
pub struct Host {
    engine: WasmEngine,
    platform: Arc<dyn Platform>,
    options: HostOptions,
    http: Arc<dyn HttpFetcher>,
    apps: RwLock<BTreeMap<String, Arc<Running>>>,
    /// The bridges of applications starting or running: where their subscriptions' values go,
    /// `init`'s included.
    bridges: Mutex<BTreeMap<String, Arc<Bridge>>>,
    /// Why an application that should be running is not.
    failed: Mutex<BTreeMap<String, String>>,
    /// Tool calls and hooks running, by their ids: what [`Control::cancel`] reaches.
    calls: Mutex<HashMap<String, CancellationToken>>,
    /// See [`Host::generation`].
    generation: Arc<AtomicU64>,
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

fn read<T>(l: &RwLock<T>) -> std::sync::RwLockReadGuard<'_, T> {
    l.read().unwrap_or_else(|e| e.into_inner())
}

fn write<T>(l: &RwLock<T>) -> std::sync::RwLockWriteGuard<'_, T> {
    l.write().unwrap_or_else(|e| e.into_inner())
}

fn tool_def(t: &harnix_wasm::ToolDef) -> ToolDef {
    ToolDef {
        name: t.name.clone(),
        description: t.description.clone(),
        doc: t.doc.clone(),
        input_schema: t.input_schema.clone(),
        read_only: t.read_only,
        concurrency_safe: t.concurrency_safe,
    }
}

fn world_name(world: World) -> String {
    match world {
        World::Tools => "tools",
        World::Backend => "backend",
    }
    .to_string()
}

/// A tool call's `host.progress`, in order, to the platform: the sink is called on the VM's side
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
    /// A host whose applications reach the product through `platform`.
    pub fn new(options: HostOptions, platform: Arc<dyn Platform>) -> anyhow::Result<Arc<Self>> {
        Ok(Arc::new(Self {
            engine: WasmEngine::new()?,
            platform,
            options,
            http: Arc::new(crate::http::Http::new()),
            apps: RwLock::new(BTreeMap::new()),
            bridges: Mutex::new(BTreeMap::new()),
            failed: Mutex::new(BTreeMap::new()),
            calls: Mutex::new(HashMap::new()),
            generation: Arc::default(),
        }))
    }

    /// A number that moves whenever what [`Control::status`] answers may have: an application
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
        read(&self.apps).get(id).cloned()
    }

    /// The registration of `manifest`'s names, and its methods' schemas compiled.
    fn declare(manifest: &Manifest) -> Result<(Registration, BTreeMap<String, Method>), RpcError> {
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
        let registration = Registration {
            app: id.clone(),
            methods: decls,
            topics: manifest.topics.iter().map(|t| t.name.clone()).collect(),
            spaces,
            hub: manifest.capabilities.hub.clone(),
        };
        Ok((registration, methods))
    }

    /// Register the application, then start its VM: so what `init` does already reaches the
    /// product, and a method called before the VM is up is answered "not running". A VM that does
    /// not start takes the registration back with it.
    async fn boot(
        &self,
        manifest: &Manifest,
        root: &Path,
        app_dir: &Path,
        registration: Registration,
        bridge: Arc<Bridge>,
        verified: &Verified,
    ) -> Result<Booted, RpcError> {
        let id = &manifest.app.id;
        self.platform.register(registration).await?;
        match self.start_vm(manifest, root, app_dir, bridge.clone(), verified).await {
            Ok((vm, tools)) => {
                if let Some(vm) = &vm {
                    bridge.started(vm);
                }
                Ok(Booted { vm, tools })
            }
            Err(e) => {
                if let Err(w) = self.platform.withdraw(id.clone()).await {
                    tracing::debug!(app = %id, error = %w.message, "a registration was not taken back");
                }
                Err(e)
            }
        }
    }

    /// Start the VM and list its tools. `verified`: the package's signatures, checked over `root`.
    async fn start_vm(
        &self,
        manifest: &Manifest,
        root: &Path,
        app_dir: &Path,
        bridge: Arc<Bridge>,
        verified: &Verified,
    ) -> Result<(Option<AppVm>, Vec<ToolDef>), RpcError> {
        let Some(component) = &manifest.component else {
            return Ok((None, Vec::new()));
        };
        let id = &manifest.app.id;
        let fingerprint = self.engine.fingerprint();
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
        if !verified.trust.signed() && !self.options.admission.dev {
            return Err(refused(format!(
                "`{id}` was installed unsigned, and its code runs only with `[apps] dev = true`"
            )));
        }
        // The bytes about to be deserialized are the ones that were signed, not merely a file of
        // that name that was signed a moment ago.
        verified
            .check(
                &format!("wasm/{}.{fingerprint}.cwasm", component.name),
                &bytes,
            )
            .map_err(|e| refused(format!("`{id}`: {e}")))?;
        // SAFETY: an artifact is machine code. Signed, its bytes were just checked against the
        // sha256 the store (or a publisher the profile trusts) signed, whose signature was
        // verified before this was called. Unsigned, it is loaded only under `[apps] dev = true`
        // (checked just above), which is the operator saying that unsigned code is theirs to trust
        // — the developer's own build, on the developer's own machine.
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
                storage_bytes: u64::from(caps.storage_mb) << 20,
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
                memory_bytes: u64::from(component.memory_mb) << 20,
                timeout: std::time::Duration::from_millis(component.timeout_ms),
            },
            dirs: harnix_wasm::AppDirs {
                data: app_dir.join("data"),
            },
            config: manifest.config_with(manifest::overrides_for(&self.options.config, id).as_ref()),
        };
        let kv = harnix_wasm::FileKv::open(app_dir.join("kv"))
            .map_err(|e| internal(format!("`{id}`'s storage: {e}")))?;
        let services = harnix_wasm::Services {
            host: bridge,
            kv: Arc::new(kv),
            http: self.http.clone(),
        };
        let vm = AppVm::start(&self.engine, &loaded, spec, services)
            .await
            .map_err(|e| refused(format!("`{id}` did not start: {e:#}")))?;
        let tools = match vm.list_tools(&CancellationToken::new()).await {
            Ok(tools) => tools,
            Err(e) => {
                vm.stop().await;
                return Err(refused(format!("`{id}` did not list its tools: {e}")));
            }
        };
        let mut seen = std::collections::BTreeSet::new();
        for tool in &tools {
            let full = harnix_app_package::tool_name(id, &tool.name);
            let ok = !tool.name.is_empty()
                && tool
                    .name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
                && full.len() <= 64
                && seen.insert(tool.name.clone())
                && serde_json::from_str::<Value>(&tool.input_schema).is_ok_and(|s| s.is_object());
            if !ok {
                vm.stop().await;
                return Err(refused(format!(
                    "`{id}`'s tool `{}` cannot be offered: a tool's name is letters, digits, `_` and \
                     `-`, unique, `{full}` at most 64 characters, and its input schema a JSON object",
                    tool.name
                )));
            }
        }
        // A row drawn for a tool it does not have is a manifest that does not match its component.
        let names: Vec<String> = tools.iter().map(|t| t.name.clone()).collect();
        let unknown = manifest.inline.unknown_tools(&names);
        if !unknown.is_empty() {
            vm.stop().await;
            return Err(refused(format!(
                "`{id}`'s manifest draws a tool row for `{}`, which its component does not list as a tool",
                unknown.join("`, `")
            )));
        }
        Ok((Some(vm), tools.iter().map(tool_def).collect()))
    }

    /// Everything [`Control::start`] does but keeping why it failed.
    async fn start_app(&self, start: &Start) -> Result<Started, RpcError> {
        let id = &start.app;
        let root = start.app_dir.join(&start.version);
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
        // Every start, over the files as they are now: what is on disk may not be what was
        // installed.
        let verified = self.options.admission.admit_root(&root, &manifest)?;
        let (registration, methods) = Self::declare(&manifest)?;
        placement::switch(self.platform.as_ref(), id, &root, &start.parts, true).await?;
        let bridge = Bridge::new(
            id,
            manifest.topics.iter().map(|t| t.name.clone()),
            self.options.secrets.get(id).cloned().unwrap_or_default(),
            self.platform.clone(),
            self.generation.clone(),
        );
        lock(&self.bridges).insert(id.clone(), bridge.clone());
        let booted = match self
            .boot(&manifest, &root, &start.app_dir, registration, bridge.clone(), &verified)
            .await
        {
            Ok(booted) => booted,
            Err(e) => {
                lock(&self.bridges).remove(id);
                placement::switch(self.platform.as_ref(), id, &root, &start.parts, false)
                    .await
                    .ok();
                return Err(e);
            }
        };
        let full = |name: &String| format!("app.{id}.{name}");
        let world = booted.vm.as_ref().map(|vm| world_name(vm.world()));
        let backend = booted.vm.as_ref().filter(|vm| vm.world() == World::Backend);
        let idle = backend.and(manifest.backend.on_demand().then(|| {
            self.options
                .idle
                .unwrap_or(std::time::Duration::from_secs(manifest.backend.idle_secs))
        }));
        let activity = Arc::new(Activity::default());
        let watch = CancellationToken::new();
        if let (Some(vm), Some(idle)) = (backend, idle) {
            idle::watch(vm, activity.clone(), idle, watch.clone(), self.generation.clone());
        }
        let running = Arc::new(Running {
            version: start.version.clone(),
            root: root.clone(),
            trust: verified.trust,
            vm: booted.vm,
            tools: booted.tools.clone(),
            bridge,
            methods,
            activity,
            resident: manifest.backend.resident.clone(),
            idle,
            watch,
            method_names: manifest.methods.iter().map(|m| full(&m.name)).collect(),
            topics: manifest.topics.iter().map(|t| full(&t.name)).collect(),
            spaces: manifest.spaces.iter().map(|s| s.id.clone()).collect(),
            hub: manifest.capabilities.hub.clone(),
            events_all: manifest.events.all(),
            own_tools: booted
                .tools
                .iter()
                .map(|t| harnix_app_package::tool_name(id, &t.name))
                .collect(),
        });
        let offer = Offer {
            app: id.clone(),
            title: manifest.app.title.clone(),
            version: start.version.clone(),
            component: running.vm.is_some(),
            tools: booted.tools.clone(),
        };
        write(&self.apps).insert(id.clone(), running);
        if let Err(e) = self.platform.expose_tools(offer).await {
            tracing::warn!(app = %id, error = %e.message, "its tools were not offered");
        }
        tracing::info!(app = %id, tools = booted.tools.len(), "application started");
        Ok(Started {
            app: id.clone(),
            trust: verified.trust.as_str().to_string(),
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
                "hub": app.hub,
                "calls": app.bridge.stats.report(),
                "tools": app.tools.iter().map(|t| json!({
                    "name": harnix_app_package::tool_name(id, &t.name),
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

    /// Whether a lifecycle event is about a call of one of the application's own tools.
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
    async fn place(&self, place: Place) -> Result<Placement, RpcError> {
        let root = place.app_dir.join(&place.version);
        let mut placed = Placed::default();
        match placement::place(&self.platform, &root, &place.app_dir, &mut placed).await {
            Ok(()) => Ok(Placement {
                parts: placed.parts(),
                disclosure: placed.disclosure,
            }),
            Err(e) => {
                tracing::warn!(app = %place.app, error = %e.message, "placing it failed; undoing it");
                placed.undo(self.platform.as_ref(), &place.app).await;
                Err(e)
            }
        }
    }

    async fn unplace(&self, unplace: Unplace) -> Result<Value, RpcError> {
        let placed = Placed::of(&unplace.parts);
        placed.undo(self.platform.as_ref(), &unplace.app).await;
        Ok(json!({
            "agent": placed.agent.as_ref().map(|(package, _, instances)| json!({"package": package, "instances": instances})),
            "skills": placed.skills,
        }))
    }

    async fn start(&self, start: Start) -> Result<Started, RpcError> {
        if let Some(running) = self.running(&start.app) {
            let world = running.vm.as_ref().map(|vm| world_name(vm.world()));
            return Ok(Started {
                app: start.app.clone(),
                trust: running.trust.as_str().to_string(),
                world,
                tools: running.tools.clone(),
            });
        }
        let outcome = match self.start_app(&start).await {
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

    async fn stop(&self, stop: Stop) -> Result<(), RpcError> {
        let id = &stop.app;
        lock(&self.failed).remove(id);
        self.moved();
        let Some(app) = write(&self.apps).remove(id) else {
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
        let _ = self.platform.expose_tools(none).await;
        // Then its names: a call decided from here is unknown. (The product has usually withdrawn
        // them already; then there is nothing left to take.) Calls queued behind the backend are
        // answered that it stopped.
        if self.platform.withdraw(id.clone()).await.is_ok() {
            tracing::debug!(app = %id, "registration withdrawn");
        }
        lock(&self.bridges).remove(id);
        if let Some(vm) = &app.vm {
            vm.stop().await;
        }
        placement::switch(self.platform.as_ref(), id, &app.root, &stop.parts, false)
            .await
            .ok();
        tracing::info!(app = %id, "application stopped");
        self.moved();
        Ok(())
    }

    async fn forget(&self, app: String) -> Result<(), RpcError> {
        lock(&self.failed).remove(&app);
        self.moved();
        Ok(())
    }

    async fn status(&self, app: String) -> Result<Value, RpcError> {
        Ok(self.status_of(&app))
    }

    async fn call_method(&self, call: MethodCall) -> Result<Value, RpcError> {
        let full = format!("app.{}.{}", call.app, call.method);
        let Some(app) = self.running(&call.app) else {
            return Err(crate::bridge::unavailable(
                &call.app,
                &full,
                "the application is not running",
                "stopped",
            ));
        };
        let Some(method) = app.methods.get(&call.method) else {
            return Err(RpcError::not_found(format!("`{full}` is not one of its methods")));
        };
        let outcome = {
            let _busy = app.activity.hold();
            method.answer(app.vm.as_ref(), &call.caller, call.params).await
        };
        app.bridge.stats.method(&call.method, outcome.as_ref().err());
        self.moved();
        outcome
    }

    async fn deliver(&self, value: TopicValue) -> Result<(), RpcError> {
        let bridge = lock(&self.bridges).get(&value.app).cloned();
        if let Some(bridge) = bridge {
            bridge.deliver(value.topic, value.value);
            self.moved();
        }
        Ok(())
    }

    async fn call_tool(&self, call: ToolCall) -> Result<ToolReply, RpcError> {
        let Some(app) = self.running(&call.app) else {
            return Ok(ToolReply::Failed {
                failure: harnix_app_port::Failure::Unavailable,
                message: harnix_wasm::CallFailure::Unavailable(format!("`{}` is stopped", call.app))
                    .to_string(),
            });
        };
        let Some(vm) = &app.vm else {
            return Err(RpcError::not_found(format!("`{}` has no component", call.app)));
        };
        let (cancel, _running) = self.cancellation(&call.call_id);
        let _busy = app.activity.hold();
        let (sink, forwarded) = if call.progress {
            let (tx, mut rx) = tokio::sync::mpsc::channel::<String>(crate::bridge::QUEUE);
            let (platform, app_id, tool, call_id) = (
                self.platform.clone(),
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
                    let _ = platform.progress(progress).await;
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

    async fn hook(&self, call: HookCall) -> Result<HookReply, RpcError> {
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
        let decided = vm.on_event(&call.event, &call.payload, &cancel).await;
        self.moved();
        Ok(match decided {
            Ok(HookDecision::Proceed) => HookReply::Proceed,
            Ok(HookDecision::Block(reason)) => HookReply::Block(reason),
            Ok(HookDecision::AddContext(note)) => HookReply::AddContext(note),
            Err(failure) => HookReply::Failed(failure.to_string()),
        })
    }

    async fn cancel(&self, call_id: String) -> Result<(), RpcError> {
        if let Some(token) = lock(&self.calls).get(&call_id) {
            token.cancel();
        }
        Ok(())
    }

    async fn shutdown(&self, reason: String) -> Result<(), RpcError> {
        let apps: Vec<(String, Arc<Running>)> = std::mem::take(&mut *write(&self.apps)).into_iter().collect();
        lock(&self.bridges).clear();
        for (id, app) in apps {
            app.watch.cancel();
            if let Some(vm) = &app.vm {
                vm.stop().await;
            }
            tracing::debug!(app = %id, %reason, "application stopped");
        }
        self.moved();
        Ok(())
    }
}
