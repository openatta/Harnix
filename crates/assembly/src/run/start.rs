//! Step 5: every extension still enabled seated and started, in order.

use harnix_contract::sync::write;

use super::admission::Admission;
use crate::host::{lifecycle, Record};

/// Start each extension with its context. One that fails is disabled with the reason, and takes
/// what it bound and registered with it; the rest start anyway.
pub(super) async fn start(records: &mut [Record], admission: &Admission) {
    let Admission { router, diagnostics, seats, ui, instances, .. } = admission;
    let failed = lifecycle::start(records, seats).await;
    for record in records.iter().filter(|r| failed.iter().any(|f| f == r.manifest.id())) {
        let id = record.manifest.id();
        // What it registered with the extension host before failing goes with it: an extension
        // that is not running answers nothing, and what it registered would be names with nobody
        // behind them.
        if let Some(packages) = diagnostics.dynamic.get() {
            for package in packages.registered().into_iter().filter(|a| a.by == id) {
                if let Err(e) = packages.withdraw(id, &package.id).await {
                    tracing::warn!(extension = %id, package = %package.id, error = %e.message, "could not withdraw");
                }
            }
        }
        if record.installed {
            router.withdraw(&harnix_contract::Subject::Kernel, &record.owner(), "its extension did not start");
        }
        instances.module_disabled(id).await;
    }
    *write(&diagnostics.extensions) = records.iter().map(|r| r.status.clone()).collect();
    // Which are enabled is settled: a page that connected meanwhile is told.
    ui.changed();
}
