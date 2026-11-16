//! `harnix daemon`, as an instance: the engine served on a socket, with the instance's lock held
//! on its data root for as long as it serves (concept §3.1.1).

use std::path::{Path, PathBuf};

use harnix_engine::EngineConfig;

use crate::profile::EngineProfile;

#[derive(Debug, Clone)]
pub struct DaemonArgs {
    pub data_root: PathBuf,
    pub socket: PathBuf,
    pub engine: EngineProfile,
    /// Exit when stdin closes: how a spawned daemon learns its parent is
    /// gone, even one that was killed outright.
    pub exit_with_stdin: bool,
    /// The instance this daemon carries. Given, the daemon holds that instance's lock on the data
    /// root for its whole life, waiting up to `lock_wait` for a previous daemon of the same
    /// instance to let go before it builds anything (ipdrun-08).
    pub instance: Option<String>,
    pub lock_wait: std::time::Duration,
    /// The instance reaps on stop: the engine records its commands' process groups in the data
    /// root, and stops what a daemon that was killed left there before it builds anything.
    pub reap: bool,
}

/// Serve the engine `args` describe. What a template contributes natively is not built here: the
/// process that spawned this one offers it on its link.
pub async fn serve_daemon(args: DaemonArgs) -> anyhow::Result<()> {
    std::fs::create_dir_all(&args.data_root)?;
    // Held to the end of this function: past the engine's own shutdown, which is what writes last.
    let _lock = match &args.instance {
        Some(instance) => {
            let path = crate::data_lock::lock_path(&args.data_root, instance);
            match crate::data_lock::acquire(&path, args.lock_wait).await? {
                Ok(lock) => Some(lock),
                Err(held) => anyhow::bail!(
                    "{held} ({}), still after {}s; not starting a second engine on it",
                    path.display(),
                    args.lock_wait.as_secs()
                ),
            }
        }
        None => None,
    };
    let mut config = EngineConfig { socket: Some(args.socket.clone()), ..EngineConfig::from_profile(&args.engine, &args.data_root) };
    if args.reap {
        config = config.reaping();
    }
    harnix_engine::serve_daemon(config, args.exit_with_stdin).await
}

/// The exit status of a daemon that refused what it was started with, and will refuse it again
/// however often it is restarted: the link takes the instance down rather than retrying.
pub const REFUSED_EXIT: i32 = 78;

/// Where a refusing daemon writes why, for the link that spawned it to read: beside its socket.
pub fn refusal_path(socket: &Path) -> PathBuf {
    let mut name = socket.as_os_str().to_owned();
    name.push(".refused");
    PathBuf::from(name)
}
