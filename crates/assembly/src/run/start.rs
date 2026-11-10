//! Step 5: every module still enabled started, in order.

use std::sync::Arc;

use harnix_contract::sync::write;
use harnix_hub::Hub;
use harnix_module::{Credentials, ModuleContext};

use super::admission::Admission;
use crate::modules::Record;

/// Start each module with its context. One that fails is disabled with the reason, and takes
/// what it bound and registered with it; the rest start anyway.
pub(super) async fn start(
    records: &mut [Record],
    admission: &Admission,
    credentials: &Arc<dyn Credentials>,
    hub: &Arc<Hub>,
) {
    let Admission {
        app_gate,
        authorizer,
        kernel,
        diagnostics,
        ..
    } = admission;
    for record in records.iter_mut() {
        if !record.is_enabled() {
            continue;
        }
        let cx = ModuleContext::new(
            &record.manifest.id,
            &record.prefix,
            record.config.clone(),
            record.data_dir.clone(),
            credentials.clone(),
            kernel.clone(),
            record.topics.clone(),
        );
        // Bound before `start`, so a module that calls its own methods while
        // starting reaches them; unbound again if it fails.
        for method in &record.methods {
            app_gate.bind(&method.name, method.handler.clone(), cx.clone());
        }
        match record.module.start(cx.clone()).await {
            Ok(()) => record.context = Some(cx),
            Err(e) => {
                // What it registered with the application host before failing goes with it: a
                // module that is not running answers nothing, and its applications would be names
                // with nobody behind them.
                if let Some(apps) = kernel.apps.get() {
                    let id = &record.manifest.id;
                    for app in apps.registered().into_iter().filter(|a| &a.by == id) {
                        if let Err(e) = apps.withdraw(id, &app.id).await {
                            tracing::warn!(module = %id, app = %app.id, error = %e.message, "could not withdraw");
                        }
                    }
                }
                app_gate.unbind(&record.declared);
                authorizer.retract(&harnix_contract::Subject::Kernel, &record.declared, "its module did not start");
                record.disabled("start", format!("{e:#}"));
                hub.module_disabled(&record.manifest.id).await;
                if !record.status.scenes.is_empty() {
                    record.status.note = Some(format!(
                        "its scenes ({}) stay in the system instance until the next start; \
                         its session customizer is no longer applied",
                        record.status.scenes.join(", ")
                    ));
                }
            }
        }
    }
    *write(&diagnostics.modules) = records.iter().map(|r| r.status.clone()).collect();
}
