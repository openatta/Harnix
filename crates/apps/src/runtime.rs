//! Installing, starting, stopping and removing applications: [`Runtime`], the
//! `harnix_builtin::apps::AppsRuntime` the `apps` module drives.
//!
//! One VM per application, in this process (`harnix-wasm`). What is running is in a [`Registry`]
//! the plugin (`plugin.rs`) and the HTTP side (`serve.rs`) read: an application is in it exactly
//! while its tools are on offer and its interface modules are served. Every change to it is told
//! to whoever assembled the process ([`Runtime::on_change`]), which republishes the interface
//! modules and the MCP servers spawned daemons are configured with.
//!
//! # Installing, in order, and backwards on a failure
//!
//! 1. The package is read and checked (`package.rs`).
//! 2. It is unpacked to `<apps>/<id>/<version>/`.
//! 3. Its `agent/` is zipped and handed to Agix's `plugin.install` in each instance it names; its
//!    `agent/skills/<name>/` are copied into each one's skills directory and the instance re-reads
//!    its configuration.
//! 4. It is started: registered with the kernel — its methods, topics and spaces, under
//!    `app.<id>.*`, and its `hub` capability (`backend.rs`) — then the component loaded (refused if
//!    it was compiled for another engine), its VM started (a backend's `init` runs, and may already
//!    publish, call and subscribe), its tools listed, its interface modules served.
//!
//! A step that fails undoes every step before it, and the application is not installed. The
//! `apps` module writes the record last.
//!
//! # Signed, and unsigned
//!
//! A package's signatures (`signature.rs`) are verified before anything of it is unpacked, and
//! again — over the files as they are on disk — every time it is started, before its compiled
//! component is deserialized; the component's bytes are checked against their signed sha256 as
//! they are read. A package countersigned by the store, or signed by a publisher the profile
//! trusts, runs in any process that hosts applications. An unsigned one is installed and started
//! only with `[apps] dev = true`: an application installed unsigned does not start in a process
//! that does not trust unsigned code.
//!
//! Updating, the catalog and fetching from the store are in `acquire.rs`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};

use harnix_builtin::apps::{AppsRuntime, Disclosed, InstallRecord, PackageInfo};
use harnix_contract::{codes, RpcError};
use harnix_module::ModuleContext;
use harnix_module::{AppHandle, AppMethod, AppTopic, Principal};
use harnix_wasm::{AppSpec, AppVm, Capabilities, FsGrant, Limits, ToolDef, VmState, WasmEngine, World};
use serde_json::{json, Map, Value};
use tokio_util::sync::CancellationToken;

use crate::backend::{Bridge, Method, Stats, VmSlot};
use crate::manifest::{toml_to_json, Interface, Manifest, Surface, Title, UiEntry, MANIFEST};
use crate::package;
use crate::signature::{Keys, Trust, Verified};

/// What marks a skill directory as an application's, inside an instance's skills directory.
const SKILL_MARK: &str = ".harnix-app";

/// One application that is running.
pub struct Loaded {
    pub id: String,
    pub version: String,
    pub title: Option<String>,
    /// The unpacked package.
    pub root: PathBuf,
    pub vm: Option<AppVm>,
    /// What the component says it provides, listed once when it started.
    pub tools: Vec<ToolDef>,
    pub ui: Vec<UiEntry>,
    /// Its isolated interface: `[interface]` and the `[[surface]]`s its frames fill.
    pub interface: Interface,
    pub surfaces: Vec<Surface>,
    /// How far its package was trusted when it started: whether its interface runs in the page
    /// (its `[[ui]]` modules) or in frames (its surfaces) is decided from this.
    pub trust: Trust,
    /// Its `[[space]]` ids, in order: where a frame's new sessions land.
    pub backend_spaces: Vec<String>,
    /// What a spawned daemon's MCP client presents for this application's endpoint.
    pub token: String,
    /// Its registration with the kernel: its methods, topics, spaces and `hub` capability.
    pub handle: Option<AppHandle>,
    /// What it declared, by full name: `app.<id>.<name>`.
    pub methods: Vec<String>,
    pub topics: Vec<String>,
    pub spaces: Vec<String>,
    pub hub: Vec<String>,
    /// Its calls, errors and refusals, for the system space.
    pub stats: Arc<Stats>,
}

/// What starting an application made.
struct Started {
    vm: Option<AppVm>,
    tools: Vec<ToolDef>,
    handle: Option<AppHandle>,
    stats: Arc<Stats>,
}

impl Loaded {
    /// The name a session sees one of its tools by: `app__<id>__<tool>`, with the id's dots as
    /// `_` (a model's tool names are letters, digits, `_` and `-`; an id has no `_`, so this is
    /// one-to-one).
    pub fn tool_name(&self, tool: &str) -> String {
        tool_name(&self.id, tool)
    }
}

/// Every application tool's name matches this: what each scene of an instance with the plugin
/// has to admit besides its own tool list.
pub const TOOL_PATTERN: &str = "app__*";

pub fn tool_name(app: &str, tool: &str) -> String {
    format!("app__{}__{tool}", app.replace('.', "_"))
}

/// The name a spawned daemon knows this application's MCP server by (its tools are
/// `mcp__<server>__<tool>` there).
pub fn server_name(app: &str) -> String {
    format!("app-{}", app.replace('.', "_"))
}

/// Where a running tool call's progress is told: `{app, tool, session_id, tool_use_id, text}`.
pub type OnProgress = Arc<dyn Fn(Value) + Send + Sync>;

/// What is running, shared with the plugin and the HTTP side.
#[derive(Default)]
pub struct Registry {
    apps: RwLock<BTreeMap<String, Arc<Loaded>>>,
    progress: RwLock<Option<OnProgress>>,
}

impl Registry {
    /// Tell whoever listens about a tool call's progress.
    pub fn progress(&self, value: Value) {
        let f = self
            .progress
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        if let Some(f) = f {
            f(value);
        }
    }

    pub fn loaded(&self) -> Vec<Arc<Loaded>> {
        self.apps
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .values()
            .cloned()
            .collect()
    }

    pub fn get(&self, id: &str) -> Option<Arc<Loaded>> {
        self.apps
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(id)
            .cloned()
    }

    fn insert(&self, loaded: Arc<Loaded>) {
        self.apps
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(loaded.id.clone(), loaded);
    }

    fn remove(&self, id: &str) -> Option<Arc<Loaded>> {
        self.apps
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .remove(id)
    }
}

type OnChange = Arc<dyn Fn() + Send + Sync>;

/// What the process tells the runtime at assembly.
#[derive(Default)]
pub struct Options {
    /// `[apps] dev`.
    pub dev: bool,
    /// `[apps.config]`, by application id.
    pub config: BTreeMap<String, toml::Value>,
    /// What signatures are verified with: the built-in store key (`store::store_keys`) and the
    /// profile's `[apps.trusted_publishers]`.
    pub keys: Keys,
    /// `[apps] store`: the catalog's URL (`store::DEFAULT_STORE` when the profile names none).
    pub store: Option<String>,
}

pub struct Runtime {
    pub(crate) engine: WasmEngine,
    registry: Arc<Registry>,
    pub(crate) options: Options,
    on_change: Mutex<Option<OnChange>>,
    /// Where this process listens, once it does: the MCP endpoints' base.
    base: RwLock<Option<String>>,
    /// Why an application that should be running is not.
    pub(crate) failed: Mutex<BTreeMap<String, String>>,
}

pub(crate) fn refused(message: impl Into<String>) -> RpcError {
    RpcError::refused(message)
}

pub(crate) fn internal(message: impl Into<String>) -> RpcError {
    RpcError::new(codes::INTERNAL_ERROR, message)
}

impl Runtime {
    pub fn new(options: Options) -> anyhow::Result<Arc<Self>> {
        Ok(Arc::new(Self {
            engine: WasmEngine::new()?,
            registry: Arc::new(Registry::default()),
            options,
            on_change: Mutex::new(None),
            base: RwLock::new(None),
            failed: Mutex::new(BTreeMap::new()),
        }))
    }

    pub fn registry(&self) -> Arc<Registry> {
        self.registry.clone()
    }

    /// The Agix plugin an in-process instance's assembly carries ([`crate::AppsPlugin`]).
    pub fn plugin(&self) -> Arc<dyn runtime::plugin::Plugin> {
        Arc::new(crate::AppsPlugin::new(self.registry.clone()))
    }

    pub fn engine(&self) -> &WasmEngine {
        &self.engine
    }

    /// Told every `host.progress` of a tool call from an in-process session. (The engine's own
    /// progress channel for a tool is not delivered to a client by Agix 0.3.9, so this is how a
    /// client sees an application's tool working.)
    pub fn on_progress(&self, f: impl Fn(Value) + Send + Sync + 'static) {
        *self
            .registry
            .progress
            .write()
            .unwrap_or_else(|e| e.into_inner()) = Some(Arc::new(f));
    }

    /// Told after every change to what is running, and when the listener's address is learnt.
    pub fn on_change(&self, f: impl Fn() + Send + Sync + 'static) {
        *self.on_change.lock().unwrap_or_else(|e| e.into_inner()) = Some(Arc::new(f));
    }

    fn changed(&self) {
        let f = self
            .on_change
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        if let Some(f) = f {
            f();
        }
    }

    /// Where this process listens (`http://127.0.0.1:<port>/`): what a spawned daemon is pointed
    /// at for an application's MCP endpoint.
    pub fn set_listening(&self, url: &str) {
        *self.base.write().unwrap_or_else(|e| e.into_inner()) =
            Some(url.trim_end_matches('/').to_string());
        self.changed();
    }

    /// The interface of every running application, as the page is told it at hello: a trusted
    /// publisher's `[[ui]]` modules, which the page imports; everyone else's (and a trusted one
    /// that asked for it) as its surfaces, which the shell draws as isolated frames
    /// (`crate::surface`). A module that will not run is listed as such, with why, so the page can
    /// say so in its diagnostics.
    pub fn ui_contributions(&self) -> Vec<Value> {
        self.registry
            .loaded()
            .iter()
            .flat_map(|app| ui_entries(app))
            .collect()
    }

    /// The MCP servers a spawned daemon is configured with: one per running application with
    /// tools. Empty until the listener's address is known.
    pub fn mcp_servers(&self) -> Map<String, Value> {
        let Some(base) = self.base.read().unwrap_or_else(|e| e.into_inner()).clone() else {
            return Map::new();
        };
        self.registry
            .loaded()
            .iter()
            .filter(|app| !app.tools.is_empty())
            .map(|app| {
                (
                    server_name(&app.id),
                    json!({
                        "type": "streamable_http",
                        "url": format!("{base}/apps/{}/mcp", app.id),
                        "headers": {"Authorization": format!("Bearer {}", app.token)},
                    }),
                )
            })
            .collect()
    }

    pub(crate) fn manifest_in(root: &Path) -> Result<Manifest, RpcError> {
        let text = std::fs::read_to_string(root.join(MANIFEST))
            .map_err(|e| internal(format!("{}: {e}", root.join(MANIFEST).display())))?;
        Manifest::parse(&text).map_err(refused)
    }

    /// Register the application with the kernel, then start its VM: so what `init` does already
    /// reaches the process, and a method called before the VM is up is answered "not running".
    /// A VM that does not start takes the registration back with it. `verified`: as for
    /// [`Self::start_vm`].
    async fn start_app(
        &self,
        cx: &ModuleContext,
        manifest: &Manifest,
        root: &Path,
        app_dir: &Path,
        verified: &Verified,
    ) -> Result<Started, RpcError> {
        let id = &manifest.app.id;
        let bridge = Bridge::new(id, manifest.topics.iter().map(|t| t.name.clone()));
        let slot = Arc::new(VmSlot::default());
        let mut spec = harnix_module::AppSpec::new(id.clone())
            .hub(manifest.capabilities.hub.clone())
            .topic_sink(bridge.sink());
        for m in &manifest.methods {
            let schema = match &m.params {
                Some(params) => Some(jsonschema::validator_for(&toml_to_json(params)).map_err(|e| {
                    refused(format!("`{id}`'s method `{}` has a schema that does not compile: {e}", m.name))
                })?),
                None => None,
            };
            let mut principals = Vec::new();
            for subject in m.subject.names() {
                principals.push(match subject {
                    "device" => Principal::Device,
                    "module" => Principal::AnyModule,
                    _ => Principal::Kernel,
                });
            }
            principals.push(Principal::Kernel);
            principals.dedup();
            spec = spec.method_with(AppMethod {
                name: m.name.clone(),
                principals,
                description: m.description.clone().unwrap_or_default(),
                handler: Arc::new(Method {
                    app: id.clone(),
                    name: m.name.clone(),
                    schema,
                    vm: slot.clone(),
                    stats: bridge.stats.clone(),
                }),
            });
        }
        for t in &manifest.topics {
            spec = spec.topic_with(AppTopic {
                name: t.name.clone(),
                params: Vec::new(),
                principals: vec![Principal::Device, Principal::AnyModule],
            });
        }
        for space in &manifest.spaces {
            let mut decl = harnix_contract::SpaceDecl::new(space.id.clone());
            if let Some(instance) = &space.instance {
                decl = decl.instance(instance);
            }
            if let Some(scene) = &space.scene {
                decl = decl.scene(scene.clone());
            }
            spec = spec.space(decl);
        }
        let handle = match cx.apps() {
            Some(host) => {
                let handle = host.register(spec).await?;
                bridge.bind(handle.clone());
                Some(handle)
            }
            None => None,
        };
        match self.start_vm(manifest, root, app_dir, bridge.clone(), verified).await {
            Ok((vm, tools)) => {
                if let Some(vm) = &vm {
                    slot.set(vm.downgrade());
                    bridge.started(vm.downgrade());
                }
                Ok(Started {
                    vm,
                    tools,
                    handle,
                    stats: bridge.stats.clone(),
                })
            }
            Err(e) => {
                if let Some(handle) = &handle {
                    if let Err(w) = handle.withdraw().await {
                        tracing::warn!(app = %id, error = %w.message, "a registration was not taken back");
                    }
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
        if !verified.trust.signed() && !self.options.dev {
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
        let declared = if component.world == crate::manifest::WORLDS[1] {
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
            config: manifest
                .config_with(crate::manifest::overrides_for(&self.options.config, id).as_ref()),
        };
        let kv = harnix_wasm::FileKv::open(app_dir.join("kv"))
            .map_err(|e| internal(format!("`{id}`'s storage: {e}")))?;
        let services = harnix_wasm::Services {
            host: bridge,
            kv: Arc::new(kv),
            http: Arc::new(crate::host::Http::new()),
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
            let full = tool_name(id, &tool.name);
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
        Ok((Some(vm), tools))
    }

    /// The agent package and skills, in the instances the install put them in: switched on (or
    /// off) with the application.
    async fn switch_agent(
        &self,
        cx: &ModuleContext,
        id: &str,
        root: &Path,
        parts: &Value,
        on: bool,
    ) -> Result<(), RpcError> {
        if let Some(agent) = parts.get("agent").filter(|a| !a.is_null()) {
            let name = agent["package"].as_str().unwrap_or_default();
            for instance in strings(&agent["instances"]) {
                let method = if on {
                    "plugin.enable"
                } else {
                    "plugin.disable"
                };
                if let Err(e) = cx
                    .call(method, json!({"name": name, "instance": instance}))
                    .await
                {
                    if on {
                        return Err(refused(format!(
                            "`{id}`'s agent package could not be switched on in `{instance}`: {}",
                            e.message
                        )));
                    }
                    tracing::warn!(app = %id, %instance, error = %e.message, "the agent package was not switched off");
                }
            }
        }
        if let Some(skills) = parts.get("skills").filter(|s| s.is_object()) {
            for (instance, names) in skills.as_object().into_iter().flatten() {
                let names = strings(names);
                let result = if on {
                    place_skills(cx, id, instance, root, &names)
                } else {
                    remove_skills(cx, id, instance, &names);
                    Ok(())
                };
                result?;
                reload(cx, instance).await;
            }
        }
        Ok(())
    }
}

/// Whether a running application's interface is drawn in frames rather than imported into the page.
fn frames(app: &Loaded) -> bool {
    crate::surface::runs_in_frames(app.trust, &app.interface, !app.ui.is_empty())
}

/// What the page is told of one running application's interface (`Runtime::ui_contributions`).
fn ui_entries(app: &Loaded) -> Vec<Value> {
    let id = &app.id;
    if !frames(app) {
        return app
            .ui
            .iter()
            .map(|entry| {
                json!({
                    "plugin": id,
                    "point": entry.point,
                    "api_version": entry.api_version,
                    "module": format!("/apps/{id}/ui/{}", entry.module),
                })
            })
            .collect();
    }
    let interface = json!({
        "hub": app.interface.hub,
        "links": app.interface.links,
        "trust": app.trust.as_str(),
        "title": app.title,
        "spaces": app.backend_spaces,
    });
    // A module that runs in the page runs with the page's rights: not this application's.
    let refused = app.ui.iter().map(|entry| {
        json!({
            "plugin": id,
            "point": entry.point,
            "api_version": entry.api_version,
            "refused": entry.module,
            "reason": format!(
                "`{}` runs in the page, which only a trusted publisher's interface does ({} here); this \
                 application's interface is its isolated surfaces",
                entry.module,
                app.trust.as_str()
            ),
        })
    });
    let surfaces = app.surfaces.iter().map(|s| {
        // Where a space's sessions go: its own backend space when the application declares one by
        // that id, else the application's first.
        let sessions = s.space.as_ref().and_then(|space| {
            app.backend_spaces
                .iter()
                .find(|b| *b == space)
                .or(app.backend_spaces.first())
                .cloned()
        });
        json!({
            "plugin": id,
            "point": s.point,
            "api_version": harnix_contract::CONTRIB_API_VERSION,
            "frame": s.page.as_ref().map(|page| format!("/apps/{id}/ui/{page}")),
            "surface": {
                "id": s.id(),
                "title": s.title.as_ref().map(title_json),
                "space": s.space,
                "sessions": sessions,
                "icon": s.icon,
                "order": s.order,
                "center": s.center,
                "kinds": s.kinds,
                "wide": s.wide,
            },
            "interface": interface,
        })
    });
    refused.chain(surfaces).collect()
}

fn title_json(title: &Title) -> Value {
    match title {
        Title::One(text) => json!({"en": text}),
        Title::ByLocale(by) => json!(by),
    }
}

/// A stamp of everything under an application's `ui/`: each file's path, size and modification
/// time. Read on every look at the application (once a second, by the `apps` module's topic), so
/// it is a watcher that needs no thread: a file saved there changes it.
fn ui_rev(dir: &Path) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    let mut stack = vec![dir.to_path_buf()];
    let mut files = Vec::new();
    while let Some(at) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&at) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            match entry.metadata() {
                Ok(meta) if meta.is_dir() => stack.push(path),
                Ok(meta) => files.push((path, meta.len(), meta.modified().ok())),
                Err(_) => {}
            }
        }
    }
    files.sort();
    files.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array()
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// An instance re-reads its configuration — its skills directory included.
async fn reload(cx: &ModuleContext, instance: &str) {
    if let Err(e) = cx
        .call("config.reload", json!({"instance": instance}))
        .await
    {
        tracing::warn!(%instance, error = %e.message, "the instance did not reload");
    }
}

fn skills_dir(cx: &ModuleContext, instance: &str) -> Result<PathBuf, RpcError> {
    cx.instance(instance)
        .map(|h| h.data_root.join("skills"))
        .ok_or_else(|| refused(format!("there is no instance `{instance}`")))
}

/// Copy `agent/skills/<name>/` into the instance's skills directory, marked as `id`'s. A
/// directory of that name that is not `id`'s is somebody else's skill, and refuses it.
fn place_skills(
    cx: &ModuleContext,
    id: &str,
    instance: &str,
    root: &Path,
    names: &[String],
) -> Result<(), RpcError> {
    let dir = skills_dir(cx, instance)?;
    for name in names {
        let target = dir.join(name);
        if target.exists()
            && std::fs::read_to_string(target.join(SKILL_MARK))
                .ok()
                .as_deref()
                != Some(id)
        {
            return Err(refused(format!(
                "the instance `{instance}` has a skill `{name}` of its own; `{id}` would replace it"
            )));
        }
        let _ = std::fs::remove_dir_all(&target);
        let source = root.join("agent").join("skills").join(name);
        let mut files = Vec::new();
        package::walk(&source, &source, &mut files)
            .map_err(|e| internal(format!("{}: {e}", source.display())))?;
        for (relative, path) in files {
            let to = target.join(&relative);
            if let Some(parent) = to.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| internal(format!("{}: {e}", parent.display())))?;
            }
            std::fs::copy(&path, &to).map_err(|e| internal(format!("{}: {e}", to.display())))?;
        }
        std::fs::write(target.join(SKILL_MARK), id).map_err(|e| internal(e.to_string()))?;
    }
    Ok(())
}

/// Remove the skills marked as `id`'s, and nothing else.
fn remove_skills(cx: &ModuleContext, id: &str, instance: &str, names: &[String]) {
    let Ok(dir) = skills_dir(cx, instance) else {
        return;
    };
    for name in names {
        let target = dir.join(name);
        if std::fs::read_to_string(target.join(SKILL_MARK))
            .ok()
            .as_deref()
            == Some(id)
        {
            if let Err(e) = std::fs::remove_dir_all(&target) {
                tracing::warn!(app = %id, skill = %name, error = %e, "a skill was not removed");
            }
        }
    }
}

/// What installing put in place, as the record keeps it.
#[derive(Default)]
pub(crate) struct Placed {
    pub(crate) agent: Option<(String, String, Vec<String>)>,
    pub(crate) skills: BTreeMap<String, Vec<String>>,
    /// The engine's disclosure of the agent package, for the installer to show.
    pub(crate) disclosure: Value,
    /// What its signatures made it.
    pub(crate) trust: Option<Trust>,
}

impl Placed {
    pub(crate) fn parts(&self) -> Value {
        json!({
            "agent": self.agent.as_ref().map(|(package, version, instances)| json!({
                "package": package, "version": version, "instances": instances,
            })),
            "skills": self.skills,
            "trust": self.trust.unwrap_or(Trust::Unsigned).as_str(),
        })
    }

    /// What a record's `parts` say was put in place.
    pub(crate) fn of(parts: &Value) -> Self {
        let mut placed = Placed::default();
        if let Some(agent) = parts.get("agent").filter(|a| !a.is_null()) {
            placed.agent = Some((
                agent["package"].as_str().unwrap_or_default().to_string(),
                agent["version"].as_str().unwrap_or_default().to_string(),
                strings(&agent["instances"]),
            ));
        }
        if let Some(skills) = parts.get("skills").and_then(Value::as_object) {
            for (instance, names) in skills {
                placed.skills.insert(instance.clone(), strings(names));
            }
        }
        placed
    }

    /// Take it all back, as far as it goes.
    pub(crate) async fn undo(&self, cx: &ModuleContext, id: &str) {
        for (instance, names) in &self.skills {
            remove_skills(cx, id, instance, names);
            reload(cx, instance).await;
        }
        if let Some((package, version, instances)) = &self.agent {
            for instance in instances {
                let params = json!({"name": package, "version": version, "instance": instance});
                if let Err(e) = cx.call("plugin.uninstall", params).await {
                    tracing::warn!(app = %id, %instance, error = %e.message, "the agent package was not uninstalled");
                }
            }
        }
    }
}

impl Runtime {
    /// Put a package in place under `app_dir` and start it: unpack it (from `package`, when it is
    /// not unpacked already), install its agent package and skills into the instances it names,
    /// and load it. What was put where is written to `placed` as it is done, so that whoever
    /// called this can take back exactly that when it fails.
    pub(crate) async fn place(
        &self,
        cx: &ModuleContext,
        package: Option<&Path>,
        inspected: &package::Inspected,
        app_dir: &Path,
        placed: &mut Placed,
    ) -> Result<Value, RpcError> {
        let manifest = &inspected.manifest;
        let id = manifest.app.id.clone();
        let root = app_dir.join(&manifest.app.version);
        if let Some(package) = package {
            package::extract(package, &root).map_err(refused)?;
        }
        let targets = &manifest.agent.install_into;
        for target in targets {
            if cx.instance(target).is_none() {
                return Err(refused(format!(
                    "`{id}` installs its agent package into `{target}`, and there is no such instance"
                )));
            }
        }
        if let Some(agent) = &inspected.agent {
            let zip = app_dir.join("agent.zip");
            package::zip_dir(&root.join("agent"), &zip).map_err(internal)?;
            let mut instances = Vec::new();
            let mut failure = None;
            let mut disclosed = Value::Null;
            for target in targets {
                let params = json!({
                    "name": agent.name,
                    "version": agent.version,
                    "download_url": format!("file://{}", zip.display()),
                    "instance": target,
                });
                match cx.call("plugin.install", params).await {
                    Ok(answer) => {
                        // What the engine says the package puts in front of the model: the
                        // installer shows it.
                        if disclosed.is_null() {
                            disclosed = answer.get("disclosure").cloned().unwrap_or(answer);
                        }
                        instances.push(target.clone());
                    }
                    Err(e) => {
                        failure = Some(refused(format!(
                            "`{id}`'s agent package did not install into `{target}`: {}",
                            e.message
                        )));
                        break;
                    }
                }
            }
            let _ = std::fs::remove_file(&zip);
            placed.agent = Some((agent.name.clone(), agent.version.clone(), instances));
            placed.disclosure = disclosed;
            if let Some(e) = failure {
                return Err(e);
            }
        }
        if !inspected.skills.is_empty() {
            for target in targets {
                // Recorded before it is done: undoing removes only what is marked as ours.
                placed
                    .skills
                    .insert(target.clone(), inspected.skills.clone());
                place_skills(cx, &id, target, &root, &inspected.skills)?;
                reload(cx, target).await;
            }
        }
        let record = InstallRecord {
            id: id.clone(),
            version: manifest.app.version.clone(),
            title: manifest.app.title.clone(),
            enabled: true,
            parts: placed.parts(),
        };
        self.load(cx, &record, app_dir).await
    }

    /// What an install or an update tells its caller.
    pub(crate) fn summary(&self, placed: &Placed, started: Value) -> Value {
        json!({
            "fingerprint": self.engine.fingerprint(),
            "trust": placed.trust.unwrap_or(Trust::Unsigned).as_str(),
            "agent": placed.agent.as_ref().map(|(package, _, instances)| json!({
                "package": package, "instances": instances, "disclosure": placed.disclosure,
            })),
            "skills": placed.skills,
            "started": started,
        })
    }
}

#[async_trait::async_trait]
impl AppsRuntime for Runtime {
    fn fingerprint(&self) -> String {
        self.engine.fingerprint().to_string()
    }

    fn inspect(&self, path: &Path) -> Result<PackageInfo, RpcError> {
        let inspected = package::inspect(path, self.engine.fingerprint()).map_err(refused)?;
        let app = inspected.manifest.app;
        Ok(PackageInfo {
            id: app.id,
            version: app.version,
            title: app.title,
        })
    }

    async fn install(
        &self,
        cx: &ModuleContext,
        path: &Path,
        app_dir: &Path,
    ) -> Result<(Value, Value), RpcError> {
        let inspected = package::inspect(path, self.engine.fingerprint()).map_err(refused)?;
        // Before a byte of it is written.
        let verified = self.admit_zip(path, &inspected.manifest)?;
        let id = inspected.manifest.app.id.clone();
        std::fs::create_dir_all(app_dir)
            .map_err(|e| internal(format!("{}: {e}", app_dir.display())))?;
        let mut placed = Placed {
            trust: Some(verified.trust),
            ..Placed::default()
        };
        let outcome = self
            .place(cx, Some(path), &inspected, app_dir, &mut placed)
            .await;
        match outcome {
            Ok(started) => Ok((placed.parts(), self.summary(&placed, started))),
            Err(e) => {
                tracing::warn!(app = %id, error = %e.message, "the install failed; undoing it");
                placed.undo(cx, &id).await;
                let _ = std::fs::remove_dir_all(app_dir);
                self.failed
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .remove(&id);
                Err(e)
            }
        }
    }

    async fn load(
        &self,
        cx: &ModuleContext,
        record: &InstallRecord,
        app_dir: &Path,
    ) -> Result<Value, RpcError> {
        let id = &record.id;
        if let Some(running) = self.registry.get(id) {
            // Set aside after faulting: enabling it again is starting it again, from nothing.
            let faulted = running
                .vm
                .as_ref()
                .is_some_and(|vm| matches!(vm.health().state, VmState::Disabled { .. }));
            if !faulted {
                return Ok(self.status(id));
            }
            self.unload(cx, record).await;
        }
        let root = app_dir.join(&record.version);
        let attempt = async {
            let manifest = Self::manifest_in(&root)?;
            if &manifest.app.id != id || manifest.app.version != record.version {
                return Err(internal(format!(
                    "{} says it is `{}` {}, and the record `{id}` {}",
                    root.display(),
                    manifest.app.id,
                    manifest.app.version,
                    record.version
                )));
            }
            // Every start, over the files as they are now: what is on disk may not be what was
            // installed.
            let verified = self.admit_root(&root, &manifest)?;
            self.switch_agent(cx, id, &root, &record.parts, true)
                .await?;
            let started = match self.start_app(cx, &manifest, &root, app_dir, &verified).await {
                Ok(started) => started,
                Err(e) => {
                    self.switch_agent(cx, id, &root, &record.parts, false)
                        .await
                        .ok();
                    return Err(e);
                }
            };
            let full = |name: &String| format!("app.{id}.{name}");
            Ok(Arc::new(Loaded {
                id: id.clone(),
                version: record.version.clone(),
                title: manifest.app.title.clone(),
                root: root.clone(),
                vm: started.vm,
                tools: started.tools,
                ui: manifest.ui.clone(),
                interface: manifest.interface.clone(),
                surfaces: manifest.surfaces.clone(),
                trust: verified.trust,
                backend_spaces: manifest.spaces.iter().map(|s| s.id.clone()).collect(),
                token: uuid::Uuid::new_v4().simple().to_string(),
                handle: started.handle,
                methods: manifest.methods.iter().map(|m| full(&m.name)).collect(),
                topics: manifest.topics.iter().map(|t| full(&t.name)).collect(),
                spaces: manifest.spaces.iter().map(|s| s.id.clone()).collect(),
                hub: manifest.capabilities.hub.clone(),
                stats: started.stats,
            }))
        };
        match attempt.await {
            Ok(loaded) => {
                tracing::info!(app = %id, tools = loaded.tools.len(), ui = loaded.ui.len(), "application started");
                self.registry.insert(loaded);
                self.failed
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .remove(id);
                self.changed();
                Ok(self.status(id))
            }
            Err(e) => {
                self.failed
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(id.clone(), e.message.clone());
                Err(e)
            }
        }
    }

    async fn unload(&self, cx: &ModuleContext, record: &InstallRecord) {
        self.failed
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&record.id);
        let Some(loaded) = self.registry.remove(&record.id) else {
            return;
        };
        // Out of the registry first: a session built from here on has none of its tools, and the
        // page is told its modules are gone. A call already running holds its own handle.
        self.changed();
        // Then its names: a call decided from here is unknown. (The `apps` module has usually
        // withdrawn them already; then there is nothing left to take.) Calls queued behind the
        // backend are answered that it stopped.
        if let Some(handle) = &loaded.handle {
            if handle.withdraw().await.is_ok() {
                tracing::debug!(app = %record.id, "registration withdrawn");
            }
        }
        if let Some(vm) = &loaded.vm {
            vm.stop().await;
        }
        self.switch_agent(cx, &record.id, &loaded.root, &record.parts, false)
            .await
            .ok();
        tracing::info!(app = %record.id, "application stopped");
    }

    async fn uninstall(
        &self,
        cx: &ModuleContext,
        record: &InstallRecord,
        _app_dir: &Path,
    ) -> Result<Value, RpcError> {
        let placed = Placed::of(&record.parts);
        placed.undo(cx, &record.id).await;
        Ok(json!({
            "agent": placed.agent.as_ref().map(|(package, _, instances)| json!({"package": package, "instances": instances})),
            "skills": placed.skills,
        }))
    }

    fn status(&self, id: &str) -> Value {
        if let Some(app) = self.registry.get(id) {
            let health = app.vm.as_ref().map(|vm| vm.health());
            let (state, reason) = match health.as_ref().map(|h| &h.state) {
                Some(VmState::Disabled { reason }) => ("faulted", Some(reason.clone())),
                _ => ("running", None),
            };
            return json!({
                "state": state,
                "reason": reason,
                "world": app.vm.as_ref().map(|vm| vm.world()),
                "methods": app.methods,
                "topics": app.topics,
                "spaces": app.spaces,
                "hub": app.hub,
                "calls": app.stats.report(),
                "tools": app.tools.iter().map(|t| json!({
                    "name": app.tool_name(&t.name),
                    "tool": t.name,
                    "description": t.description,
                })).collect::<Vec<_>>(),
                "ui": app.ui.iter().map(|u| format!("/apps/{id}/ui/{}", u.module)).collect::<Vec<_>>(),
                "frames": frames(&app),
                "surfaces": app.surfaces.iter().map(|s| format!("{}:{}", s.point, s.id())).collect::<Vec<_>>(),
                // An unsigned application's interface, in developer mode, changes under it: the
                // page reloads its frames when this does.
                "ui_rev": (self.options.dev && app.trust == Trust::Unsigned).then(|| ui_rev(&app.root.join("ui"))),
                "mcp": format!("/apps/{id}/mcp"),
                "health": health.map(|h| serde_json::to_value(h).unwrap_or(Value::Null)),
            });
        }
        match self
            .failed
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(id)
        {
            Some(reason) => json!({"state": "failed", "reason": reason}),
            None => json!({"state": "stopped"}),
        }
    }

    async fn shutdown(&self) {
        for app in self.registry.loaded() {
            self.registry.remove(&app.id);
            if let Some(vm) = &app.vm {
                vm.stop().await;
            }
        }
    }

    fn disclose(
        &self,
        package: &Path,
        installed: Option<(&InstallRecord, &Path)>,
    ) -> Result<Disclosed, RpcError> {
        self.disclose_package(package, installed)
    }

    async fn catalog(&self) -> Result<Value, RpcError> {
        self.catalog_view().await
    }

    async fn fetch(
        &self,
        id: &str,
        version: Option<&str>,
        staging: &Path,
    ) -> Result<PathBuf, RpcError> {
        self.fetch_package(id, version, staging).await
    }

    async fn update(
        &self,
        cx: &ModuleContext,
        record: &InstallRecord,
        package: &Path,
        app_dir: &Path,
    ) -> Result<(Value, Value), RpcError> {
        self.update_to(cx, record, package, app_dir).await
    }

    fn describe(&self, record: &InstallRecord, app_dir: &Path) -> Value {
        self.describe_app(record, app_dir)
    }
}
