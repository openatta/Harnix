//! Starting an extension process for a host component (`HostApi::spawn_child`): the product's own
//! binary, re-executed as `<program> ext-proc`, kept by a supervisor for one run.
//!
//! Each start is a fresh socket pair (two with an engine of its own): the process's end goes to it
//! as fd 3 (and 4), the product's end to whoever keeps the handle, with the start. Nothing listens:
//! no other process has anything to connect to, so the stream needs no credential — the subject is
//! bound to it before the process starts ([`harnix_authz::ChildBindings`]).
//!
//! The process gets an emptied environment: `PATH` (the system's, not this process's — a shell the
//! process starts finds the system's tools first), `LANG` and `RUST_LOG` when this process has them,
//! `HOME` and `TMPDIR` in its run directory, which is also where it starts. It reads no profile and
//! no state directory of the product's.

use std::os::fd::OwnedFd;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use harnix_authz::{ChildBindings, ChildSubject};
use harnix_contract::s1::ExtId;
use harnix_contract::RpcError;
use harnix_host_api::{ChildEvent, ChildHandle, ChildRequest, ChildRun, RestartPolicy};
use harnix_kernel::Rpc;
use harnix_supervise::{ChildSpec, Supervisor};
use tokio::sync::mpsc;

/// The subcommand an extension process is.
pub(crate) const SUBCOMMAND: &str = "ext-proc";

/// Where an extension process finds programs: the system's own, whatever this process was started
/// with.
pub(crate) const SYSTEM_PATH: &str = "/usr/bin:/bin:/usr/sbin:/sbin";

/// What of this process's environment an extension process is handed, when it is set.
const PASSED: &[&str] = &["LANG", "RUST_LOG"];

/// The product's ends of one start's streams.
type Ends = (UnixStream, Option<UnixStream>);

/// Start the extension process `req` asks for, as `id`, kept by a supervisor.
pub(crate) fn spawn(children: &Arc<ChildBindings>, id: &ExtId, req: ChildRequest) -> Rpc<ChildHandle> {
    let refused = |why: String| RpcError::refused(format!("`{id}`'s extension process: {why}"));
    let program = match req.program_path() {
        Some(program) => program.clone(),
        None => std::env::current_exe().map_err(|e| refused(format!("this binary's path: {e}")))?,
    };
    let pkg = req.pkg().cloned().ok_or_else(|| refused("no package directory".into()))?;
    let run = req.run().cloned().ok_or_else(|| refused("no run directory".into()))?;
    for dir in [run.join("home"), run.join("tmp")] {
        std::fs::create_dir_all(&dir).map_err(|e| refused(format!("{}: {e}", dir.display())))?;
    }
    let env = environment(&run, |name| std::env::var(name).ok());
    let args = arguments(id, &pkg, &run, req.has_engine());
    let subject = children.bind_child(id);
    let (ends_tx, ends) = mpsc::unbounded_channel::<Ends>();
    let with_engine = req.has_engine();
    let make = move || {
        let (product, process) = UnixStream::pair()?;
        let mut spec = ChildSpec::new(&program)
            .args(args.iter().cloned())
            .env_clear()
            .envs(env.iter().cloned())
            .current_dir(&run)
            .inherit_fd(3, OwnedFd::from(process))
            .own_session();
        let mut engine = None;
        if with_engine {
            let (product, process) = UnixStream::pair()?;
            spec = spec.inherit_fd(4, OwnedFd::from(process));
            engine = Some(product);
        }
        let _ = ends_tx.send((product, engine));
        Ok(spec)
    };
    let supervisor = Supervisor::start(make, policy(req.restart_policy()));
    tracing::info!(extension = %id, "extension process started");
    Ok(ChildHandle::new(id.clone(), Box::new(Run { supervisor: Some(supervisor), ends, pending: None, _subject: subject })))
}

/// `ext-proc --ext <id> --pkg <dir> --run <dir> --wire-fd 3 [--engine-fd 4]`.
fn arguments(id: &ExtId, pkg: &Path, run: &Path, engine: bool) -> Vec<String> {
    let mut args = vec![
        SUBCOMMAND.to_string(),
        "--ext".into(),
        id.to_string(),
        "--pkg".into(),
        pkg.display().to_string(),
        "--run".into(),
        run.display().to_string(),
        "--wire-fd".into(),
        "3".into(),
    ];
    if engine {
        args.extend(["--engine-fd".to_string(), "4".to_string()]);
    }
    args
}

/// What the process's environment is: nothing of this process's but [`PASSED`].
fn environment(run: &Path, get: impl Fn(&str) -> Option<String>) -> Vec<(String, String)> {
    let mut env = vec![
        ("PATH".to_string(), SYSTEM_PATH.to_string()),
        ("HOME".to_string(), run.join("home").display().to_string()),
        ("TMPDIR".to_string(), run.join("tmp").display().to_string()),
    ];
    env.extend(PASSED.iter().filter_map(|name| get(name).map(|value| (name.to_string(), value))));
    env
}

fn policy(restart: RestartPolicy) -> harnix_supervise::RestartPolicy {
    match restart {
        RestartPolicy::Never => harnix_supervise::RestartPolicy {
            backoff: Vec::new(),
            reset_after: Duration::MAX,
            quick: Duration::MAX,
            give_up_after: 1,
            fatal: vec![harnix_supervise::EXIT_SETUP],
        },
        _ => harnix_supervise::RestartPolicy::extension(),
    }
}

/// One run, as its handle has it.
struct Run {
    supervisor: Option<Supervisor>,
    ends: mpsc::UnboundedReceiver<Ends>,
    /// The ends of the start being made: handed out with its `Started`.
    pending: Option<Ends>,
    /// Bound to the streams for as long as the handle is held.
    _subject: ChildSubject,
}

impl Run {
    /// The latest ends made: those of a start that failed are dropped with it.
    fn take_ends(&mut self) {
        while let Ok(ends) = self.ends.try_recv() {
            self.pending = Some(ends);
        }
    }
}

#[async_trait::async_trait]
impl ChildRun for Run {
    async fn next(&mut self) -> Option<ChildEvent> {
        let event = self.supervisor.as_mut()?.next_event().await?;
        self.take_ends();
        Some(match event {
            harnix_supervise::ChildEvent::Started { pid } => match self.pending.take() {
                Some((wire, engine)) => ChildEvent::Started { pid, wire, engine },
                None => ChildEvent::SpawnFailed { error: "started with no stream to hand it".into() },
            },
            harnix_supervise::ChildEvent::SpawnFailed { error } => {
                self.pending = None;
                ChildEvent::SpawnFailed { error }
            }
            harnix_supervise::ChildEvent::Exited { status, ran_for } => ChildEvent::Exited { code: status.code(), ran_for },
            harnix_supervise::ChildEvent::Restarting { after } => ChildEvent::Restarting { after },
            harnix_supervise::ChildEvent::GaveUp { reason } => ChildEvent::GaveUp { reason },
            harnix_supervise::ChildEvent::Stopped => ChildEvent::Stopped,
        })
    }

    fn kill(&self) {
        if let Some(supervisor) = &self.supervisor {
            supervisor.kill();
        }
    }

    async fn stop(mut self: Box<Self>) {
        if let Some(supervisor) = self.supervisor.take() {
            supervisor.stop().await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_process_is_given_its_own_directories_and_the_systems_path() {
        let run = Path::new("/state/ext/acme.notes/run");
        let env = environment(run, |name| match name {
            "LANG" => Some("en_US.UTF-8".into()),
            "HOME" | "PATH" => Some("/Users/someone".into()),
            _ => None,
        });
        let get = |name: &str| env.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str());
        assert_eq!(get("PATH"), Some(SYSTEM_PATH));
        assert_eq!(get("HOME"), Some("/state/ext/acme.notes/run/home"));
        assert_eq!(get("TMPDIR"), Some("/state/ext/acme.notes/run/tmp"));
        assert_eq!(get("LANG"), Some("en_US.UTF-8"));
        assert_eq!(get("RUST_LOG"), None);
        assert_eq!(env.len(), 4, "{env:?}");
    }

    #[test]
    fn the_command_line_names_the_extension_its_directories_and_its_streams() {
        let id = ExtId::package("acme.notes").unwrap();
        let args = arguments(&id, Path::new("/p/1.0.0"), Path::new("/p/run"), false);
        assert_eq!(args, ["ext-proc", "--ext", "acme.notes", "--pkg", "/p/1.0.0", "--run", "/p/run", "--wire-fd", "3"]);
        let args = arguments(&id, Path::new("/p/1.0.0"), Path::new("/p/run"), true);
        assert_eq!(&args[args.len() - 2..], ["--engine-fd", "4"]);
    }
}
