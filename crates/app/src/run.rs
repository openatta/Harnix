//! Starting a process, in the order design §7.3 gives.
//!
//! 1. Read the profile.
//! 2. **Register** every module, in order: its manifest and what it declares.
//! 3. **Migrate** every module, in order.
//! 4. **Build the instances**: `system`, with the native contributions of the
//!    modules still enabled; the profile's `[[instances]]`; and what the
//!    ledger says was built from templates.
//! 5. **Start** every module, in order.
//! 6. Serve.
//!
//! A module that fails at 2, 3 or 5 is disabled and everything else goes on;
//! `harnix.diagnostics` says which one and why. Every decision here is either
//! a flag or a profile line; none of them is buried.

use std::net::{IpAddr, Ipv6Addr, SocketAddr};
use std::sync::{Arc, OnceLock, RwLock};

use std::collections::BTreeMap;

use harnix_authz::{Audit, Authorizer, Devices};
use harnix_builtin::Builtin;
use harnix_contract::sync::{lock, write};
use harnix_contract::{Gate, Topology};
use harnix_hub::{Hub, HubConfig, HubGate, Registry};
use harnix_instance::link::{now_ms, DownLink, EngineLink};
use harnix_instance::{Built, EngineConfig, Factory, Facts, InProcessLink, Ledger, Owner};
use harnix_module::{Credentials, EnvCredentials, ModuleContext};
use serde_json::json;

use crate::cli::{self, Command, UiCommand};
use crate::gate::{AppGate, Diagnostics, ProcessKernel, Routes};
use crate::modules::{self, Entry, Namespace};
use crate::paths::{RootArgs, Roots};
use crate::{audit_log, devices, methods, AppBuilder};

/// The topic a running application's tool calls report their progress on (`host.progress`).
#[cfg(feature = "apps")]
const APPS_PROGRESS_TOPIC: &str = "harnix.apps.progress";

pub(crate) async fn run(app: AppBuilder) -> anyhow::Result<()> {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info,harnix=debug")),
        )
        .try_init();

    let cli = cli::parse(&app.name, &app.version, app.args.clone());
    match &cli.command {
        Some(Command::Ui(UiCommand::Export { dir })) => {
            let count = harnix_transport::StaticFace::export(dir)?;
            println!("\n  {count} files → {}\n", dir.display());
            return Ok(());
        }
        Some(Command::Daemon(daemon)) => {
            let contrib = match &daemon.template {
                None => harnix_instance::EngineContrib::default(),
                Some(id) => match modules::native_of(&app.modules, id) {
                    Ok(contrib) => contrib,
                    Err(reason) => harnix_instance::refuse_daemon(&daemon.socket, &reason),
                },
            };
            return harnix_instance::serve_daemon_with(daemon.args(), contrib).await;
        }
        None => {}
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
        product_id: profile.brand.product_id.clone(),
    });
    tracing::info!(
        engine = %roots.engine.display(),
        projects = %roots.projects.display(),
        state = %roots.state.display(),
        "directories"
    );
    // Before anything in the state directory is read: a second process on it is refused here,
    // with who has it, rather than starting and overwriting the first one's token and ledger.
    let state_lock = Arc::new(crate::state_lock::StateLock::acquire(&roots.state)?);

    // The runtime applications run in, when this build has one and the profile asks for it.
    // Made before anything is registered: the `apps` module drives it, and the instances built
    // below carry its plugin.
    #[cfg(feature = "apps")]
    let app_runtime = if profile.apps.enabled && app.builtins {
        Some(harnix_apps::Runtime::new(harnix_apps::Options {
            dev: profile.apps.dev,
            config: profile.apps.config.clone(),
            secrets: profile.apps.secrets.clone(),
            keys: signing_keys(&profile.apps, app.store_keys.as_deref())?,
            store: profile.apps.store.clone(),
            idle: app.app_idle,
        })?)
    } else {
        None
    };

    // ── 2. Register ─────────────────────────────────────────────────────
    // The devices paired before, read back: pairing carries trust across once, not once per start.
    let paired = Arc::new(load_devices(&roots.state.join(DEVICES_FILE))?);
    // The transport's channels, once it exists (step 6): what revoking a device ends.
    let channels: Arc<OnceLock<harnix_transport::Channels>> = Arc::new(OnceLock::new());
    let mut entries = Vec::new();
    let builtin = if app.builtins {
        let builtin = Builtin::new(roots.state.clone(), roots.projects.clone())?;
        for module in builtin.modules_with(
            profile.shell.keeps(harnix_contract::SYSTEM_SPACE),
            profile.shell.model_switch,
        ) {
            entries.push(Entry {
                module: Box::new(module),
                namespace: Namespace::Kernel,
            });
        }
        // Runtime applications' install records and switches: only when this process hosts them.
        if profile.apps.enabled {
            #[allow(unused_mut)]
            let mut module =
                harnix_builtin::apps::AppsModule::new(roots.state.join("apps")).dev(profile.apps.dev);
            #[cfg(feature = "apps")]
            if let Some(runtime) = &app_runtime {
                module = module.runtime(runtime.clone());
            }
            entries.push(Entry {
                module: Box::new(module),
                namespace: Namespace::Kernel,
            });
        }
        // Revoking is wired to the transport's session registry, because
        // ending a device's channels needs the layer that knows there are
        // channels. Nothing above transport ever learns there were.
        let disconnect = {
            let channels = channels.clone();
            Arc::new(move |id: &str| {
                let ended = channels.get().map_or(0, |c| c.revoke_device(id));
                tracing::info!(device = id, channels = ended, "revoked; its channels are closed");
            })
        };
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
    if profile.apps.enabled {
        // What changes the list of spaces after assembly is a runtime application; only then is
        // there anything to watch.
        table = table.allow(&harnix_authz::topic(harnix_hub::SPACES_TOPIC), harnix_authz::Reach::Device);
    }
    // A running application's tool calls, step by step, as their components report them.
    #[cfg(feature = "apps")]
    if app_runtime.is_some() {
        table = table.allow(&harnix_authz::topic(APPS_PROGRESS_TOPIC), harnix_authz::Reach::Device);
    }
    let mut records = modules::register(entries, &mut table, &profile.modules, &roots.state);

    // ── 3. Migrate ──────────────────────────────────────────────────────
    modules::migrate(&mut records, &mut table).await;

    // ── 4. Instances ────────────────────────────────────────────────────
    let credentials: Arc<dyn Credentials> = app
        .credentials
        .clone()
        .unwrap_or_else(|| Arc::new(EnvCredentials));
    let resolve = {
        let credentials = credentials.clone();
        Arc::new(move |name: &str| credentials.get(name).ok().map(|s| s.expose().to_string()))
    };
    #[allow(unused_mut)]
    let mut factory = Factory::new(resolve)?;
    // Every engine built in this process gives its sessions the running applications' tools; a
    // spawned daemon reaches them over MCP instead (`set_shared_servers`, below).
    #[cfg(feature = "apps")]
    let apps_plugin = app_runtime.as_ref().map(|r| r.plugin());
    #[cfg(feature = "apps")]
    if let Some(plugin) = &apps_plugin {
        factory.plugins.push(plugin.clone());
        factory.admitted_tools.push(harnix_apps::TOOL_PATTERN.to_string());
    }
    let instances_dir = roots.state.join("instances");
    let mut registry = Registry {
        templates: BTreeMap::new(),
        ledger: Ledger::load(roots.state.join("instances.json"))?,
        factory,
        instances_dir: instances_dir.clone(),
        disabled: records
            .iter()
            .filter(|r| !r.is_enabled())
            .map(|r| r.manifest.id.clone())
            .collect(),
        fixed: BTreeMap::new(),
    };
    for decl in &profile.instance_templates {
        let template = decl.template()?;
        registry
            .add_template(template, Owner::Kernel)
            .map_err(anyhow::Error::msg)?;
    }
    // A disabled module's templates are registered too, so what was built
    // from them is listed as down because of that module rather than as
    // built from a template that does not exist.
    for record in records.iter_mut() {
        for template in record.templates.clone() {
            if let Err(reason) =
                registry.add_template(template, Owner::Module(record.manifest.id.clone()))
            {
                if record.is_enabled() {
                    record.disable("registration", reason, &mut table);
                }
                break;
            }
        }
    }

    let engine = harnix_instance::build_engine(EngineConfig {
        scene: profile.engine.scene.clone(),
        scenes: profile.engine.scenes.clone(),
        model: profile.engine.model.clone(),
        max_tokens: profile.engine.max_tokens,
        session_cap: profile.engine.session_cap,
        session_idle_timeout_secs: profile.engine.session_idle_timeout_secs,
        permission_prompt_timeout_secs: profile.engine.permission_prompt_timeout_secs,
        data_root: roots.engine.clone(),
        cwd: None,
        contrib: {
            #[allow(unused_mut)]
            let mut contrib = modules::engine_contrib(&records);
            #[cfg(feature = "apps")]
            if let Some(plugin) = &apps_plugin {
                contrib.plugin(plugin.clone()).admit_tools(harnix_apps::TOOL_PATTERN);
            }
            contrib
        },
    })
    .await?;
    tracing::info!(
        scenes = ?engine.active_scenes,
        model = %engine.model,
        "engine ready (in-process)"
    );
    let system_facts = Facts {
        data_root: engine.data_root.clone(),
        recordings_root: Some(engine.recordings_root.clone()),
        history: engine.history.clone(),
        active_scenes: engine.active_scenes.clone(),
        scene: Some(engine.scene.clone()),
        model: Some(engine.model.clone()),
        has_credentials: engine.has_credentials,
    };
    let history = engine.history.clone();
    let system_link: Arc<dyn EngineLink> = InProcessLink::start(engine).await;

    // The profile's other instances. One that cannot be built is listed
    // down with the reason, and the rest start: an instance is not the
    // process.
    let mut fixed = Vec::new();
    for decl in &profile.instances {
        if decl.id == harnix_hub::SYSTEM {
            anyhow::bail!(
                "[[instances]] cannot be called `system`: that is the profile's [engine]"
            );
        }
        harnix_instance::templates::check_value("[[instances]] id", &decl.id)
            .map_err(anyhow::Error::msg)?;
        let root = instances_dir.join(decl.data_root.clone().unwrap_or_else(|| decl.id.clone()));
        let built = match decl.carrier() {
            Ok(carrier) => {
                registry
                    .fixed
                    .insert(decl.id.clone(), (carrier.clone(), root.clone()));
                match registry.factory.build(&decl.id, &carrier, root).await {
                    Ok(built) => built,
                    Err(e) => down(carrier.carrier(), format!("{e:#}")),
                }
            }
            Err(e) => down(harnix_instance::Carrier::InProcess, format!("{e:#}")),
        };
        fixed.push((decl.id.clone(), built));
    }

    if let Some(dir) = &cli.replay_dir {
        tracing::warn!(dir = %dir.display(), strict = cli.replay_strict, "replaying recordings; no model will be called");
    }
    // The three ceilings, assembled where all three are known. The hub
    // reports them at hello and the transport enforces the frame one; both
    // read this rather than restating a number.
    let limits = harnix_contract::Limits {
        max_frame_bytes: harnix_contract::MAX_FRAME_BYTES,
        max_upload_bytes: harnix_contract::MAX_UPLOAD_BYTES,
        replay_max_frames: harnix_hub::REPLAY_MAX_FRAMES,
        max_in_flight: harnix_contract::MAX_IN_FLIGHT,
    };
    // The spaces the enabled modules are the backend of. A space id taken twice keeps the first.
    let mut declared: Vec<harnix_module::SpaceDecl> = Vec::new();
    for record in records.iter().filter(|r| r.is_enabled()) {
        for space in &record.spaces {
            let reserved = [
                harnix_contract::SESSIONS_SPACE,
                harnix_contract::PROJECTS_SPACE,
                harnix_contract::SYSTEM_SPACE,
            ];
            if reserved.contains(&space.id.as_str()) || declared.iter().any(|d| d.id == space.id) {
                tracing::warn!(module = %record.manifest.id, space = %space.id, "a space with this id already exists; ignored");
                continue;
            }
            declared.push(space.clone());
        }
    }
    let spaces = harnix_hub::Spaces::load(
        roots.state.join("spaces.json"),
        declared,
        profile.shell.keeps(harnix_contract::SYSTEM_SPACE),
    )?;
    let hub = Hub::new(HubConfig {
        replay_from: cli.replay_dir.clone(),
        limits,
        prompt_timeout_secs: profile.engine.permission_prompt_timeout_secs,
        registry,
        spaces,
    });
    hub.set_replay_strict(cli.replay_strict);
    // What the shell is told at hello: the brand, which built-in spaces this product keeps,
    // whether installed packages' interface modules load (design §10.5), and whether it starts new
    // sessions at all (`new_session`, ipdrun-46).
    let brand_icon = profile.brand.icon.as_ref().map(|icon| match &cli.profile {
        Some(path) => path
            .parent()
            .unwrap_or(std::path::Path::new("."))
            .join(icon),
        None => icon.clone(),
    });
    hub.set_shell(json!({
        "spaces": profile.shell.spaces,
        "brand": {
            "name": profile.brand.name,
            "icon": brand_icon.as_ref().map(|_| harnix_transport::BRAND_ICON_PATH),
            "product_id": profile.brand.product_id.as_deref().unwrap_or("harnix"),
        },
        "runtime_modules": profile.ui.runtime_modules,
        "new_session": profile.shell.new_session,
        "narrow_details": profile.shell.narrow_details.as_str(),
        "state_dir": roots.state.display().to_string(),
        "projects_dir": roots.projects.display().to_string(),
        // Whose process this is: the avatar's name. One process, one person (design §2).
        "user": std::env::var("USER").or_else(|_| std::env::var("USERNAME")).ok(),
    }));
    let agix = daemon::discovery::identity();
    #[allow(unused_mut)]
    let mut process = json!({
        "app": { "name": app.name, "version": app.version },
        "harnix": crate::HARNIX_VERSION,
        "agix": agix.get("version").cloned().unwrap_or(json!(null)),
        "started_ms": now_ms(),
    });
    // What a store has to have compiled an application for, for this process to run it.
    #[cfg(feature = "apps")]
    if let Some(runtime) = &app_runtime {
        process["apps_engine"] = json!(runtime.engine().fingerprint());
    }
    hub.set_process(process);
    let started = now_ms();
    hub.add_instance(
        harnix_hub::SYSTEM,
        Built {
            link: system_link,
            facts: system_facts,
        },
        None,
        BTreeMap::new(),
        Owner::Kernel,
        started,
    )
    .await;
    for (id, built) in fixed {
        hub.add_instance(&id, built, None, BTreeMap::new(), Owner::Kernel, started)
            .await;
    }
    hub.restore().await;
    if profile.apps.enabled {
        hub.declare_spaces_topic().await;
    }
    #[cfg(feature = "apps")]
    if let Some(runtime) = &app_runtime {
        hub.declare_topic(APPS_PROGRESS_TOPIC, Vec::new()).await;
        let hub = hub.clone();
        runtime.on_progress(move |value| {
            let hub = hub.clone();
            tokio::spawn(async move { hub.publish(APPS_PROGRESS_TOPIC, json!({}), value).await });
        });
    }
    for record in records.iter().filter(|r| r.is_enabled()) {
        for topic in &record.topics {
            hub.declare_topic(&topic.name, topic.params.clone()).await;
        }
    }

    // ── Admission ───────────────────────────────────────────────────────
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
            .chain(app.app_hosts.iter().cloned())
            .collect(),
        hub: hub.clone(),
        listen: tokio::sync::watch::channel(None).0,
    });
    let _ = kernel.gate.set(gate.clone());
    if profile.apps.enabled {
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

    // ── 5. Start ────────────────────────────────────────────────────────
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
                record.disable("start", format!("{e:#}"), &mut authorizer.table_mut());
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
    let runtime_modules = profile.ui.runtime_modules;
    let (router, statics, transport) = harnix_transport::router(
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
            #[cfg(feature = "apps")]
            runtime_apps: app_runtime
                .as_ref()
                .map(|r| harnix_apps::serve::router(r.registry())),
            #[cfg(not(feature = "apps"))]
            runtime_apps: None,
        },
    );

    statics.set_brand_icon(brand_icon);
    let _ = channels.set(transport.clone());

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
    let ui_modules = Arc::new(UiModules::default());
    if let Some(builtin) = &builtin {
        // What the installed packages contribute to this side, resolved once
        // before anything is served. A package installed later refreshes
        // this — one that needed a restart to appear is one nobody will
        // believe installed.
        apply_packages(
            &hub,
            &statics,
            builtin.contributions().await,
            runtime_modules,
            &ui_modules,
        );
        let hub = hub.clone();
        let statics = statics.clone();
        let ui_modules = ui_modules.clone();
        builtin
            .on_packages_changed(move |contributions| {
                apply_packages(&hub, &statics, contributions, runtime_modules, &ui_modules);
            })
            .await;
    }
    // A running application's interface modules go to the page with the packages' (under the
    // same `[ui] runtime_modules`); its tools go to every daemon this process spawned, as an MCP
    // endpoint each. Both follow what is running, from the start (the enabled applications were
    // started with the `apps` module) to every enable, disable, install and uninstall.
    #[cfg(feature = "apps")]
    if let Some(runtime) = &app_runtime {
        let follow = {
            let runtime = Arc::downgrade(runtime);
            let hub = hub.clone();
            let ui_modules = ui_modules.clone();
            move || {
                let Some(runtime) = runtime.upgrade() else { return };
                if runtime_modules {
                    ui_modules.set_apps(&hub, runtime.ui_contributions());
                }
                let hub = hub.clone();
                let servers = runtime.mcp_servers();
                tokio::spawn(async move { hub.set_shared_servers(servers).await });
            }
        };
        follow();
        runtime.on_change(follow);
    }

    // Before anything is served: a reachable node with no paired device has
    // no way to acquire one, because pairing is a method and methods need
    // admission. This is where that circle is broken.
    devices::bootstrap(&paired, !host.is_loopback());

    let scheme = if cli.tls_cert.is_some() {
        "https"
    } else {
        "http"
    };
    // Where it listens, once it does: to the modules (ipdrun-14) and into the lock file (ipdrun-09).
    let told = {
        let kernel = kernel.clone();
        let state_lock = state_lock.clone();
        #[cfg(feature = "apps")]
        let told_runtime = app_runtime.clone();
        move |bound: Vec<SocketAddr>| {
            let Some(first) = bound.first() else { return };
            let url = format!("{scheme}://{first}/");
            state_lock.listening(&url);
            #[cfg(feature = "apps")]
            if let Some(runtime) = &told_runtime {
                runtime.set_listening(&url);
            }
            kernel.listen.send_replace(Some(harnix_module::Listen { addrs: bound, url }));
        }
    };
    // Shutting down stops the listener first (below), so it is handed the signal here.
    let (stop_serving, serving_stops) = tokio::sync::oneshot::channel::<()>();
    let mut server = tokio::spawn(async move {
        harnix_transport::serve_with(&addrs, router, tls, told, async move {
            let _ = serving_stops.await;
        })
        .await
    });
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

    // The parent's end of stdin closing is the parent gone (`--exit-with-stdin`). Read on a thread
    // of its own: tokio's stdin is a blocking task, and a runtime does not finish shutting down
    // while one is still blocked on a read.
    let orphaned = async {
        if !cli.exit_with_stdin {
            return std::future::pending::<()>().await;
        }
        let (gone, parent) = tokio::sync::oneshot::channel::<()>();
        std::thread::spawn(move || {
            use std::io::Read;
            let mut buf = [0u8; 64];
            while matches!(std::io::stdin().read(&mut buf), Ok(n) if n > 0) {}
            let _ = gone.send(());
        });
        let _ = parent.await;
    };
    // SIGTERM is how a service manager, a desktop shell, `kill` and a test harness ask a process to
    // go. It takes the same way out as Ctrl-C (ipdrun-02): under the default disposition it ended
    // the process on the spot, and no module's `stop` ran.
    let mut signals = Signals::install()?;
    tokio::select! {
        result = &mut server => { result??; }
        _ = tokio::signal::ctrl_c() => println!("shutting down…"),
        _ = signals.terminate() => println!("terminated; shutting down…"),
        _ = orphaned => println!("stdin closed; the parent is gone — shutting down…"),
    }

    let uploads = roots
        .state
        .join("uploads")
        .join(std::process::id().to_string());
    let shutdown = async {
        // Outside in. The listener stops taking connections and every open channel is told and
        // closed, so nothing reaches a module once it is stopping: a page that reconnected in the
        // grace period used to call modules that had already stopped. Then the modules, then the
        // hub under them.
        let _ = stop_serving.send(());
        transport.end_all().await;
        if !server.is_finished() {
            match (&mut server).await {
                Ok(Ok(())) => {}
                Ok(Err(e)) => tracing::warn!(error = %e, "the listener did not stop cleanly"),
                Err(e) => tracing::warn!(error = %e, "the listener's task ended badly"),
            }
        }
        // Modules stop in reverse: a later module may depend on an earlier one.
        for record in records.iter().rev().filter(|r| r.is_enabled()) {
            record.module.stop().await;
        }
        hub.shutdown().await;
    };
    // Bounded, and a second signal cuts it short: a `stop` that never returned used to keep the
    // process up for good.
    let forced = tokio::select! {
        _ = shutdown => None,
        _ = signals.any() => Some("a second signal; exiting without waiting"),
        _ = tokio::time::sleep(crate::SHUTDOWN_GRACE) => Some("shutting down took too long; exiting"),
    };
    // Uploads are this process's scratch; the rest of the state root outlives
    // it.
    let _ = std::fs::remove_dir_all(&uploads);
    if let Some(why) = forced {
        tracing::warn!(grace_secs = crate::SHUTDOWN_GRACE.as_secs(), "{why}");
        // Not a return: the runtime would wait on whatever is still stuck before letting go.
        std::process::exit(1);
    }
    Ok(())
}

/// The two signals that end this process, installed once when serving begins, so the second of
/// either is noticed while the first one's shutdown runs.
struct Signals {
    #[cfg(unix)]
    term: tokio::signal::unix::Signal,
}

impl Signals {
    fn install() -> std::io::Result<Self> {
        Ok(Self {
            #[cfg(unix)]
            term: tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?,
        })
    }

    async fn terminate(&mut self) {
        #[cfg(unix)]
        self.term.recv().await;
        #[cfg(not(unix))]
        std::future::pending::<()>().await
    }

    async fn any(&mut self) {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = self.terminate() => {}
        }
    }
}

fn down(carrier: harnix_instance::Carrier, reason: String) -> Built {
    tracing::error!(%reason, "instance not built");
    Built {
        link: DownLink::new(carrier, reason),
        facts: Facts::default(),
    }
}

/// The paired devices, beside the token in the state directory.
const DEVICES_FILE: &str = "devices.json";

/// The devices paired before this start, and every change written back.
///
/// A file that is not there is no devices. One that does not parse is set aside as
/// `devices.json.broken` and the node starts with none — on a reachable listener it then prints a
/// pairing code, which is the one way back in. One that cannot be read stops the start: taken as
/// empty, the next change would write over it.
fn load_devices(path: &std::path::Path) -> anyhow::Result<Devices> {
    let paired: Vec<harnix_authz::Device> = match std::fs::read(path) {
        Ok(bytes) => match serde_json::from_slice(&bytes) {
            Ok(paired) => paired,
            Err(e) => {
                let broken = path.with_extension("json.broken");
                tracing::warn!(error = %e, kept = %broken.display(), "the paired devices could not be read; starting with none");
                std::fs::rename(path, &broken)?;
                Vec::new()
            }
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(e) => anyhow::bail!("could not read {}: {e}", path.display()),
    };
    let file = path.to_path_buf();
    Ok(Devices::new(paired).persist_with(Box::new(move |devices| {
        if let Err(e) = write_devices(&file, devices) {
            tracing::warn!(error = %e, file = %file.display(), "could not write the paired devices");
        }
    })))
}

/// Replace the devices file whole, readable by its owner only: it holds no secret — public keys —
/// but it decides who is let in, so it is kept like the token.
fn write_devices(path: &std::path::Path, devices: &[harnix_authz::Device]) -> std::io::Result<()> {
    let body = serde_json::to_vec_pretty(devices)?;
    let temp = path.with_extension("json.tmp");
    let _ = std::fs::remove_file(&temp);
    write_private(&temp, &body)?;
    std::fs::rename(&temp, path)
}

/// Write the token readable by its owner only (IPDrun N-302).
///
/// The token admits a client to everything a person at this machine can do;
/// with launch links it is also what mints them. The default umask leaves a
/// fresh file readable by every account. An existing file is narrowed too,
/// since `create` does not change the mode of a file that is already there.
fn write_token(path: &std::path::Path, token: &str) -> std::io::Result<()> {
    write_private(path, token.as_bytes())
}

/// Write `bytes` to a file only its owner can read, narrowing a file that is already there.
fn write_private(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
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
    options.open(path)?.write_all(bytes)
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
///
/// With `[ui] runtime_modules = false` nothing is served and nothing is told: a package is then
/// the engine's alone, and its interface modules are not even reachable over HTTP.
fn apply_packages(
    hub: &Arc<Hub>,
    statics: &Arc<harnix_transport::StaticFace>,
    contributions: Vec<harnix_builtin::packages::Contributions>,
    runtime_modules: bool,
    ui_modules: &Arc<UiModules>,
) {
    let contributions = if runtime_modules {
        contributions
    } else {
        if contributions.iter().any(|c| !c.ui.is_empty()) {
            tracing::info!(
                "[ui] runtime_modules = false: packages' interface modules are not loaded"
            );
        }
        Vec::new()
    };
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
                    "api_version": entry.api_version,
                    "module": format!("/plugins/{}/ui/{}", c.plugin, entry.module),
                })
            })
        })
        .collect();
    ui_modules.set_packages(hub, modules);
}

/// The interface modules the page is told at hello: the installed packages', and the running
/// applications'. Two sources that change apart, told as one list.
#[derive(Default)]
struct UiModules {
    lists: std::sync::Mutex<(Vec<serde_json::Value>, Vec<serde_json::Value>)>,
}

impl UiModules {
    fn set_packages(self: &Arc<Self>, hub: &Arc<Hub>, modules: Vec<serde_json::Value>) {
        lock(&self.lists).0 = modules;
        self.publish(hub);
    }

    #[cfg_attr(not(feature = "apps"), allow(dead_code))]
    fn set_apps(self: &Arc<Self>, hub: &Arc<Hub>, modules: Vec<serde_json::Value>) {
        lock(&self.lists).1 = modules;
        self.publish(hub);
    }

    /// Spawned rather than awaited: this is called from callbacks that cannot be async. The lists
    /// are read when the task runs, so whichever task runs last sets the latest of both.
    fn publish(self: &Arc<Self>, hub: &Arc<Hub>) {
        let (hub, this) = (hub.clone(), self.clone());
        tokio::spawn(async move {
            let all: Vec<serde_json::Value> = {
                let lists = lock(&this.lists);
                lists.0.iter().chain(&lists.1).cloned().collect()
            };
            hub.set_ui_contributions(json!(all)).await;
        });
    }
}

/// What the application runtime verifies packages with: the store's key built in (or, for a test
/// application, the ones it was assembled with), and the profile's trusted publishers. A key that
/// does not read stops the start, naming it: a typo here would otherwise quietly trust nobody.
#[cfg(feature = "apps")]
fn signing_keys(
    apps: &harnix_instance::profile::AppsProfile,
    store: Option<&[String]>,
) -> anyhow::Result<harnix_apps::signature::Keys> {
    use harnix_apps::signature::PublicKey;
    let store = match store {
        Some(keys) => keys
            .iter()
            .map(|k| PublicKey::parse(k).map_err(anyhow::Error::msg))
            .collect::<anyhow::Result<Vec<_>>>()?,
        None => harnix_apps::store::store_keys(),
    };
    let mut trusted = BTreeMap::new();
    for (publisher, keys) in &apps.trusted_publishers {
        let keys = keys
            .iter()
            .map(|k| {
                PublicKey::parse(k)
                    .map_err(|e| anyhow::anyhow!("[apps.trusted_publishers] {publisher}: {e}"))
            })
            .collect::<anyhow::Result<Vec<_>>>()?;
        trusted.insert(publisher.clone(), keys);
    }
    Ok(harnix_apps::signature::Keys { store, trusted })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(id: &str) -> harnix_authz::Device {
        harnix_authz::Device {
            id: id.into(),
            label: "laptop".into(),
            public_key: "pk".into(),
            paired_at: "1".into(),
            last_seen: None,
        }
    }

    /// Paired devices survive a restart, in a file only the owner can read; a broken one is set
    /// aside rather than trusted or written over.
    #[test]
    fn paired_devices_are_kept_across_starts() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(DEVICES_FILE);
        assert!(load_devices(&path).unwrap().list().is_empty(), "no file is no devices");

        let devices = load_devices(&path).unwrap();
        devices.begin_pairing("ABC123", "laptop", std::time::SystemTime::now());
        let paired = devices
            .complete_pairing("ABC123", "pk", std::time::SystemTime::now(), "1")
            .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "{mode:o}");
        }
        let again = load_devices(&path).unwrap();
        assert!(again.is_paired(&paired.id));
        assert!(again.revoke(&paired.id));
        assert!(load_devices(&path).unwrap().list().is_empty(), "the revocation was kept");

        write_devices(&path, &[device("dev-a")]).unwrap();
        assert!(load_devices(&path).unwrap().is_paired("dev-a"));

        std::fs::write(&path, "not json").unwrap();
        assert!(load_devices(&path).unwrap().list().is_empty());
        assert!(path.with_extension("json.broken").exists());

        // Unreadable — a directory where the file should be — stops the start.
        std::fs::remove_file(&path).ok();
        std::fs::create_dir(&path).unwrap();
        assert!(load_devices(&path).is_err());
    }
}
