//! The `apps` built-in module: runtime applications' install records and switches.
//!
//! Registered only with `[apps] enabled = true`; without it none of this exists. Installing is a
//! later stage — here are the records an installation leaves, one directory per application under
//! the program's own directory, and the switches on them:
//!
//! ```text
//! ~/.<product_id>/apps/<id>/install.json   {"id", "version", "title"?, "enabled"}
//! ```
//!
//! - `harnix.apps.list` — every record (and the ones that do not read, with why), and every
//!   application registered with the host now, by whichever module registered it.
//! - `harnix.apps.enable {id}` / `harnix.apps.disable {id}` — flip a record's `enabled`. Disabling
//!   one this module registered withdraws it from the host as well; nothing loads one on enable yet,
//!   so `registered` says whether it is in the host.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use harnix_contract::{codes, RpcError};
use harnix_module::{CallContext, Handler, Manifest, MethodRegistry, Module, ModuleContext, Principal};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// Every method this module adds.
pub const METHODS: &[&str] = &["harnix.apps.list", "harnix.apps.enable", "harnix.apps.disable"];

/// One install record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstallRecord {
    pub id: String,
    pub version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default)]
    pub enabled: bool,
}

const RECORD: &str = "install.json";

/// The `apps` module, over the install-records directory it is given.
pub struct AppsModule {
    dir: PathBuf,
}

impl AppsModule {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }
}

#[async_trait::async_trait]
impl Module for AppsModule {
    const API_VERSION: u32 = harnix_module::MODULE_API_VERSION;

    fn manifest(&self) -> Manifest {
        Manifest::new("apps", env!("CARGO_PKG_VERSION"))
            .disclosure("运行期应用：读写安装记录（<状态目录>/apps/），启停应用；经宿主登记、撤下应用的方法、主题与 space")
    }

    fn methods(&self, registry: &mut MethodRegistry) {
        for name in METHODS {
            registry
                .method(name.strip_prefix("harnix.").unwrap_or(name), Principal::Device)
                .allow(Principal::Kernel)
                .with(Arc::new(AppsMethod {
                    dir: self.dir.clone(),
                    name,
                }));
        }
    }

    async fn start(&self, _cx: ModuleContext) -> anyhow::Result<()> {
        std::fs::create_dir_all(&self.dir)
            .map_err(|e| anyhow::anyhow!("the install records' directory {}: {e}", self.dir.display()))?;
        Ok(())
    }
}

struct AppsMethod {
    dir: PathBuf,
    name: &'static str,
}

#[async_trait::async_trait]
impl Handler for AppsMethod {
    async fn call(&self, cx: CallContext, params: Value) -> Result<Value, RpcError> {
        let host = cx.module().apps();
        match self.name {
            "harnix.apps.list" => {
                let (installed, unreadable) = read_all(&self.dir);
                let registered = host.as_ref().map(|h| h.registered()).unwrap_or_default();
                let apps: Vec<Value> = installed
                    .iter()
                    .map(|r| {
                        let live = registered.iter().find(|s| s.id == r.id);
                        json!({
                            "id": r.id,
                            "version": r.version,
                            "title": r.title,
                            "enabled": r.enabled,
                            "registered": live.is_some(),
                        })
                    })
                    .collect();
                Ok(json!({
                    "dir": self.dir.display().to_string(),
                    "apps": apps,
                    "unreadable": unreadable,
                    "registered": registered,
                }))
            }
            "harnix.apps.enable" | "harnix.apps.disable" => {
                let id = params
                    .get("id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| RpcError::invalid_params("missing id"))?
                    .to_string();
                let enabled = self.name == "harnix.apps.enable";
                let mut record = read_one(&self.dir, &id)?;
                record.enabled = enabled;
                write_one(&self.dir, &record)?;
                let mut withdrawn = false;
                if let Some(host) = &host {
                    let ours = host.registered().iter().any(|s| s.id == id && s.by == cx.module().id());
                    if !enabled && ours {
                        host.withdraw(&id).await?;
                        withdrawn = true;
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
                }))
            }
            other => Err(RpcError::not_found(format!("`{other}` is not the apps module's"))),
        }
    }
}

/// Whether a directory name can be an application id (the host's rule: dot-separated segments of
/// lowercase letters, digits and `-`, each starting with a letter).
fn valid_id(id: &str) -> bool {
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
        assert_eq!(unreadable.len(), 3);

        let mut record = records[0].clone();
        record.enabled = true;
        write_one(dir.path(), &record).unwrap();
        assert!(read_one(dir.path(), "acme.notes").unwrap().enabled);
        assert_eq!(read_one(dir.path(), "nobody").unwrap_err().code, codes::INVALID_PARAMS);
        // Never a path out of the directory.
        assert_eq!(read_one(dir.path(), "../etc").unwrap_err().code, codes::INVALID_PARAMS);
    }
}
