//! Packages in processes of their own (mode 3): [`ProcCarrier`], the carrier the `packages`
//! component runs them on when the profile says `[ext.packages] carrier = "process"`.
//!
//! A package runs in an extension process: this product's own binary re-executed as `ext-proc`
//! (`harnix-proc`), its VM and host there, its wire the only way in or out. The carrier asks the
//! host to start it (`HostApi::spawn_child`, which binds the extension's subject to the stream
//! and hands the process to a supervisor) and keeps it (`process.rs`). The product answers its
//! process over the wire (`kernel.rs`) and never takes its word:
//!
//! - **The product registers it.** Its descriptor is made from its manifest and its consent and
//!   registered before the process starts, marked starting; the process's `proc.register` must be
//!   within it, with the digest of the manifest consented to, or the process is refused and ended.
//!   The rows stay while the process restarts, so the page's subscriptions do not flicker.
//! - **The same build.** A process whose hello names another wire, Harnix, Agix or VM engine than
//!   this product's ([`ProcCarrier::new`]) is refused: its precompiled component would not be the
//!   one this product verified for its engine.
//! - **Kept.** Ready once it said hello, registered and started, within 30 s; asked how it is every
//!   15 s, killed after three answers missed; started again after an exit, with backoff; three
//!   exits within 60 s of starting — across runs, which the carrier counts, not its supervisor —
//!   and it is set aside (`faulted`: its rows stay and answer `EXTENSION_UNAVAILABLE {faulted}`)
//!   until it is switched off and on.
//!
//! What runs is in a [`Registry`] the rest of the process reaches through what the carrier hands
//! out, as with the in-process carrier: [`ProcTools`], the tool provider every engine built in this
//! process gets (each call over the package's wire); each package's routes (`/ext/<id>/ui/`,
//! `/ext/<id>/mcp`); the component's interface entries; a generation number that moves whenever
//! the status may have.

mod frames;
mod kernel;
mod process;
mod registry;
mod routes;
mod sync;
mod tools;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use harnix_ext::{codes, ExtId, ExtensionContext, PackageCarrier, Rpc, RpcError};
use harnix_host_api::{
    Availability, Carrier, CarrierRegistrar, CarrierSettings, ChildRequest, HostApi, HostServices, Installed, Running,
    StopReason, ToolEndpoint,
};
use harnix_package::egress::Egress;
use harnix_package::signature::Trust;
use harnix_wasm_port::socket::Hello;
use serde_json::{json, Value};
use tokio::sync::{oneshot, watch};

pub use harnix_package::{server_name, tool_name, TOOL_PATTERN};
pub use registry::{Loaded, Registry};
pub use routes::FRAME_CSP;
pub use tools::{ProcTools, MAX_NOTE, NAME as TOOLS_NAME};

use crate::process::{Package, Phase};
use crate::sync::{lock, read, write};

/// How long a package's process has, from its start, to say hello, register and start.
pub const READY_WITHIN: Duration = Duration::from_secs(30);

/// A process that exits this soon after it starts has crashed, as the count of crashes goes.
pub const CRASH_WINDOW: Duration = Duration::from_secs(60);

/// Crashes in a row after which a package is set aside.
pub const CRASHES: usize = 3;

/// How the default timeout of a package's call is, when its component names none.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

/// Packages in processes of their own.
#[derive(Clone)]
pub struct ProcCarrier(Arc<Inner>);

pub(crate) struct Inner {
    /// This build, as a process of it says hello.
    pub(crate) ours: Hello,
    pub(crate) registry: Arc<Registry>,
    pub(crate) egress: Arc<Egress>,
    settings: OnceLock<CarrierSettings>,
    /// The component's context, from the first package it started.
    cx: RwLock<Option<ExtensionContext>>,
    /// The packages started, whether their processes are up or not.
    packages: Mutex<BTreeMap<String, Arc<Package>>>,
    /// Why a package that is not started did not start.
    failed: Mutex<BTreeMap<String, String>>,
    /// Each package's crashes in a row, across runs: what sets it aside.
    crashes: Mutex<BTreeMap<String, Vec<Instant>>>,
    changes: watch::Sender<u64>,
    following: OnceLock<()>,
}

impl ProcCarrier {
    /// Packages in processes of this build: `ours` is what one says in its hello but for who it is
    /// (`harnix_proc::this_build()`, which only a build that links the process has).
    pub fn new(ours: Hello) -> Self {
        Self(Arc::new(Inner {
            ours,
            registry: Arc::default(),
            egress: Arc::new(Egress::new()),
            settings: OnceLock::new(),
            cx: RwLock::new(None),
            packages: Mutex::new(BTreeMap::new()),
            failed: Mutex::new(BTreeMap::new()),
            crashes: Mutex::new(BTreeMap::new()),
            changes: watch::channel(0).0,
            following: OnceLock::new(),
        }))
    }

    /// What is running, for the tool provider and the routes.
    pub fn registry(&self) -> Arc<Registry> {
        self.0.registry.clone()
    }
}

impl Inner {
    pub(crate) fn moved(&self) {
        self.changes.send_modify(|n| *n = n.wrapping_add(1));
    }

    fn bind(&self, cx: &ExtensionContext) {
        let mut slot = write(&self.cx);
        if slot.is_none() {
            *slot = Some(cx.clone());
        }
    }

    fn cx(&self) -> Option<ExtensionContext> {
        read(&self.cx).clone()
    }

    fn settings(&self) -> CarrierSettings {
        self.settings.get().cloned().unwrap_or_default()
    }

    /// An exit after `ran_for`: true when it is the last of [`CRASHES`] in a row, each within
    /// [`CRASH_WINDOW`] of its start. A longer run starts the count over.
    pub(crate) fn crashed(&self, id: &str, ran_for: Duration) -> bool {
        let mut crashes = lock(&self.crashes);
        let record = crashes.entry(id.to_string()).or_default();
        if ran_for >= CRASH_WINDOW {
            record.clear();
            return false;
        }
        record.push(Instant::now());
        record.len() >= CRASHES
    }

    /// Tell the host what runs now: the page's interface entries, and every spawned daemon's tool
    /// endpoints (once this process listens).
    pub(crate) fn refresh(&self) {
        let Some(cx) = self.cx() else { return };
        let Ok(host) = cx.service::<HostServices>() else { return };
        let loaded = self.registry.loaded();
        host.publish_ui(loaded.iter().flat_map(|app| frames::ui_entries(app)).collect());
        if let Some(listen) = cx.listen() {
            let base = listen.url().trim_end_matches('/').to_string();
            let endpoints = loaded
                .iter()
                .filter(|app| !app.tools.is_empty())
                .map(|app| ToolEndpoint::new(&server_name(&app.id), &format!("{base}/ext/{}/mcp", app.id)))
                .collect();
            if let Err(e) = host.share_tools(endpoints) {
                tracing::warn!(error = %e.message, "the packages' tool endpoints were not shared");
            }
        }
        self.moved();
    }

    /// From the first start on: a tool call's progress on the component's topic, and the
    /// endpoints again once the listener is up.
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
        let listening = cx.clone();
        tokio::spawn(async move {
            listening.listening().await;
            if let Some(me) = me.upgrade() {
                me.refresh();
            }
        });
    }

    /// What the carrier says of it, with what its process says when it is up, and what of its
    /// interface this process serves.
    async fn status_of(&self, id: &str) -> Value {
        let package = lock(&self.packages).get(id).cloned();
        let Some(package) = package else {
            return match lock(&self.failed).get(id) {
                Some(reason) => json!({"state": "failed", "reason": reason, "carrier": "process"}),
                None => json!({"state": "stopped", "carrier": "process"}),
            };
        };
        let phase = package.phase();
        let mut status = match self.registry.control(id) {
            Some(control) => process::asked(&control, id).await.unwrap_or_else(|e| json!({"reason": e.message})),
            None => json!({}),
        };
        let Some(map) = status.as_object_mut() else { return status };
        map.insert("state".into(), json!(phase.state));
        map.insert("reason".into(), json!(phase.reason));
        map.insert("carrier".into(), json!("process"));
        map.insert("process".into(), phase.describe());
        let app = &package.loaded;
        map.insert("ui".into(), json!(app.ui.iter().map(|u| format!("/ext/{id}/ui/{}", u.module)).collect::<Vec<_>>()));
        map.insert("frames".into(), json!(frames::frames(app)));
        map.insert(
            "surfaces".into(),
            json!(app.surfaces.iter().map(|s| format!("{}:{}", s.point, s.id())).collect::<Vec<_>>()),
        );
        map.insert("ui_rev".into(), Value::Null);
        map.insert("mcp".into(), json!(format!("/ext/{id}/mcp")));
        map.insert("inline".into(), app.inline.clone());
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
fn summary(fingerprint: &str, parts: &Value, started: Value) -> Value {
    json!({
        "fingerprint": fingerprint,
        "agent": parts.get("agent").filter(|a| !a.is_null()).map(|agent| json!({
            "package": agent["package"], "instances": agent["instances"],
        })),
        "skills": parts.get("skills").cloned().unwrap_or(json!({})),
        "started": started,
    })
}

fn unavailable(failure: &str, why: impl Into<String>) -> RpcError {
    let mut e = RpcError::new(codes::EXTENSION_UNAVAILABLE, why);
    e.data = Some(json!({"failure": failure}));
    e
}

#[async_trait]
impl Carrier for ProcCarrier {
    fn kind(&self) -> PackageCarrier {
        PackageCarrier::Process
    }

    fn name(&self) -> &'static str {
        "process"
    }

    fn fingerprint(&self) -> String {
        self.0.ours.engine.clone()
    }

    fn register(&self, r: &mut CarrierRegistrar<'_>) {
        let _ = self.0.settings.set(r.settings().clone());
        r.provider(Arc::new(ProcTools::new(self.0.registry.clone())));
        r.admit(TOOL_PATTERN);
    }

    async fn start(&self, host: &dyn HostApi, cx: &ExtensionContext, app: Installed) -> Rpc<Running> {
        let me = &self.0;
        me.bind(cx);
        me.follow(cx);
        let id = app.id().clone();
        let running = lock(&me.packages).get(id.as_str()).cloned();
        if let Some(running) = running {
            let parts = running.placed();
            return Ok(Running::new(parts.clone(), summary(&me.ours.engine, &parts, me.status_of(id.as_str()).await)));
        }
        let manifest = harnix_package::acquire::manifest_in(app.pkg_dir())?;
        let expected = kernel::declared(&manifest, &kernel::consented_digest(&app)?);
        // The product's to register, from what was consented to, before the process says a word.
        let descriptor = kernel::descriptor(&app, &manifest, &expected, &me.registry)?;
        let handle = host.registrar().register(descriptor, app.capabilities().to_vec()).await?;
        handle.mark(Availability::Starting);
        let mut request = ChildRequest::new().pkg_dir(app.pkg_dir().clone()).run_dir(app.run_dir().clone());
        if let Some(program) = me.settings().proc_program_path() {
            request = request.program(program.clone());
        }
        let child = match host.spawn_child(&id, request) {
            Ok(child) => child,
            Err(e) => {
                let _ = handle.withdraw().await;
                lock(&me.failed).insert(id.to_string(), e.message.clone());
                return Err(e);
            }
        };
        let timeout = manifest.component.as_ref().map(|c| Duration::from_millis(c.timeout_ms)).unwrap_or(DEFAULT_TIMEOUT);
        let loaded = Arc::new(Loaded {
            id: id.to_string(),
            version: app.version().to_string(),
            title: manifest.app.title.clone(),
            root: app.pkg_dir().clone(),
            component: manifest.component.is_some(),
            tools: expected.tools.clone(),
            ui: manifest.ui.clone(),
            interface: manifest.interface.clone(),
            surfaces: manifest.surfaces.clone(),
            trust: Trust::parse(app.trust_level()),
            backend_spaces: manifest.spaces.iter().map(|s| s.id.clone()).collect(),
            inline: serde_json::to_value(&manifest.inline).unwrap_or(Value::Null),
            timeout,
        });
        let (commands, received) = tokio::sync::mpsc::unbounded_channel();
        let package = Arc::new(Package {
            id: id.clone(),
            parts: placed_of(&app),
            net: manifest.capabilities.net.clone(),
            installed: app,
            handle: handle.clone(),
            loaded: loaded.clone(),
            expected,
            commands,
            phase: Mutex::new(Phase::default()),
        });
        lock(&me.packages).insert(id.to_string(), package.clone());
        lock(&me.failed).remove(id.as_str());
        let (ready, readied) = oneshot::channel();
        tokio::spawn(process::keep(me.clone(), package.clone(), child, received, ready));
        let started = match tokio::time::timeout(READY_WITHIN + Duration::from_secs(5), readied).await {
            Ok(Ok(started)) => started,
            Ok(Err(_)) => Err(unavailable("stopped", format!("`{id}`'s process went before it was ready"))),
            Err(_) => Err(unavailable("starting", format!("`{id}`'s process was not ready within {}s", READY_WITHIN.as_secs()))),
        };
        if let Err(e) = started {
            package.stop().await;
            lock(&me.packages).remove(id.as_str());
            let _ = handle.withdraw().await;
            lock(&me.failed).insert(id.to_string(), e.message.clone());
            me.moved();
            return Err(e);
        }
        // What its pages in frames may call, told to the router as well as the shell.
        let calls = frames::frames(&loaded).then(|| loaded.interface.calls.iter().cloned().collect());
        if let Err(e) = handle.frame(calls) {
            tracing::warn!(package = %id, error = %e.message, "its interface in frames was refused");
        }
        me.refresh();
        let parts = package.placed();
        Ok(Running::new(parts.clone(), summary(&me.ours.engine, &parts, me.status_of(id.as_str()).await)))
    }

    async fn stop(&self, id: &ExtId, why: StopReason) {
        let me = &self.0;
        let package = lock(&me.packages).remove(id.as_str());
        lock(&me.failed).remove(id.as_str());
        // Switched off and on again, it starts its count of crashes over.
        lock(&me.crashes).remove(id.as_str());
        let Some(package) = package else { return };
        tracing::info!(package = %id, ?why, "stopping its extension process");
        package.stop().await;
        if let Err(e) = package.handle.withdraw().await {
            tracing::warn!(package = %id, error = %e.message, "its registration was not withdrawn");
        }
        me.refresh();
    }

    /// What it placed is in its process's engine, and goes with it: nothing to take out here.
    async fn unplace(&self, _cx: &ExtensionContext, app: &Installed) -> Rpc<Value> {
        let placed = placed_of(app);
        Ok(json!({"agent": placed.get("agent").cloned().unwrap_or(Value::Null), "skills": placed.get("skills").cloned().unwrap_or(json!({}))}))
    }

    async fn status(&self, id: &ExtId) -> Value {
        self.0.status_of(id.as_str()).await
    }

    fn changed(&self) -> watch::Receiver<u64> {
        self.0.changes.subscribe()
    }
}

/// The carrier, for `Packages::carrier`, of the build `ours`.
pub fn carrier(ours: Hello) -> Arc<dyn Carrier> {
    Arc::new(ProcCarrier::new(ours))
}
