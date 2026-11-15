//! `harnix daemon`: this binary as an Agix daemon (concept §3.1.1).
//!
//! The binary links Agix already, so a spawned instance needs no second executable, and its
//! protocol version is this build's by construction. It builds the engine exactly as an
//! in-process instance would, then serves it on a unix socket instead of keeping it. Whatever
//! guards the data root while it does — the instance's lock — is its caller's to hold.

use std::path::Path;

use daemon::DaemonServer;
use tokio::io::AsyncReadExt;

use crate::engine::EngineConfig;

/// Build the engine `config` describes and serve it on `socket` until told to stop — or, with
/// `exit_with_stdin`, until stdin closes: how a spawned daemon learns its parent is gone, even
/// one that was killed outright.
pub async fn serve_daemon(config: EngineConfig, socket: &Path, exit_with_stdin: bool) -> anyhow::Result<()> {
    let data_root = config.data_root.clone();
    let engine = crate::engine::build(config).await?;
    let server = engine.server.clone();
    let listener = DaemonServer::bind_unix(socket)?;
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(socket, std::fs::Permissions::from_mode(0o600))?;
    }
    let stop = server.shutdown_token();
    if exit_with_stdin {
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
        socket = %socket.display(),
        data_root = %data_root.display(),
        scenes = ?engine.active_scenes,
        "daemon serving"
    );
    server.serve_unix_listener(listener).await?;
    engine.shutdown().await;
    let _ = std::fs::remove_file(socket);
    Ok(())
}
