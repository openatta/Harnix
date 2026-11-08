//! Getting an application, and a newer one: verifying a package, disclosing it, the store's
//! catalog and downloads, and updating an installed application in place.
//!
//! # Updating, in order, and back on a failure
//!
//! 1. The new package is read and its signatures verified, before anything else.
//! 2. The installed version is stopped, and what installing it put in the instances (its agent
//!    package, its skills) is taken back. Its unpacked files stay where they are, and its data —
//!    `data/`, `kv/` — is not touched at all.
//! 3. The new version is put in place as an install would (`Runtime::place`) and started.
//! 4. Done: the old version's files go. Failed: whatever the new version put in place is taken
//!    back, its files go, and the old version is put back from its files and started as it was.

use std::path::{Path, PathBuf};

use harnix_builtin::apps::{AppsRuntime, Disclosed, InstallRecord, PackageInfo};
use harnix_contract::RpcError;
use harnix_module::ModuleContext;
use serde_json::{json, Value};

use crate::disclosure;
use crate::manifest::Manifest;
use crate::package;
use crate::runtime::{internal, refused, Placed, Runtime};
use crate::signature::{self, Trust, Verified};
use crate::store::{self, Store};

impl Trust {
    /// As a record keeps it; anything else is unsigned.
    pub fn parse(text: &str) -> Self {
        match text {
            "store" => Trust::Store,
            "trusted" => Trust::Trusted,
            _ => Trust::Unsigned,
        }
    }
}

impl Runtime {
    fn verify(
        &self,
        signed: Result<
            (
                std::collections::BTreeMap<String, String>,
                Option<signature::Signatures>,
            ),
            String,
        >,
        manifest: &Manifest,
    ) -> Result<Verified, RpcError> {
        let app = &manifest.app;
        let (files, signatures) = signed.map_err(|e| refused(format!("`{}`: {e}", app.id)))?;
        signature::verify(
            signatures.as_ref(),
            &files,
            (&app.id, &app.version),
            &app.publisher,
            &self.options.keys,
        )
        .map_err(|e| refused(format!("`{}` {} is refused: {e}", app.id, app.version)))
    }

    /// Unsigned code goes no further than this unless the process trusts it.
    fn admit(&self, verified: Verified, manifest: &Manifest) -> Result<Verified, RpcError> {
        if !verified.trust.signed() && !self.options.dev {
            return Err(refused(format!(
                "`{}` is unsigned, and an unsigned application is installed and started only with \
                 `[apps] dev = true`",
                manifest.app.id
            )));
        }
        Ok(verified)
    }

    /// The package file's signatures, over its files as they are in the zip: before any of it is
    /// unpacked.
    pub(crate) fn admit_zip(&self, path: &Path, manifest: &Manifest) -> Result<Verified, RpcError> {
        let verified = self.verify(signature::of_zip(path), manifest)?;
        self.admit(verified, manifest)
    }

    /// An installed version's signatures, over its files as they are on disk: before it starts.
    pub(crate) fn admit_root(
        &self,
        root: &Path,
        manifest: &Manifest,
    ) -> Result<Verified, RpcError> {
        let verified = self.verify(signature::of_dir(root), manifest)?;
        self.admit(verified, manifest)
    }

    pub(crate) fn disclose_package(
        &self,
        path: &Path,
        installed: Option<(&InstallRecord, &Path)>,
    ) -> Result<Disclosed, RpcError> {
        let inspected = package::inspect(path, self.engine.fingerprint()).map_err(refused)?;
        let verified = self.verify(signature::of_zip(path), &inspected.manifest)?;
        let previous = match installed {
            Some((record, app_dir)) => Some(
                package::inspect_dir(&app_dir.join(&record.version))
                    .map_err(|e| internal(format!("the installed `{}`: {e}", record.id)))?,
            ),
            None => None,
        };
        let disclosure = disclosure::disclose(
            &inspected,
            &verified,
            previous.as_ref().map(|p| (&p.manifest, p.agent.is_some())),
        );
        let bytes = std::fs::read(path).map_err(|e| refused(format!("{}: {e}", path.display())))?;
        let needs_consent =
            previous.is_none() || disclosure["update"]["needs_consent"] == json!(true);
        let app = inspected.manifest.app;
        Ok(Disclosed {
            info: PackageInfo {
                id: app.id,
                version: app.version,
                title: app.title,
            },
            trust: verified.trust.as_str().into(),
            digest: disclosure::digest(
                &bytes,
                &verified,
                installed.map(|(r, _)| r.version.as_str()),
            ),
            disclosure,
            needs_consent,
        })
    }

    fn store(&self) -> Result<Store, RpcError> {
        Store::new(
            self.options
                .store
                .as_deref()
                .unwrap_or(store::DEFAULT_STORE),
        )
        .map_err(refused)
    }

    pub(crate) async fn catalog_view(&self) -> Result<Value, RpcError> {
        let store = self.store()?;
        let index = store.index().await.map_err(refused)?;
        let mut view = store::view(&index, self.engine.fingerprint());
        view["store"] = json!(store.url());
        view["fingerprint"] = json!(self.engine.fingerprint());
        Ok(view)
    }

    pub(crate) async fn fetch_package(
        &self,
        id: &str,
        version: Option<&str>,
        staging: &Path,
    ) -> Result<PathBuf, RpcError> {
        let store = self.store()?;
        let index = store.index().await.map_err(refused)?;
        let resolved = store
            .resolve(&index, id, version, self.engine.fingerprint())
            .map_err(refused)?;
        let to = staging.join(format!("{id}-{}.hxapp", resolved.version.version));
        store.download(&resolved, &to).await.map_err(refused)?;
        Ok(to)
    }

    pub(crate) async fn update_to(
        &self,
        cx: &ModuleContext,
        record: &InstallRecord,
        path: &Path,
        app_dir: &Path,
    ) -> Result<(Value, Value), RpcError> {
        let id = &record.id;
        let inspected = package::inspect(path, self.engine.fingerprint()).map_err(refused)?;
        if inspected.manifest.app.id != *id {
            return Err(refused(format!(
                "the package is `{}`, not `{id}`",
                inspected.manifest.app.id
            )));
        }
        let verified = self.admit_zip(path, &inspected.manifest)?;
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
        Placed::of(&record.parts).undo(cx, id).await;

        let mut placed = Placed {
            trust: Some(verified.trust),
            ..Placed::default()
        };
        match self
            .place(cx, Some(path), &inspected, app_dir, &mut placed)
            .await
        {
            Ok(started) => {
                if !record.enabled {
                    // It stays switched off, as it was.
                    self.unload(cx, &record_with(record, &version, placed.parts()))
                        .await;
                }
                if let Err(e) = std::fs::remove_dir_all(&old_root) {
                    tracing::warn!(app = %id, error = %e, "the old version's files were not removed");
                }
                let mut summary = self.summary(&placed, started);
                summary["from"] = json!(record.version);
                Ok((placed.parts(), summary))
            }
            Err(e) => {
                tracing::warn!(app = %id, error = %e.message, "the update failed; putting the installed version back");
                placed.undo(cx, id).await;
                let _ = std::fs::remove_dir_all(&new_root);
                self.failed
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .remove(id);
                let trust = record
                    .parts
                    .get("trust")
                    .and_then(Value::as_str)
                    .map(Trust::parse);
                let mut back = Placed {
                    trust,
                    ..Placed::default()
                };
                match self.place(cx, None, &old, app_dir, &mut back).await {
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

    pub(crate) fn describe_app(&self, record: &InstallRecord, app_dir: &Path) -> Value {
        let root = app_dir.join(&record.version);
        let Ok(inspected) = package::inspect_dir(&root) else {
            return Value::Null;
        };
        let manifest = &inspected.manifest;
        let caps = &manifest.capabilities;
        json!({
            "publisher": manifest.app.publisher,
            "description": manifest.app.description,
            "capabilities": {
                "net": caps.net, "env": caps.env, "storage_mb": caps.storage_mb, "hub": caps.hub, "fs": caps.fs,
            },
            "lines": disclosure::lines(manifest, inspected.agent.is_some()),
            "component": manifest.component.as_ref().map(|c| json!({"world": c.world, "memory_mb": c.memory_mb})),
            "ui": manifest.ui.iter().map(|u| json!({"point": u.point, "module": u.module})).collect::<Vec<_>>(),
        })
    }
}

fn record_with(record: &InstallRecord, version: &str, parts: Value) -> InstallRecord {
    InstallRecord {
        version: version.to_string(),
        parts,
        ..record.clone()
    }
}
