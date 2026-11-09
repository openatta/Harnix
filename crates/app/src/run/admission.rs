//! Admission: the one gate every call goes through, the audit it keeps, and the kernel the
//! modules are handed when they start.

use std::collections::BTreeMap;
use std::sync::{Arc, OnceLock, RwLock};

use harnix_authz::{Audit, Authorizer, MethodTable};
use harnix_contract::Gate;
use harnix_hub::{Hub, HubGate};
use serde_json::json;

use crate::audit_log;
use crate::gate::{AppGate, Diagnostics, ProcessKernel};
use crate::modules::Record;

/// The gate, in the layers it is made of, and what is handed to the modules.
pub(super) struct Admission {
    pub diagnostics: Arc<Diagnostics>,
    /// The modules' own methods, bound as each one starts.
    pub app_gate: Arc<AppGate>,
    /// The authorization table, deciding before anything is called.
    pub authorizer: Arc<Authorizer>,
    /// What the transport calls: the authorizer.
    pub gate: Arc<dyn Gate>,
    pub kernel: Arc<ProcessKernel>,
}

/// Who this process is, for diagnostics and the application host.
pub(super) struct Identity<'a> {
    pub name: &'a str,
    pub version: &'a str,
    /// The modules besides the `apps` built-in that are handed the runtime-application host.
    pub app_hosts: &'a [String],
}

pub(super) fn admit(
    hub: &Arc<Hub>,
    table: MethodTable,
    records: &[Record],
    history: Option<Arc<dyn history::store::HistoryStore>>,
    sections: &BTreeMap<String, toml::Value>,
    apps_enabled: bool,
    identity: Identity<'_>,
) -> Admission {
    let audit = audit(hub, history);
    let (short, _) = crate::cli::versions(identity.name, identity.version);
    let diagnostics = Arc::new(Diagnostics {
        versions: json!({
            "app": {"name": identity.name, "version": short},
            "harnix": crate::HARNIX_VERSION,
            "agix": daemon::discovery::identity(),
            "module_api": harnix_module::MODULE_API_VERSION,
            "contribution_api": harnix_contract::CONTRIB_API_VERSION,
            "protocol": harnix_contract::PROTOCOL_VERSION,
        }),
        modules: RwLock::new(Vec::new()),
        unknown_sections: unknown_sections(sections, records),
    });
    let app_gate = Arc::new(AppGate::new(
        Arc::new(HubGate(hub.clone())),
        diagnostics.clone(),
        audit.clone(),
    ));
    let authorizer = Arc::new(Authorizer::new(table, app_gate.clone(), audit.clone()));
    let gate: Arc<dyn Gate> = authorizer.clone();
    let kernel = Arc::new(ProcessKernel {
        gate: OnceLock::new(),
        apps: OnceLock::new(),
        app_hosts: std::iter::once(harnix_builtin::apps::MODULE_ID.to_string())
            .chain(identity.app_hosts.iter().cloned())
            .collect(),
        hub: hub.clone(),
        listen: tokio::sync::watch::channel(None).0,
    });
    let _ = kernel.gate.set(gate.clone());
    if apps_enabled {
        // Every compiled-in module's id is reserved, enabled or not: a runtime application never
        // takes one, or nests under one.
        let apps = crate::apps::Apps::new(
            authorizer.clone(),
            app_gate.clone(),
            Arc::new(crate::apps::HubSurfaces(hub.clone())),
            records.iter().map(|r| r.manifest.id.clone()),
        );
        let _ = kernel.apps.set(Arc::new(apps));
        tracing::info!("[apps] enabled: this process hosts runtime applications");
    }
    Admission {
        diagnostics,
        app_gate,
        authorizer,
        gate,
        kernel,
    }
}

/// The audit: a ring for the diagnostics page, and a durable sink in the session timeline.
fn audit(hub: &Arc<Hub>, history: Option<Arc<dyn history::store::HistoryStore>>) -> Arc<Audit> {
    let audit = Arc::new(Audit::default());
    {
        // The system space reaching across spaces: allowed, and on the record (D17).
        let audit = audit.clone();
        hub.set_audit(Arc::new(move |subject, method, session, owner| {
            tracing::info!(subject = %subject.label(), method, session, owner, "the system space acted on another space's session");
            // The durable sink files an entry in the session's own transcript, which is the
            // `system` instance's store; a session elsewhere is kept in the ring only.
            let filed = session.strip_prefix("system/").map(str::to_string);
            audit.record(subject, method, harnix_authz::Decision::Allow, Some("cross-space, from the system space"), filed);
        }));
    }
    // Decisions worth going back for land in the session timeline, as engine
    // extension entries — in order with everything else, and skippable by
    // anything that does not know what they are (§6.5). The ring stays for
    // the diagnostics page.
    if let Some(store) = history {
        audit.set_sink(Box::new(audit_log::TimelineAudit::new(store)));
    } else {
        tracing::warn!("no history store; audit entries live only in memory");
    }
    audit
}

/// The profile's `[modules.<id>]` sections that no module here reads: a typo, or a module this
/// build does not have.
fn unknown_sections(sections: &BTreeMap<String, toml::Value>, records: &[Record]) -> Vec<String> {
    sections
        .keys()
        .filter(|k| !records.iter().any(|r| &r.manifest.id == *k))
        .cloned()
        .collect()
}
