//! `harnix daemon`: this binary as an Agix daemon (design §5.1).
//!
//! The binary links Agix already, so a spawned instance needs no second
//! executable, and its protocol version is this build's by construction. It
//! builds the engine exactly as an in-process instance would, then serves it
//! on a unix socket instead of keeping it.

use std::path::PathBuf;

use daemon::DaemonServer;
use tokio::io::AsyncReadExt;

use crate::factory::engine_config;
use crate::profile::EngineProfile;

#[derive(Debug, Clone)]
pub struct DaemonArgs {
    pub data_root: PathBuf,
    pub socket: PathBuf,
    pub engine: EngineProfile,
    /// Exit when stdin closes: how a spawned daemon learns its parent is
    /// gone, even one that was killed outright.
    pub exit_with_stdin: bool,
}

pub async fn serve_daemon(args: DaemonArgs) -> anyhow::Result<()> {
    std::fs::create_dir_all(&args.data_root)?;
    let engine = crate::engine::build(engine_config(
        &args.engine,
        &args.data_root,
        crate::EngineContrib::default(),
    ))
    .await?;
    let server = engine.server.clone();
    let listener = DaemonServer::bind_unix(&args.socket)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&args.socket, std::fs::Permissions::from_mode(0o600))?;
    }
    let stop = server.shutdown_token();
    if args.exit_with_stdin {
        let stop = stop.clone();
        tokio::spawn(async move {
            let mut stdin = tokio::io::stdin();
            let mut buf = [0u8; 64];
            while matches!(stdin.read(&mut buf).await, Ok(n) if n > 0) {}
            tracing::info!("stdin closed; the parent is gone");
            stop.cancel();
        });
    }
    tracing::info!(
        socket = %args.socket.display(),
        data_root = %args.data_root.display(),
        scenes = ?engine.active_scenes,
        "daemon serving"
    );
    server.serve_unix_listener(listener).await?;
    engine.shutdown().await;
    let _ = std::fs::remove_file(&args.socket);
    Ok(())
}
