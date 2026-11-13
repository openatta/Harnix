//! The methods, each a function of the extension's state and its params.

use std::collections::BTreeMap;
use std::path::PathBuf;

use harnix_ext::{Call, Rpc, RpcError};
use harnix_host_api::StopReason;
use serde_json::{json, Value};

use crate::records::{self, internal, read_one, refuse_installed, write_one, STAGING};
use crate::{changed, id_param, snapshot, starting, State};

/// Every method, and what it does.
pub(crate) const METHODS: &[(&str, &str)] = &[
    ("list", "every installed package, what its carrier says of it, and every extension registered while the process runs"),
    ("enable", "switch a package on and start it"),
    ("disable", "stop a package and switch it off"),
    ("installFile", "install a local .hxext: a signed one, or under dev an unsigned one"),
    ("uninstall", "stop a package, take back what it put in place, delete its directory"),
    ("catalog", "what the store offers this Harnix, and what of it is installed"),
    ("install", "install from the store: disclosed first, installed on consent to that digest"),
    ("update", "move a package to another of the store's versions, keeping its data"),
    ("clearData", "delete what a package keeps, restarting it from nothing if it runs"),
];

/// Answer `name`. A method that may have changed the list pushes it.
pub(crate) async fn answer(name: &'static str, call: Call, params: Value) -> Rpc<Value> {
    let state = call.state::<State>()?;
    let id = params.get("id").and_then(Value::as_str).map(str::to_string);
    let answer = match name {
        "list" => return Ok(snapshot::snapshot(&state, true).await),
        "catalog" => return catalog(&state).await,
        "enable" => switch(&state, params, true).await,
        "disable" => switch(&state, params, false).await,
        "installFile" => install_file(&state, params).await,
        "uninstall" => uninstall(&state, params).await,
        "install" => from_store(&state, params, false).await,
        "update" => from_store(&state, params, true).await,
        "clearData" => clear(&state, params).await,
        other => Err(RpcError::not_found(format!("`packages.{other}` is not a method"))),
    };
    changed(&state, id.as_deref()).await;
    answer
}

/// `packages.installFile {path}`.
async fn install_file(state: &State, params: Value) -> Rpc<Value> {
    let path = params
        .get("path")
        .and_then(Value::as_str)
        .map(PathBuf::from)
        .ok_or_else(|| RpcError::invalid_params("missing path"))?;
    let _serial = state.serial.lock().await;
    let disclosed = state.admission.disclose(&path, None, &state.carrier.fingerprint())?;
    if !disclosed.trust.signed() && !state.dev {
        return Err(RpcError::refused(
            "the package is unsigned, and installing an unsigned package from a file needs \
             `[ext.packages] dev = true`: that is for developing a package, not for installing one",
        ));
    }
    refuse_installed(&state.dir, &disclosed.id)?;
    starting::install(state, &path, &disclosed, "file").await
}

/// `packages.catalog`.
async fn catalog(state: &State) -> Rpc<Value> {
    let (installed, _) = records::read_all(&state.dir);
    let versions: BTreeMap<String, String> = installed.iter().map(|r| (r.id.clone(), r.version.clone())).collect();
    let catalog = harnix_package::acquire::catalog(
        state.store.as_deref(),
        &state.admission.keys,
        &state.carrier.fingerprint(),
        &versions,
    )
    .await?;
    // What the store says of a version installed here stays with its record: the list shows it
    // without asking the store.
    let _serial = state.serial.lock().await;
    for app in catalog["packages"].as_array().into_iter().flatten() {
        let (Some(id), Some(version)) = (app["id"].as_str(), app["installed"].as_str()) else {
            continue;
        };
        let revoked = app["installed_revoked"].as_str().map(str::to_string);
        match read_one(&state.dir, id) {
            Ok(mut record) if record.version == version && record.revoked != revoked => {
                if let Some(why) = &revoked {
                    tracing::warn!(package = %id, %version, reason = %why, "the store has withdrawn an installed version");
                }
                record.revoked = revoked;
                write_one(&state.dir, &record)?;
            }
            _ => {}
        }
    }
    Ok(catalog)
}

/// `packages.install` and, `updating`, `packages.update`.
async fn from_store(state: &State, params: Value, updating: bool) -> Rpc<Value> {
    let id = id_param(&params)?;
    if !records::is_package_id(&id) {
        return Err(RpcError::invalid_params(format!("`{id}` is not a package id")));
    }
    let version = params.get("version").and_then(Value::as_str);
    let consent = params.get("consent").and_then(Value::as_str);
    let allow_downgrade = params.get("allow_downgrade").and_then(Value::as_bool) == Some(true);
    let _serial = state.serial.lock().await;
    let installed = read_one(&state.dir, &id).ok();
    match (&installed, updating) {
        (Some(record), false) => {
            return Err(RpcError::refused(format!(
                "`{id}` {} is installed already; `packages.update` moves it to another version",
                record.version
            )))
        }
        (None, true) => return Err(RpcError::invalid_params(format!("no package `{id}` is installed"))),
        _ => {}
    }
    if installed.is_none() && state.app_dir(&id).exists() {
        refuse_installed(&state.dir, &id)?;
    }
    let staging = state.dir.join(STAGING).join(uuid::Uuid::new_v4().simple().to_string());
    let fingerprint = state.carrier.fingerprint();
    let outcome = async {
        let package = harnix_package::acquire::fetch(
            state.store.as_deref(),
            &state.admission.keys,
            &id,
            version,
            &fingerprint,
            &staging,
        )
        .await?;
        let app_dir = state.app_dir(&id);
        let disclosed = state.admission.disclose(
            &package,
            installed.as_ref().map(|r| (r.version.as_str(), app_dir.as_path())),
            &fingerprint,
        )?;
        if disclosed.id != id {
            return Err(RpcError::refused(format!("the store's package for `{id}` is the package `{}`", disclosed.id)));
        }
        if let Some(record) = &installed {
            // Without a version asked for, an update goes forward or nowhere.
            let behind = version.is_none() && disclosed.downgrade;
            if record.version == disclosed.version || behind {
                return Ok(json!({"id": id, "version": record.version, "latest": disclosed.version, "up_to_date": true}));
            }
            // Back to an older version only when that is what was asked for: whoever serves the
            // store cannot make an update one.
            if disclosed.downgrade && !allow_downgrade {
                return Err(RpcError::refused(format!(
                    "`{id}` {} is older than the installed {}: going back to it is asked for with \
                     `allow_downgrade: true`",
                    disclosed.version, record.version
                )));
            }
        }
        let consented = consent == Some(disclosed.digest.as_str());
        if disclosed.needs_consent && !consented {
            // Disclosed, and nothing done: the same call with the digest goes ahead.
            return Ok(json!({
                "id": id,
                "version": disclosed.version,
                "consent": "required",
                "digest": disclosed.digest,
                "disclosure": disclosed.disclosure,
                // A digest that was passed and is not this one was consent to something else.
                "changed": consent.is_some(),
            }));
        }
        match &installed {
            None => starting::install(state, &package, &disclosed, "store").await,
            Some(record) => starting::update(state, record, &package, &disclosed).await,
        }
    }
    .await;
    let _ = std::fs::remove_dir_all(&staging);
    outcome
}

/// `packages.enable` and, not `enabled`, `packages.disable`.
async fn switch(state: &State, params: Value, enabled: bool) -> Rpc<Value> {
    let id = id_param(&params)?;
    let _serial = state.serial.lock().await;
    let mut record = read_one(&state.dir, &id)?;
    record.enabled = enabled;
    write_one(&state.dir, &record)?;
    let ext = harnix_ext::ExtId::package(&id).map_err(|e| RpcError::invalid_params(e.to_string()))?;
    let mut started = Value::Null;
    if enabled {
        match starting::start(state, &record).await {
            Ok(summary) => started = summary,
            Err(e) => {
                // What was asked for did not happen: it stays as it was.
                record.enabled = false;
                write_one(&state.dir, &record)?;
                return Err(e);
            }
        }
    } else {
        state.carrier.stop(&ext, StopReason::Disabled).await;
    }
    let registered = state.host.registrar().registered().iter().any(|s| s.id() == id);
    Ok(json!({
        "id": id,
        "enabled": enabled,
        "registered": registered,
        "runtime": state.carrier.status(&ext).await,
        "started": started,
    }))
}

/// `packages.clearData {id}`.
async fn clear(state: &State, params: Value) -> Rpc<Value> {
    let id = id_param(&params)?;
    let _serial = state.serial.lock().await;
    let record = read_one(&state.dir, &id)?;
    let ext = harnix_ext::ExtId::package(&id).map_err(|e| RpcError::invalid_params(e.to_string()))?;
    let run_dir = state.run_dir(&id);
    let status = state.carrier.status(&ext).await;
    // What the list said it keeps, counted the same way.
    let before = snapshot::kept_bytes(&status, &run_dir);
    // Stopped around it: a backend's memory, or a store it has open, would outlive the files
    // otherwise.
    let running = record.enabled && status["state"] != json!("stopped");
    if running {
        state.carrier.stop(&ext, StopReason::Disabled).await;
    }
    let cleared = records::clear_data(&run_dir);
    let mut started = Value::Null;
    if running {
        started = match starting::start(state, &record).await {
            Ok(summary) => summary,
            // Cleared, and not running: said, and the list says why.
            Err(e) => json!({"error": e.message}),
        };
    }
    cleared.map_err(|e| internal(format!("{}: {e}", run_dir.display())))?;
    let after = snapshot::kept_bytes(&state.carrier.status(&ext).await, &run_dir);
    tracing::info!(package = %id, bytes = ?before, "package data cleared");
    Ok(json!({"id": id, "cleared_bytes": before, "data_bytes": after, "restarted": running, "started": started}))
}

/// `packages.uninstall {id}`.
async fn uninstall(state: &State, params: Value) -> Rpc<Value> {
    let id = id_param(&params)?;
    let _serial = state.serial.lock().await;
    let record = read_one(&state.dir, &id)?;
    let ext = harnix_ext::ExtId::package(&id).map_err(|e| RpcError::invalid_params(e.to_string()))?;
    state.carrier.stop(&ext, StopReason::Uninstalled).await;
    let removed = state.carrier.unplace(&state.cx, &starting::recorded(state, &record)?).await?;
    let app_dir = state.app_dir(&id);
    std::fs::remove_dir_all(&app_dir).map_err(|e| internal(format!("{}: {e}", app_dir.display())))?;
    tracing::info!(package = %id, "package uninstalled");
    Ok(json!({"id": id, "uninstalled": true, "removed": removed}))
}
