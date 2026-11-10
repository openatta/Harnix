//! Step 5: every extension still enabled seated and started, in order.

use std::sync::Arc;

use harnix_contract::sync::write;
use harnix_ext::Credentials;
use harnix_module::apps::AppRuntime;

use super::admission::Admission;
use crate::host::{lifecycle, Record};

/// Start each extension with its context. One that fails is disabled with the reason, and takes
/// what it bound and registered with it; the rest start anyway.
pub(super) async fn start(records: &mut [Record], admission: &Admission, credentials: &Arc<dyn Credentials>) {
    let Admission { router, kernel, diagnostics, seats, ui, .. } = admission;
    let failed = lifecycle::start(records, seats, kernel, credentials).await;
    for record in records.iter().filter(|r| failed.iter().any(|f| f == r.manifest.id())) {
        let id = record.manifest.id();
        // What it registered with the application host before failing goes with it: an extension
        // that is not running answers nothing, and what it registered would be names with nobody
        // behind them.
        if let Some(apps) = diagnostics.dynamic.get() {
            for app in apps.registered().into_iter().filter(|a| a.by == id) {
                if let Err(e) = apps.withdraw(id, &app.id).await {
                    tracing::warn!(extension = %id, app = %app.id, error = %e.message, "could not withdraw");
                }
            }
        }
        if record.installed {
            router.withdraw(&harnix_contract::Subject::Kernel, &record.owner(), "its extension did not start");
        }
        kernel.instances.module_disabled(id).await;
    }
    *write(&diagnostics.extensions) = records.iter().map(|r| r.status.clone()).collect();
    // Which are enabled is settled: a page that connected meanwhile is told.
    ui.changed();
}
