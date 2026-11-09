//! Installing, starting, stopping and removing applications embedded in this process:
//! [`Runtime`], the `harnix_builtin::apps::AppsRuntime` the `apps` module drives.
//!
//! The applications run in a `harnix_app_host::Host` — one VM each, in this process — which this
//! drives through its `Control` and lends this process through its `Platform`
//! (`platform.rs`). What is running is in a [`Registry`] the plugin (`plugin.rs`) and the HTTP
//! side (`serve.rs`) read: an application is in it exactly while its tools are on offer and its
//! interface is served. Every change to it is told to whoever assembled the process
//! ([`Runtime::on_change`]), which republishes the interface and the MCP servers spawned daemons
//! are configured with.
//!
//! # Installing, in order, and backwards on a failure
//!
//! 1. The package is read and checked (`harnix_app_package::package`), and its signatures
//!    verified before anything of it is unpacked.
//! 2. It is unpacked to `<apps>/<id>/<version>/`.
//! 3. Its agent package and skills are put in place by the host (`Control::place`): its `agent/`
//!    handed to Agix's `plugin.install` in each instance it names, its `agent/skills/<name>/`
//!    copied into each one's skills directory, and the instance re-reads its configuration.
//! 4. It is started (`Control::start`): verified again over its files on disk, registered with the
//!    kernel — its methods, topics and spaces, under `app.<id>.*`, and its `hub` capability — its
//!    VM started, its tools listed and offered; then its interface served.
//!
//! A step that fails undoes every step before it, and the application is not installed. The
//! `apps` module writes the record last.
//!
//! # Signed, and unsigned
//!
//! A package countersigned by the store, or signed by a publisher the profile trusts, runs in any
//! process that hosts applications. An unsigned one is installed and started only with
//! `[apps] dev = true`: an application installed unsigned does not start in a process that does
//! not trust unsigned code.
//!
//! Updating is in `update.rs`.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, RwLock, Weak};

use harnix_app_host::{Host, HostOptions};
use harnix_app_package::acquire::{self, internal, refused, Admission};
use harnix_app_package::manifest::{Interface, Manifest, Surface, Title, UiEntry};
use harnix_app_package::package;
use harnix_app_package::signature::{Keys, Trust};
use harnix_app_port::{Control, Offer, Place, Start, Stop, ToolDef, Unplace};
use harnix_builtin::apps::{AppsRuntime, Disclosed, InstallRecord, PackageInfo, UpdateFailed};
use harnix_contract::RpcError;
use harnix_module::ModuleContext;
use serde_json::{json, Map, Value};

use crate::platform::KernelPlatform;
pub use harnix_app_package::{server_name, tool_name, TOOL_PATTERN};

/// One application that is running.
pub struct Loaded {
    pub id: String,
    pub version: String,
    pub title: Option<String>,
    /// The unpacked package.
    pub root: PathBuf,
    /// It has a component: its tools are offered, and its events run around every tool call.
    pub component: bool,
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
    /// `[inline]`, as the shell draws it (`harnix.apps.list` → `runtime.inline`).
    pub inline: Value,
}

impl Loaded {
    /// The name a session sees one of its tools by ([`tool_name`]).
    pub fn tool_name(&self, tool: &str) -> String {
        tool_name(&self.id, tool)
    }
}

/// Where a running tool call's progress is told: `{app, tool, session_id, tool_use_id, text}`.
pub type OnProgress = Arc<dyn Fn(Value) + Send + Sync>;

/// Where one tool call's progress goes, by the call's id.
pub(crate) type Listener = Arc<dyn Fn(&str) + Send + Sync>;

/// What is running, shared with the plugin and the HTTP side.
#[derive(Default)]
pub struct Registry {
    apps: RwLock<BTreeMap<String, Arc<Loaded>>>,
    /// What the host offered, by application: read when it is loaded.
    offered: RwLock<BTreeMap<String, Offer>>,
    progress: RwLock<Option<OnProgress>>,
    listeners: Mutex<HashMap<String, Listener>>,
    control: OnceLock<Weak<dyn Control>>,
}

fn read<T>(l: &RwLock<T>) -> std::sync::RwLockReadGuard<'_, T> {
    l.read().unwrap_or_else(|e| e.into_inner())
}

fn write<T>(l: &RwLock<T>) -> std::sync::RwLockWriteGuard<'_, T> {
    l.write().unwrap_or_else(|e| e.into_inner())
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl Registry {
    /// Tell whoever listens about a tool call's progress.
    pub fn progress(&self, value: Value) {
        let f = read(&self.progress).clone();
        if let Some(f) = f {
            f(value);
        }
    }

    pub fn loaded(&self) -> Vec<Arc<Loaded>> {
        read(&self.apps).values().cloned().collect()
    }

    pub fn get(&self, id: &str) -> Option<Arc<Loaded>> {
        read(&self.apps).get(id).cloned()
    }

    fn insert(&self, loaded: Arc<Loaded>) {
        write(&self.apps).insert(loaded.id.clone(), loaded);
    }

    fn remove(&self, id: &str) -> Option<Arc<Loaded>> {
        write(&self.apps).remove(id)
    }

    /// The host that runs what is listed here: what tool calls and hooks go to.
    pub fn control(&self) -> Option<Arc<dyn Control>> {
        self.control.get().and_then(Weak::upgrade)
    }

    /// The host offered an application's tools (or, with none, took them back).
    pub(crate) fn offer(&self, offer: Offer) {
        let mut offered = write(&self.offered);
        if offer.component {
            offered.insert(offer.app.clone(), offer);
        } else {
            offered.remove(&offer.app);
        }
    }

    fn offered(&self, id: &str) -> Option<Offer> {
        read(&self.offered).get(id).cloned()
    }

    /// Hear one tool call's progress, until [`Self::unlisten`].
    pub(crate) fn listen(&self, call_id: &str, listener: Listener) {
        lock(&self.listeners).insert(call_id.to_string(), listener);
    }

    pub(crate) fn unlisten(&self, call_id: &str) {
        lock(&self.listeners).remove(call_id);
    }

    /// The host told a tool call's progress.
    pub(crate) fn told(&self, call_id: &str, text: &str) {
        let listener = lock(&self.listeners).get(call_id).cloned();
        if let Some(listener) = listener {
            listener(text);
        }
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
    /// `[apps.secrets]`, by application id: what `host.secret` reads.
    pub secrets: BTreeMap<String, BTreeMap<String, String>>,
    /// What signatures are verified with: the built-in store key (`store::store_keys`) and the
    /// profile's `[apps.trusted_publishers]`.
    pub keys: Keys,
    /// `[apps] store`: the catalog's URL (`store::DEFAULT_STORE` when the profile names none).
    pub store: Option<String>,
    /// How long an on-demand backend idles before it hibernates, in place of its `idle_secs`:
    /// for a test application only (`AppBuilder::app_idle`).
    #[doc(hidden)]
    pub idle: Option<std::time::Duration>,
}

pub struct Runtime {
    host: Arc<Host>,
    control: Arc<dyn Control>,
    platform: Arc<KernelPlatform>,
    registry: Arc<Registry>,
    pub(crate) admission: Admission,
    store: Option<String>,
    fingerprint: String,
    on_change: Mutex<Option<OnChange>>,
    /// Moved by every change to what is running here: with the host's, what
    /// [`AppsRuntime::generation`] answers.
    changes: AtomicU64,
    /// Where this process listens, once it does: the MCP endpoints' base.
    base: RwLock<Option<String>>,
}

impl Runtime {
    pub fn new(options: Options) -> anyhow::Result<Arc<Self>> {
        let registry = Arc::new(Registry::default());
        let platform = Arc::new(KernelPlatform::new(registry.clone()));
        let admission = Admission {
            keys: options.keys,
            dev: options.dev,
        };
        let host = Host::new(
            HostOptions {
                admission: admission.clone(),
                config: options.config,
                secrets: options.secrets,
                idle: options.idle,
            },
            platform.clone(),
        )?;
        let control: Arc<dyn Control> = host.clone();
        platform.lend_to(Arc::downgrade(&control));
        let _ = registry.control.set(Arc::downgrade(&control));
        Ok(Arc::new(Self {
            fingerprint: host.engine().fingerprint().to_string(),
            host,
            control,
            platform,
            registry,
            admission,
            store: options.store,
            on_change: Mutex::new(None),
            changes: AtomicU64::new(0),
            base: RwLock::new(None),
        }))
    }

    pub fn registry(&self) -> Arc<Registry> {
        self.registry.clone()
    }

    /// The Agix plugin an in-process instance's assembly carries ([`crate::AppsPlugin`]).
    pub fn plugin(&self) -> Arc<dyn runtime::plugin::Plugin> {
        Arc::new(crate::AppsPlugin::new(self.registry.clone()))
    }

    /// The engine the applications run on.
    pub fn engine(&self) -> &harnix_app_host::WasmEngine {
        self.host.engine()
    }

    pub(crate) fn control(&self) -> &Arc<dyn Control> {
        &self.control
    }

    /// Told every `host.progress` of a tool call from an in-process session. (The engine's own
    /// progress channel for a tool is not delivered to a client by Agix 0.3.9, so this is how a
    /// client sees an application's tool working.)
    pub fn on_progress(&self, f: impl Fn(Value) + Send + Sync + 'static) {
        *write(&self.registry.progress) = Some(Arc::new(f));
    }

    /// Told after every change to what is running, and when the listener's address is learnt.
    pub fn on_change(&self, f: impl Fn() + Send + Sync + 'static) {
        *lock(&self.on_change) = Some(Arc::new(f));
    }

    fn changed(&self) {
        self.changes.fetch_add(1, Ordering::Relaxed);
        let f = lock(&self.on_change).clone();
        if let Some(f) = f {
            f();
        }
    }

    /// Where this process listens (`http://127.0.0.1:<port>/`): what a spawned daemon is pointed
    /// at for an application's MCP endpoint.
    pub fn set_listening(&self, url: &str) {
        *write(&self.base) = Some(url.trim_end_matches('/').to_string());
        self.changed();
    }

    /// The interface of every running application, as the page is told it at hello: a trusted
    /// publisher's `[[ui]]` modules, which the page imports; everyone else's (and a trusted one
    /// that asked for it) as its surfaces, which the shell draws as isolated frames
    /// (`harnix_app_package::surface`). A module that will not run is listed as such, with why, so
    /// the page can say so in its diagnostics.
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
        let Some(base) = read(&self.base).clone() else {
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

    /// What the host says of it, and what of its interface this process serves.
    pub async fn status_of(&self, id: &str) -> Value {
        let mut status = self
            .control
            .status(id.to_string())
            .await
            .unwrap_or_else(|e| json!({"state": "failed", "reason": e.message}));
        if let (Some(app), Some(map)) = (self.registry.get(id), status.as_object_mut()) {
            map.insert(
                "ui".into(),
                json!(app.ui.iter().map(|u| format!("/apps/{id}/ui/{}", u.module)).collect::<Vec<_>>()),
            );
            map.insert("frames".into(), json!(frames(&app)));
            map.insert(
                "surfaces".into(),
                json!(app.surfaces.iter().map(|s| format!("{}:{}", s.point, s.id())).collect::<Vec<_>>()),
            );
            // An unsigned application's interface, in developer mode, changes under it: the page
            // reloads its frames when this does.
            map.insert(
                "ui_rev".into(),
                json!(self.watches_ui(&app).then(|| format!("{:016x}", ui_stamp(&app.root.join("ui"))))),
            );
            map.insert("mcp".into(), json!(format!("/apps/{id}/mcp")));
            map.insert("inline".into(), app.inline.clone());
        }
        status
    }

    /// An unsigned application's interface, in developer mode, changes under it on disk.
    fn watches_ui(&self, app: &Loaded) -> bool {
        self.admission.dev && app.trust == Trust::Unsigned
    }

    /// Put an unpacked package in place under `app_dir` and start it. What was put elsewhere is
    /// written to `parts` as soon as it is, so that whoever called this can take back exactly that
    /// when it fails. `trust`: what its signatures made it.
    pub(crate) async fn place(
        &self,
        cx: &ModuleContext,
        manifest: &Manifest,
        app_dir: &Path,
        trust: Trust,
        parts: &mut Option<Value>,
    ) -> Result<(Value, Value), RpcError> {
        let id = manifest.app.id.clone();
        let placement = self
            .control
            .place(Place {
                app: id.clone(),
                app_dir: app_dir.to_path_buf(),
                version: manifest.app.version.clone(),
            })
            .await?;
        let mut placed = placement.parts;
        placed["trust"] = json!(trust.as_str());
        *parts = Some(placed.clone());
        let record = InstallRecord {
            id,
            version: manifest.app.version.clone(),
            title: manifest.app.title.clone(),
            enabled: true,
            parts: placed.clone(),
            ..InstallRecord::default()
        };
        let started = self.load(cx, &record, app_dir).await?;
        Ok((placed, summary(&self.fingerprint, &record.parts, &placement.disclosure, started)))
    }

    /// Take back what a placement put elsewhere.
    pub(crate) async fn unplace(&self, id: &str, parts: &Value) -> Value {
        self.control
            .unplace(Unplace {
                app: id.to_string(),
                parts: parts.clone(),
            })
            .await
            .unwrap_or(Value::Null)
    }
}

/// What an install or an update tells its caller.
pub(crate) fn summary(fingerprint: &str, parts: &Value, disclosure: &Value, started: Value) -> Value {
    json!({
        "fingerprint": fingerprint,
        "trust": parts.get("trust").cloned().unwrap_or(json!("unsigned")),
        "agent": parts.get("agent").filter(|a| !a.is_null()).map(|agent| json!({
            "package": agent["package"], "instances": agent["instances"], "disclosure": disclosure,
        })),
        "skills": parts.get("skills").cloned().unwrap_or(json!({})),
        "started": started,
    })
}

/// Whether a running application's interface is drawn in frames rather than imported into the page.
fn frames(app: &Loaded) -> bool {
    harnix_app_package::surface::runs_in_frames(app.trust, &app.interface, !app.ui.is_empty())
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
/// time. Read on every look at an application that [`Runtime::watches_ui`] (once a second, by the
/// `apps` module's topic, through the generation), so it is a watcher that needs no thread: a file
/// saved there changes it.
fn ui_stamp(dir: &Path) -> u64 {
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
    hasher.finish()
}

#[async_trait::async_trait]
impl AppsRuntime for Runtime {
    fn fingerprint(&self) -> String {
        self.fingerprint.clone()
    }

    fn inspect(&self, path: &Path) -> Result<PackageInfo, RpcError> {
        let inspected = package::inspect(path, &self.fingerprint).map_err(refused)?;
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
        self.platform.bind(cx);
        let inspected = package::inspect(path, &self.fingerprint).map_err(refused)?;
        // Before a byte of it is written.
        let verified = self.admission.admit_zip(path, &inspected.manifest)?;
        let manifest = &inspected.manifest;
        let id = manifest.app.id.clone();
        std::fs::create_dir_all(app_dir)
            .map_err(|e| internal(format!("{}: {e}", app_dir.display())))?;
        let mut parts = None;
        let outcome = async {
            package::extract(path, &app_dir.join(&manifest.app.version)).map_err(refused)?;
            self.place(cx, manifest, app_dir, verified.trust, &mut parts).await
        }
        .await;
        match outcome {
            Ok(done) => Ok(done),
            Err(e) => {
                tracing::warn!(app = %id, error = %e.message, "the install failed; undoing it");
                if let Some(parts) = &parts {
                    self.unplace(&id, parts).await;
                }
                let _ = std::fs::remove_dir_all(app_dir);
                let _ = self.control.forget(id).await;
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
        self.platform.bind(cx);
        let id = &record.id;
        if self.registry.get(id).is_some() {
            // Set aside after faulting: enabling it again is starting it again, from nothing.
            if self.status_of(id).await["state"] != json!("faulted") {
                return Ok(self.status_of(id).await);
            }
            self.unload(cx, record).await;
        }
        let started = self
            .control
            .start(Start {
                app: id.clone(),
                version: record.version.clone(),
                app_dir: app_dir.to_path_buf(),
                parts: record.parts.clone(),
            })
            .await?;
        let root = app_dir.join(&record.version);
        let manifest = match acquire::manifest_in(&root) {
            Ok(manifest) => manifest,
            Err(e) => {
                self.unload(cx, record).await;
                return Err(e);
            }
        };
        let offer = self.registry.offered(id);
        let loaded = Arc::new(Loaded {
            id: id.clone(),
            version: record.version.clone(),
            title: manifest.app.title.clone(),
            root,
            component: offer.as_ref().is_some_and(|o| o.component),
            tools: offer.map(|o| o.tools).unwrap_or_default(),
            ui: manifest.ui.clone(),
            interface: manifest.interface.clone(),
            surfaces: manifest.surfaces.clone(),
            trust: Trust::parse(&started.trust),
            backend_spaces: manifest.spaces.iter().map(|s| s.id.clone()).collect(),
            token: uuid::Uuid::new_v4().simple().to_string(),
            inline: serde_json::to_value(&manifest.inline).unwrap_or(Value::Null),
        });
        tracing::debug!(app = %id, ui = loaded.ui.len(), "application served");
        self.registry.insert(loaded);
        self.changed();
        Ok(self.status_of(id).await)
    }

    async fn unload(&self, cx: &ModuleContext, record: &InstallRecord) {
        self.platform.bind(cx);
        let removed = self.registry.remove(&record.id);
        // Out of the registry first: a session built from here on has none of its tools, and the
        // page is told its modules are gone. A call already running holds its own instance.
        if removed.is_some() {
            self.changed();
        }
        let stop = Stop {
            app: record.id.clone(),
            parts: record.parts.clone(),
        };
        if let Err(e) = self.control.stop(stop).await {
            tracing::warn!(app = %record.id, error = %e.message, "the application did not stop cleanly");
        }
    }

    async fn uninstall(
        &self,
        cx: &ModuleContext,
        record: &InstallRecord,
        _app_dir: &Path,
    ) -> Result<Value, RpcError> {
        self.platform.bind(cx);
        Ok(self.unplace(&record.id, &record.parts).await)
    }

    async fn status(&self, id: &str) -> Value {
        self.status_of(id).await
    }

    async fn shutdown(&self) {
        for app in self.registry.loaded() {
            self.registry.remove(&app.id);
        }
        let _ = self.control.shutdown("the process is stopping".into()).await;
    }

    fn disclose(
        &self,
        package: &Path,
        installed: Option<(&InstallRecord, &Path)>,
    ) -> Result<Disclosed, RpcError> {
        let disclosed = self.admission.disclose(
            package,
            installed.map(|(record, app_dir)| (record.version.as_str(), app_dir)),
            &self.fingerprint,
        )?;
        Ok(Disclosed {
            info: PackageInfo {
                id: disclosed.id,
                version: disclosed.version,
                title: disclosed.title,
            },
            trust: disclosed.trust.as_str().into(),
            digest: disclosed.digest,
            disclosure: disclosed.disclosure,
            needs_consent: disclosed.needs_consent,
            downgrade: disclosed.downgrade,
        })
    }

    async fn catalog(&self, installed: &BTreeMap<String, String>) -> Result<Value, RpcError> {
        acquire::catalog(self.store.as_deref(), &self.admission.keys, &self.fingerprint, installed).await
    }

    async fn fetch(
        &self,
        id: &str,
        version: Option<&str>,
        staging: &Path,
    ) -> Result<PathBuf, RpcError> {
        acquire::fetch(
            self.store.as_deref(),
            &self.admission.keys,
            id,
            version,
            &self.fingerprint,
            staging,
        )
        .await
    }

    async fn update(
        &self,
        cx: &ModuleContext,
        record: &InstallRecord,
        package: &Path,
        app_dir: &Path,
    ) -> Result<(Value, Value), UpdateFailed> {
        self.platform.bind(cx);
        self.update_to(cx, record, package, app_dir).await
    }

    fn describe(&self, record: &InstallRecord, app_dir: &Path) -> Value {
        acquire::describe(&app_dir.join(&record.version))
    }

    /// The host's generation and this runtime's own changes; in developer mode, with the stamp of
    /// each unsigned application's `ui/` too, which changes on disk with nobody saying so.
    fn generation(&self) -> Option<u64> {
        let mut generation = self
            .host
            .generation()
            .wrapping_add(self.changes.load(Ordering::Relaxed));
        for app in self.registry.loaded() {
            if self.watches_ui(&app) {
                generation = generation.rotate_left(7) ^ ui_stamp(&app.root.join("ui"));
            }
        }
        Some(generation)
    }

    fn data_bytes(&self, app_dir: &Path) -> Option<u64> {
        harnix_app_host::data_bytes(app_dir)
    }
}
