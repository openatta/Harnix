//! The install records: one directory per installed package under the state's `ext/`, beside the
//! native extensions' own.
//!
//! ```text
//! ~/.<product_id>/ext/<id>/install.json   {"id", "version", "title"?, "enabled", "parts"?, …}
//! ~/.<product_id>/ext/<id>/<version>/     the unpacked package
//! ~/.<product_id>/ext/<id>/run/           what its carrier writes for it: `data/`, `kv/`, and
//!                                         for a process of its own `engine/`, `work/`, `home/`, `tmp/`
//! ```
//!
//! A package reaches nothing of its directory but `run/` to write: its record (its consent) and
//! its unpacked files are this component's.
//!
//! A native extension's directory is named by a one-segment id, a package's by two: only a
//! directory whose name is a package's id is a record's.

use std::path::Path;

use harnix_ext::{codes, ExtId, RpcError};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// Under a package's directory, what its carrier writes for it.
pub(crate) const RUN: &str = "run";

/// The file that makes a directory an installed package.
pub(crate) const RECORD: &str = "install.json";

/// Where downloads wait to be disclosed and installed: under the records' directory, so on the
/// same disk, and never a package's id.
pub(crate) const STAGING: &str = ".staging";

/// One install record.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct InstallRecord {
    pub id: String,
    pub version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default)]
    pub enabled: bool,
    /// What installing it put where, as its carrier wrote it down (its agent package and in which
    /// instances, its skills), how far it was trusted (`trust`) and where it came from (`source`):
    /// what uninstalling has to take back.
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub parts: Value,
    /// The sha256 of the manifest it was installed with — the one disclosed and consented to:
    /// every start checks the manifest on disk against it, and a process of its own must report
    /// the same.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub digest: Option<String>,
    /// Why the store withdrew this version, as its catalog said when it was last read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revoked: Option<String>,
    /// Why it is switched off when nobody switched it off: an update failed and the version it
    /// was to replace did not come back. Cleared when it starts again.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl InstallRecord {
    /// How far its signatures vouched for it when it was installed.
    pub fn trust(&self) -> &str {
        self.parts.get("trust").and_then(Value::as_str).unwrap_or("unsigned")
    }

    /// What its carrier put in place, with what this component wrote beside it left out.
    pub fn placed(&self) -> Value {
        let mut placed = self.parts.clone();
        if let Some(map) = placed.as_object_mut() {
            map.remove("trust");
            map.remove("source");
        }
        placed
    }
}

pub(crate) fn internal(e: impl std::fmt::Display) -> RpcError {
    RpcError::new(codes::INTERNAL_ERROR, e.to_string())
}

/// Whether a directory name under the records' directory can be a package's.
pub(crate) fn is_package_id(id: &str) -> bool {
    harnix_package::valid_id(id) && ExtId::package(id).is_ok()
}

/// Every record that reads, by id; and what did not, with why.
pub(crate) fn read_all(dir: &Path) -> (Vec<InstallRecord>, Vec<Value>) {
    let mut records = Vec::new();
    let mut unreadable = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return (records, unreadable);
    };
    let mut names: Vec<String> = entries
        .flatten()
        .filter(|e| e.path().is_dir())
        .filter_map(|e| e.file_name().into_string().ok())
        // A native extension's directory, or downloads waiting: not a package's.
        .filter(|name| is_package_id(name))
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

pub(crate) fn read_one(dir: &Path, id: &str) -> Result<InstallRecord, RpcError> {
    if !is_package_id(id) {
        return Err(RpcError::invalid_params(format!("`{id}` is not a package id")));
    }
    let path = dir.join(id).join(RECORD);
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(RpcError::invalid_params(format!("no package `{id}` is installed")))
        }
        Err(e) => return Err(internal(format!("{}: {e}", path.display()))),
    };
    let record: InstallRecord = serde_json::from_str(&text).map_err(|e| internal(format!("{}: {e}", path.display())))?;
    if record.id != id {
        return Err(internal(format!("{} says it is `{}`, in the directory of `{id}`", path.display(), record.id)));
    }
    Ok(record)
}

pub(crate) fn write_one(dir: &Path, record: &InstallRecord) -> Result<(), RpcError> {
    let path = dir.join(&record.id).join(RECORD);
    let tmp = path.with_extension("json.tmp");
    let bytes = serde_json::to_vec_pretty(record).map_err(internal)?;
    std::fs::write(&tmp, bytes).map_err(internal)?;
    std::fs::rename(&tmp, &path).map_err(internal)
}

pub(crate) fn refuse_installed(dir: &Path, id: &str) -> Result<(), RpcError> {
    if read_one(dir, id).is_ok() || dir.join(id).exists() {
        return Err(RpcError::refused(format!("a package `{id}` is installed already; uninstall it first")));
    }
    Ok(())
}

/// Delete what a package keeps — its data directory and its key-value store, under its `run_dir` —
/// leaving the directories in place and empty.
pub(crate) fn clear_data(run_dir: &Path) -> std::io::Result<()> {
    for part in ["data", "kv"] {
        let dir = run_dir.join(part);
        match std::fs::remove_dir_all(&dir) {
            Ok(()) => std::fs::create_dir_all(&dir)?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

/// What a package that is not running keeps under `run_dir`, in bytes: its `data/` and `kv/`,
/// symlinks not followed; `None` past as many entries as no quota allows.
pub(crate) fn data_bytes(run_dir: &Path) -> Option<u64> {
    const MAX_ENTRIES: usize = 100_000;
    let mut total = 0u64;
    let mut seen = 0usize;
    let mut stack: Vec<std::path::PathBuf> = ["data", "kv"].iter().map(|p| run_dir.join(p)).collect();
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            seen += 1;
            if seen > MAX_ENTRIES {
                return None;
            }
            match entry.metadata() {
                Ok(meta) if meta.is_dir() => stack.push(entry.path()),
                Ok(meta) if meta.is_file() => total += meta.len(),
                _ => {}
            }
        }
    }
    Some(total)
}

/// Make the record of an update that failed say what is installed now: the version put back, with
/// what putting it back put where; or, when it did not come back, switched off and why — not
/// "installed and enabled" over nothing that runs.
pub(crate) fn settle_failed_update(dir: &Path, record: &InstallRecord, to: &str, restored: Option<Result<Value, String>>) {
    let next = match restored {
        None => return,
        Some(Ok(mut parts)) => {
            if parts.is_object() {
                parts["source"] = record.parts.get("source").cloned().unwrap_or(json!("store"));
                parts["trust"] = json!(record.trust());
            }
            InstallRecord { parts, ..record.clone() }
        }
        Some(Err(why)) => {
            tracing::error!(package = %record.id, from = %record.version, %to, error = %why, "after a failed update the installed version did not come back; it is switched off");
            InstallRecord {
                enabled: false,
                reason: Some(format!("the update to {to} failed, and {} did not come back: {why}", record.version)),
                ..record.clone()
            }
        }
    };
    if let Err(e) = write_one(dir, &next) {
        tracing::error!(package = %record.id, error = %e.message, "the record of a failed update was not rewritten");
    }
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
        seed(dir.path(), "acme.broken", "not json");
        seed(dir.path(), "acme.liar", r#"{"id": "someone-else", "version": "1"}"#);
        // A native extension's own directory, and one no package could be named: not records.
        std::fs::create_dir_all(dir.path().join("system/data")).unwrap();
        seed(dir.path(), "Bad_Id", r#"{"id": "Bad_Id", "version": "1"}"#);
        let (records, unreadable) = read_all(dir.path());
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].id, "acme.notes");
        assert!(!records[0].enabled);
        assert!(records[0].parts.is_null());
        assert_eq!(unreadable.len(), 2, "{unreadable:?}");

        let mut record = records[0].clone();
        record.enabled = true;
        record.parts = json!({"agent": {"package": "acme-notes"}, "trust": "store"});
        write_one(dir.path(), &record).unwrap();
        let back = read_one(dir.path(), "acme.notes").unwrap();
        assert!(back.enabled);
        assert_eq!(back.parts["agent"]["package"], json!("acme-notes"));
        assert_eq!((back.trust(), back.placed()), ("store", json!({"agent": {"package": "acme-notes"}})));
        assert_eq!(read_one(dir.path(), "acme.nobody").unwrap_err().code, codes::INVALID_PARAMS);
        // Never a path out of the directory, and never a native extension's.
        assert_eq!(read_one(dir.path(), "../etc").unwrap_err().code, codes::INVALID_PARAMS);
        assert_eq!(read_one(dir.path(), "system").unwrap_err().code, codes::INVALID_PARAMS);
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
        assert_eq!(data_bytes(&app), Some(15));
        clear_data(&app).unwrap();
        // The directories are there to be written again, empty; the package and the record are
        // untouched.
        assert_eq!(std::fs::read_dir(app.join("data")).unwrap().count(), 0);
        assert_eq!(std::fs::read_dir(app.join("kv")).unwrap().count(), 0);
        assert!(app.join("1.0.0/package-file").exists() && app.join(RECORD).exists());
        assert_eq!(data_bytes(&app), Some(0));
        // Nothing kept, nothing to clear.
        let bare = dir.path().join("acme.bare");
        std::fs::create_dir_all(&bare).unwrap();
        clear_data(&bare).unwrap();
        assert!(!bare.join("data").exists());
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
        settle_failed_update(dir.path(), &record, "2.0.0", Some(Ok(json!({"agent": {"package": "again"}}))));
        let back = read_one(dir.path(), "acme.notes").unwrap();
        assert!(back.enabled && back.reason.is_none());
        assert_eq!(back.parts["agent"]["package"], json!("again"));
        assert_eq!((back.parts["source"].as_str(), back.trust()), (Some("store"), "store"));
        // Gone: switched off, saying why.
        settle_failed_update(dir.path(), &record, "2.0.0", Some(Err("no such instance".into())));
        let gone = read_one(dir.path(), "acme.notes").unwrap();
        assert!(!gone.enabled);
        assert_eq!(gone.reason.as_deref(), Some("the update to 2.0.0 failed, and 1.0.0 did not come back: no such instance"));
    }
}
