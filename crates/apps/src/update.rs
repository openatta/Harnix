//! Updating an installed application in place.
//!
//! # In order, and back on a failure
//!
//! 1. The new package is read and its signatures verified, before anything else.
//! 2. The installed version is stopped, and what installing it put in the instances (its agent
//!    package, its skills) is taken back. Its unpacked files stay where they are, and its data —
//!    `data/`, `kv/` — is not touched at all.
//! 3. The new version is put in place as an install would (`Runtime::place`) and started.
//! 4. Done: the old version's files go. Failed: whatever the new version put in place is taken
//!    back, its files go, and the old version is put back from its files and started as it was.

use std::path::Path;

use harnix_app_package::acquire::{internal, refused};
use harnix_app_package::package;
use harnix_app_package::signature::Trust;
use harnix_builtin::apps::{AppsRuntime, InstallRecord};
use harnix_contract::RpcError;
use harnix_module::ModuleContext;
use serde_json::{json, Value};

use crate::runtime::Runtime;

impl Runtime {
    pub(crate) async fn update_to(
        &self,
        cx: &ModuleContext,
        record: &InstallRecord,
        path: &Path,
        app_dir: &Path,
    ) -> Result<(Value, Value), RpcError> {
        let id = &record.id;
        let inspected = package::inspect(path, &self.fingerprint()).map_err(refused)?;
        if inspected.manifest.app.id != *id {
            return Err(refused(format!(
                "the package is `{}`, not `{id}`",
                inspected.manifest.app.id
            )));
        }
        let verified = self.admission.admit_zip(path, &inspected.manifest)?;
        let version = inspected.manifest.app.version.clone();
        if version == record.version {
            return Err(refused(format!(
                "`{id}` {version} is the version installed"
            )));
        }
        let old_root = app_dir.join(&record.version);
        let old = package::inspect_dir(&old_root)
            .map_err(|e| internal(format!("the installed `{id}`: {e}")))?;
        let new_root = app_dir.join(&version);
        // What an update that did not finish may have left.
        let _ = std::fs::remove_dir_all(&new_root);

        self.unload(cx, record).await;
        self.unplace(id, &record.parts).await;

        let mut parts = None;
        let outcome = async {
            package::extract(path, &new_root).map_err(refused)?;
            self.place(cx, &inspected.manifest, app_dir, verified.trust, &mut parts)
                .await
        }
        .await;
        match outcome {
            Ok((placed, mut summary)) => {
                if !record.enabled {
                    // It stays switched off, as it was.
                    self.unload(cx, &record_with(record, &version, placed.clone()))
                        .await;
                }
                if let Err(e) = std::fs::remove_dir_all(&old_root) {
                    tracing::warn!(app = %id, error = %e, "the old version's files were not removed");
                }
                summary["from"] = json!(record.version);
                Ok((placed, summary))
            }
            Err(e) => {
                tracing::warn!(app = %id, error = %e.message, "the update failed; putting the installed version back");
                if let Some(parts) = &parts {
                    self.unplace(id, parts).await;
                }
                let _ = std::fs::remove_dir_all(&new_root);
                let _ = self.control().forget(id.clone()).await;
                let trust = record
                    .parts
                    .get("trust")
                    .and_then(Value::as_str)
                    .map(Trust::parse)
                    .unwrap_or(Trust::Unsigned);
                let mut back = None;
                match self.place(cx, &old.manifest, app_dir, trust, &mut back).await {
                    Ok(_) if !record.enabled => self.unload(cx, record).await,
                    Ok(_) => {}
                    Err(again) => {
                        tracing::error!(app = %id, error = %again.message, "the installed version did not come back after a failed update");
                    }
                }
                Err(e)
            }
        }
    }
}

fn record_with(record: &InstallRecord, version: &str, parts: Value) -> InstallRecord {
    InstallRecord {
        version: version.to_string(),
        parts,
        ..record.clone()
    }
}
