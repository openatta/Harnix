//! Installing, starting and updating a package, in order, and backwards on a failure: what its
//! carrier is asked, around what this component writes down.
//!
//! **Installing**: the package is read and its signatures verified before anything of it is
//! unpacked; it is unpacked to `<ext>/<id>/<version>/`; its carrier starts it — putting its agent
//! package and skills in place, registering it as an extension, starting its VM; then its record
//! is written. A step that fails undoes every step before it, and the package is not installed.
//!
//! **Updating**: the new package is verified first; the installed version is stopped and what it
//! put in place taken back (its files and its data stay); the new one is unpacked and started.
//! Done, the old files go. Failed, whatever the new one left goes, and the old one is started
//! again from its files; the record says how that went.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use harnix_ext::{caps, ExtId, Rpc, RpcError};
use harnix_host_api::{Installed, StopReason};
use harnix_package::acquire::{internal, no_such_instance, refused, Disclosure};
use harnix_package::manifest::Manifest;
use harnix_package::package;
use harnix_package::signature::Verified;
use serde_json::{json, Value};

use crate::records::{write_one, InstallRecord};
use crate::State;

fn ext_id(id: &str) -> Rpc<ExtId> {
    ExtId::package(id).map_err(|e| RpcError::invalid_params(e.to_string()))
}

/// What a package's consent grants it, from the manifest it was installed with: the package
/// capabilities it asks for, and what it may call.
fn grants(manifest: &Manifest) -> (Vec<harnix_ext::Capability>, BTreeSet<String>) {
    let mut held = Vec::new();
    if manifest.events.all() {
        held.push(caps::EVENTS_ALL_TOOLS);
    }
    (held, manifest.capabilities.calls.iter().cloned().collect())
}

/// Each instance its `[agent] install_into` names is one of this product's: checked before its
/// carrier is asked, so that a package is accepted or refused alike on both. The in-process
/// carrier puts its agent package there; the process carrier puts it in the package's own engine,
/// and the names have no further effect.
fn instances_named(state: &State, manifest: &Manifest) -> Rpc<()> {
    match manifest.agent.install_into.iter().find(|target| state.cx.engine(target).is_none()) {
        Some(target) => Err(no_such_instance(&manifest.app.id, target)),
        None => Ok(()),
    }
}

/// What the installed packages' tools leave room for: no name another package's already has once
/// mapped (`harnix_package::tool_name`), and at most 64 in all. A record whose files do not read
/// is skipped. `manifest`'s own package, when installed, is replaced rather than counted.
fn tools_named(state: &State, manifest: &Manifest) -> Rpc<()> {
    let (records, _) = crate::records::read_all(&state.dir);
    let others: Vec<Manifest> = records
        .iter()
        .filter(|r| r.id != manifest.app.id)
        .filter_map(|r| harnix_package::acquire::manifest_in(&state.package_dir(&r.id).join(&r.version)).ok())
        .collect();
    harnix_package::check_tools_beside(manifest, &others).map_err(refused)
}

/// The manifest unpacked at `root`, as JSON for the carrier.
fn manifest_value(root: &Path) -> Value {
    std::fs::read_to_string(root.join(harnix_package::manifest::MANIFEST))
        .ok()
        .and_then(|text| toml::from_str::<toml::Value>(&text).ok())
        .and_then(|toml| serde_json::to_value(toml).ok())
        .unwrap_or(Value::Null)
}

/// One installed package, as its carrier is handed it: everything checked already. `verified`:
/// its signatures, when they were checked just now; otherwise they are checked here, over its
/// files as they are on disk — what is there may not be what was installed.
pub(crate) fn installed(state: &State, record: &InstallRecord, verified: Option<&Verified>) -> Rpc<Installed> {
    let pkg_dir = state.package_dir(&record.id).join(&record.version);
    // Its record without its files: nothing to start, and nothing to check them against.
    if !pkg_dir.join(harnix_package::manifest::MANIFEST).is_file() {
        return Err(refused(format!(
            "`{}` {}: its files are not there ({}); install it again",
            record.id,
            record.version,
            pkg_dir.display()
        )));
    }
    match verified {
        Some(verified) => described(state, record, Some(verified)),
        None => {
            let manifest = harnix_package::acquire::manifest_in(&pkg_dir)?;
            let on_disk = state.admission.admit_root(&pkg_dir, &manifest)?;
            described(state, record, Some(&on_disk))
        }
    }
}

/// One installed package as its record says it was trusted, its signatures not checked: for
/// taking back what it put in place, which a package whose files changed under it still has to
/// allow.
pub(crate) fn recorded(state: &State, record: &InstallRecord) -> Rpc<Installed> {
    described(state, record, None)
}

/// The sha256 of the manifest unpacked at `pkg_dir`, as its bytes are on disk.
pub(crate) fn manifest_digest(pkg_dir: &Path) -> Rpc<String> {
    let path = pkg_dir.join(harnix_package::manifest::MANIFEST);
    let bytes = std::fs::read(&path).map_err(|e| internal(format!("{}: {e}", path.display())))?;
    Ok(harnix_package::signature::sha256(&bytes))
}

fn described(state: &State, record: &InstallRecord, verified: Option<&Verified>) -> Rpc<Installed> {
    let id = ext_id(&record.id)?;
    let app_dir = state.package_dir(&record.id);
    let pkg_dir = app_dir.join(&record.version);
    // Checked whenever it is to run: the manifest it runs by is the one consented to.
    if verified.is_some() && record.digest.as_deref() != Some(manifest_digest(&pkg_dir)?.as_str()) {
        return Err(refused(format!(
            "`{}`'s manifest on disk is not the one consented to (its digest is not its record's); install it again",
            record.id
        )));
    }
    let installed = Installed::new(id, &record.version, pkg_dir.clone(), app_dir.join(crate::records::RUN))
        .record(serde_json::to_value(record).map_err(|e| internal(e.to_string()))?)
        .config(state.sections.get(&record.id).cloned().unwrap_or(Value::Null))
        .trust(verified.map(|v| v.trust.as_str()).unwrap_or(record.trust()));
    // Taking back what it put in place needs only its record: its files may be gone.
    if verified.is_none() && !pkg_dir.join(harnix_package::manifest::MANIFEST).is_file() {
        return Ok(installed);
    }
    let manifest = harnix_package::acquire::manifest_in(&pkg_dir)?;
    let (held, calls) = grants(&manifest);
    // Only the secrets its `env` names: what the person consented to it reading.
    let secrets: BTreeMap<String, String> = state
        .secrets
        .get(&record.id)
        .into_iter()
        .flatten()
        .filter(|(name, _)| manifest.capabilities.env.contains(name))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    let mut installed = installed.manifest(manifest_value(&pkg_dir)).grants(held, calls).secrets(secrets);
    let fingerprint = state.carrier.fingerprint();
    if let (Some(verified), Some(component)) = (verified, &manifest.component) {
        if let Some(sha) = verified.files.get(&format!("wasm/{}.{fingerprint}.cwasm", component.name)) {
            installed = installed.cwasm_sha256(sha);
        }
    }
    Ok(installed)
}

/// What its record says from now on, from what its carrier placed and how it was trusted.
fn parts(placed: Value, trust: &str, source: &str) -> Value {
    let mut parts = if placed.is_object() { placed } else { json!({}) };
    parts["trust"] = json!(trust);
    parts["source"] = json!(source);
    parts
}

/// Start an installed package: at process start, when it is enabled, after its data is cleared.
/// What its carrier placed for it is written to its record when it changed.
pub(crate) async fn start(state: &State, record: &InstallRecord) -> Rpc<Value> {
    let running = async { state.carrier.start(&*state.host, &state.cx, installed(state, record, None)?).await }.await;
    // Why it is not running, for the list, until it starts or is switched off.
    let running = match running {
        Ok(running) => {
            crate::lock(&state.failed).remove(&record.id);
            running
        }
        Err(e) => {
            crate::lock(&state.failed).insert(record.id.clone(), e.message.clone());
            return Err(e);
        }
    };
    let (placed, summary) = running.into_parts();
    let source = record.parts.get("source").and_then(Value::as_str).unwrap_or("file");
    let next = InstallRecord { parts: parts(placed, record.trust(), source), reason: None, ..record.clone() };
    if next.parts != record.parts || record.reason.is_some() {
        write_one(&state.dir, &next)?;
    }
    Ok(summary)
}

/// Install a package that was disclosed (and consented to, from the store), and write its record.
pub(crate) async fn install(state: &State, path: &Path, disclosed: &Disclosure, source: &str) -> Rpc<Value> {
    let fingerprint = state.carrier.fingerprint();
    let inspected = package::inspect(path, &fingerprint).map_err(refused)?;
    // Before a byte of it is written.
    let verified = state.admission.admit_zip(path, &inspected.manifest)?;
    instances_named(state, &inspected.manifest)?;
    tools_named(state, &inspected.manifest)?;
    let app = &inspected.manifest.app;
    let app_dir = state.package_dir(&app.id);
    std::fs::create_dir_all(&app_dir).map_err(|e| internal(format!("{}: {e}", app_dir.display())))?;
    let mut record = InstallRecord {
        id: app.id.clone(),
        version: app.version.clone(),
        title: app.title.clone(),
        enabled: true,
        parts: parts(Value::Null, verified.trust.as_str(), source),
        ..InstallRecord::default()
    };
    let started = async {
        package::extract(path, &app_dir.join(&app.version)).map_err(refused)?;
        record.digest = Some(manifest_digest(&app_dir.join(&app.version))?);
        state.carrier.start(&*state.host, &state.cx, installed(state, &record, Some(&verified))?).await
    }
    .await;
    let (placed, summary) = match started {
        Ok(running) => running.into_parts(),
        Err(e) => {
            tracing::warn!(package = %app.id, error = %e.message, "the install failed; undoing it");
            let _ = std::fs::remove_dir_all(&app_dir);
            return Err(e);
        }
    };
    record.parts = parts(placed, verified.trust.as_str(), source);
    if let Err(e) = write_one(&state.dir, &record) {
        // The record is what makes it installed; without it, take the rest back.
        let id = ext_id(&record.id)?;
        state.carrier.stop(&id, StopReason::Uninstalled).await;
        if let Ok(installed) = recorded(state, &record) {
            let _ = state.carrier.unplace(&state.cx, &installed).await;
        }
        let _ = std::fs::remove_dir_all(&app_dir);
        return Err(e);
    }
    tracing::info!(package = %record.id, version = %record.version, trust = %disclosed.trust.as_str(), source, "package installed");
    Ok(json!({
        "id": record.id,
        "version": record.version,
        "title": record.title,
        "enabled": true,
        "trust": verified.trust.as_str(),
        "source": source,
        "installed": summary,
    }))
}

/// Move an installed package to the version in `path`, and rewrite its record. The installed
/// version is put back if it cannot be.
pub(crate) async fn update(state: &State, record: &InstallRecord, path: &Path, disclosed: &Disclosure) -> Rpc<Value> {
    let id = ext_id(&record.id)?;
    let fingerprint = state.carrier.fingerprint();
    let inspected = package::inspect(path, &fingerprint).map_err(refused)?;
    if inspected.manifest.app.id != record.id {
        return Err(refused(format!("the package is `{}`, not `{}`", inspected.manifest.app.id, record.id)));
    }
    let verified = state.admission.admit_zip(path, &inspected.manifest)?;
    // Before the installed version is taken down.
    instances_named(state, &inspected.manifest)?;
    tools_named(state, &inspected.manifest)?;
    let version = inspected.manifest.app.version.clone();
    if version == record.version {
        return Err(refused(format!("`{}` {version} is the version installed", record.id)));
    }
    let app_dir = state.package_dir(&record.id);
    let old_root = app_dir.join(&record.version);
    package::inspect_dir(&old_root).map_err(|e| internal(format!("the installed `{}`: {e}", record.id)))?;
    let new_root = app_dir.join(&version);
    // What an update that did not finish may have left.
    let _ = std::fs::remove_dir_all(&new_root);
    // From here on the installed version is taken down: a failure says what became of it.
    state.carrier.stop(&id, StopReason::Updating).await;
    state.carrier.unplace(&state.cx, &recorded(state, record)?).await?;
    let mut next = InstallRecord {
        version: version.clone(),
        title: inspected.manifest.app.title.clone(),
        parts: parts(Value::Null, verified.trust.as_str(), "store"),
        // Another version: what the store said of the last one is not said of this.
        revoked: None,
        reason: None,
        ..record.clone()
    };
    let started = async {
        package::extract(path, &new_root).map_err(refused)?;
        next.digest = Some(manifest_digest(&new_root)?);
        state.carrier.start(&*state.host, &state.cx, installed(state, &next, Some(&verified))?).await
    }
    .await;
    match started {
        Ok(running) => {
            let (placed, summary) = running.into_parts();
            if !record.enabled {
                // It stays switched off, as it was.
                state.carrier.stop(&id, StopReason::Disabled).await;
            }
            if let Err(e) = std::fs::remove_dir_all(&old_root) {
                tracing::warn!(package = %record.id, error = %e, "the old version's files were not removed");
            }
            let next = InstallRecord { parts: parts(placed, verified.trust.as_str(), "store"), ..next };
            write_one(&state.dir, &next)?;
            tracing::info!(package = %next.id, from = %record.version, to = %next.version, downgrade = disclosed.downgrade, "package updated");
            Ok(json!({
                "id": next.id,
                "version": next.version,
                "from": record.version,
                "downgrade": disclosed.downgrade,
                "title": next.title,
                "enabled": next.enabled,
                "trust": verified.trust.as_str(),
                "updated": summary,
            }))
        }
        Err(e) => {
            tracing::warn!(package = %record.id, error = %e.message, "the update failed; putting the installed version back");
            let _ = std::fs::remove_dir_all(&new_root);
            let back = InstallRecord { parts: parts(Value::Null, record.trust(), "store"), ..record.clone() };
            let restored = match installed(state, &back, None) {
                Ok(old) => match state.carrier.start(&*state.host, &state.cx, old).await {
                    Ok(running) => {
                        if !record.enabled {
                            state.carrier.stop(&id, StopReason::Disabled).await;
                        }
                        Ok(running.into_parts().0)
                    }
                    Err(again) => Err(again.message),
                },
                Err(again) => Err(again.message),
            };
            if let Err(why) = &restored {
                tracing::error!(package = %record.id, error = %why, "the installed version did not come back after a failed update");
            }
            crate::records::settle_failed_update(&state.dir, record, &version, Some(restored));
            Err(e)
        }
    }
}
