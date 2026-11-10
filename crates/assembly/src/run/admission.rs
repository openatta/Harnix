//! Admission: the router every call goes through, the audit it keeps, what the extensions are
//! seated with when they start, and the kernel a module is handed while modules are read through
//! their shim.

use std::path::PathBuf;
use std::sync::{Arc, OnceLock, RwLock};

use harnix_authz::Audit;
use harnix_hub::Hub;
use harnix_kernel::seat::Listening;
use harnix_router::{RouteTable, Router};
use serde_json::json;

use crate::audit_log;
use crate::devices::DeviceBook;
use crate::gate::{Behind, Diagnostics, ProcessKernel};
use crate::host::dynamic::DynamicHost;
use crate::host::seat::{
    HostServicesRegistry, HostUploads, HubTopics, InstanceViews, ProcessView, Seats, Secrets, UiEntries,
};
use crate::kernel_rows::UiModules;

/// The router, and what is handed to the extensions.
pub(crate) struct Admission {
    pub diagnostics: Arc<Diagnostics>,
    /// The route table, deciding every call and handing it to its row: what the transport calls.
    pub router: Arc<Router>,
    /// What a module's context reaches the process through.
    pub kernel: Arc<ProcessKernel>,
    /// What an extension's context does.
    pub seats: Arc<Seats>,
    /// Where the listener is, once it is bound: told to every extension's context.
    pub listen: Arc<tokio::sync::watch::Sender<Option<Listening>>>,
    /// The host components' pipes, when anything may register extensions while the process runs.
    /// Held here for as long as the process runs: what hands them out holds them weakly.
    #[allow(dead_code)]
    pub dynamic: Option<Arc<DynamicHost>>,
}

/// Who this process is, for diagnostics and the application host.
pub(super) struct Identity<'a> {
    pub name: &'a str,
    pub version: &'a str,
    /// The modules besides the `apps` built-in that are handed the runtime-application host.
    pub app_hosts: &'a [String],
}

/// What the extensions' seats are made from, besides the router.
pub(super) struct Parts {
    pub ui: Arc<UiModules>,
    pub devices: Arc<DeviceBook>,
    pub credentials: Arc<dyn harnix_ext::Credentials>,
    pub servers: Arc<harnix_instance::SharedServers>,
    pub state_root: PathBuf,
    /// Whether the legacy runtime-application host is on (`[apps] enabled`).
    pub apps_enabled: bool,
    /// Whether anything may register extensions while the process runs.
    pub dynamic: bool,
}

#[allow(clippy::too_many_arguments)]
pub(super) fn admit(
    hub: &Arc<Hub>,
    instances: &Arc<harnix_instance::InstanceRegistry>,
    table: RouteTable,
    grants: Arc<harnix_kernel::GrantTable>,
    behind: &Behind,
    history: Option<Arc<dyn harnix_instance::HistoryStore>>,
    parts: Parts,
    identity: Identity<'_>,
) -> Admission {
    let audit = audit(hub, history);
    let (short, _) = crate::cli::versions(identity.name, identity.version);
    let diagnostics = Arc::new(Diagnostics {
        versions: json!({
            "app": {"name": identity.name, "version": short},
            "harnix": crate::HARNIX_VERSION,
            "agix": harnix_engine::about(),
            "ext_api": harnix_ext::EXT_API_VERSION,
            "module_api": harnix_module::MODULE_API_VERSION,
            "contribution_api": harnix_contract::CONTRIB_API_VERSION,
            "protocol": harnix_contract::PROTOCOL_VERSION,
        }),
        extensions: RwLock::new(Vec::new()),
        dynamic: OnceLock::new(),
        router: OnceLock::new(),
    });
    let router = Arc::new(Router::new(table, grants.clone(), audit));
    let _ = diagnostics.router.set(Arc::downgrade(&router));
    let _ = behind.diagnostics.set(diagnostics.clone());
    let kernel = Arc::new(ProcessKernel {
        router: OnceLock::new(),
        apps: OnceLock::new(),
        app_hosts: std::iter::once(harnix_builtin::apps::MODULE_ID.to_string())
            .chain(identity.app_hosts.iter().cloned())
            .collect(),
        hub: hub.clone(),
        instances: instances.clone(),
        listen: tokio::sync::watch::channel(None).0,
    });
    let _ = kernel.router.set(router.clone());
    let (listen, listening) = tokio::sync::watch::channel(None);
    let listen = Arc::new(listen);
    let mut dynamic = None;
    let services = Arc::new(HostServicesRegistry::default());
    let seats = Arc::new(Seats {
        dispatch: router.clone(),
        grants: grants.clone(),
        clients: hub.clone(),
        topics: Arc::new(HubTopics(hub.clone())),
        instances: Arc::new(InstanceViews(instances.clone())),
        process: Arc::new(ProcessView(behind.process.clone())),
        bulk: Arc::new(HostUploads::new(
            parts.state_root.join("uploads").join(std::process::id().to_string()).join("ext"),
        )),
        ui: Arc::new(UiEntries(parts.ui)),
        devices: parts.devices,
        credentials: Arc::new(Secrets(parts.credentials)),
        services: services.clone(),
        listen: listening,
    });
    if parts.apps_enabled || parts.dynamic {
        // Every linked extension's id was registered in the table, enabled or not: one registered
        // while the process runs never takes one, or nests under one.
        let apps = Arc::new(crate::apps::Apps::new(router.clone(), Arc::new(crate::apps::HubSurfaces(hub.clone())), grants));
        if parts.apps_enabled {
            let _ = kernel.apps.set(apps.clone());
            tracing::info!("this process hosts runtime applications");
        }
        let host = DynamicHost::new(apps.clone(), router.clone(), seats.clone(), parts.servers, parts.state_root);
        services.set(&host);
        let _ = diagnostics.dynamic.set(apps);
        dynamic = Some(host);
    }
    Admission {
        diagnostics,
        router,
        kernel,
        seats,
        listen,
        dynamic,
    }
}

/// The audit: a ring for the diagnostics page, and a durable sink in the session timeline.
fn audit(hub: &Arc<Hub>, history: Option<Arc<dyn harnix_instance::HistoryStore>>) -> Arc<Audit> {
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
            // `system` instance's store; a session elsewhere is kept in the ring only.
            let filed = session.strip_prefix("system/").map(str::to_string);
            audit.record(subject, method, harnix_authz::Decision::Allow, Some("cross-space, from the system space"), filed);
        }));
    }
    // Decisions worth going back for land in the session timeline, as engine
    // extension entries — in order with everything else, and skippable by
    // anything that does not know what they are (concept §6.5). The ring stays for
    // the diagnostics page.
    if let Some(store) = history {
        audit.set_sink(Box::new(audit_log::TimelineAudit::new(harnix_engine::timeline::Timeline::new(store))));
    } else {
        tracing::warn!("no history store; audit entries live only in memory");
    }
    audit
}
