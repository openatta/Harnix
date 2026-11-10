//! An extension process: the product's own binary, re-executed as `<product> ext-proc`, running
//! one extension package's host and VM and reaching the product only over the extension wire.
//!
//! A library, not a program. The product links it and registers [`entry`] as the `ext-proc`
//! subcommand (`AppBuilder::child_entry`), which is dispatched before the product reads a profile,
//! takes its state lock or assembles anything: so the process is the same build as the product
//! that started it, and there is no second binary to ship.
//!
//! What [`entry`] does, in order:
//!
//! 1. Takes the stream it inherited at `--wire-fd` (3) — before anything else could open a
//!    descriptor that takes the number — and keeps it from its own children.
//! 2. On Linux, sandboxes itself as the product asked (`sandbox`), before it reads anything of
//!    its package; on macOS it was started inside its sandbox.
//! 3. Builds the host (`harnix-wasm-host`): one VM engine, deserializing only. The host's
//!    [`Kernel`] is the wire's client end; its agent engine is this process's own, beside it.
//! 4. Says `proc.hello` (its wire, Harnix and package-interface versions) and waits for the
//!    product to answer: the product refuses a process that is not of its own build.
//! 5. Answers the product's `ctl.*` with the host — `ctl.start` starts the package, whose names
//!    go back as `proc.register` — until the wire closes, stdin closes (the product is gone), or
//!    the product says `ctl.shutdown`.
//!
//! Who it is, the product knows from the stream: it is the child it gave fd 3 to. It reads no
//! profile and no state directory, and listens on nothing.

mod agent;
mod answer;
pub mod args;
#[cfg(feature = "probe")]
mod probe;
mod sandbox;

use std::ffi::OsString;
use std::future::Future;
use std::os::fd::{FromRawFd, RawFd};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Context};
use harnix_wasm_host::{Host, HostOptions};
use harnix_wasm_port::rpc::{KernelClient, Wire};
use harnix_wasm_port::socket::{now_ms, CallMeta, CancellationToken, Hello, Options as WireOptions, SocketWire, HELLO};
use harnix_wasm_port::{Control, Kernel, EXT_WIRE_VERSION};
use tokio::io::{AsyncRead, AsyncWrite};

pub use args::Args;

use crate::agent::NoEngine;
use crate::answer::Answering;

/// The exit status of a process that could not be set up — no wire at the descriptor it was
/// given, no sandbox where one was asked for, no VM engine — with why on the first line of stderr
/// (`EX_CONFIG`).
pub const EXIT_SETUP: i32 = 78;

/// The Harnix this process is: what `proc.hello` says, and the product compares with its own.
pub const HARNIX_VERSION: &str = env!("CARGO_PKG_VERSION");

/// How long the product has to answer `proc.hello`: as long as it gives the process to be ready.
pub const HELLO_WITHIN: Duration = Duration::from_secs(30);

/// After answering `ctl.shutdown`, how long the process waits for the product to close the wire
/// before closing it itself: the answer is written first either way.
const SHUTDOWN_LINGER: Duration = Duration::from_secs(1);

/// What a process is set up with: who it is, and where its package and run directory are — its
/// own, from its command line, whatever a start says.
pub struct Options {
    pub ext: String,
    pub pkg: Option<std::path::PathBuf>,
    pub run: Option<std::path::PathBuf>,
}

/// The `ext-proc` subcommand: what the product registers with `AppBuilder::child_entry`.
///
/// What could not be set up ends the process with [`EXIT_SETUP`]; what ends it once it runs is
/// the product's doing, and is not an error.
pub fn entry(args: Vec<OsString>) -> anyhow::Result<()> {
    let args = Args::parse(args).unwrap_or_else(|e| setup_failed(&e));
    let stream = take_stream(args.wire_fd).unwrap_or_else(|e| setup_failed(&e));
    if args.engine_fd.is_some() {
        setup_failed(&anyhow!("`--engine-fd`: this extension process serves no engine of its own yet"));
    }
    sandbox::apply().context("its sandbox").unwrap_or_else(|e| setup_failed(&e));
    if let Some(run) = &args.run {
        std::env::set_current_dir(run)
            .with_context(|| format!("its run directory {}", run.display()))
            .unwrap_or_else(|e| setup_failed(&e));
    }
    #[cfg(feature = "probe")]
    if let Some(targets) = &args.probe {
        return probe::run(targets, &args, stream);
    }
    let _ = tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .try_init();
    let runtime = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
    let options = Options {
        ext: args.ext,
        pkg: args.pkg,
        run: args.run,
    };
    let ran = runtime.block_on(async move {
        let stream = tokio::net::UnixStream::from_std(stream)?;
        run(options, stream, stdin_closed()).await
    });
    // What the host left running does not hold the process up: the product has already moved on.
    runtime.shutdown_timeout(Duration::from_secs(1));
    match ran {
        Err(e) if e.downcast_ref::<Setup>().is_some() => setup_failed(&e),
        ran => ran,
    }
}

/// Serve the product over `stream` until `until` resolves (stdin closed, in a process), the wire
/// closes, or the product says `ctl.shutdown`. What [`entry`] runs once it has its wire; a test
/// runs it on one end of a socket pair.
pub async fn run<S>(options: Options, stream: S, until: impl Future<Output = ()>) -> anyhow::Result<()>
where
    S: AsyncRead + AsyncWrite + Send + 'static,
{
    let answering = Arc::new(Answering::new(options.pkg.clone(), options.run.clone()));
    let wire = SocketWire::client(stream, answering.clone(), WireOptions::default());
    let kernel: Arc<dyn Kernel> = Arc::new(KernelClient(wire.clone() as Arc<dyn Wire>));
    let host = match Host::new(HostOptions::default(), kernel, Arc::new(NoEngine)) {
        Ok(host) => host,
        Err(e) => {
            wire.close("the extension process could not be set up").await;
            return Err(e.context(Setup));
        }
    };
    answering.answer_with(host.clone());

    let hello = Hello {
        wire: EXT_WIRE_VERSION,
        ext: options.ext.clone(),
        version: HARNIX_VERSION.to_string(),
        // This process serves no engine of its own yet, so it links no Agix it could name.
        agix_version: None,
        engine: host.engine().fingerprint().to_string(),
        harnix_api: harnix_package::manifest::HARNIX_API,
        pid: std::process::id(),
    };
    let hello = serde_json::to_value(&hello)?;
    let meta = CallMeta::default().deadline_ms(now_ms() + HELLO_WITHIN.as_millis() as u64);
    if let Err(e) = wire.call(HELLO, hello, meta, &CancellationToken::new()).await {
        wire.close("the product refused it").await;
        return Err(anyhow!("the product refused this extension process: {}", e.message));
    }
    tracing::debug!(ext = %options.ext, "the product answered proc.hello");

    tokio::pin!(until);
    let (why, shut_down) = tokio::select! {
        why = wire.closed() => (format!("the product's wire closed: {why}"), false),
        _ = &mut until => ("stdin closed: the product is gone".to_string(), false),
        _ = answering.shutdown.notified() => {
            tokio::select! {
                _ = wire.closed() => {}
                _ = &mut until => {}
                _ = tokio::time::sleep(SHUTDOWN_LINGER) => {}
            }
            ("the product asked it to go".to_string(), true)
        }
    };
    tracing::debug!(ext = %options.ext, %why, "the extension process is going");
    if !shut_down {
        let _ = host.shutdown(CallMeta::default(), why.clone()).await;
    }
    wire.close(&why).await;
    Ok(())
}

/// Marks an error as one of setting the process up: [`entry`] exits [`EXIT_SETUP`] with it.
#[derive(Debug)]
struct Setup;

impl std::fmt::Display for Setup {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the extension process could not be set up")
    }
}

fn setup_failed(why: &anyhow::Error) -> ! {
    eprintln!("ext-proc: {why:#}");
    std::process::exit(EXIT_SETUP)
}

/// The stream inherited at `fd`, owned from here on, and not passed on to anything this process
/// starts: a stream socket, or nothing.
fn take_stream(fd: RawFd) -> anyhow::Result<std::os::unix::net::UnixStream> {
    // SAFETY: fstat writes into the zeroed struct it is given and fails on a closed number.
    let mut stat: libc::stat = unsafe { std::mem::zeroed() };
    if unsafe { libc::fstat(fd, &mut stat) } != 0 {
        return Err(anyhow!(
            "no extension wire at descriptor {fd}: {} (it is started by the product, which hands it one)",
            std::io::Error::last_os_error()
        ));
    }
    if stat.st_mode & libc::S_IFMT != libc::S_IFSOCK {
        return Err(anyhow!("descriptor {fd} is not a socket, so not an extension wire"));
    }
    // SAFETY: fcntl on a descriptor just seen to be open.
    if unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) } != 0 {
        return Err(anyhow!("descriptor {fd}: {}", std::io::Error::last_os_error()));
    }
    // SAFETY: the descriptor was inherited for this process to own, and is taken once, here.
    let stream = unsafe { std::os::unix::net::UnixStream::from_raw_fd(fd) };
    stream.set_nonblocking(true)?;
    Ok(stream)
}

/// Resolves when stdin closes: the product's end of it is held for as long as the product lives,
/// killed outright included. Read on a thread of its own, which a runtime shutting down does not
/// wait for.
fn stdin_closed() -> impl Future<Output = ()> {
    let (gone, parent) = tokio::sync::oneshot::channel::<()>();
    std::thread::spawn(move || {
        use std::io::Read;
        let mut buf = [0u8; 64];
        while matches!(std::io::stdin().read(&mut buf), Ok(n) if n > 0) {}
        let _ = gone.send(());
    });
    async move {
        let _ = parent.await;
    }
}
