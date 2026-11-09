//! Publishing what the kernel holds: the `run {run}` snapshots and the `runs` list, the `defs`
//! list, a conversation's `focus`, and the dry runs.

use std::collections::BTreeSet;
use std::sync::Arc;

use serde_json::{json, Value};

use super::shared::{Binding, Shared};
use crate::kernel::engine::view;
use crate::kernel::run::{RunQuery, RunRecord, StationStatus, TaskStatus};

/// How many runs the `runs` topic carries, newest first.
pub const RUNS_LISTED: usize = 200;

/// The newest snapshot of each run, and the list; bindings of claims that are over dropped.
pub(crate) async fn publish(sh: &Arc<Shared>, runs: &BTreeSet<String>) {
    publish_runs(sh, runs, true).await;
}

/// [`publish`]; the list only when `list` — not after bookkeeping, which changes no summary.
pub(crate) async fn publish_runs(sh: &Arc<Shared>, runs: &BTreeSet<String>, list: bool) {
    let (Some(engine), Some(cx)) = (sh.engine.get(), sh.cx.get()) else {
        return;
    };
    let _order = sh.publish.lock().await;
    let events = cx.events();
    for run in runs {
        if let Ok(rec) = engine.record(run).await {
            sh.prune_bindings(run, |b| binding_current(&rec, b));
            if let Err(e) = events.publish("run", json!({"run": run}), view(rec)).await {
                tracing::warn!(run, error = %e, "flow: run not published");
            }
        }
    }
    if !list {
        return;
    }
    match engine
        .list(RunQuery {
            limit: Some(RUNS_LISTED),
            ..Default::default()
        })
        .await
    {
        Ok(list) => {
            let _ = events.publish("runs", json!({}), &list).await;
        }
        Err(e) => tracing::warn!(error = %e, "flow: runs not listed"),
    }
}

/// Whether a binding's claim still counts: its station open, its task claimed or handed in
/// under the binding's token. A task handed in keeps its session bound so it can hand in
/// again until the station passes.
pub(crate) fn binding_current(rec: &RunRecord, b: &Binding) -> bool {
    rec.is_active()
        && rec.stations.get(&b.station).is_some_and(|st| {
            matches!(st.status, StationStatus::Working | StationStatus::Unmet)
                && st.task(&b.task).is_some_and(|t| {
                    matches!(t.status, TaskStatus::Claimed | TaskStatus::Submitted)
                        && t.lease.as_ref().is_some_and(|l| l.token == b.token)
                })
        })
}

/// The list, and every run in it: what a subscriber arriving after start sees.
pub(crate) async fn publish_all(sh: &Arc<Shared>) {
    let Some(engine) = sh.engine.get() else {
        return;
    };
    let listed: BTreeSet<String> = engine
        .list(RunQuery {
            limit: Some(RUNS_LISTED),
            ..Default::default()
        })
        .await
        .map(|l| l.into_iter().map(|s| s.id).collect())
        .unwrap_or_default();
    publish(sh, &listed).await;
}

/// The flows on offer, as the `defs` topic carries them: `[{name, title, digest, origin, valid}]`.
pub(crate) fn defs_list(packs: &dyn crate::kernel::host::Packs) -> Vec<Value> {
    packs
        .flows()
        .into_iter()
        .filter_map(|n| {
            let (def, digest) = packs.flow(&n)?;
            let valid = crate::kernel::validate::validate(&def, packs).is_empty();
            Some(json!({
                "name": n, "title": def.title, "digest": digest,
                "origin": packs.origin(&n), "valid": valid,
            }))
        })
        .collect()
}

/// The `defs` topic, after the packs changed.
pub(crate) async fn publish_defs(sh: &Arc<Shared>) {
    let (Some(engine), Some(cx)) = (sh.engine.get(), sh.cx.get()) else {
        return;
    };
    let list = defs_list(engine.packs().as_ref());
    if let Err(e) = cx.events().publish("defs", json!({}), &list).await {
        tracing::warn!(error = %e, "flow: defs not published");
    }
}

/// The `focus {instance, session_id}` topic: `{flow}` (`null` when nothing is selected).
pub(crate) async fn publish_focus(sh: &Arc<Shared>, instance: &str, session_id: &str) {
    let Some(cx) = sh.cx.get() else { return };
    let flow = sh.focus(instance, session_id);
    let params = json!({"instance": instance, "session_id": session_id});
    if let Err(e) = cx
        .events()
        .publish("focus", params, json!({"flow": flow}))
        .await
    {
        tracing::warn!(error = %e, "flow: focus not published");
    }
}

/// A dry run's runs on `run {run}` (with their previews) and the `dry_runs` list.
pub(crate) async fn publish_dry(sh: &Arc<Shared>, run: Option<&str>) {
    let (Some(sandbox), Some(cx)) = (sh.sandbox.get(), sh.cx.get()) else {
        return;
    };
    let _order = sh.publish.lock().await;
    let events = cx.events();
    if let Some(root) = match run {
        Some(r) => sandbox.root_of(r).await,
        None => None,
    } {
        for id in sandbox.runs_of(&root).await {
            if let Ok(v) = sandbox.get(&id).await {
                if let Err(e) = events.publish("run", json!({"run": id}), v).await {
                    tracing::warn!(run = id, error = %e, "flow: dry run not published");
                }
            }
        }
    }
    let list = sandbox.list().await;
    if let Err(e) = events.publish("dry_runs", json!({}), &list).await {
        tracing::warn!(error = %e, "flow: dry runs not listed");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_binding_counts_until_its_station_passes_or_its_claim_is_superseded() {
        let mut rec: RunRecord = serde_json::from_value(json!({
            "id": "r", "flow": "f", "def_digest": "d", "subject": "s", "status": "active",
            "rev": 1, "created_at": 0, "created_by": {"kind": "x", "id": "y"},
            "def": {"name": "f", "stations": [{"id": "a", "exit": [{"kind": "confirm"}]}]},
            "stations": {"a": {"status": "working", "round": 1, "tasks": [
                {"id": "t", "status": "submitted", "attempt": 1,
                 "lease": {"by": {"kind": "flow", "id": "dispatch"}, "token": "tok", "until": 9}}
            ]}}
        }))
        .unwrap();
        let b = Binding {
            instance: "system".into(),
            run: "r".into(),
            station: "a".into(),
            round: 1,
            task: "t".into(),
            token: "tok".into(),
            permissions: vec![],
        };
        assert!(
            binding_current(&rec, &b),
            "handed in, station open: it may hand in again"
        );
        let other = Binding {
            token: "old".into(),
            ..b.clone()
        };
        assert!(!binding_current(&rec, &other));
        rec.stations.get_mut("a").unwrap().status = StationStatus::Passed;
        assert!(!binding_current(&rec, &b));
    }
}
