//! Packages in the product's process (mode 2): [`WasmCarrier`], the carrier the `packages`
//! component runs them on.
//!
//! A package runs in a `harnix_wasm_host::Host`, one VM per package in this process, which this
//! drives through the host's `Control` and lends the process through its `Kernel` (`kernel.rs`:
//! the package's extension handle) and its `Agent` (`agent.rs`: the component's context, over this
//! process's instances). What is running is in a [`Registry`] the rest of the process reaches
//! through what the carrier hands out:
//!
//! - [`WasmTools`], the tool provider the component gives every engine this process builds or
//!   spawns: the running packages' tools in each session, and their `events` as a hook before its
//!   tool calls — registered on each engine's link, so a call from a spawned daemon's session runs
//!   here too;
//! - each package's route, in its descriptor (`routes.rs`): `/ext/<id>/ui/` (its interface's files,
//!   under their CSP);
//! - the component's interface entries (`frames.rs`), told the host again after every change;
//! - a generation number ([`Carrier::changed`]) that moves whenever what the status says may have.
//!
//! A package's HTTP is decided here too: its host lends it this process's network only for
//! the hosts its `net` lists (`harnix_package::egress`).

mod agent;
mod kernel;
mod registry;
mod routes;
mod tools;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use async_trait::async_trait;
use harnix_carrier_shared::sync::lock;
use harnix_carrier_shared::frames;
use harnix_carrier_shared::DEFAULT_TIMEOUT;
use harnix_ext::{ExtId, ExtensionContext, PackageCarrier, Rpc};
use harnix_host_api::{
    Carrier, CarrierRegistrar, CarrierSettings, HostApi, HostServices, Installed, Running, StopReason,
};
use harnix_package::acquire::internal;
use harnix_package::egress::{Egress, Request};
use harnix_package::signature::Trust;
use harnix_wasm_host::{Host, HostOptions, HttpFetcher, HttpRequest, HttpResponse};
use harnix_wasm_port::{CallMeta, Control, Place, Start, Stop, Unplace};
use serde_json::{json, Value};
use tokio::sync::watch;

pub use harnix_carrier_shared::{tool_name, Loaded, FRAME_CSP};
pub use registry::Registry;
pub use tools::{WasmTools, MAX_NOTE, NAME as TOOLS_NAME};

use kernel::CarrierKernel;

/// How often the host's generation (and, in developer mode, an unsigned package's files) is looked
/// at for a move to tell: an atomic read, and a walk of a directory only for a package that asked.
const TICK: Duration = Duration::from_secs(1);

/// Packages' WebAssembly in this process, one VM each.
#[derive(Clone, Default)]
pub struct WasmCarrier(Arc<Inner>);

struct Inner {
    registry: Arc<Registry>,
    kernel: Arc<CarrierKernel>,
    settings: OnceLock<CarrierSettings>,
    host: OnceLock<Arc<Host>>,
    /// What each running package was started with: what stopping it switches off.
    started: Mutex<BTreeMap<String, Value>>,
    changes: watch::Sender<u64>,
    /// The tasks that follow the process (the listener, progress, the tick), once a package
    /// started.
    following: OnceLock<()>,
}

impl WasmCarrier {
    pub fn new() -> Self {
        Self::default()
    }

    /// What is running, for the tool provider and the routes.
    pub fn registry(&self) -> Arc<Registry> {
        self.0.registry.clone()
    }
}

impl Default for Inner {
    fn default() -> Self {
        let registry = Arc::new(Registry::default());
        Self {
            kernel: Arc::new(CarrierKernel::new(registry.clone())),
            registry,
            settings: OnceLock::new(),
            host: OnceLock::new(),
            started: Mutex::new(BTreeMap::new()),
            changes: watch::channel(0).0,
            following: OnceLock::new(),
        }
    }

}

impl Inner {
    /// The host, made the first time it is needed, under the settings the component registered.
    fn host(&self) -> Rpc<Arc<Host>> {
        if let Some(host) = self.host.get() {
            return Ok(host.clone());
        }
        let fetcher = Fetcher { registry: self.registry.clone(), kernel: self.kernel.clone(), egress: Egress::new() };
        let options = HostOptions { http: Some(Arc::new(fetcher)) };
        let host = Host::new(options, self.kernel.clone(), self.kernel.clone()).map_err(|e| internal(format!("{e:#}")))?;
        let control: Arc<dyn Control> = host.clone();
        self.kernel.lend_to(Arc::downgrade(&control));
        self.registry.lend(Arc::downgrade(&control));
        Ok(self.host.get_or_init(|| host).clone())
    }

    fn control(&self) -> Rpc<Arc<dyn Control>> {
        Ok(self.host()?)
    }

    fn moved(&self) {
        self.changes.send_modify(|n| *n = n.wrapping_add(1));
    }

    /// An unsigned package's interface, in developer mode, changes under it on disk.
    fn watches_ui(&self, app: &Loaded) -> bool {
        self.settings.get().is_some_and(CarrierSettings::is_dev) && app.trust == Trust::Unsigned
    }

    /// Tell the host what runs now: the page's interface entries.
    fn refresh(&self, cx: &ExtensionContext) {
        let Ok(host) = cx.service::<HostServices>() else { return };
        host.publish_ui(self.registry.loaded().iter().flat_map(|app| frames::ui_entries(app)).collect());
        self.moved();
    }

    /// From the first start on: a tool call's progress on the component's topic, and the host's
    /// generation followed.
    fn follow(self: &Arc<Self>, cx: &ExtensionContext) {
        if self.following.set(()).is_err() {
            return;
        }
        let topics = cx.clone();
        self.registry.on_progress(Arc::new(move |value: Value| {
            let cx = topics.clone();
            tokio::spawn(async move {
                let _ = cx.topics().publish("progress", json!({}), value).await;
            });
        }));
        let me = Arc::downgrade(self);
        tokio::spawn(async move {
            let mut last = None;
            loop {
                tokio::time::sleep(TICK).await;
                let Some(me) = me.upgrade() else { break };
                let Some(host) = me.host.get() else { continue };
                let mut now = host.generation();
                for app in me.registry.loaded().iter().filter(|app| me.watches_ui(app)) {
                    now = now.rotate_left(7) ^ harnix_carrier_shared::ui_stamp(&app.root.join("ui"));
                }
                if last.is_some_and(|last| last != now) {
                    me.moved();
                }
                last = Some(now);
            }
        });
    }

    /// What the host says of it, and what of its interface this process serves.
    async fn status_of(&self, id: &str) -> Value {
        let Ok(control) = self.control() else { return json!({"state": "stopped"}) };
        let mut status = control
            .status(CallMeta::default(), id.to_string())
            .await
            .unwrap_or_else(|e| json!({"state": "failed", "reason": e.message}));
        if let (Some(app), Some(map)) = (self.registry.get(id), status.as_object_mut()) {
            map.insert("ui".into(), json!(app.ui.iter().map(|u| format!("/ext/{id}/ui/{}", u.module)).collect::<Vec<_>>()));
            map.insert("frames".into(), json!(frames::frames(&app)));
            map.insert(
                "surfaces".into(),
                json!(app.surfaces.iter().map(|s| format!("{}:{}", s.point, s.id())).collect::<Vec<_>>()),
            );
            // An unsigned package's interface, in developer mode, changes under it: the page
            // reloads its frames when this does.
            map.insert(
                "ui_rev".into(),
                json!(self.watches_ui(&app).then(|| format!("{:016x}", harnix_carrier_shared::ui_stamp(&app.root.join("ui"))))),
            );
            map.insert("inline".into(), app.inline.clone());
        }
        status
    }
}

/// What its record's `parts` say its carrier put in place, without what the component wrote
/// beside it.
fn placed_of(installed: &Installed) -> Value {
    let mut placed = installed.install_record().get("parts").cloned().unwrap_or(Value::Null);
    if let Some(map) = placed.as_object_mut() {
        map.remove("trust");
        map.remove("source");
    }
    placed
}

/// What starting it tells whoever asked.
fn summary(fingerprint: &str, parts: &Value, disclosure: &Value, started: Value) -> Value {
    json!({
        "fingerprint": fingerprint,
        "agent": parts.get("agent").filter(|a| !a.is_null()).map(|agent| json!({
            "package": agent["package"], "instances": agent["instances"], "disclosure": disclosure,
        })),
        "started": started,
    })
}

#[async_trait]
impl Carrier for WasmCarrier {
    fn kind(&self) -> PackageCarrier {
        PackageCarrier::Wasm
    }

    fn name(&self) -> &'static str {
        "wasm"
    }

    fn fingerprint(&self) -> String {
        self.0.host().map(|h| h.engine().fingerprint().to_string()).unwrap_or_default()
    }

    fn register(&self, r: &mut CarrierRegistrar<'_>) {
        let _ = self.0.settings.set(r.settings().clone());
        r.provider(Arc::new(WasmTools::new(self.0.registry.clone())));
    }

    async fn start(&self, _host: &dyn HostApi, cx: &ExtensionContext, app: Installed) -> Rpc<Running> {
        self.0.kernel.bind(cx);
        self.0.follow(cx);
        let host = self.0.host()?;
        let control = self.0.control()?;
        let id = app.id().to_string();
        // Put in place before it first starts: its agent package and skills, in the instances its
        // manifest names.
        let mut parts = placed_of(&app);
        let mut disclosure = Value::Null;
        let mut placed_now = false;
        if parts.get("agent").is_none() {
            // The product's own instances, as its manifest names them.
            let app_dir = app.pkg_dir().parent().map(PathBuf::from).unwrap_or_else(|| app.run_dir().clone());
            let place = Place { app: id.clone(), app_dir, version: app.version().to_string(), instances: None };
            let placement = control.place(CallMeta::default(), place).await?;
            parts = placement.parts;
            disclosure = placement.disclosure;
            placed_now = true;
        }
        let pkg_dir = app.pkg_dir().clone();
        let manifest = harnix_package::acquire::manifest_in(&pkg_dir)?;
        // Its HTTP is decided from its start: its backend's `init` may already send.
        self.0.registry.set_net(&id, Some(manifest.capabilities.net.clone()));
        self.0.kernel.expect(app.clone());
        let undo = |control: Arc<dyn Control>, parts: Value| {
            let id = id.clone();
            async move {
                if placed_now {
                    let _ = control.unplace(CallMeta::default(), Unplace { app: id.clone(), parts }).await;
                }
                let _ = control.forget(CallMeta::default(), id).await;
            }
        };
        // Verified by the component just now: the host checks the component it loads against it.
        let start = Start {
            app: id.clone(),
            version: app.version().to_string(),
            trust: app.trust_level().to_string(),
            cwasm_sha256: app.cwasm_digest().map(str::to_string),
            config: app.config_value().clone(),
            secrets: app.secret_values().clone(),
            parts: parts.clone(),
            limits: harnix_carrier_shared::limits(&manifest, self.0.settings.get().and_then(|s| s.idle_for())),
            pkg: Some(pkg_dir.clone()),
            run: Some(app.run_dir().clone()),
        };
        let started = control.start(CallMeta::default(), start).await;
        self.0.kernel.forget(&id);
        let started = match started {
            Ok(started) => started,
            Err(e) => {
                self.0.registry.set_net(&id, None);
                undo(control.clone(), parts).await;
                return Err(e);
            }
        };
        let offer = self.0.registry.offered(&id);
        let loaded = Arc::new(Loaded {
            id: id.clone(),
            version: app.version().to_string(),
            title: manifest.app.title.clone(),
            root: pkg_dir,
            component: offer.as_ref().is_some_and(|o| o.component),
            tools: offer.map(|o| o.tools).unwrap_or_default(),
            ui: manifest.ui.clone(),
            interface: manifest.interface.clone(),
            surfaces: manifest.surfaces.clone(),
            trust: Trust::parse(&started.trust),
            backend_spaces: manifest.spaces.iter().map(|s| s.id.clone()).collect(),
            inline: serde_json::to_value(&manifest.inline).unwrap_or(Value::Null),
            // Only a call over a wire has a deadline made of it; the VM keeps its own.
            timeout: manifest.component.as_ref().map(|c| Duration::from_millis(c.timeout_ms)).unwrap_or(DEFAULT_TIMEOUT),
        });
        // What its pages in frames may call, told to the router as well as the shell: the router
        // holds a frame's calls to it whatever the page asks the shell for.
        if let Ok(handle) = self.0.kernel.handle(&id) {
            let calls = frames::frames(&loaded).then(|| loaded.interface.calls.iter().cloned().collect());
            if let Err(e) = handle.frame(calls) {
                tracing::warn!(package = %id, error = %e.message, "its interface in frames was refused");
            }
        }
        self.0.registry.insert(loaded);
        lock(&self.0.started).insert(id.clone(), parts.clone());
        self.0.refresh(cx);
        let fingerprint = host.engine().fingerprint().to_string();
        Ok(Running::new(parts.clone(), summary(&fingerprint, &parts, &disclosure, self.0.status_of(&id).await)))
    }

    async fn stop(&self, id: &ExtId, why: StopReason) {
        let parts = lock(&self.0.started).remove(id.as_str());
        // Out of the registry first: a session built from here on has none of its tools, and the
        // page is told its interface is gone. A call already running holds its own instance.
        let removed = self.0.registry.remove(id.as_str());
        self.0.registry.set_net(id.as_str(), None);
        let Some(parts) = parts else { return };
        let Ok(control) = self.0.control() else { return };
        if let Err(e) = control.stop(CallMeta::default(), Stop { app: id.to_string(), parts }).await {
            tracing::warn!(package = %id, ?why, error = %e.message, "the package did not stop cleanly");
        }
        if removed.is_some() {
            if let Ok(cx) = self.0.kernel.cx() {
                self.0.refresh(&cx);
            }
        }
    }

    async fn unplace(&self, _cx: &ExtensionContext, app: &Installed) -> Rpc<Value> {
        let control = self.0.control()?;
        Ok(control.unplace(CallMeta::default(), Unplace { app: app.id().to_string(), parts: placed_of(app) }).await.unwrap_or(Value::Null))
    }

    async fn status(&self, id: &ExtId) -> Value {
        self.0.status_of(id.as_str()).await
    }

    fn changed(&self) -> watch::Receiver<u64> {
        self.0.changes.subscribe()
    }
}

/// A package's HTTP, sent only to the hosts its `net` lists: decided here, where it is sent, not
/// on the VM's word alone.
struct Fetcher {
    registry: Arc<Registry>,
    kernel: Arc<CarrierKernel>,
    egress: Egress,
}

#[async_trait]
impl HttpFetcher for Fetcher {
    async fn fetch(&self, app: &str, request: HttpRequest) -> Result<HttpResponse, String> {
        let Some(net) = self.registry.net(app) else {
            return Err(format!("`{app}` is not running"));
        };
        self.kernel.allowed(app, &net, &request.url)?;
        let request = Request { method: request.method, url: request.url, headers: request.headers, body: request.body };
        let response = self.egress.send(request).await?;
        Ok(HttpResponse { status: response.status, headers: response.headers, body: response.body })
    }
}

/// The carrier, for `Packages::carrier`.
pub fn carrier() -> Arc<dyn Carrier> {
    Arc::new(WasmCarrier::new())
}
