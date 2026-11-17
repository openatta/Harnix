//! Starting a process, in this order (concept §3.1).
//!
//! 1. Read the profile.
//! 2. **Register** every extension, in order: its manifest and what it declares.
//! 3. **Migrate** every extension, in order.
//! 4. **Build the instances**: `main`, with the native contributions of the
//!    extensions still enabled; the profile's `[[instances]]`; and what the
//!    ledger says was built from templates.
//! 5. **Start** every extension, in order.
//! 6. Serve.
//!
//! An extension that fails at 2, 3 or 5 is disabled and everything else goes
//! on; `harnix.diagnostics` says which one and why. Every decision here is
//! either a flag or a profile line; none of them is buried.
//!
//! Each step is a function of its own, in `run/`, returning what the next ones need; `run()`
//! is the order. Nothing here depends on which carriers are linked: a package carrier is an
//! extension's, and reaches the process the way any extension does.

mod admission;
mod hub;
mod instances;
mod register;
mod serve;
mod start;
mod state_files;

use std::net::IpAddr;
use std::sync::Arc;

use harnix_ext::{Credentials, EnvCredentials};
use harnix_instance::EngineProfile;
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::util::SubscriberInitExt as _;
use tracing_subscriber::Layer as _;

use crate::cli::{self, Cli, Command, UiCommand};
use crate::host::{lifecycle, Entry, ExtensionStatus, Record};
use crate::paths::{RootArgs, Roots};
use crate::profile::Profile;
use crate::state_lock::StateLock;
use crate::AppBuilder;

/// Step 2 alone, for [`crate::testing`]: the built-ins (or not) and these extensions registered
/// into the kernel's table under `profile`, with the grants they were given.
pub(crate) fn register_only(
    entries: Vec<Entry>,
    profile: &Profile,
    state: &std::path::Path,
) -> anyhow::Result<(harnix_router::RouteTable, Arc<harnix_kernel::GrantTable>, Vec<ExtensionStatus>)> {
    let roots = Roots {
        engine: state.join("engine"),
        projects: state.join("projects"),
        state: state.join("state"),
    };
    std::fs::create_dir_all(&roots.state)?;
    let registered = register::register(entries, profile, &roots)?;
    let statuses = registered.records.iter().map(|r| r.status.clone()).collect();
    Ok((registered.table, registered.grants, statuses))
}

/// A process started — every step up to serving — and what serving and stopping it need.
pub(crate) struct Started {
    pub name: String,
    pub records: Vec<Record>,
    pub hub: Arc<harnix_hub::Hub>,
    pub instances: Arc<harnix_instance::InstanceRegistry>,
    pub brand_icon: Option<std::path::PathBuf>,
    pub held: register::Held,
    pub admission: admission::Admission,
}

/// Steps 2 to 5: register, migrate, the instances, the hub and admission, start.
pub(crate) async fn start_process(app: AppBuilder, cli: &Cli, basis: &Basis) -> anyhow::Result<Started> {
    let AppBuilder { name, version, entries, credentials, .. } = app;
    let profile = &basis.profile;

    // ── 2. Register ─────────────────────────────────────────────────────
    let register::Registered { mut records, mut table, behind, held, grants, dynamic, package_sections } =
        register::register(entries, profile, &basis.roots)?;

    // ── 3. Migrate ──────────────────────────────────────────────────────
    lifecycle::migrate(&mut records, &mut table).await;

    // ── 4. Instances ────────────────────────────────────────────────────
    let credentials: Arc<dyn Credentials> = credentials.unwrap_or_else(|| Arc::new(EnvCredentials));
    // Replay is the operator's decision, for the whole process: every link records or plays
    // back as this says.
    let recording = harnix_instance::Recording::new(cli.replay_dir.clone());
    recording.set_strict(cli.replay_strict);
    let instances = instances::build(
        profile,
        &basis.roots,
        &mut records,
        &mut table,
        instances::Wiring { credentials: &credentials, recording, grants: grants.clone(), names: &basis.names },
    )
    .await?;
    let hub::Assembled { hub, instances, ui, brand_icon } =
        hub::assemble(cli, profile, &basis.roots, (&name, &version), &records, instances, &behind).await?;
    // The hub's rows have been in the route table since registration; from here they answer.
    let _ = behind.hub.set(hub.clone());

    // ── Admission ───────────────────────────────────────────────────────
    let admission = admission::admit(
        &hub,
        &instances,
        table,
        grants,
        &behind,
        admission::Parts {
            ui: ui.clone(),
            devices: held.devices.clone(),
            credentials: credentials.clone(),
            state_root: basis.roots.state.clone(),
            projects_root: basis.roots.projects.clone(),
            product: basis.names.clone(),
            dynamic,
            runtime_modules: profile.ui.runtime_modules,
            package_sections,
            unsandboxed: crate::host::spawn::unsandboxed(&records),
        },
        admission::Identity { name: &name, version: &version },
    );

    // ── 5. Start ────────────────────────────────────────────────────────
    start::start(&mut records, &admission).await;
    Ok(Started { name, records, hub, instances, brand_icon, held, admission })
}

pub(crate) async fn run(app: AppBuilder) -> anyhow::Result<()> {
    // The process's log, filtered by `RUST_LOG`; beside it, each extension's own, filtered apart
    // (`ext_log`).
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info,harnix=debug"));
    let _ = tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer().with_filter(filter))
        .with(crate::ext_log::layer(crate::ext_log::LOG.clone()))
        .try_init();

    let cli = cli::parse(&app.name, &app.version, app.args.clone());
    if let Some(done) = subcommand(&cli).await {
        return done;
    }

    // ── 1. Profile ──────────────────────────────────────────────────────
    let basis = Basis::read(&cli, app.default_engine.as_ref())?;
    let started = start_process(app, &cli, &basis).await?;

    // ── 6. Serve ────────────────────────────────────────────────────────
    let mut serving = serve::serve(
        &cli,
        &basis,
        &started.name,
        &started.held,
        &started.records,
        &started.hub,
        &started.admission,
        started.brand_icon.clone(),
    )
    .await?;
    // SIGTERM is how a service manager, a desktop shell, `kill` and a test harness ask a process to
    // go. It takes the same way out as Ctrl-C (ipdrun-02): under the default disposition it ended
    // the process on the spot, and no module's `stop` ran.
    let mut signals = Signals::install()?;
    until_stopped(&cli, &mut serving.server, &mut signals).await?;
    shut_down(serving, &started.records, (&started.hub, &started.instances), &mut signals, &basis.roots).await;
    Ok(())
}

/// Serve until asked to stop: a signal, or the parent gone. A listener that fails is the error.
async fn until_stopped(
    cli: &Cli,
    server: &mut tokio::task::JoinHandle<anyhow::Result<()>>,
    signals: &mut Signals,
) -> anyhow::Result<()> {
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
    tokio::select! {
        result = server => { result??; }
        _ = tokio::signal::ctrl_c() => println!("shutting down…"),
        _ = signals.terminate() => println!("terminated; shutting down…"),
        _ = orphaned => println!("stdin closed; the parent is gone — shutting down…"),
    }
    Ok(())
}

/// Stop, outside in, within [`crate::SHUTDOWN_GRACE`].
async fn shut_down(
    serving: serve::Serving,
    records: &[Record],
    (hub, instances): (&Arc<harnix_hub::Hub>, &Arc<harnix_instance::InstanceRegistry>),
    signals: &mut Signals,
    roots: &Roots,
) {
    let serve::Serving {
        mut server,
        stop,
        channels: transport,
    } = serving;
    let uploads = roots
        .state
        .join("uploads")
        .join(std::process::id().to_string());
    let shutdown = async {
        // Outside in. The listener stops taking connections and every open channel is told and
        // closed, so nothing reaches a module once it is stopping: a page that reconnected in the
        // grace period used to call modules that had already stopped. Then the modules, then the
        // hub under them, then the instances under it.
        let _ = stop.send(());
        transport.end_all().await;
        if !server.is_finished() {
            match (&mut server).await {
                Ok(Ok(())) => {}
                Ok(Err(e)) => tracing::warn!(error = %e, "the listener did not stop cleanly"),
                Err(e) => tracing::warn!(error = %e, "the listener's task ended badly"),
            }
        }
        lifecycle::stop(records).await;
        hub.shutdown().await;
        instances.shutdown().await;
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
}

/// What a subcommand does instead of serving: `ui export`, and `daemon` (a spawned instance's
/// process). `None` is no subcommand.
async fn subcommand(cli: &Cli) -> Option<anyhow::Result<()>> {
    match &cli.command {
        Some(Command::Ui(UiCommand::Export { dir })) => Some(
            harnix_transport::StaticFace::export(dir)
                .map(|count| println!("\n  {count} files → {}\n", dir.display()))
                .map_err(Into::into),
        ),
        Some(Command::Daemon(daemon)) => Some(match daemon.args() {
            Ok(args) => harnix_instance::serve_daemon(args).await,
            Err(e) => Err(e),
        }),
        None => None,
    }
}

/// Step 1: the profile, and what is decided from it and the flags before anything is
/// registered.
pub(crate) struct Basis {
    profile: Profile,
    /// Where this listens.
    host: IpAddr,
    tls: Option<harnix_transport::Tls>,
    roots: Roots,
    /// The directory the engine and Harnix keep in each project, and the name the engine goes by:
    /// the brand's.
    names: harnix_contract::ProductNames,
    /// Held for as long as the process runs.
    state_lock: Arc<StateLock>,
}

impl Basis {
    pub(crate) fn read(cli: &Cli, default_engine: Option<&EngineProfile>) -> anyhow::Result<Self> {
        let profile = cli::resolve_profile(cli, default_engine)?;
        let host: IpAddr = profile.transport.host.parse().map_err(|e| {
            anyhow::anyhow!("[transport] host = {:?} is not an IP address: {e}", profile.transport.host)
        })?;
        // A non-loopback bind needs TLS, and `harnix_transport::serve` refuses it
        // without. Refused at the point of binding rather than here, so every
        // caller of that function inherits the rule instead of remembering it.
        let tls = tls(cli)?;
        let names = profile.brand.names()?;
        let roots = Roots::resolve(&RootArgs {
            engine_dir: cli.engine_dir.clone(),
            data_dir: cli.data_dir.clone(),
            product_id: profile.brand.product_id.clone(),
        });
        tracing::info!(
            engine = %roots.engine.display(),
            projects = %roots.projects.display(),
            state = %roots.state.display(),
            project_dir = names.project_dir(),
            product_name = names.product_name(),
            "directories"
        );
        // Before anything in the state directory is read: a second process on it is refused here,
        // with who has it, rather than starting and overwriting the first one's token and ledger.
        let state_lock = Arc::new(StateLock::acquire(&roots.state)?);
        Ok(Self {
            profile,
            host,
            tls,
            roots,
            names,
            state_lock,
        })
    }
}

/// `--tls-cert` and `--tls-key`, both or neither.
fn tls(cli: &Cli) -> anyhow::Result<Option<harnix_transport::Tls>> {
    match (&cli.tls_cert, &cli.tls_key) {
        (Some(cert), Some(key)) => Ok(Some(harnix_transport::Tls {
            cert: cert.clone(),
            key: key.clone(),
        })),
        (None, None) => Ok(None),
        _ => anyhow::bail!("--tls-cert and --tls-key go together"),
    }
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
