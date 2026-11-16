//! Admission: the router every call goes through, the audit it keeps, and what the extensions are
//! seated with when they start.

use std::path::PathBuf;
use std::sync::{Arc, OnceLock, RwLock};

use harnix_authz::Audit;
use harnix_hub::Hub;
use harnix_kernel::seat::Listening;
use harnix_router::{RouteTable, Router};
use serde_json::json;

use crate::audit_log;
use crate::pairing::DeviceBook;
use crate::gate::{Behind, Diagnostics};
use crate::host::dynamic::{DynamicHost, DynamicParts};
use crate::host::seat::{
    HostServicesRegistry, HostUploads, HubTopics, InstanceViews, ProcessView, Seats, Secrets, UiEntries,
};
use crate::kernel_rows::UiModules;

/// The router, and what is handed to the extensions.
pub(crate) struct Admission {
    pub diagnostics: Arc<Diagnostics>,
    /// The route table, deciding every call and handing it to its row: what the transport calls.
    pub router: Arc<Router>,
    /// The instances, for what the start step tells them of an extension that did not start.
    pub instances: Arc<harnix_instance::InstanceRegistry>,
    /// What an extension's context does.
    pub seats: Arc<Seats>,
    /// Where the listener is, once it is bound: told to every extension's context.
    pub listen: Arc<tokio::sync::watch::Sender<Option<Listening>>>,
    /// The host components' pipes, when anything may register extensions while the process runs.
    /// Held here for as long as the process runs: what hands them out holds them weakly.
    pub dynamic: Option<Arc<DynamicHost>>,
    /// What `harnix.ui` tells: pushed again once every extension has started.
    pub ui: Arc<UiModules>,
}

/// Who this process is, for diagnostics.
pub(super) struct Identity<'a> {
    pub name: &'a str,
    pub version: &'a str,
}

/// What the extensions' seats are made from, besides the router.
pub(super) struct Parts {
    pub ui: Arc<UiModules>,
    pub devices: Arc<DeviceBook>,
    pub credentials: Arc<dyn harnix_ext::Credentials>,
    pub state_root: PathBuf,
    /// Where projects live (`--data-dir`).
    pub projects_root: PathBuf,
    /// Whether anything may register extensions while the process runs.
    pub dynamic: bool,
    /// `[ui] runtime_modules`: whether packages' interfaces reach the page.
    pub runtime_modules: bool,
    /// The profile's `[ext."<package id>"]` sections.
    pub package_sections: std::collections::BTreeMap<String, serde_json::Value>,
    /// The components whose section turns their processes' sandbox off.
    pub unsandboxed: std::collections::BTreeSet<String>,
}

#[allow(clippy::too_many_arguments)]
pub(super) fn admit(
    hub: &Arc<Hub>,
    instances: &Arc<harnix_instance::InstanceRegistry>,
    table: RouteTable,
    grants: Arc<harnix_kernel::GrantTable>,
    behind: &Behind,
    parts: Parts,
    identity: Identity<'_>,
) -> Admission {
    let audit = audit(hub);
    let (short, _) = crate::cli::versions(identity.name, identity.version);
    let diagnostics = Arc::new(Diagnostics {
        versions: json!({
            "app": {"name": identity.name, "version": short},
            "harnix": crate::HARNIX_VERSION,
            "agix": {"version": harnix_engine::AGIX_VERSION, "protocol_version": harnix_engine::AGIX_PROTOCOL},
            "ext_api": harnix_ext::EXT_API_VERSION,
            "contribution_api": harnix_contract::CONTRIB_API_VERSION,
            "protocol": harnix_contract::PROTOCOL_VERSION,
        }),
        instances: behind.instances.clone(),
        extensions: RwLock::new(Vec::new()),
        dynamic: OnceLock::new(),
        router: OnceLock::new(),
    });
    let router = Arc::new(Router::new(table, grants.clone(), audit));
    let _ = diagnostics.router.set(Arc::downgrade(&router));
    let _ = behind.diagnostics.set(diagnostics.clone());
    let (listen, listening) = tokio::sync::watch::channel(None);
    let listen = Arc::new(listen);
    let mut dynamic = None;
    let services = Arc::new(HostServicesRegistry::new(grants.clone(), instances.clone()));
    // This process's uploads: a device's (`harnix.bulk.upload`) and each extension's.
    let uploads = Arc::new(HostUploads::new(parts.state_root.join("uploads").join(std::process::id().to_string())));
    let _ = behind.uploads.set(uploads.clone());
    let seats = Arc::new(Seats {
        dispatch: router.clone(),
        grants: grants.clone(),
        clients: hub.clone(),
        topics: Arc::new(HubTopics(hub.clone())),
        instances: Arc::new(InstanceViews::new(instances.clone())),
        process: Arc::new(ProcessView(behind.process.clone())),
        bulk: uploads.clone(),
        ui: Arc::new(UiEntries(parts.ui.clone())),
        devices: parts.devices,
        credentials: Arc::new(Secrets(parts.credentials)),
        services: services.clone(),
        listen: listening,
        projects_dir: parts.projects_root.clone(),
    });
    if parts.dynamic {
        // Every linked extension's id was registered in the table, enabled or not: one registered
        // while the process runs never takes one, or nests under one.
        let packages = Arc::new(crate::package_table::PackageTable::new(
            router.clone(),
            Arc::new(crate::package_table::HubSurfaces(hub.clone())),
            grants,
        ));
        let host = DynamicHost::new(
            packages.clone(),
            DynamicParts {
                router: router.clone(),
                instances: instances.clone(),
                hub: hub.clone(),
                seats: seats.clone(),
                state_root: parts.state_root,
                ui: parts.ui.clone(),
                runtime_modules: parts.runtime_modules,
                sections: parts.package_sections,
                unsandboxed: parts.unsandboxed,
            },
        );
        services.set(&host);
        let _ = diagnostics.dynamic.set(packages);
        dynamic = Some(host);
    }
    Admission {
        diagnostics,
        router,
        instances: instances.clone(),
        seats,
        listen,
        dynamic,
        ui: parts.ui,
    }
}

/// The audit: a ring for the diagnostics page, and a durable sink in the session timeline.
fn audit(hub: &Arc<Hub>) -> Arc<Audit> {
    let audit = Arc::new(Audit::default());
    {
        // The system space reaching across spaces: allowed, and on the record (concept §5.4). And
        // an extension's engine sending more than it may: dropped, and on the record too.
        let audit = audit.clone();
        hub.set_audit(Arc::new(move |subject, method, session, owner| {
            if let harnix_contract::Subject::Engine { .. } = subject {
                audit.record(subject, method, harnix_authz::Decision::Refuse("flooding"), Some("an extension's engine sent more than it may; dropped"), None);
                return;
            }
            tracing::info!(subject = %subject.label(), method, session, owner, "the system space acted on another space's session");
            // The durable sink files an entry in the session's own transcript, which is the
            // `main` instance's store; a session elsewhere is kept in the ring only.
            let filed = session.strip_prefix("main/").map(str::to_string);
            audit.record(subject, method, harnix_authz::Decision::Allow, Some("cross-space, from the system space"), filed);
        }));
    }
    // Decisions worth going back for land in the session timeline, as engine
    // extension entries (`session.appendEntry` on the `main` instance's link) — in order with everything else, and skippable by
    // anything that does not know what they are (concept §6.5). The ring stays for
    // the diagnostics page.
    audit.set_sink(Box::new(audit_log::TimelineAudit::new(hub.clone())));
    audit
}
