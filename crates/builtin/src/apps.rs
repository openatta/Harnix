//! The `apps` built-in module: runtime applications' install records, their switches, and — in a
//! build that carries an application runtime — installing, starting, stopping and removing them.
//!
//! Registered only with `[apps] enabled = true`; without it none of this exists. Each installed
//! application is one directory under the program's own directory:
//!
//! ```text
//! ~/.<product_id>/apps/<id>/install.json   {"id", "version", "title"?, "enabled", "parts"?}
//! ~/.<product_id>/apps/<id>/<version>/     the unpacked package (the runtime's)
//! ~/.<product_id>/apps/<id>/data/, kv/     what the application keeps (the runtime's)
//! ```
//!
//! - `harnix.apps.list` — every record (and the ones that do not read, with why), what the runtime
//!   says of each, and every application registered with the host now, by whichever module.
//! - `harnix.apps.installFile {path}` — install a local `.hxapp`. A signed one (by the store, or by
//!   a publisher the profile trusts) is verified and installed; an unsigned one only with
//!   `[apps] dev = true`, the developer's door and nobody else's. All or nothing: a step that fails
//!   undoes the ones before it.
//! - `harnix.apps.catalog` — what the store (`[apps] store`) offers this Harnix, and what of it is
//!   installed here: an installed version the store has withdrawn (revoked, or older than its
//!   `min_version`) is flagged there, and its record keeps the flag (`revoked`) for the list.
//! - `harnix.apps.install {id, version?, consent?}` — from the store, by name: resolved, downloaded,
//!   verified, and **disclosed** — the first call answers what the application would be granted and
//!   a digest; the same call with `consent: <digest>` installs exactly that, or is disclosed afresh
//!   if what would be installed has changed since. All or nothing.
//! - `harnix.apps.update {id, version?, consent?, allow_downgrade?}` — to the store's newest (or the
//!   given) version. An update that asks for nothing more than the installed version had goes
//!   ahead; one that asks for more lists only what it adds and waits for consent as `install` does.
//!   An older version than the installed one is refused unless `allow_downgrade: true`, and then
//!   disclosed (`update.downgrade`) and consented to however little it asks for. A failed update
//!   leaves the installed version as it was. Its data is kept.
//! - `harnix.apps.enable {id}` / `harnix.apps.disable {id}` — flip a record's `enabled`, and start
//!   or stop the application: new sessions have its tools, or do not. An enable that fails to start
//!   it leaves it disabled and says why.
//! - `harnix.apps.uninstall {id}` — stop it, take back what installing it put in place (its agent
//!   package, its skills, its interface modules), and delete its directory, data included.
//! - `harnix.apps.clearData {id}` — delete what it keeps (its data directory and its key-value
//!   store) and nothing else: a running one is stopped first and started again after, from nothing,
//!   so no instance holds on to what was deleted. Answers how much was cleared.
//! - The topic `harnix.apps` — the same value as `harnix.apps.list`, pushed whenever it changes:
//!   what the system space's Apps tab draws (each application's VM health, its calls and errors,
//!   the calls it was refused). Looked at once a second while the process runs, and pushed only
//!   when something in it moved. A look is cheap: what each application's manifest describes is
//!   kept by id and version; each one's status is asked again only when the runtime's
//!   `generation` moved (or every [`RESYNC_EVERY`], for what a VM changes without saying); and
//!   what it keeps is what its VM counted, not a walk of its directory.
//!
//! The record keeps how far the package was trusted (`parts.trust`: `store`, `trusted`,
//! `unsigned`) and where it came from (`parts.source`: `store` or `file`), as the runtime verified
//! it: whether its interface runs in the page or in an isolated frame is decided from that.
//!
//! What "start" and "install" mean is the runtime's ([`AppsRuntime`], `harnix-apps`): this crate
//! names no WebAssembly. A build without it keeps the records and the switches and refuses to
//! install, saying why.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use harnix_contract::{codes, RpcError};
use harnix_module::{
    AppHost, CallContext, EventRegistry, Handler, Manifest, MethodRegistry, Module, ModuleContext, Principal,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// The module's id: the one module the kernel hands its application host to.
pub const MODULE_ID: &str = "apps";

/// The topic the list is pushed on: `harnix.apps`.
pub const TOPIC: &str = "apps";

/// How often the list is looked at for a change to push.
const WATCH_EVERY: std::time::Duration = std::time::Duration::from_secs(1);

/// How often a look asks the runtime for every status although its generation did not move: for
/// what a VM changes without passing through anything that counts (the memory it holds).
pub const RESYNC_EVERY: std::time::Duration = std::time::Duration::from_secs(10);

/// Every method this module adds.
pub const METHODS: &[&str] = &[
    "harnix.apps.list",
    "harnix.apps.enable",
    "harnix.apps.disable",
    "harnix.apps.installFile",
    "harnix.apps.uninstall",
    "harnix.apps.catalog",
    "harnix.apps.install",
    "harnix.apps.update",
    "harnix.apps.clearData",
];

/// Engine methods the runtime calls as this module: installing an application's agent package
/// into the instances it names, switching it with the application, and having an instance re-read
/// its skills.
pub const GRANTS: &[&str] = &[
    "plugin.install",
    "plugin.uninstall",
    "plugin.enable",
    "plugin.disable",
    "plugin.list",
    "config.reload",
];

/// One install record.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct InstallRecord {
    pub id: String,
    pub version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default)]
    pub enabled: bool,
    /// What installing it put where, as the runtime wrote it down (its agent package and in which
    /// instances, its skills): what uninstalling has to take back. Opaque here.
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub parts: Value,
    /// Why the store withdrew this version, as its catalog said when it was last read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revoked: Option<String>,
    /// Why it is switched off when nobody switched it off: an update failed and the version it
    /// was to replace did not come back. Cleared when it starts again.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// An update that did not happen, and what became of the version it was to replace.
#[derive(Debug)]
pub struct UpdateFailed {
    pub error: RpcError,
    /// `None`: the installed version was not touched. `Some(Ok(parts))`: it was taken down and put
    /// back, and `parts` is what putting it back put where, which its record must say from now on.
    /// `Some(Err(why))`: it did not come back.
    pub restored: Option<Result<Value, String>>,
}

impl From<RpcError> for UpdateFailed {
    fn from(error: RpcError) -> Self {
        Self {
            error,
            restored: None,
        }
    }
}

/// What a package says it is, read before anything is written.
#[derive(Debug, Clone)]
pub struct PackageInfo {
    pub id: String,
    pub version: String,
    pub title: Option<String>,
}

/// A package read, its signatures verified, and described: what installing it would grant, and
/// the digest that consenting to exactly that is.
#[derive(Debug, Clone)]
pub struct Disclosed {
    pub info: PackageInfo,
    /// `store`, `trusted` or `unsigned`.
    pub trust: String,
    /// What a person is shown before consenting (the runtime's shape).
    pub disclosure: Value,
    /// Changes when the package, its trust or the version it replaces does.
    pub digest: String,
    /// An update that asks for more than the installed version had, or goes back to an older
    /// one; an install always does.
    pub needs_consent: bool,
    /// An update to an older version than the installed one.
    pub downgrade: bool,
}

/// What loads, runs and removes applications. `harnix-apps` implements it, in a build with the
/// `apps` feature; the record keeping above it is this module's.
#[async_trait::async_trait]
pub trait AppsRuntime: Send + Sync {
    /// What precompiled artifacts must have been compiled for to load here.
    fn fingerprint(&self) -> String;

    /// Read and check a package file — its manifest, its layout, an artifact for this engine —
    /// without installing anything.
    fn inspect(&self, package: &Path) -> Result<PackageInfo, RpcError>;

    /// Install it under `app_dir` (`<apps>/<id>/`) and start it: everything but the record. On an
    /// error nothing it did is left behind. Answers what goes in the record's `parts`, and a
    /// summary for the caller.
    async fn install(
        &self,
        cx: &ModuleContext,
        package: &Path,
        app_dir: &Path,
    ) -> Result<(Value, Value), RpcError>;

    /// Start an installed application: at process start, or when it is enabled.
    async fn load(&self, cx: &ModuleContext, record: &InstallRecord, app_dir: &Path) -> Result<Value, RpcError>;

    /// Stop it: new sessions no longer get its tools, its interface modules are no longer served.
    async fn unload(&self, cx: &ModuleContext, record: &InstallRecord);

    /// Take back what installing it put elsewhere (after `unload`). The directory is this
    /// module's to delete.
    async fn uninstall(&self, cx: &ModuleContext, record: &InstallRecord, app_dir: &Path) -> Result<Value, RpcError>;

    /// What is running of it, or why it is not. Asked of the host that runs it, which may be in
    /// another process: so it is awaited.
    async fn status(&self, id: &str) -> Value;

    /// The process is going: stop every application.
    async fn shutdown(&self);

    /// Read, verify and describe a package file; `installed`, the record and directory of the
    /// version it would replace. A runtime that verifies nothing says so: unsigned.
    fn disclose(
        &self,
        package: &Path,
        installed: Option<(&InstallRecord, &Path)>,
    ) -> Result<Disclosed, RpcError> {
        let _ = installed;
        Ok(Disclosed {
            info: self.inspect(package)?,
            trust: "unsigned".into(),
            disclosure: Value::Null,
            digest: String::new(),
            needs_consent: true,
            downgrade: false,
        })
    }

    /// What the store offers this Harnix; `installed`, the version of each application installed
    /// here (by id), which each of the catalog's applications says it has and flags as
    /// `installed_revoked` (why) when the store withdrew it.
    async fn catalog(&self, installed: &BTreeMap<String, String>) -> Result<Value, RpcError> {
        let _ = installed;
        Err(RpcError::refused("this application runtime has no store"))
    }

    /// Resolve `id` (at `version`, or the newest this Harnix runs) in the store and download it into
    /// `staging`, checked against what the store lists: the package file, for `disclose` and
    /// `install` or `update`.
    async fn fetch(&self, id: &str, version: Option<&str>, staging: &Path) -> Result<PathBuf, RpcError> {
        let _ = (id, version, staging);
        Err(RpcError::refused("this application runtime has no store"))
    }

    /// Replace the installed version (`record`, stopped or running) with `package`, keeping its
    /// data. On an error the installed version is put back as it was — running again if it was —
    /// and the failure says whether that worked ([`UpdateFailed::restored`]). Answers the new
    /// record's `parts`, and a summary.
    async fn update(
        &self,
        cx: &ModuleContext,
        record: &InstallRecord,
        package: &Path,
        app_dir: &Path,
    ) -> Result<(Value, Value), UpdateFailed> {
        let _ = (cx, record, package, app_dir);
        Err(RpcError::refused("this application runtime does not update applications").into())
    }

    /// What an installed application is and asks for, from its own manifest. Asked once per id
    /// and version.
    fn describe(&self, record: &InstallRecord, app_dir: &Path) -> Value {
        let _ = (record, app_dir);
        Value::Null
    }

    /// A number that moves whenever [`status`](Self::status) of some application may answer
    /// something else: what is looked at once a second asks for the statuses again only then.
    /// `None`: the runtime cannot tell, and is asked every time.
    fn generation(&self) -> Option<u64> {
        None
    }

    /// What an application that is not running keeps under `app_dir`, in bytes; `None` when it is
    /// not measured. (One that runs says so in its status: `health.storage_bytes`.)
    fn data_bytes(&self, app_dir: &Path) -> Option<u64> {
        let _ = app_dir;
        None
    }
}

const RECORD: &str = "install.json";

/// The `apps` module, over the install-records directory it is given.
pub struct AppsModule {
    shared: Arc<Shared>,
}

struct Shared {
    dir: PathBuf,
    /// `[apps] dev`: installing an unsigned package from a file.
    dev: bool,
    runtime: Option<Arc<dyn AppsRuntime>>,
    /// One install, enable, disable or uninstall at a time.
    serial: tokio::sync::Mutex<()>,
    /// What pushes `harnix.apps`, while the module runs.
    watcher: std::sync::Mutex<Option<tokio::task::AbortHandle>>,
    /// What the last look was made of ([`snapshot`]).
    looked: std::sync::Mutex<Looked>,
}

/// What the last look at the applications read, to read again only what may have changed.
#[derive(Default)]
struct Looked {
    /// `describe`, by id and version: an unpacked version's manifest does not change under it.
    described: std::collections::HashMap<(String, String), Value>,
    /// The runtime's generation the statuses were read at, and when; `None`: read them again.
    generation: Option<u64>,
    at: Option<std::time::Instant>,
    /// Each application's status, and what it keeps, as read then.
    status: std::collections::HashMap<String, (Value, Option<u64>)>,
}

impl Shared {
    fn looked(&self) -> std::sync::MutexGuard<'_, Looked> {
        self.looked.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Something was done to the applications here: the next look reads everything of `id` again.
    fn changed(&self, id: Option<&str>) {
        let mut looked = self.looked();
        looked.generation = None;
        if let Some(id) = id {
            looked.described.retain(|(of, _), _| of != id);
        }
    }
}

impl AppsModule {
    pub fn new(dir: PathBuf) -> Self {
        Self {
            shared: Arc::new(Shared {
                dir,
                dev: false,
                runtime: None,
                serial: tokio::sync::Mutex::new(()),
                watcher: std::sync::Mutex::new(None),
                looked: std::sync::Mutex::default(),
            }),
        }
    }

    /// `[apps] dev = true`: `harnix.apps.installFile` accepts an unsigned package.
    pub fn dev(self, dev: bool) -> Self {
        self.rebuild(|s| s.dev = dev)
    }

    /// The runtime that installs and runs applications.
    pub fn runtime(self, runtime: Arc<dyn AppsRuntime>) -> Self {
        self.rebuild(|s| s.runtime = Some(runtime))
    }

    fn rebuild(self, f: impl FnOnce(&mut Shared)) -> Self {
        let Shared { dir, dev, runtime, .. } = Arc::try_unwrap(self.shared)
            .unwrap_or_else(|_| unreachable!("configured before it is shared"));
        let mut shared = Shared {
            dir,
            dev,
            runtime,
            serial: tokio::sync::Mutex::new(()),
            watcher: std::sync::Mutex::new(None),
            looked: std::sync::Mutex::default(),
        };
        f(&mut shared);
        Self {
            shared: Arc::new(shared),
        }
    }
}

#[async_trait::async_trait]
impl Module for AppsModule {
    const API_VERSION: u32 = harnix_module::MODULE_API_VERSION;

    fn manifest(&self) -> Manifest {
        let mut manifest = Manifest::new(MODULE_ID, env!("CARGO_PKG_VERSION")).disclosure(
            "运行期应用：读写安装记录（<状态目录>/apps/），安装、启停、卸载应用；把应用的 agent 包装进实例、\
             技能写进实例的技能目录；经宿主登记、撤下应用的方法、主题与 space",
        );
        for grant in GRANTS {
            manifest = manifest.grant(*grant);
        }
        manifest
    }

    fn methods(&self, registry: &mut MethodRegistry) {
        for name in METHODS {
            registry
                .method(name.strip_prefix("harnix.").unwrap_or(name), Principal::Device)
                .allow(Principal::Kernel)
                .with(Arc::new(AppsMethod {
                    shared: self.shared.clone(),
                    name,
                }));
        }
    }

    fn events(&self, events: &mut EventRegistry) {
        events.topic(TOPIC);
    }

    async fn start(&self, cx: ModuleContext) -> anyhow::Result<()> {
        let dir = &self.shared.dir;
        std::fs::create_dir_all(dir)
            .map_err(|e| anyhow::anyhow!("the install records' directory {}: {e}", dir.display()))?;
        // The enabled ones, started where they were left. One that does not start is reported by
        // `harnix.apps.list` and the rest go on: an application is not the process.
        if let Some(runtime) = &self.shared.runtime {
            let (records, _) = read_all(dir);
            for record in records.into_iter().filter(|r| r.enabled) {
                if let Err(e) = runtime.load(&cx, &record, &dir.join(&record.id)).await {
                    tracing::error!(app = %record.id, error = %e.message, "an enabled application did not start");
                }
            }
        }
        // `harnix.apps`: pushed when it changes. The runtime's counters move without telling anyone,
        // so it is looked at rather than waited for.
        let shared = self.shared.clone();
        let task = tokio::spawn(async move {
            let mut last = Value::Null;
            loop {
                let now = snapshot(&shared, cx.apps().as_ref(), false).await;
                if now != last {
                    if let Err(e) = cx.events().publish(TOPIC, json!({}), &now).await {
                        tracing::warn!(error = %e.message, "harnix.apps was not published");
                    }
                    last = now;
                }
                tokio::time::sleep(WATCH_EVERY).await;
            }
        });
        *self.shared.watcher.lock().unwrap_or_else(|e| e.into_inner()) = Some(task.abort_handle());
        Ok(())
    }

    async fn stop(&self) {
        if let Some(task) = self.shared.watcher.lock().unwrap_or_else(|e| e.into_inner()).take() {
            task.abort();
        }
        if let Some(runtime) = &self.shared.runtime {
            runtime.shutdown().await;
        }
    }
}

struct AppsMethod {
    shared: Arc<Shared>,
    name: &'static str,
}

fn id_param(params: &Value) -> Result<String, RpcError> {
    params
        .get("id")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| RpcError::invalid_params("missing id"))
}

fn no_runtime() -> RpcError {
    RpcError::refused(
        "this build carries no application runtime (it was built without the `apps` feature): \
         install records are kept, nothing is installed or run",
    )
}

#[async_trait::async_trait]
impl Handler for AppsMethod {
    async fn call(&self, cx: CallContext, params: Value) -> Result<Value, RpcError> {
        let id = params.get("id").and_then(Value::as_str).map(str::to_string);
        let answer = self.answer(cx, params).await;
        if !matches!(self.name, "harnix.apps.list" | "harnix.apps.catalog") {
            self.shared.changed(id.as_deref());
        }
        answer
    }
}

impl AppsMethod {
    async fn answer(&self, cx: CallContext, params: Value) -> Result<Value, RpcError> {
        let shared = &self.shared;
        let module = cx.module();
        let host = module.apps();
        let runtime = shared.runtime.as_ref();
        match self.name {
            // Asked: read afresh, but for what does not change under a version.
            "harnix.apps.list" => Ok(snapshot(shared, host.as_ref(), true).await),
            "harnix.apps.installFile" => {
                let runtime = runtime.ok_or_else(no_runtime)?;
                let path = params
                    .get("path")
                    .and_then(Value::as_str)
                    .map(PathBuf::from)
                    .ok_or_else(|| RpcError::invalid_params("missing path"))?;
                let _serial = shared.serial.lock().await;
                let disclosed = runtime.disclose(&path, None)?;
                if disclosed.trust == "unsigned" && !shared.dev {
                    return Err(RpcError::refused(
                        "the package is unsigned, and installing an unsigned package from a file \
                         needs `[apps] dev = true`: that is for developing an application, not for \
                         installing one",
                    ));
                }
                refuse_installed(&shared.dir, &disclosed.info.id)?;
                install(shared, module, runtime, &path, &disclosed, "file").await
            }
            "harnix.apps.catalog" => {
                let runtime = runtime.ok_or_else(no_runtime)?;
                let (installed, _) = read_all(&shared.dir);
                let versions = installed.iter().map(|r| (r.id.clone(), r.version.clone())).collect();
                let catalog = runtime.catalog(&versions).await?;
                // What the store says of a version installed here stays with its record: the list
                // shows it without asking the store.
                let _serial = shared.serial.lock().await;
                for app in catalog["apps"].as_array().into_iter().flatten() {
                    let (Some(id), Some(version)) = (app["id"].as_str(), app["installed"].as_str()) else {
                        continue;
                    };
                    let revoked = app["installed_revoked"].as_str().map(str::to_string);
                    match read_one(&shared.dir, id) {
                        Ok(mut record) if record.version == version && record.revoked != revoked => {
                            if let Some(why) = &revoked {
                                tracing::warn!(app = %id, %version, reason = %why, "the store has withdrawn an installed version");
                            }
                            record.revoked = revoked;
                            write_one(&shared.dir, &record)?;
                        }
                        _ => {}
                    }
                }
                Ok(catalog)
            }
            "harnix.apps.install" | "harnix.apps.update" => {
                let runtime = runtime.ok_or_else(no_runtime)?;
                let id = id_param(&params)?;
                if !valid_id(&id) {
                    return Err(RpcError::invalid_params(format!("`{id}` is not an application id")));
                }
                let version = params.get("version").and_then(Value::as_str);
                let consent = params.get("consent").and_then(Value::as_str);
                let allow_downgrade = params.get("allow_downgrade").and_then(Value::as_bool) == Some(true);
                let updating = self.name == "harnix.apps.update";
                let _serial = shared.serial.lock().await;
                let installed = read_one(&shared.dir, &id).ok();
                match (&installed, updating) {
                    (Some(record), false) => {
                        return Err(RpcError::refused(format!(
                            "`{id}` {} is installed already; `harnix.apps.update` moves it to another version",
                            record.version
                        )))
                    }
                    (None, true) => return Err(RpcError::invalid_params(format!("no application `{id}` is installed"))),
                    _ => {}
                }
                if installed.is_none() && shared.dir.join(&id).exists() {
                    refuse_installed(&shared.dir, &id)?;
                }
                let staging = shared.dir.join(STAGING).join(uuid::Uuid::new_v4().simple().to_string());
                let outcome = async {
                    let package = runtime.fetch(&id, version, &staging).await?;
                    let app_dir = shared.dir.join(&id);
                    let disclosed = runtime.disclose(&package, installed.as_ref().map(|r| (r, app_dir.as_path())))?;
                    if disclosed.info.id != id {
                        return Err(RpcError::refused(format!(
                            "the store's package for `{id}` is the application `{}`",
                            disclosed.info.id
                        )));
                    }
                    if let Some(record) = &installed {
                        // Without a version asked for, an update goes forward or nowhere.
                        let behind = version.is_none() && disclosed.downgrade;
                        if record.version == disclosed.info.version || behind {
                            return Ok(json!({
                                "id": id,
                                "version": record.version,
                                "latest": disclosed.info.version,
                                "up_to_date": true,
                            }));
                        }
                        // Back to an older version only when that is what was asked for: whoever
                        // serves the store cannot make an update one.
                        if disclosed.downgrade && !allow_downgrade {
                            return Err(RpcError::refused(format!(
                                "`{id}` {} is older than the installed {}: going back to it is asked for \
                                 with `allow_downgrade: true`",
                                disclosed.info.version, record.version
                            )));
                        }
                    }
                    let consented = consent == Some(disclosed.digest.as_str());
                    if disclosed.needs_consent && !consented {
                        // Disclosed, and nothing done: the same call with the digest goes ahead.
                        return Ok(json!({
                            "id": id,
                            "version": disclosed.info.version,
                            "consent": "required",
                            "digest": disclosed.digest,
                            "disclosure": disclosed.disclosure,
                            // A digest that was passed and is not this one was consent to something else.
                            "changed": consent.is_some(),
                        }));
                    }
                    match &installed {
                        None => install(shared, module, runtime, &package, &disclosed, "store").await,
                        Some(record) => update(shared, module, runtime, record, &package, &disclosed).await,
                    }
                }
                .await;
                let _ = std::fs::remove_dir_all(&staging);
                outcome
            }
            "harnix.apps.enable" | "harnix.apps.disable" => {
                let id = id_param(&params)?;
                let enabled = self.name == "harnix.apps.enable";
                let _serial = shared.serial.lock().await;
                let mut record = read_one(&shared.dir, &id)?;
                record.enabled = enabled;
                write_one(&shared.dir, &record)?;
                let mut withdrawn = false;
                if let Some(host) = &host {
                    let ours = host.registered().iter().any(|s| s.id == id && s.by == module.id());
                    if !enabled && ours {
                        host.withdraw(&id).await?;
                        withdrawn = true;
                    }
                }
                let mut started = Value::Null;
                if let Some(runtime) = runtime {
                    if enabled {
                        match runtime.load(module, &record, &shared.dir.join(&id)).await {
                            Ok(summary) => {
                                started = summary;
                                // Started again: whatever left it off is over.
                                if record.reason.take().is_some() {
                                    write_one(&shared.dir, &record)?;
                                }
                            }
                            Err(e) => {
                                // What was asked for did not happen: it stays as it was.
                                record.enabled = false;
                                write_one(&shared.dir, &record)?;
                                return Err(e);
                            }
                        }
                    } else {
                        runtime.unload(module, &record).await;
                    }
                }
                let registered = host
                    .as_ref()
                    .is_some_and(|h| h.registered().iter().any(|s| s.id == id));
                Ok(json!({
                    "id": id,
                    "enabled": enabled,
                    "registered": registered,
                    "withdrawn": withdrawn,
                    "runtime": match runtime {
                        Some(r) => Some(r.status(&id).await),
                        None => None,
                    },
                    "started": started,
                }))
            }
            "harnix.apps.clearData" => {
                let id = id_param(&params)?;
                let _serial = shared.serial.lock().await;
                let record = read_one(&shared.dir, &id)?;
                let app_dir = shared.dir.join(&id);
                let status = match runtime {
                    Some(r) => r.status(&id).await,
                    None => Value::Null,
                };
                // What the list said it keeps, counted the same way.
                let before = runtime.and_then(|r| kept_bytes(r.as_ref(), &status, &app_dir));
                // Stopped around it: a backend's memory, or a store it has open, would outlive the
                // files otherwise.
                let running = record.enabled && runtime.is_some() && status["state"] != json!("stopped");
                if running {
                    if let Some(runtime) = runtime {
                        runtime.unload(module, &record).await;
                    }
                }
                let cleared = clear_data(&app_dir);
                let mut started = Value::Null;
                if running {
                    if let Some(runtime) = runtime {
                        started = match runtime.load(module, &record, &app_dir).await {
                            Ok(summary) => summary,
                            // Cleared, and not running: said, and the list says why.
                            Err(e) => json!({"error": e.message}),
                        };
                    }
                }
                cleared.map_err(|e| internal(format!("{}: {e}", app_dir.display())))?;
                let after = match runtime {
                    Some(r) => kept_bytes(r.as_ref(), &r.status(&id).await, &app_dir),
                    None => None,
                };
                tracing::info!(app = %id, bytes = ?before, "application data cleared");
                Ok(json!({
                    "id": id,
                    "cleared_bytes": before,
                    "data_bytes": after,
                    "restarted": running,
                    "started": started,
                }))
            }
            "harnix.apps.uninstall" => {
                let id = id_param(&params)?;
                let _serial = shared.serial.lock().await;
                let record = read_one(&shared.dir, &id)?;
                if let Some(host) = &host {
                    if host.registered().iter().any(|s| s.id == id && s.by == module.id()) {
                        host.withdraw(&id).await?;
                    }
                }
                let app_dir = shared.dir.join(&id);
                let mut removed = Value::Null;
                if let Some(runtime) = runtime {
                    runtime.unload(module, &record).await;
                    removed = runtime.uninstall(module, &record, &app_dir).await?;
                }
                std::fs::remove_dir_all(&app_dir)
                    .map_err(|e| internal(format!("{}: {e}", app_dir.display())))?;
                tracing::info!(app = %id, "application uninstalled");
                Ok(json!({"id": id, "uninstalled": true, "removed": removed}))
            }
            other => Err(RpcError::not_found(format!("`{other}` is not the apps module's"))),
        }
    }
}

/// Every record (and the ones that do not read, with why), what the runtime says of each, and every
/// application registered with the host now: `harnix.apps.list`, and the value of `harnix.apps`.
/// Each application's status (and what it keeps) is read again only when `fresh`, when the
/// runtime's generation moved since the last look, or when that look is [`RESYNC_EVERY`] old; its
/// description once per version.
async fn snapshot(shared: &Shared, host: Option<&AppHost>, fresh: bool) -> Value {
    let runtime = shared.runtime.as_ref();
    let (installed, unreadable) = read_all(&shared.dir);
    let registered = host.map(|h| h.registered()).unwrap_or_default();
    let generation = runtime.and_then(|r| r.generation());
    let reuse = {
        let looked = shared.looked();
        !fresh
            && generation.is_some()
            && looked.generation == generation
            && looked.at.is_some_and(|at| at.elapsed() < RESYNC_EVERY)
    };
    let mut apps: Vec<Value> = Vec::with_capacity(installed.len());
    let mut statuses = std::collections::HashMap::new();
    for r in &installed {
        let live = registered.iter().find(|s| s.id == r.id);
        let app_dir = shared.dir.join(&r.id);
        let cached = if reuse { shared.looked().status.get(&r.id).cloned() } else { None };
        let (status, kept) = match (cached, runtime) {
            (Some(cached), _) => cached,
            (None, Some(runtime)) => {
                let status = runtime.status(&r.id).await;
                let kept = kept_bytes(runtime.as_ref(), &status, &app_dir);
                (status, kept)
            }
            (None, None) => (Value::Null, None),
        };
        let mut entry = json!({
            "id": r.id,
            "version": r.version,
            "title": r.title,
            "enabled": r.enabled,
            "registered": live.is_some(),
            "trust": r.parts.get("trust").cloned().unwrap_or(json!("unsigned")),
            "source": r.parts.get("source").cloned().unwrap_or(json!("file")),
            "data_bytes": kept,
        });
        if let Some(why) = &r.revoked {
            entry["revoked"] = json!(why);
        }
        if let Some(why) = &r.reason {
            entry["reason"] = json!(why);
        }
        if let Some(runtime) = runtime {
            entry["runtime"] = status.clone();
            entry["app"] = described(shared, runtime.as_ref(), r, &app_dir);
        }
        statuses.insert(r.id.clone(), (status, kept));
        apps.push(entry);
    }
    {
        let mut looked = shared.looked();
        if !reuse {
            looked.generation = generation;
            looked.at = Some(std::time::Instant::now());
        }
        looked.status = statuses;
        looked
            .described
            .retain(|(id, version), _| installed.iter().any(|r| r.id == *id && r.version == *version));
    }
    json!({
        "dir": shared.dir.display().to_string(),
        "apps": apps,
        "unreadable": unreadable,
        "registered": registered,
        "runtime": runtime.map(|r| json!({"fingerprint": r.fingerprint(), "dev": shared.dev})),
    })
}

/// What an installed version's manifest describes, read once per id and version.
fn described(shared: &Shared, runtime: &dyn AppsRuntime, record: &InstallRecord, app_dir: &Path) -> Value {
    let key = (record.id.clone(), record.version.clone());
    if let Some(described) = shared.looked().described.get(&key) {
        return described.clone();
    }
    let described = runtime.describe(record, app_dir);
    // One that does not read yet (being unpacked, or broken) is asked again next time.
    if !described.is_null() {
        shared.looked().described.insert(key, described.clone());
    }
    described
}

/// What an application keeps, in bytes: as its VM counts it against its quota while it runs
/// (`health.storage_bytes`), measured on disk by the runtime when it does not.
fn kept_bytes(runtime: &dyn AppsRuntime, status: &Value, app_dir: &Path) -> Option<u64> {
    status["health"]["storage_bytes"]
        .as_u64()
        .or_else(|| runtime.data_bytes(app_dir))
}

/// Whether a directory name can be an application id: the rule a package's manifest is held to
/// as well (`harnix_contract::apps`).
pub use harnix_contract::apps::valid_id;

/// Where downloads wait to be disclosed and installed: under the records' directory, so on the
/// same disk, and skipped as not a record.
const STAGING: &str = ".staging";

fn refuse_installed(dir: &Path, id: &str) -> Result<(), RpcError> {
    if read_one(dir, id).is_ok() || dir.join(id).exists() {
        return Err(RpcError::refused(format!(
            "an application `{id}` is installed already; uninstall it first"
        )));
    }
    Ok(())
}

/// Install a package that was disclosed (and consented to, from the store), and write its record.
async fn install(
    shared: &Shared,
    module: &ModuleContext,
    runtime: &Arc<dyn AppsRuntime>,
    path: &Path,
    disclosed: &Disclosed,
    source: &str,
) -> Result<Value, RpcError> {
    let info = &disclosed.info;
    let app_dir = shared.dir.join(&info.id);
    let (mut parts, summary) = runtime.install(module, path, &app_dir).await?;
    if parts.is_object() {
        parts["source"] = json!(source);
    }
    let record = InstallRecord {
        id: info.id.clone(),
        version: info.version.clone(),
        title: info.title.clone(),
        enabled: true,
        parts,
        ..InstallRecord::default()
    };
    if let Err(e) = write_one(&shared.dir, &record) {
        // The record is what makes it installed; without it, take the rest back.
        runtime.unload(module, &record).await;
        let _ = runtime.uninstall(module, &record, &app_dir).await;
        let _ = std::fs::remove_dir_all(&app_dir);
        return Err(e);
    }
    tracing::info!(app = %record.id, version = %record.version, trust = %disclosed.trust, source, "application installed");
    Ok(json!({
        "id": record.id,
        "version": record.version,
        "title": record.title,
        "enabled": true,
        "trust": disclosed.trust,
        "source": source,
        "installed": summary,
    }))
}

/// Move an installed application to the version in `path`, and rewrite its record. The runtime
/// leaves the installed version as it was if it cannot.
async fn update(
    shared: &Shared,
    module: &ModuleContext,
    runtime: &Arc<dyn AppsRuntime>,
    record: &InstallRecord,
    path: &Path,
    disclosed: &Disclosed,
) -> Result<Value, RpcError> {
    let app_dir = shared.dir.join(&record.id);
    let (mut parts, summary) = match runtime.update(module, record, path, &app_dir).await {
        Ok(done) => done,
        Err(failed) => {
            settle_failed_update(&shared.dir, record, &disclosed.info.version, failed.restored);
            return Err(failed.error);
        }
    };
    if parts.is_object() {
        parts["source"] = json!("store");
    }
    let next = InstallRecord {
        id: record.id.clone(),
        version: disclosed.info.version.clone(),
        title: disclosed.info.title.clone(),
        enabled: record.enabled,
        parts,
        // Another version: what the store said of the last one is not said of this.
        ..InstallRecord::default()
    };
    write_one(&shared.dir, &next)?;
    tracing::info!(app = %next.id, from = %record.version, to = %next.version, downgrade = disclosed.downgrade, "application updated");
    Ok(json!({
        "id": next.id,
        "version": next.version,
        "from": record.version,
        "downgrade": disclosed.downgrade,
        "title": next.title,
        "enabled": next.enabled,
        "trust": disclosed.trust,
        "updated": summary,
    }))
}

/// Make the record of an update that failed say what is installed now: the version put back, with
/// what putting it back put where; or, when it did not come back, switched off and why — not
/// "installed and enabled" over nothing that runs.
fn settle_failed_update(dir: &Path, record: &InstallRecord, to: &str, restored: Option<Result<Value, String>>) {
    let next = match restored {
        None => return,
        Some(Ok(mut parts)) => {
            if parts.is_object() {
                parts["source"] = record.parts.get("source").cloned().unwrap_or(json!("store"));
            }
            InstallRecord {
                parts,
                ..record.clone()
            }
        }
        Some(Err(why)) => {
            tracing::error!(app = %record.id, from = %record.version, %to, error = %why, "after a failed update the installed version did not come back; it is switched off");
            InstallRecord {
                enabled: false,
                reason: Some(format!(
                    "the update to {to} failed, and {} did not come back: {why}",
                    record.version
                )),
                ..record.clone()
            }
        }
    };
    if let Err(e) = write_one(dir, &next) {
        tracing::error!(app = %record.id, error = %e.message, "the record of a failed update was not rewritten");
    }
}

/// Delete what an application keeps — its data directory and its key-value store — leaving the
/// directories in place and empty.
fn clear_data(app_dir: &Path) -> std::io::Result<()> {
    for part in ["data", "kv"] {
        let dir = app_dir.join(part);
        match std::fs::remove_dir_all(&dir) {
            Ok(()) => std::fs::create_dir_all(&dir)?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

fn internal(e: impl std::fmt::Display) -> RpcError {
    RpcError::new(codes::INTERNAL_ERROR, e.to_string())
}

/// Every record that reads, by id; and what did not, with why.
fn read_all(dir: &Path) -> (Vec<InstallRecord>, Vec<Value>) {
    let mut records = Vec::new();
    let mut unreadable = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return (records, unreadable);
    };
    let mut names: Vec<String> = entries
        .flatten()
        .filter(|e| e.path().is_dir())
        .filter_map(|e| e.file_name().into_string().ok())
        // Downloads waiting to be installed, not an application.
        .filter(|name| name != STAGING)
        .collect();
    names.sort();
    for name in names {
        match read_one(dir, &name) {
            Ok(record) => records.push(record),
            Err(e) => unreadable.push(json!({"dir": name, "reason": e.message})),
        }
    }
    (records, unreadable)
}

fn read_one(dir: &Path, id: &str) -> Result<InstallRecord, RpcError> {
    if !valid_id(id) {
        return Err(RpcError::invalid_params(format!("`{id}` is not an application id")));
    }
    let path = dir.join(id).join(RECORD);
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(RpcError::invalid_params(format!("no application `{id}` is installed")))
        }
        Err(e) => return Err(internal(format!("{}: {e}", path.display()))),
    };
    let record: InstallRecord =
        serde_json::from_str(&text).map_err(|e| internal(format!("{}: {e}", path.display())))?;
    if record.id != id {
        return Err(internal(format!(
            "{} says it is `{}`, in the directory of `{id}`",
            path.display(),
            record.id
        )));
    }
    Ok(record)
}

fn write_one(dir: &Path, record: &InstallRecord) -> Result<(), RpcError> {
    let path = dir.join(&record.id).join(RECORD);
    let tmp = path.with_extension("json.tmp");
    let bytes = serde_json::to_vec_pretty(record).map_err(internal)?;
    std::fs::write(&tmp, bytes).map_err(internal)?;
    std::fs::rename(&tmp, &path).map_err(internal)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seed(dir: &Path, id: &str, body: &str) {
        std::fs::create_dir_all(dir.join(id)).unwrap();
        std::fs::write(dir.join(id).join(RECORD), body).unwrap();
    }

    #[test]
    fn records_read_and_write_and_say_why_they_do_not() {
        let dir = tempfile::tempdir().unwrap();
        seed(dir.path(), "acme.notes", r#"{"id": "acme.notes", "version": "1.2.0", "title": "Notes"}"#);
        seed(dir.path(), "broken", "not json");
        seed(dir.path(), "liar", r#"{"id": "someone-else", "version": "1"}"#);
        seed(dir.path(), "Bad_Id", r#"{"id": "Bad_Id", "version": "1"}"#);
        let (records, unreadable) = read_all(dir.path());
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].id, "acme.notes");
        assert!(!records[0].enabled);
        assert!(records[0].parts.is_null());
        assert_eq!(unreadable.len(), 3);

        let mut record = records[0].clone();
        record.enabled = true;
        record.parts = json!({"agent": {"package": "acme-notes"}});
        write_one(dir.path(), &record).unwrap();
        let back = read_one(dir.path(), "acme.notes").unwrap();
        assert!(back.enabled);
        assert_eq!(back.parts["agent"]["package"], json!("acme-notes"));
        assert_eq!(read_one(dir.path(), "nobody").unwrap_err().code, codes::INVALID_PARAMS);
        // Never a path out of the directory.
        assert_eq!(read_one(dir.path(), "../etc").unwrap_err().code, codes::INVALID_PARAMS);
    }

    #[test]
    fn downloads_are_not_records() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("acme.notes/1.0.0")).unwrap();
        std::fs::create_dir_all(dir.path().join(STAGING).join("x")).unwrap();
        let (records, unreadable) = read_all(dir.path());
        assert!(records.is_empty());
        assert_eq!(unreadable.len(), 1, "{unreadable:?}");
        assert_eq!(unreadable[0]["dir"], json!("acme.notes"));
    }

    #[test]
    fn clearing_takes_what_it_keeps_and_nothing_else() {
        let dir = tempfile::tempdir().unwrap();
        let app = dir.path().join("acme.notes");
        std::fs::create_dir_all(app.join("data/sub")).unwrap();
        std::fs::create_dir_all(app.join("kv")).unwrap();
        std::fs::create_dir_all(app.join("1.0.0")).unwrap();
        std::fs::write(app.join("data/sub/a"), [0u8; 10]).unwrap();
        std::fs::write(app.join("kv/b"), [0u8; 5]).unwrap();
        std::fs::write(app.join("1.0.0/package-file"), [0u8; 100]).unwrap();
        std::fs::write(app.join(RECORD), "{}").unwrap();
        clear_data(&app).unwrap();
        // The directories are there to be written again, empty; the package and the record are
        // untouched.
        assert_eq!(std::fs::read_dir(app.join("data")).unwrap().count(), 0);
        assert_eq!(std::fs::read_dir(app.join("kv")).unwrap().count(), 0);
        assert!(app.join("1.0.0/package-file").exists() && app.join(RECORD).exists());
        // Nothing kept, nothing to clear.
        let bare = dir.path().join("acme.bare");
        std::fs::create_dir_all(&bare).unwrap();
        clear_data(&bare).unwrap();
        assert!(!bare.join("data").exists());
    }

    /// A runtime that counts what it is asked.
    #[derive(Default)]
    struct Counting {
        generation: std::sync::atomic::AtomicU64,
        statuses: std::sync::atomic::AtomicUsize,
        described: std::sync::atomic::AtomicUsize,
        measured: std::sync::atomic::AtomicUsize,
        /// Whether its status has a VM's health.
        health: std::sync::atomic::AtomicBool,
    }

    #[async_trait::async_trait]
    impl AppsRuntime for Counting {
        fn fingerprint(&self) -> String {
            "fp".into()
        }
        fn inspect(&self, _: &Path) -> Result<PackageInfo, RpcError> {
            unreachable!()
        }
        async fn install(&self, _: &ModuleContext, _: &Path, _: &Path) -> Result<(Value, Value), RpcError> {
            unreachable!()
        }
        async fn load(&self, _: &ModuleContext, _: &InstallRecord, _: &Path) -> Result<Value, RpcError> {
            unreachable!()
        }
        async fn unload(&self, _: &ModuleContext, _: &InstallRecord) {}
        async fn uninstall(&self, _: &ModuleContext, _: &InstallRecord, _: &Path) -> Result<Value, RpcError> {
            unreachable!()
        }
        async fn status(&self, _: &str) -> Value {
            use std::sync::atomic::Ordering::SeqCst;
            let n = self.statuses.fetch_add(1, SeqCst);
            if self.health.load(SeqCst) {
                json!({"state": "running", "n": n, "health": {"storage_bytes": 42}})
            } else {
                json!({"state": "stopped", "n": n})
            }
        }
        async fn shutdown(&self) {}
        fn describe(&self, _: &InstallRecord, _: &Path) -> Value {
            self.described.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            json!({"publisher": "acme"})
        }
        fn generation(&self) -> Option<u64> {
            Some(self.generation.load(std::sync::atomic::Ordering::SeqCst))
        }
        fn data_bytes(&self, _: &Path) -> Option<u64> {
            self.measured.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Some(7)
        }
    }

    #[tokio::test]
    async fn a_look_reads_again_only_what_may_have_changed() {
        use std::sync::atomic::Ordering::SeqCst;
        let dir = tempfile::tempdir().unwrap();
        seed(dir.path(), "acme.notes", r#"{"id": "acme.notes", "version": "1.0.0", "enabled": true}"#);
        let runtime = Arc::new(Counting::default());
        runtime.health.store(true, SeqCst);
        let module = AppsModule::new(dir.path().to_path_buf()).runtime(runtime.clone());
        let shared = &module.shared;
        let counts = || {
            (
                runtime.statuses.load(SeqCst),
                runtime.described.load(SeqCst),
                runtime.measured.load(SeqCst),
            )
        };

        let first = snapshot(shared, None, false).await;
        assert_eq!(first["apps"][0]["data_bytes"], json!(42), "what the VM counted");
        assert_eq!(first["apps"][0]["app"]["publisher"], json!("acme"));
        assert_eq!(counts(), (1, 1, 0));
        // Nothing moved: nothing is asked, and the value is the same.
        for _ in 0..5 {
            assert_eq!(snapshot(shared, None, false).await, first);
        }
        assert_eq!(counts(), (1, 1, 0));
        // The runtime's generation moved: its status is asked again, its manifest is not.
        runtime.generation.fetch_add(1, SeqCst);
        let moved = snapshot(shared, None, false).await;
        assert_ne!(moved, first);
        assert_eq!(counts(), (2, 1, 0));
        // `harnix.apps.list` asks afresh; a method that changed something has the next look do so.
        snapshot(shared, None, true).await;
        assert_eq!(counts(), (3, 1, 0));
        shared.changed(Some("acme.notes"));
        snapshot(shared, None, false).await;
        assert_eq!(counts(), (4, 2, 0));
        // Not running: what it keeps is measured on disk, once per reading of its status.
        runtime.health.store(false, SeqCst);
        runtime.generation.fetch_add(1, SeqCst);
        let stopped = snapshot(shared, None, false).await;
        assert_eq!(stopped["apps"][0]["data_bytes"], json!(7));
        snapshot(shared, None, false).await;
        assert_eq!(counts(), (5, 2, 1));
        // A new version is described anew.
        seed(dir.path(), "acme.notes", r#"{"id": "acme.notes", "version": "1.1.0", "enabled": true}"#);
        snapshot(shared, None, false).await;
        assert_eq!(counts().1, 3);
        // Without a generation, every look asks.
        let unknowing = Arc::new(Unknowing(Counting::default()));
        let plain = AppsModule::new(dir.path().to_path_buf()).runtime(unknowing.clone());
        snapshot(&plain.shared, None, false).await;
        snapshot(&plain.shared, None, false).await;
        assert_eq!(unknowing.0.statuses.load(SeqCst), 2);
    }

    /// A runtime that cannot tell when its statuses change.
    struct Unknowing(Counting);

    #[async_trait::async_trait]
    impl AppsRuntime for Unknowing {
        fn fingerprint(&self) -> String {
            self.0.fingerprint()
        }
        fn inspect(&self, p: &Path) -> Result<PackageInfo, RpcError> {
            self.0.inspect(p)
        }
        async fn install(&self, cx: &ModuleContext, p: &Path, d: &Path) -> Result<(Value, Value), RpcError> {
            self.0.install(cx, p, d).await
        }
        async fn load(&self, cx: &ModuleContext, r: &InstallRecord, d: &Path) -> Result<Value, RpcError> {
            self.0.load(cx, r, d).await
        }
        async fn unload(&self, _: &ModuleContext, _: &InstallRecord) {}
        async fn uninstall(&self, cx: &ModuleContext, r: &InstallRecord, d: &Path) -> Result<Value, RpcError> {
            self.0.uninstall(cx, r, d).await
        }
        async fn status(&self, id: &str) -> Value {
            self.0.status(id).await
        }
        async fn shutdown(&self) {}
    }

    #[test]
    fn a_failed_update_leaves_a_record_of_what_is_installed() {
        let dir = tempfile::tempdir().unwrap();
        seed(dir.path(), "acme.notes", "{}");
        let record = InstallRecord {
            id: "acme.notes".into(),
            version: "1.0.0".into(),
            enabled: true,
            parts: json!({"trust": "store", "source": "store", "agent": {"package": "old"}}),
            ..InstallRecord::default()
        };
        write_one(dir.path(), &record).unwrap();
        // Not touched: the record is not either.
        settle_failed_update(dir.path(), &record, "2.0.0", None);
        assert_eq!(read_one(dir.path(), "acme.notes").unwrap().parts, record.parts);
        // Put back: with what putting it back put where.
        settle_failed_update(
            dir.path(),
            &record,
            "2.0.0",
            Some(Ok(json!({"trust": "store", "agent": {"package": "again"}}))),
        );
        let back = read_one(dir.path(), "acme.notes").unwrap();
        assert!(back.enabled && back.reason.is_none());
        assert_eq!(back.parts["agent"]["package"], json!("again"));
        assert_eq!(back.parts["source"], json!("store"));
        // Gone: switched off, saying why.
        settle_failed_update(dir.path(), &record, "2.0.0", Some(Err("no such instance".into())));
        let gone = read_one(dir.path(), "acme.notes").unwrap();
        assert!(!gone.enabled);
        assert_eq!(
            gone.reason.as_deref(),
            Some("the update to 2.0.0 failed, and 1.0.0 did not come back: no such instance")
        );
    }

    #[test]
    fn the_module_asks_for_the_engine_methods_the_runtime_calls() {
        let manifest = AppsModule::new(PathBuf::from("/nowhere")).dev(true).manifest();
        for grant in GRANTS {
            assert!(manifest.grants.iter().any(|g| g == grant), "{grant} is not granted");
        }
    }
}
