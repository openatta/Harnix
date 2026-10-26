//! Starting a process, in the order design §7.3 gives.
//!
//! 1. Read the profile.
//! 2. **Register** every module, in order: its manifest and what it declares.
//! 3. **Migrate** every module, in order.
//! 4. **Build the instances**: `system`, with the native contributions of the
//!    modules still enabled.
//! 5. **Start** every module, in order.
//! 6. Serve.
//!
//! A module that fails at 2, 3 or 5 is disabled and everything else goes on;
//! `harnix.diagnostics` says which one and why. Every decision here is either
//! a flag or a profile line; none of them is buried.

use std::net::{IpAddr, Ipv6Addr, SocketAddr};
use std::sync::{Arc, OnceLock, RwLock};

use harnix_assembly::EngineConfig;
use harnix_authz::{Audit, Authorizer, Devices};
use harnix_builtin::Builtin;
use harnix_contract::{Gate, Topology};
use harnix_hub::{Hub, HubGate};
use harnix_module::{EnvCredentials, InstanceHandle, ModuleContext};
use serde_json::json;

use crate::cli::{self, Command, UiCommand};
use crate::gate::{AppGate, Diagnostics, ProcessKernel, Routes};
use crate::modules::{self, Entry, Namespace};
use crate::paths::{RootArgs, Roots};
use crate::{audit_log, devices, methods, AppBuilder};

pub(crate) async fn run(app: AppBuilder) -> anyhow::Result<()> {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info,harnix=debug")),
        )
        .try_init();

    let cli = cli::parse(&app.name, &app.version, app.args.clone());
    if let Some(Command::Ui(UiCommand::Export { dir })) = &cli.command {
        let count = harnix_transport::StaticFace::export(dir)?;
        println!("\n  {count} files → {}\n", dir.display());
        return Ok(());
    }

    // ── 1. Profile ──────────────────────────────────────────────────────
    let profile = cli::resolve_profile(&cli)?;
    let host: IpAddr = profile.transport.host.parse()?;
    // A non-loopback bind needs TLS, and `harnix_transport::serve` refuses it
    // without. Refused at the point of binding rather than here, so every
    // caller of that function inherits the rule instead of remembering it.
    let tls = match (&cli.tls_cert, &cli.tls_key) {
        (Some(cert), Some(key)) => Some(harnix_transport::Tls {
            cert: cert.clone(),
            key: key.clone(),
        }),
        (None, None) => None,
        _ => anyhow::bail!("--tls-cert and --tls-key go together"),
    };
    let roots = Roots::resolve(&RootArgs {
        agix_dir: cli.agix_dir.clone(),
        data_dir: cli.data_dir.clone(),
    });
    tracing::info!(
        engine = %roots.engine.display(),
        projects = %roots.projects.display(),
        state = %roots.state.display(),
        "directories"
    );

    // ── 2. Register ─────────────────────────────────────────────────────
    let paired = Arc::new(Devices::default());
    let mut entries = Vec::new();
    let builtin = if app.builtins {
        let builtin = Builtin::new(roots.state.clone(), roots.projects.clone())?;
        for module in builtin.modules() {
            entries.push(Entry {
                module: Box::new(module),
                namespace: Namespace::Kernel,
            });
        }
        // Revoking is wired to the transport's session registry, because
        // ending a device's channels needs the layer that knows there are
        // channels. Nothing above transport ever learns there were.
        let disconnect = Arc::new(|id: &str| {
            tracing::info!(device = id, "revoked; its channels are being closed");
        });
        entries.push(Entry {
            module: Box::new(devices::DevicesModule::new(paired.clone(), disconnect)),
            namespace: Namespace::Kernel,
        });
        Some(builtin)
    } else {
        None
    };
    for module in app.modules {
        entries.push(Entry {
            module,
            namespace: Namespace::App,
        });
    }
    let mut table = methods::table();
    let mut records = modules::register(entries, &mut table, &profile.modules, &roots.state);

    // ── 3. Migrate ──────────────────────────────────────────────────────
    modules::migrate(&mut records, &mut table).await;

    // ── 4. Instances ────────────────────────────────────────────────────
    let engine = harnix_assembly::build_engine(EngineConfig {
        scene: profile.engine.scene.clone(),
        scenes: profile.engine.scenes.clone(),
        model: profile.engine.model.clone(),
        max_tokens: profile.engine.max_tokens,
        session_cap: profile.engine.session_cap,
        session_idle_timeout_secs: profile.engine.session_idle_timeout_secs,
        permission_prompt_timeout_secs: profile.engine.permission_prompt_timeout_secs,
        data_root: roots.engine.clone(),
        cwd: None,
        contrib: modules::engine_contrib(&records),
    })
    .await?;
    tracing::info!(
        scenes = ?engine.active_scenes,
        model = %engine.model,
        "engine ready (in-process)"
    );
    let system = InstanceHandle {
        id: harnix_hub::SYSTEM.to_string(),
        data_root: engine.data_root.clone(),
        recordings_root: engine.recordings_root.clone(),
        history: engine.history.clone(),
        active_scenes: engine.active_scenes.clone(),
        model: engine.model.clone(),
    };
    let history = engine.history.clone();

    if let Some(dir) = &cli.replay_dir {
        tracing::warn!(dir = %dir.display(), "replaying recordings; no model will be called");
    }
    // The three ceilings, assembled where all three are known. The hub
    // reports them at hello and the transport enforces the frame one; both
    // read this rather than restating a number.
    let limits = harnix_contract::Limits {
        max_frame_bytes: harnix_contract::MAX_FRAME_BYTES,
        max_upload_bytes: harnix_contract::MAX_UPLOAD_BYTES,
        replay_max_frames: harnix_hub::REPLAY_MAX_FRAMES,
    };
    let hub = Hub::new(engine, cli.replay_dir.clone(), limits).await?;
    for record in records.iter().filter(|r| r.is_enabled()) {
        for topic in &record.topics {
            hub.declare_topic(&topic.name, topic.params.clone()).await;
        }
    }

    // ── Admission ───────────────────────────────────────────────────────
    let audit = Arc::new(Audit::default());
    // Decisions worth going back for land in the session timeline, as engine
    // extension entries — in order with everything else, and skippable by
    // anything that does not know what they are (§6.5). The ring stays for
    // the diagnostics page.
    if let Some(store) = history {
        audit.set_sink(Box::new(audit_log::TimelineAudit::new(store)));
    } else {
        tracing::warn!("no history store; audit entries live only in memory");
    }
    let (short, _) = cli::versions(&app.name, &app.version);
    let diagnostics = Arc::new(Diagnostics {
        versions: json!({
            "app": {"name": app.name, "version": short},
            "harnix": crate::HARNIX_VERSION,
            "agix": daemon::discovery::identity(),
            "module_api": harnix_module::MODULE_API_VERSION,
            "contribution_api": harnix_contract::CONTRIB_API_VERSION,
            "protocol": harnix_contract::PROTOCOL_VERSION,
        }),
        modules: RwLock::new(Vec::new()),
        unknown_sections: profile
            .modules
            .keys()
            .filter(|k| !records.iter().any(|r| &r.manifest.id == *k))
            .cloned()
            .collect(),
    });
    let app_gate = Arc::new(AppGate::new(Arc::new(HubGate(hub.clone())), diagnostics.clone()));
    let authorizer = Arc::new(Authorizer::new(table, app_gate.clone(), audit.clone()));
    let gate: Arc<dyn Gate> = authorizer.clone();
    let kernel = Arc::new(ProcessKernel {
        gate: OnceLock::new(),
        hub: hub.clone(),
        system,
    });
    let _ = kernel.gate.set(gate.clone());

    // ── 5. Start ────────────────────────────────────────────────────────
    let credentials = app
        .credentials
        .clone()
        .unwrap_or_else(|| Arc::new(EnvCredentials));
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
                app_gate.unbind(&record.declared);
                record.disable("start", format!("{e:#}"), &mut authorizer.table_mut());
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
    *diagnostics.modules.write().unwrap_or_else(|e| e.into_inner()) =
        records.iter().map(|r| r.status.clone()).collect();

    // ── 6. Serve ────────────────────────────────────────────────────────
    let token = uuid::Uuid::new_v4().simple().to_string();
    let token_file = roots.state.join("token");
    if let Err(e) = write_token(&token_file, &token) {
        tracing::warn!(error = %e, "could not write the token file");
    }

    // Which interface, said once and logged, because "I edited a file and
    // nothing changed" is the failure this choice produces when it is silent.
    let face = match (cli.headless, profile.ui.dir.clone()) {
        (true, _) => harnix_transport::Face::Headless,
        (false, Some(dir)) => harnix_transport::Face::Directory(dir),
        (false, None) => harnix_transport::Face::Embedded,
    };
    match &face {
        harnix_transport::Face::Embedded => tracing::info!("serving the interface compiled in"),
        harnix_transport::Face::Directory(dir) => {
            tracing::info!(dir = %dir.display(), "serving the interface from disk, not the one compiled in")
        }
        harnix_transport::Face::Headless => tracing::info!("headless: no interface, RPC only"),
    }

    let links = Arc::new(harnix_transport::LaunchLinks::default());
    let browser = if profile.transport.launch_link {
        harnix_transport::BrowserAdmission::LaunchLink
    } else {
        harnix_transport::BrowserAdmission::PageToken
    };
    let apps: Vec<harnix_transport::AppRoutes> = records
        .iter_mut()
        .filter(|r| r.is_enabled())
        .filter_map(|r| {
            r.route.take().map(|decl| harnix_transport::AppRoutes {
                id: r.manifest.id.clone(),
                router: decl.router,
            })
        })
        .collect();
    let bulk: Arc<dyn harnix_transport::BulkStore> = match &builtin {
        Some(builtin) => builtin.clone(),
        None => Arc::new(NoUploads),
    };
    let max_upload_bytes = builtin
        .as_ref()
        .map(|b| b.max_upload_bytes())
        .unwrap_or(harnix_contract::MAX_UPLOAD_BYTES);
    let (router, statics) = harnix_transport::router(
        gate,
        hub.clone(),
        bulk,
        paired.clone(),
        harnix_transport::Config {
            topologies: profile.transport.topologies.clone(),
            token: token.clone(),
            face: face.clone(),
            tls: tls.clone(),
            // Decided by where this listens, not by a setting. A reachable
            // listener needs a paired device; loopback does not have one to
            // ask for yet, and does not need one (§6.3).
            admission: if host.is_loopback() {
                harnix_transport::Admission::Token
            } else {
                harnix_transport::Admission::PairedDevice
            },
            max_upload_bytes,
            browser,
            links: links.clone(),
            extra_origins: profile.transport.extra_origins.clone(),
        },
        harnix_transport::Extensions {
            apps,
            route_gate: Some(Arc::new(Routes(authorizer.clone()))),
        },
    );

    // Both loopback families, or exactly what was asked for. `localhost`
    // resolves to `::1` first in every current browser, so a v4-only listener
    // is a page that will not open.
    let mut addrs = vec![SocketAddr::new(host, profile.transport.port)];
    if host.is_loopback() && host.is_ipv4() {
        addrs.push(SocketAddr::new(
            IpAddr::V6(Ipv6Addr::LOCALHOST),
            profile.transport.port,
        ));
    }
    let addr = addrs[0];
    if let Some(builtin) = &builtin {
        // What the installed packages contribute to this side, resolved once
        // before anything is served. A package installed later refreshes
        // this — one that needed a restart to appear is one nobody will
        // believe installed.
        apply_packages(&hub, &statics, builtin.contributions().await);
        let hub = hub.clone();
        let statics = statics.clone();
        builtin
            .on_packages_changed(move |contributions| {
                apply_packages(&hub, &statics, contributions);
            })
            .await;
    }

    // Before anything is served: a reachable node with no paired device has
    // no way to acquire one, because pairing is a method and methods need
    // admission. This is where that circle is broken.
    devices::bootstrap(&paired, !host.is_loopback());

    let server = tokio::spawn(async move { harnix_transport::serve(&addrs, router, tls).await });

    let scheme = if cli.tls_cert.is_some() { "https" } else { "http" };
    println!(
        "\n  {} → {scheme}://{addr}/\n  token → {}\n  topology → {}\n",
        app.name,
        token_file.display(),
        profile
            .transport
            .topologies
            .iter()
            .map(Topology::as_str)
            .collect::<Vec<_>>()
            .join(", ")
    );
    for record in records.iter().filter(|r| !r.is_enabled()) {
        println!(
            "  module `{}` disabled at {}: {}\n",
            record.manifest.id,
            record.status.phase.unwrap_or("?"),
            record.status.reason.clone().unwrap_or_default()
        );
    }
    if browser == harnix_transport::BrowserAdmission::LaunchLink {
        // One link for whoever is reading this console; it is single-use and
        // short-lived, so scrollback is not a leak.
        let code = links.mint(std::time::SystemTime::now());
        println!(
            "  open → {scheme}://{addr}/auth/{code}  (one use, {}s)\n",
            harnix_transport::LINK_TTL.as_secs()
        );
    }

    tokio::select! {
        result = server => { result??; }
        _ = tokio::signal::ctrl_c() => println!("shutting down…"),
    }

    // Modules stop in reverse: a later module may depend on an earlier one.
    for record in records.iter().rev().filter(|r| r.is_enabled()) {
        record.module.stop().await;
    }
    hub.shutdown().await;
    // Uploads are this process's scratch; the rest of the state root outlives
    // it.
    let _ = std::fs::remove_dir_all(
        roots
            .state
            .join("uploads")
            .join(std::process::id().to_string()),
    );
    Ok(())
}

/// Write the token readable by its owner only (IPDrun N-302).
///
/// The token admits a client to everything a person at this machine can do;
/// with launch links it is also what mints them. The default umask leaves a
/// fresh file readable by every account. An existing file is narrowed too,
/// since `create` does not change the mode of a file that is already there.
fn write_token(path: &std::path::Path, token: &str) -> std::io::Result<()> {
    use std::io::Write as _;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _};
        options.mode(0o600);
        if path.exists() {
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
        }
    }
    options.open(path)?.write_all(token.as_bytes())
}

/// The bulk store of an application built without the built-ins: nothing
/// was ever granted, so nothing is claimed.
struct NoUploads;

#[async_trait::async_trait]
impl harnix_transport::BulkStore for NoUploads {
    async fn claim(&self, _token: &str) -> Option<(std::path::PathBuf, usize)> {
        None
    }
}

/// Serve what these packages contribute, and tell clients about it.
///
/// Spawned rather than awaited, because this is called from a callback that
/// cannot be async — the alternative was making the whole notification path
/// async for the sake of two assignments.
fn apply_packages(
    hub: &Arc<Hub>,
    statics: &Arc<harnix_transport::StaticFace>,
    contributions: Vec<harnix_builtin::packages::Contributions>,
) {
    if !contributions.is_empty() {
        tracing::info!(
            packages = contributions.len(),
            ui = contributions.iter().map(|c| c.ui.len()).sum::<usize>(),
            "packages contribute to the interface"
        );
    }
    statics.set_packages(
        contributions
            .iter()
            .filter(|c| c.enabled)
            .map(|c| (c.plugin.clone(), c.root.clone()))
            .collect(),
    );
    let modules: Vec<serde_json::Value> = contributions
        .iter()
        .filter(|c| c.enabled)
        .flat_map(|c| {
            c.ui.iter().map(|entry| {
                json!({
                    "plugin": c.plugin,
                    "point": entry.point,
                    "module": format!("/plugins/{}/ui/{}", c.plugin, entry.module),
                })
            })
        })
        .collect();
    let hub = hub.clone();
    tokio::spawn(async move {
        hub.set_ui_contributions(json!(modules)).await;
    });
}
