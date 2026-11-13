//! `packages`: the host component that installs packages and registers each one as an extension.
//!
//! The half of running packages that is the same whatever runs them: install records, the store,
//! what a person is shown and consents to, updates and putting the old version back, switching a
//! package on and off, removing it and clearing what it keeps — and the list of it all, pushed.
//! What runs a package is a carrier ([`harnix_host_api::Carrier`]), chosen by name
//! (`[ext.packages] carrier`); this crate names no VM. Each enabled package is an extension in the
//! process like any other: its carrier registers its descriptor through this component's
//! registrar while it starts it, so it is on the record as this component's.
//!
//! Each installed package is one directory beside the native extensions' (`records.rs`).
//!
//! - `packages.list` — `packages`: every record, with what the carrier says of it (`runtime`) and
//!   what its manifest describes (`ext`); `unreadable`: the directories that do not read, with
//!   why; `registered`: every extension registered while the process runs, by whichever component;
//!   `runtime`: the carrier, its engine's fingerprint, whether unsigned packages install, and — on
//!   the `process` carrier — the sandbox its processes start in (`sandbox`: `{"mode": "on",
//!   "mechanism"}`, `{"mode": "off"}`, or `{"mode": "unavailable", "why"}`, when none starts),
//!   which each package's entry repeats as `sandbox: "<mode>"`.
//! - `packages.installFile {path}` — install a local `.hxext`. A signed one (by the store, or by a
//!   publisher the profile trusts) is verified and installed; an unsigned one only with
//!   `[ext.packages] dev = true`, the developer's door and nobody else's. All or nothing: a step
//!   that fails undoes the ones before it.
//! - `packages.catalog` — what the store (`[ext.packages] store`) offers this Harnix, and what of it
//!   is installed here: an installed version the store has withdrawn (revoked, or older than its
//!   `min_version`) is flagged there, and its record keeps the flag (`revoked`) for the list.
//! - `packages.install {id, version?, consent?}` — from the store, by name: resolved, downloaded,
//!   verified, and **disclosed** — the first call answers what the package would be granted and a
//!   digest; the same call with `consent: <digest>` installs exactly that, or is disclosed afresh if
//!   what would be installed has changed since. All or nothing.
//! - `packages.update {id, version?, consent?, allow_downgrade?}` — to the store's newest (or the
//!   given) version. An update that asks for nothing more than the installed version had goes
//!   ahead; one that asks for more lists only what it adds and waits for consent as `install`
//!   does. An older version than the installed one is refused unless `allow_downgrade: true`, and
//!   then disclosed (`update.downgrade`) and consented to however little it asks for. A failed
//!   update puts the installed version back. Its data is kept.
//! - `packages.enable {id}` / `packages.disable {id}` — flip a record's `enabled`, and start or
//!   stop the package. An enable that fails to start it leaves it disabled and says why.
//! - `packages.uninstall {id}` — stop it, take back what installing it put in place (its agent
//!   package, its skills), and delete its directory, data included.
//! - `packages.clearData {id}` — delete what it keeps (its data directory and its key-value store)
//!   and nothing else: a running one is stopped first and started again after, from nothing.
//! - The topic `packages.list` — the same value as the method, pushed when it changes: after each
//!   of these methods, when the carrier's generation moves, and at least every [`RESYNC_EVERY`]
//!   for what a VM changes without saying. `packages.progress` — a running tool call's progress.
//!
//! The profile's `[ext."<package id>"]` is that package's configuration over its manifest's
//! `[config]`; `[ext.packages] secrets."<id>"` what its `env` names read.

mod methods;
mod records;
mod snapshot;
mod starting;

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use harnix_ext::{caps, Access, Extension, ExtensionContext, Manifest, PackageCarrier, Registrar};
use harnix_host_api::{Carrier, CarrierRegistrar, CarrierSettings, ChildSandbox, HostApi, HostServices, StopReason};
use harnix_package::acquire::Admission;
use serde::Deserialize;
use serde_json::{json, Value};

pub use records::InstallRecord;

/// How often the list is read again although the carrier's generation did not move: for what a VM
/// changes without passing through anything that counts (the memory it holds).
pub const RESYNC_EVERY: Duration = Duration::from_secs(10);

/// The topic the list is pushed on, by its name inside this extension's: `packages.list`.
pub const LIST_TOPIC: &str = "list";

/// The topic a running tool call's progress is told on: `packages.progress`.
pub const PROGRESS_TOPIC: &str = "progress";

/// `[ext.packages]`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// `packages.installFile` installs an unsigned package from a file, and an unsigned package
    /// starts. For developing a package: nothing vouches for such a one.
    pub dev: bool,
    /// The store's catalog (`https://…/index.json`, or `file://` on this machine). Unset, the one
    /// built in. Whatever it lists, a package installs only with a valid signature.
    pub store: Option<String>,
    /// Which carrier runs packages, by name (`wasm`, `process`). Unset: the one linked, or `wasm`
    /// when both are.
    pub carrier: Option<String>,
    /// The program an extension process is started as, with the `process` carrier: another build
    /// of this product, which must be the same build — it is refused otherwise. Unset: this
    /// binary.
    pub proc_program: Option<PathBuf>,
    /// The operating-system sandbox extension processes start in, with the `process` carrier:
    /// `required` (unset), or `off`, for development — they start unconfined, with a warning. The
    /// host reads it from this section and applies it; the carrier has no say. Where there can be
    /// no sandbox, `required` starts no process, and the list says why.
    pub sandbox: Sandbox,
    /// `[ext.packages.trusted_publishers]`: a publisher's id (the first segment of its packages'
    /// ids) and the ed25519 public keys (hex) that sign as it. A package such a key signed in full
    /// is installed and run without the store's countersignature, as `trusted`.
    pub trusted_publishers: BTreeMap<String, Vec<String>>,
    /// `[ext.packages.secrets."<id>"]`: what a package's `host.secret` reads, by the names its
    /// `env` capability declares. Only these: a declared name is never looked up in this
    /// process's environment, which holds the product's own keys.
    pub secrets: BTreeMap<String, BTreeMap<String, String>>,
}

/// `[ext.packages] sandbox`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Sandbox {
    #[default]
    Required,
    Off,
}

/// The `packages` extension, over the carriers it was linked with.
#[derive(Default)]
pub struct Packages {
    carriers: BTreeMap<&'static str, Arc<dyn Carrier>>,
    store_keys: Option<Vec<String>>,
    idle: Option<Duration>,
    state: Arc<OnceLock<Arc<State>>>,
}

impl Packages {
    pub fn new() -> Self {
        Self::default()
    }

    /// A carrier packages may run on, by its name.
    pub fn carrier(mut self, carrier: Arc<dyn Carrier>) -> Self {
        self.carriers.insert(carrier.name(), carrier);
        self
    }

    /// Verify store-countersigned packages with these keys (hex ed25519) instead of the store's
    /// own, built in. For a test application that installs from a mock store; a profile cannot.
    pub fn store_keys<I, T>(mut self, keys: I) -> Self
    where
        I: IntoIterator<Item = T>,
        T: Into<String>,
    {
        self.store_keys = Some(keys.into_iter().map(Into::into).collect());
        self
    }

    /// Let every on-demand backend go after this much idle, in place of its manifest's `idle_secs`
    /// (never under a minute). For a test application that needs to see one let go.
    pub fn idle(mut self, idle: Duration) -> Self {
        self.idle = Some(idle);
        self
    }

    /// The carrier `[ext.packages] carrier` names; unset, the one linked, or `wasm`.
    fn chosen(&self, config: &Config) -> Result<Arc<dyn Carrier>, String> {
        let linked = || self.carriers.keys().copied().collect::<Vec<_>>().join(", ");
        match &config.carrier {
            Some(name) => self.carriers.get(name.as_str()).cloned().ok_or_else(|| {
                format!("[ext.packages] carrier = {name:?} is not a carrier this build links (it links: {})", linked())
            }),
            None => self
                .carriers
                .get("wasm")
                .or_else(|| self.carriers.values().next())
                .cloned()
                .ok_or_else(|| "this build links no carrier for packages".to_string()),
        }
    }

    fn settings(&self, config: &Config) -> CarrierSettings {
        let settings = CarrierSettings::new()
            .dev(config.dev)
            .trusted_publishers(config.trusted_publishers.clone())
            .idle(self.idle)
            .proc_program(config.proc_program.clone());
        match &self.store_keys {
            Some(keys) => settings.store_keys(keys.clone()),
            None => settings,
        }
    }
}

/// What this extension's methods answer from, made when it starts.
pub(crate) struct State {
    /// The state's `ext/`: each package's directory is under it.
    dir: PathBuf,
    dev: bool,
    store: Option<String>,
    admission: Admission,
    carrier: Arc<dyn Carrier>,
    cx: ExtensionContext,
    host: Arc<dyn HostApi>,
    secrets: BTreeMap<String, BTreeMap<String, String>>,
    sections: BTreeMap<String, Value>,
    /// One install, update, enable, disable, clear or uninstall at a time.
    serial: tokio::sync::Mutex<()>,
    /// What the last look was made of ([`snapshot`]).
    looked: Mutex<snapshot::Looked>,
    /// What `packages.list` was last pushed as.
    pushed: Mutex<Value>,
    /// Why each package's last start failed, until it starts or is switched off: what the list
    /// says of one its carrier never ran (its files did not check out, or were not there).
    failed: Mutex<BTreeMap<String, String>>,
    watcher: Mutex<Option<tokio::task::AbortHandle>>,
}

impl State {
    fn package_dir(&self, id: &str) -> PathBuf {
        self.dir.join(id)
    }

    /// What its carrier writes for it: its data and key-value store, and more besides
    /// (`records::RUN`).
    fn run_dir(&self, id: &str) -> PathBuf {
        self.package_dir(id).join(records::RUN)
    }
}

#[async_trait::async_trait]
impl Extension for Packages {
    type Config = Config;

    fn manifest(&self) -> Manifest {
        Manifest::new("packages", env!("CARGO_PKG_VERSION"))
            .title("Packages")
            .disclosure(
                "扩展包：读写安装记录（<状态目录>/ext/<包 id>/），安装、启停、更新、卸载扩展包；把每个启用的包\
                 登记成扩展；把包的 agent 包装进实例、技能写进实例的技能目录",
            )
            .requires([caps::EXTENSIONS_MANAGE, caps::ENGINE_PLUGINS])
            // Placing a package's skills has the instance re-read its configuration.
            .optional([caps::ENGINE_CONFIG_WRITE])
    }

    fn register(&self, r: &mut Registrar<'_, Config>) {
        let person = || Access::kernel().with_device();
        for (name, describe) in methods::METHODS {
            r.method(name).access(person()).describe(describe).raw(move |call, params| methods::answer(name, call, params));
        }
        r.topic(LIST_TOPIC).access(person());
        r.topic(PROGRESS_TOPIC).access(person());
        // What the carrier gives every engine built in this process: its tools, in every session.
        let Ok(carrier) = self.chosen(r.config()) else { return };
        let mut c = CarrierRegistrar::new(self.settings(r.config()));
        carrier.register(&mut c);
        let everywhere = r.engine_everywhere();
        for provider in c.providers() {
            everywhere.provider(provider.clone());
        }
        for pattern in c.admitted() {
            everywhere.admit_everywhere(pattern);
        }
    }

    async fn start(&self, cx: ExtensionContext) -> anyhow::Result<()> {
        let config: Config = cx.config()?;
        let carrier = self.chosen(&config).map_err(anyhow::Error::msg)?;
        let keys = harnix_package::acquire::keys(self.store_keys.as_deref(), &config.trusted_publishers)
            .map_err(|e| anyhow::anyhow!("[ext.packages] {e}"))?;
        let host = cx.service::<HostServices>().map_err(|e| anyhow::anyhow!("{}", e.message))?;
        // `ext/packages/data` is this extension's own; the packages' are its siblings'.
        let dir = cx
            .data_dir()
            .parent()
            .and_then(|own| own.parent())
            .map(PathBuf::from)
            .ok_or_else(|| anyhow::anyhow!("no directory above {}", cx.data_dir().display()))?;
        std::fs::create_dir_all(&dir).map_err(|e| anyhow::anyhow!("{}: {e}", dir.display()))?;
        let sections = host.package_sections();
        let state = Arc::new(State {
            dir,
            dev: config.dev,
            store: config.store.clone(),
            admission: Admission { keys, dev: config.dev },
            carrier,
            cx: cx.clone(),
            host,
            secrets: config.secrets.clone(),
            sections,
            serial: tokio::sync::Mutex::new(()),
            looked: Mutex::default(),
            pushed: Mutex::new(Value::Null),
            failed: Mutex::default(),
            watcher: Mutex::new(None),
        });
        cx.provide_arc(state.clone());
        let _ = self.state.set(state.clone());
        // A section for a package that is not installed is said, not acted on: it may have just
        // been uninstalled.
        let (records, _) = records::read_all(&state.dir);
        let installed: BTreeSet<&str> = records.iter().map(|r| r.id.as_str()).collect();
        for id in state.sections.keys().filter(|id| !installed.contains(id.as_str())) {
            tracing::warn!(package = %id, "[ext.\"{id}\"] configures a package that is not installed");
        }
        // The enabled ones, started where they were left. One that does not start is reported by
        // `packages.list` and the rest go on: a package is not the process.
        for record in records.into_iter().filter(|r| r.enabled) {
            if let Err(e) = starting::start(&state, &record).await {
                tracing::error!(package = %record.id, error = %e.message, "an enabled package did not start");
            }
        }
        let task = tokio::spawn(snapshot::watch(state.clone()));
        *lock(&state.watcher) = Some(task.abort_handle());
        Ok(())
    }

    async fn stop(&self) {
        let Some(state) = self.state.get() else { return };
        if let Some(task) = lock(&state.watcher).take() {
            task.abort();
        }
        // All at once: each one's process may take its graces to go, and the process's shutdown
        // has one bound for everything.
        let (records, _) = records::read_all(&state.dir);
        let mut stopping = tokio::task::JoinSet::new();
        for record in records.into_iter().filter(|r| r.enabled) {
            if let Ok(id) = harnix_ext::ExtId::package(&record.id) {
                let carrier = state.carrier.clone();
                stopping.spawn(async move { carrier.stop(&id, StopReason::ShuttingDown).await });
            }
        }
        while stopping.join_next().await.is_some() {}
    }
}

/// What a method that changed something tells: the next look reads `id` again, and the list is
/// pushed now rather than at the next move of the carrier's generation.
pub(crate) async fn changed(state: &State, id: Option<&str>) {
    state.forget(id);
    snapshot::push(state).await;
}

impl State {
    fn forget(&self, id: Option<&str>) {
        let mut looked = lock(&self.looked);
        looked.generation = None;
        if let Some(id) = id {
            looked.described.retain(|(of, _), _| of != id);
        }
    }
}

/// A lock, whoever panicked holding it: what is behind these is rewritten whole each time.
pub(crate) fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// `{id}` of a method's params.
pub(crate) fn id_param(params: &Value) -> Result<String, harnix_ext::RpcError> {
    params
        .get("id")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| harnix_ext::RpcError::invalid_params("missing id"))
}

/// What its carrier says of it — or, when its carrier has nothing of it and its last start failed,
/// `failed` and why: a start this component refused never reached the carrier.
pub(crate) async fn status(state: &State, id: &harnix_ext::ExtId) -> Value {
    let mut status = state.carrier.status(id).await;
    if status["state"] == json!("stopped") {
        if let Some(why) = lock(&state.failed).get(id.as_str()) {
            status["state"] = json!("failed");
            status["reason"] = json!(why);
        }
    }
    status
}

/// What the list's top says of the carrier.
pub(crate) fn runtime(state: &State, sandbox: Value) -> Value {
    json!({
        "carrier": state.carrier.name(),
        "fingerprint": state.carrier.fingerprint(),
        "dev": state.dev,
        "sandbox": sandbox,
    })
}

/// The sandbox the carrier's processes start in, as the host applies it; `null` for a carrier that
/// starts none.
pub(crate) fn sandbox(state: &State) -> Value {
    if state.carrier.kind() != PackageCarrier::Process {
        return Value::Null;
    }
    match state.host.child_sandbox() {
        ChildSandbox::On(mechanism) => json!({"mode": "on", "mechanism": mechanism}),
        ChildSandbox::Off => json!({"mode": "off"}),
        ChildSandbox::Unavailable(why) => json!({"mode": "unavailable", "why": why}),
        other => json!({"mode": "unavailable", "why": format!("{other:?}")}),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_asks_for_what_registering_and_placing_packages_take() {
        let manifest = Packages::new().manifest();
        assert_eq!(manifest.required(), [caps::EXTENSIONS_MANAGE, caps::ENGINE_PLUGINS]);
        assert_eq!(manifest.optionals(), [caps::ENGINE_CONFIG_WRITE]);
    }

    #[test]
    fn the_config_has_no_switch_of_its_own() {
        let e = serde_json::from_value::<Config>(json!({"enabled": true})).unwrap_err();
        assert!(e.to_string().contains("enabled"), "{e}");
        let config: Config = serde_json::from_value(json!({"dev": true, "carrier": "wasm"})).unwrap();
        assert!(config.dev);
        assert_eq!(config.sandbox, Sandbox::Required);
        let config: Config = serde_json::from_value(json!({"sandbox": "off"})).unwrap();
        assert_eq!(config.sandbox, Sandbox::Off);
        let e = serde_json::from_value::<Config>(json!({"sandbox": "maybe"})).unwrap_err();
        assert!(e.to_string().contains("maybe"), "{e}");
        assert!(Packages::new().chosen(&config).is_err(), "no carrier linked");
    }
}
