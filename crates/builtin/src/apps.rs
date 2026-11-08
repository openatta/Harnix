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
//! - `harnix.apps.installFile {path}` — install a local `.hxapp`. Only with `[apps] dev = true`:
//!   nothing verifies a package yet (signatures arrive with the store), so this is the developer's
//!   door and nobody else's. All or nothing: a step that fails undoes the ones before it.
//! - `harnix.apps.enable {id}` / `harnix.apps.disable {id}` — flip a record's `enabled`, and start
//!   or stop the application: new sessions have its tools, or do not. An enable that fails to start
//!   it leaves it disabled and says why.
//! - `harnix.apps.uninstall {id}` — stop it, take back what installing it put in place (its agent
//!   package, its skills, its interface modules), and delete its directory, data included.
//!
//! What "start" and "install" mean is the runtime's ([`AppsRuntime`], `harnix-apps`): this crate
//! names no WebAssembly. A build without it keeps the records and the switches and refuses to
//! install, saying why.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use harnix_contract::{codes, RpcError};
use harnix_module::{CallContext, Handler, Manifest, MethodRegistry, Module, ModuleContext, Principal};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// The module's id: the one module the kernel hands its application host to.
pub const MODULE_ID: &str = "apps";

/// Every method this module adds.
pub const METHODS: &[&str] = &[
    "harnix.apps.list",
    "harnix.apps.enable",
    "harnix.apps.disable",
    "harnix.apps.installFile",
    "harnix.apps.uninstall",
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
#[derive(Debug, Clone, Serialize, Deserialize)]
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
}

/// What a package says it is, read before anything is written.
#[derive(Debug, Clone)]
pub struct PackageInfo {
    pub id: String,
    pub version: String,
    pub title: Option<String>,
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

    /// What is running of it, or why it is not.
    fn status(&self, id: &str) -> Value;

    /// The process is going: stop every application.
    async fn shutdown(&self);
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
}

impl AppsModule {
    pub fn new(dir: PathBuf) -> Self {
        Self {
            shared: Arc::new(Shared {
                dir,
                dev: false,
                runtime: None,
                serial: tokio::sync::Mutex::new(()),
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
        Ok(())
    }

    async fn stop(&self) {
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
        let shared = &self.shared;
        let module = cx.module();
        let host = module.apps();
        let runtime = shared.runtime.as_ref();
        match self.name {
            "harnix.apps.list" => {
                let (installed, unreadable) = read_all(&shared.dir);
                let registered = host.as_ref().map(|h| h.registered()).unwrap_or_default();
                let apps: Vec<Value> = installed
                    .iter()
                    .map(|r| {
                        let live = registered.iter().find(|s| s.id == r.id);
                        let mut entry = json!({
                            "id": r.id,
                            "version": r.version,
                            "title": r.title,
                            "enabled": r.enabled,
                            "registered": live.is_some(),
                        });
                        if let Some(runtime) = runtime {
                            entry["runtime"] = runtime.status(&r.id);
                        }
                        entry
                    })
                    .collect();
                Ok(json!({
                    "dir": shared.dir.display().to_string(),
                    "apps": apps,
                    "unreadable": unreadable,
                    "registered": registered,
                    "runtime": runtime.map(|r| json!({"fingerprint": r.fingerprint(), "dev": shared.dev})),
                }))
            }
            "harnix.apps.installFile" => {
                let runtime = runtime.ok_or_else(no_runtime)?;
                if !shared.dev {
                    return Err(RpcError::refused(
                        "installing a package from a file needs `[apps] dev = true`: nothing verifies \
                         an unsigned package, so this is for developing an application, not for \
                         installing one",
                    ));
                }
                let path = params
                    .get("path")
                    .and_then(Value::as_str)
                    .map(PathBuf::from)
                    .ok_or_else(|| RpcError::invalid_params("missing path"))?;
                let _serial = shared.serial.lock().await;
                let info = runtime.inspect(&path)?;
                if read_one(&shared.dir, &info.id).is_ok() || shared.dir.join(&info.id).exists() {
                    return Err(RpcError::refused(format!(
                        "an application `{}` is installed already; uninstall it first",
                        info.id
                    )));
                }
                let app_dir = shared.dir.join(&info.id);
                let (parts, summary) = runtime.install(module, &path, &app_dir).await?;
                let record = InstallRecord {
                    id: info.id.clone(),
                    version: info.version.clone(),
                    title: info.title.clone(),
                    enabled: true,
                    parts,
                };
                if let Err(e) = write_one(&shared.dir, &record) {
                    // The record is what makes it installed; without it, take the rest back.
                    runtime.unload(module, &record).await;
                    let _ = runtime.uninstall(module, &record, &app_dir).await;
                    let _ = std::fs::remove_dir_all(&app_dir);
                    return Err(e);
                }
                tracing::info!(app = %record.id, version = %record.version, "application installed");
                Ok(json!({
                    "id": record.id,
                    "version": record.version,
                    "title": record.title,
                    "enabled": true,
                    "installed": summary,
                }))
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
                            Ok(summary) => started = summary,
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
                    "runtime": runtime.map(|r| r.status(&id)),
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

/// Whether a directory name can be an application id (the host's rule: dot-separated segments of
/// lowercase letters, digits and `-`, each starting with a letter).
pub fn valid_id(id: &str) -> bool {
    id.len() <= 64
        && id.split('.').all(|segment| {
            let mut chars = segment.chars();
            matches!(chars.next(), Some('a'..='z')) && chars.all(|c| matches!(c, 'a'..='z' | '0'..='9' | '-'))
        })
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
    fn the_module_asks_for_the_engine_methods_the_runtime_calls() {
        let manifest = AppsModule::new(PathBuf::from("/nowhere")).dev(true).manifest();
        for grant in GRANTS {
            assert!(manifest.grants.iter().any(|g| g == grant), "{grant} is not granted");
        }
    }
}
