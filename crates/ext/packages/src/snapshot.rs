//! The list: `packages.list`, and the topic of the same value.
//!
//! A look is cheap: what each package's manifest describes is kept by id and version; each one's
//! status is asked of its carrier again only when the carrier's generation moved (or every
//! [`RESYNC_EVERY`], for what a VM changes without saying); and what it keeps is what its VM
//! counted, not a walk of its directory, while it runs.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use harnix_ext::ExtId;
use serde_json::{json, Value};

use crate::records::{self, InstallRecord};
use crate::{lock, runtime, State, LIST_TOPIC, RESYNC_EVERY};

/// What the last look at the packages read, to read again only what may have changed.
#[derive(Default)]
pub(crate) struct Looked {
    /// What each version's manifest describes, by id and version: it does not change under it.
    pub described: HashMap<(String, String), Value>,
    /// The carrier's generation the statuses were read at, and when; `None`: read them again.
    pub generation: Option<u64>,
    at: Option<Instant>,
    /// Each package's status, and what it keeps, as read then.
    status: HashMap<String, (Value, Option<u64>)>,
}

/// What a package keeps, in bytes: as its VM counts it against its quota while it runs
/// (`health.storage_bytes`), measured on disk when it does not.
pub(crate) fn kept_bytes(status: &Value, run_dir: &Path) -> Option<u64> {
    status["health"]["storage_bytes"].as_u64().or_else(|| records::data_bytes(run_dir))
}

/// Every record (and the ones that do not read, with why), what the carrier says of each, and
/// every extension registered while the process runs. Each package's status (and what it keeps)
/// is read again only when `fresh`, when the carrier's generation moved since the last look, or
/// when that look is [`RESYNC_EVERY`] old; its description once per version.
pub(crate) async fn snapshot(state: &State, fresh: bool) -> Value {
    let (installed, unreadable) = records::read_all(&state.dir);
    let registered = state.host.registrar().registered();
    let generation = *state.carrier.changed().borrow();
    let sandbox = crate::sandbox(state);
    let reuse = {
        let looked = lock(&state.looked);
        !fresh && looked.generation == Some(generation) && looked.at.is_some_and(|at| at.elapsed() < RESYNC_EVERY)
    };
    let mut packages = Vec::with_capacity(installed.len());
    let mut statuses = HashMap::new();
    for r in &installed {
        let app_dir = state.app_dir(&r.id);
        let cached = if reuse { lock(&state.looked).status.get(&r.id).cloned() } else { None };
        let (status, kept) = match cached {
            Some(cached) => cached,
            None => {
                let status = match ExtId::package(&r.id) {
                    Ok(id) => state.carrier.status(&id).await,
                    Err(_) => Value::Null,
                };
                let kept = kept_bytes(&status, &state.run_dir(&r.id));
                (status, kept)
            }
        };
        let mut entry = json!({
            "id": r.id,
            "version": r.version,
            "title": r.title,
            "enabled": r.enabled,
            "registered": registered.iter().any(|s| s.id() == r.id),
            "trust": r.trust(),
            "source": r.parts.get("source").cloned().unwrap_or(json!("file")),
            "data_bytes": kept,
            "runtime": status,
            "ext": described(state, r, &app_dir),
        });
        if !sandbox.is_null() {
            entry["sandbox"] = sandbox["mode"].clone();
        }
        if let Some(why) = &r.revoked {
            entry["revoked"] = json!(why);
        }
        if let Some(why) = &r.reason {
            entry["reason"] = json!(why);
        }
        statuses.insert(r.id.clone(), (entry["runtime"].clone(), kept));
        packages.push(entry);
    }
    {
        let mut looked = lock(&state.looked);
        if !reuse {
            looked.generation = Some(generation);
            looked.at = Some(Instant::now());
        }
        looked.status = statuses;
        looked.described.retain(|(id, version), _| installed.iter().any(|r| r.id == *id && r.version == *version));
    }
    json!({
        "dir": state.dir.display().to_string(),
        "packages": packages,
        "unreadable": unreadable,
        "registered": registered,
        "runtime": runtime(state, sandbox),
    })
}

/// What an installed version's manifest describes, read once per id and version.
fn described(state: &State, record: &InstallRecord, app_dir: &Path) -> Value {
    let key = (record.id.clone(), record.version.clone());
    if let Some(described) = lock(&state.looked).described.get(&key) {
        return described.clone();
    }
    let described = harnix_package::acquire::describe(&app_dir.join(&record.version));
    // One that does not read yet (being unpacked, or broken) is asked again next time.
    if !described.is_null() {
        lock(&state.looked).described.insert(key, described.clone());
    }
    described
}

/// Push the list when it is not what was pushed last.
pub(crate) async fn push(state: &State) {
    let now = snapshot(state, false).await;
    {
        let mut pushed = lock(&state.pushed);
        if *pushed == now {
            return;
        }
        *pushed = now.clone();
    }
    if let Err(e) = state.cx.topics().publish(LIST_TOPIC, json!({}), &now).await {
        tracing::warn!(error = %e.message, "packages.list was not published");
    }
}

/// While the extension runs: the list, pushed when the carrier's generation moves, and looked at
/// again at least every [`RESYNC_EVERY`].
pub(crate) async fn watch(state: Arc<State>) {
    let mut changes = state.carrier.changed();
    loop {
        push(&state).await;
        tokio::select! {
            moved = changes.changed() => {
                // A carrier that is gone moves no more: looked at on the clock alone.
                if moved.is_err() {
                    tokio::time::sleep(RESYNC_EVERY).await;
                }
            }
            _ = tokio::time::sleep(RESYNC_EVERY) => {}
        }
    }
}
