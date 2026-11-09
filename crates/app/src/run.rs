//! Starting a process, in this order (concept §3.1).
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
//!
//! Each step is a function of its own, in `run/`, returning what the next ones need; `run()`
//! is the order. What the `apps` feature adds to a step is asked of [`wiring::AppsWiring`], the
//! one place that feature is compiled in or out.

mod admission;
mod hub;
mod instances;
mod packages;
mod register;
mod serve;
mod start;
mod state_files;
mod wiring;

use std::net::IpAddr;
use std::sync::Arc;

use harnix_instance::Profile;
use harnix_module::{Credentials, EnvCredentials};

use crate::cli::{self, Cli, Command, UiCommand};
use crate::modules;
use crate::paths::{RootArgs, Roots};
use crate::state_lock::StateLock;
use crate::AppBuilder;
use wiring::AppsWiring;

pub(crate) async fn run(app: AppBuilder) -> anyhow::Result<()> {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info,harnix=debug")),
        )
        .try_init();

    let cli = cli::parse(&app.name, &app.version, app.args.clone());
    if let Some(done) = subcommand(&cli, &app).await {
        return done;
    }

    // ── 1. Profile ──────────────────────────────────────────────────────
    let basis = Basis::read(&cli)?;
    let mut apps = AppsWiring::new(&basis.profile, &app)?;
    let AppBuilder {
        name,
        version,
        builtins,
        modules: app_modules,
        credentials,
        app_hosts,
        ..
    } = app;
    let profile = &basis.profile;

    // ── 2. Register ─────────────────────────────────────────────────────
    let register::Registered {
        mut records,
        mut table,
        held,
    } = register::register(builtins, app_modules, profile, &basis.roots, &apps)?;

    // ── 3. Migrate ──────────────────────────────────────────────────────
    modules::migrate(&mut records, &mut table).await;

    // ── 4. Instances ────────────────────────────────────────────────────
    let credentials: Arc<dyn Credentials> = credentials.unwrap_or_else(|| Arc::new(EnvCredentials));
    let instances = instances::build(
        profile,
        &basis.roots,
        &mut records,
        &mut table,
        &credentials,
        &mut apps,
    )
    .await?;
    let hub::Assembled {
        hub,
        history,
        brand_icon,
    } = hub::assemble(
        &cli,
        profile,
        &basis.roots,
        (&name, &version),
        &records,
        instances,
        &apps,
    )
    .await?;

    // ── Admission ───────────────────────────────────────────────────────
    let admission = admission::admit(
        &hub,
        table,
        &records,
        history,
        &profile.modules,
        profile.apps.enabled,
        admission::Identity {
            name: &name,
            version: &version,
            app_hosts: &app_hosts,
        },
    );

    // ── 5. Start ────────────────────────────────────────────────────────
    start::start(&mut records, &admission, &credentials, &hub).await;

    // ── 6. Serve ────────────────────────────────────────────────────────
    let mut serving = serve::serve(
        &cli,
        &basis,
        &name,
        &held,
        &mut records,
        &hub,
        &admission,
        brand_icon,
        &apps,
    )
    .await?;
    // SIGTERM is how a service manager, a desktop shell, `kill` and a test harness ask a process to
    // go. It takes the same way out as Ctrl-C (ipdrun-02): under the default disposition it ended
    // the process on the spot, and no module's `stop` ran.
    let mut signals = Signals::install()?;
    until_stopped(&cli, &mut serving.server, &mut signals).await?;
    shut_down(serving, &records, &hub, &mut signals, &basis.roots).await;
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
    records: &[modules::Record],
    hub: &Arc<harnix_hub::Hub>,
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
        // hub under them.
        let _ = stop.send(());
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
}

/// What a subcommand does instead of serving: `ui export`, and `daemon` (a spawned instance's
/// process). `None` is no subcommand.
async fn subcommand(cli: &Cli, app: &AppBuilder) -> Option<anyhow::Result<()>> {
    match &cli.command {
        Some(Command::Ui(UiCommand::Export { dir })) => Some(
            harnix_transport::StaticFace::export(dir)
                .map(|count| println!("\n  {count} files → {}\n", dir.display()))
                .map_err(Into::into),
        ),
        Some(Command::Daemon(daemon)) => {
            let contrib = match &daemon.template {
                None => harnix_instance::EngineContrib::default(),
                Some(id) => match modules::native_of(&app.modules, id) {
                    Ok(contrib) => contrib,
                    Err(reason) => harnix_instance::refuse_daemon(&daemon.socket, &reason),
                },
            };
            Some(harnix_instance::serve_daemon_with(daemon.args(), contrib).await)
        }
        None => None,
    }
}

/// Step 1: the profile, and what is decided from it and the flags before anything is
/// registered.
struct Basis {
    profile: Profile,
    /// Where this listens.
    host: IpAddr,
    tls: Option<harnix_transport::Tls>,
    roots: Roots,
    /// Held for as long as the process runs.
    state_lock: Arc<StateLock>,
}

impl Basis {
    fn read(cli: &Cli) -> anyhow::Result<Self> {
        let profile = cli::resolve_profile(cli)?;
        let host: IpAddr = profile.transport.host.parse().map_err(|e| {
            anyhow::anyhow!("[transport] host = {:?} is not an IP address: {e}", profile.transport.host)
        })?;
        // A non-loopback bind needs TLS, and `harnix_transport::serve` refuses it
        // without. Refused at the point of binding rather than here, so every
        // caller of that function inherits the rule instead of remembering it.
        let tls = tls(cli)?;
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
        let state_lock = Arc::new(StateLock::acquire(&roots.state)?);
        Ok(Self {
            profile,
            host,
            tls,
            roots,
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
