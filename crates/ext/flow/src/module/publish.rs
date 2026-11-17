//! Publishing what the runner holds: the `run {run}` snapshots and the `runs` list after each
//! change the runner tells of, the `defs` list, a conversation's `focus`, and the dry runs.

use std::collections::BTreeSet;
use std::sync::Arc;

use serde_json::json;
use tokio::sync::broadcast;

use super::state::Shared;
use crate::kernel::engine::view;
use crate::kernel::run::RunQuery;
use crate::runner::Changed;

pub use harnix_flow::wire::defs_list;

/// How many runs the `runs` topic carries, newest first.
pub const RUNS_LISTED: usize = 200;

/// Publish what the runner says changed, as long as it runs: each run's newest snapshot, and the
/// list after a change that is not bookkeeping alone. Fallen behind, everything is published again.
pub(crate) async fn follow(sh: Arc<Shared>, mut changes: broadcast::Receiver<Changed>) {
    loop {
        match changes.recv().await {
            Ok(c) => publish_runs(&sh, &c.runs, c.substantive).await,
            Err(broadcast::error::RecvError::Lagged(_)) => publish_all(&sh).await,
            Err(broadcast::error::RecvError::Closed) => return,
        }
    }
}

/// The newest snapshot of each run, and the list.
pub(crate) async fn publish(sh: &Arc<Shared>, runs: &BTreeSet<String>) {
    publish_runs(sh, runs, true).await;
}

/// [`publish`]; the list only when `list` — not after bookkeeping, which changes no summary.
pub(crate) async fn publish_runs(sh: &Arc<Shared>, runs: &BTreeSet<String>, list: bool) {
    let (Some(engine), Some(cx)) = (sh.engine.get(), sh.cx.get()) else {
        return;
    };
    let _order = sh.publish.lock().await;
    let events = cx.topics();
    for run in runs {
        if let Ok(rec) = engine.record(run).await {
            if let Err(e) = events.publish("run", json!({"run": run}), view(rec)).await {
                tracing::warn!(run, error = %e, "run not published");
            }
        }
    }
    if !list {
        return;
    }
    match engine.list(RunQuery { limit: Some(RUNS_LISTED), ..Default::default() }).await {
        Ok(list) => {
            let _ = events.publish("runs", json!({}), &list).await;
        }
        Err(e) => tracing::warn!(error = %e, "runs not listed"),
    }
}

/// The list, and every run in it: what a subscriber arriving after start sees.
pub(crate) async fn publish_all(sh: &Arc<Shared>) {
    let Some(engine) = sh.engine.get() else {
        return;
    };
    let listed: BTreeSet<String> = engine
        .list(RunQuery { limit: Some(RUNS_LISTED), ..Default::default() })
        .await
        .map(|l| l.into_iter().map(|s| s.id).collect())
        .unwrap_or_default();
    publish(sh, &listed).await;
}

/// The `defs` topic, after the packs changed.
pub(crate) async fn publish_defs(sh: &Arc<Shared>) {
    let (Some(engine), Some(cx)) = (sh.engine.get(), sh.cx.get()) else {
        return;
    };
    let list = defs_list(engine.packs().as_ref());
    if let Err(e) = cx.topics().publish("defs", json!({}), &list).await {
        tracing::warn!(error = %e, "defs not published");
    }
}

/// The `focus {instance, session_id}` topic: `{flow}` (`null` when nothing is selected).
pub(crate) async fn publish_focus(sh: &Arc<Shared>, instance: &str, session_id: &str) {
    let Some(cx) = sh.cx.get() else { return };
    let flow = sh.focus(instance, session_id);
    // The same answer `flow.focus` gives: the flow, and the project the conversation works in.
    let project = sh.conversation_project(instance, session_id);
    let params = json!({"instance": instance, "session_id": session_id});
    if let Err(e) = cx.topics().publish("focus", params, json!({"flow": flow, "project": project})).await {
        tracing::warn!(error = %e, "focus not published");
    }
}

/// A dry run's runs on `run {run}` (with their previews) and the `dry_runs` list.
pub(crate) async fn publish_dry(sh: &Arc<Shared>, run: Option<&str>) {
    let (Some(sandbox), Some(cx)) = (sh.sandbox.get(), sh.cx.get()) else {
        return;
    };
    let _order = sh.publish.lock().await;
    let events = cx.topics();
    if let Some(root) = match run {
        Some(r) => sandbox.root_of(r).await,
        None => None,
    } {
        for id in sandbox.runs_of(&root).await {
            if let Ok(v) = sandbox.get(&id).await {
                if let Err(e) = events.publish("run", json!({"run": id}), v).await {
                    tracing::warn!(run = id, error = %e, "dry run not published");
                }
            }
        }
    }
    let list = sandbox.list().await;
    if let Err(e) = events.publish("dry_runs", json!({}), &list).await {
        tracing::warn!(error = %e, "dry runs not listed");
    }
}
