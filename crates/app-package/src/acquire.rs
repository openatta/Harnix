//! Getting an application: verifying a package (or an unpacked one) against the keys a process
//! trusts, disclosing it to the person about to install it, the store's catalog and downloads,
//! and describing an installed one from its own manifest.
//!
//! A package is verified before anything of it is unpacked ([`Admission::admit_zip`]), and an
//! installed one again, over its files as they are on disk, every time it starts
//! ([`Admission::admit_root`]). Unsigned code goes no further than this unless the process trusts
//! it (`[apps] dev = true`).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use harnix_contract::{codes, RpcError};
use serde_json::{json, Value};

use crate::disclosure;
use crate::manifest::Manifest;
use crate::package;
use crate::signature::{self, Keys, Signatures, Trust, Verified};
use crate::store::{self, Store};

pub fn refused(message: impl Into<String>) -> RpcError {
    RpcError::refused(message)
}

pub fn internal(message: impl Into<String>) -> RpcError {
    RpcError::new(codes::INTERNAL_ERROR, message)
}

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

/// What a process admits packages by: the keys it verifies with, and whether it runs unsigned
/// code (`[apps] dev`).
#[derive(Debug, Clone, Default)]
pub struct Admission {
    pub keys: Keys,
    pub dev: bool,
}

/// A package file, disclosed: what it is, how far it is trusted, what it asks for, and the digest
/// a consent names.
#[derive(Debug, Clone)]
pub struct Disclosure {
    pub id: String,
    pub version: String,
    pub title: Option<String>,
    pub trust: Trust,
    pub disclosure: Value,
    pub digest: String,
    pub needs_consent: bool,
}

type Signed = Result<(BTreeMap<String, String>, Option<Signatures>), String>;

impl Admission {
    fn verify(&self, signed: Signed, manifest: &Manifest) -> Result<Verified, RpcError> {
        let app = &manifest.app;
        let (files, signatures) = signed.map_err(|e| refused(format!("`{}`: {e}", app.id)))?;
        signature::verify(
            signatures.as_ref(),
            &files,
            (&app.id, &app.version),
            &app.publisher,
            &self.keys,
        )
        .map_err(|e| refused(format!("`{}` {} is refused: {e}", app.id, app.version)))
    }

    /// Unsigned code goes no further than this unless the process trusts it.
    fn admit(&self, verified: Verified, manifest: &Manifest) -> Result<Verified, RpcError> {
        if !verified.trust.signed() && !self.dev {
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
    pub fn admit_zip(&self, path: &Path, manifest: &Manifest) -> Result<Verified, RpcError> {
        let verified = self.verify(signature::of_zip(path), manifest)?;
        self.admit(verified, manifest)
    }

    /// An installed version's signatures, over its files as they are on disk: before it starts.
    pub fn admit_root(&self, root: &Path, manifest: &Manifest) -> Result<Verified, RpcError> {
        let verified = self.verify(signature::of_dir(root), manifest)?;
        self.admit(verified, manifest)
    }

    /// Read, verify and describe a package file for an engine whose fingerprint is `fingerprint`;
    /// `installed`, the version it would replace and the application's directory.
    pub fn disclose(
        &self,
        path: &Path,
        installed: Option<(&str, &Path)>,
        fingerprint: &str,
    ) -> Result<Disclosure, RpcError> {
        let inspected = package::inspect(path, fingerprint).map_err(refused)?;
        let verified = self.verify(signature::of_zip(path), &inspected.manifest)?;
        let previous = match installed {
            Some((version, app_dir)) => Some(
                package::inspect_dir(&app_dir.join(version)).map_err(|e| {
                    internal(format!("the installed `{}`: {e}", inspected.manifest.app.id))
                })?,
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
        Ok(Disclosure {
            id: app.id,
            version: app.version,
            title: app.title,
            trust: verified.trust,
            digest: disclosure::digest(&bytes, &verified, installed.map(|(version, _)| version)),
            disclosure,
            needs_consent,
        })
    }
}

fn store_at(url: Option<&str>) -> Result<Store, RpcError> {
    Store::new(url.unwrap_or(store::DEFAULT_STORE)).map_err(refused)
}

/// What the store at `url` (the default store when none) offers an engine whose fingerprint is
/// `fingerprint`.
pub async fn catalog(url: Option<&str>, fingerprint: &str) -> Result<Value, RpcError> {
    let store = store_at(url)?;
    let index = store.index().await.map_err(refused)?;
    let mut view = store::view(&index, fingerprint);
    view["store"] = json!(store.url());
    view["fingerprint"] = json!(fingerprint);
    Ok(view)
}

/// Resolve `id` (at `version`, or the newest this engine runs) in the store and download it into
/// `staging`, checked against what the store lists.
pub async fn fetch(
    url: Option<&str>,
    id: &str,
    version: Option<&str>,
    fingerprint: &str,
    staging: &Path,
) -> Result<PathBuf, RpcError> {
    let store = store_at(url)?;
    let index = store.index().await.map_err(refused)?;
    let resolved = store
        .resolve(&index, id, version, fingerprint)
        .map_err(refused)?;
    let to = staging.join(format!("{id}-{}.hxapp", resolved.version.version));
    store.download(&resolved, &to).await.map_err(refused)?;
    Ok(to)
}

/// What an installed application is and asks for, from the manifest unpacked at `root`; null when
/// it does not read.
pub fn describe(root: &Path) -> Value {
    let Ok(inspected) = package::inspect_dir(root) else {
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
        "events": manifest.events.tools,
        "lines": disclosure::lines(manifest, inspected.agent.is_some()),
        "component": manifest.component.as_ref().map(|c| json!({"world": c.world, "memory_mb": c.memory_mb})),
        "ui": manifest.ui.iter().map(|u| json!({"point": u.point, "module": u.module})).collect::<Vec<_>>(),
        "inline": manifest.inline,
    })
}
