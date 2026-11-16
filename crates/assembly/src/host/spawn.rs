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
//!
//! It starts inside an operating-system sandbox ([`harnix_supervise::sandbox`]), applied here and
//! not by the carrier that asked: it reads and writes its run directory, reads its package, and
//! reaches nothing else of the state — no other extension's, no consent record, no token. Where
//! there can be no sandbox (no `sandbox-exec`, no Landlock, Yama's `ptrace_scope` at 0, another
//! system) no process starts, and the refusal says why. Only the component's own section turns it
//! off (`sandbox = "off"`), for development: then each start is a warning.
//!
//! Its stderr is not this process's: each line goes to this process's log, and the last few are
//! kept for the component to show ([`harnix_host_api::ChildHandle::stderr`]).

use std::collections::BTreeSet;
use std::os::fd::OwnedFd;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use harnix_authz::{ChildBindings, ChildSubject};
use harnix_contract::s1::ExtId;
use harnix_contract::RpcError;
use harnix_host_api::{ChildEvent, ChildHandle, ChildRequest, ChildRun, ChildSandbox, RestartPolicy};
use harnix_kernel::Rpc;
use harnix_supervise::sandbox::{self, Mechanism};
use harnix_supervise::{ChildSpec, SandboxSpec, Supervisor, Tail};
use serde_json::Value;
use tokio::sync::mpsc;

use super::Record;

/// The subcommand an extension process is.
pub(crate) const SUBCOMMAND: &str = "ext-proc";

/// Where an extension process finds programs: the system's own, whatever this process was started
/// with.
pub(crate) const SYSTEM_PATH: &str = "/usr/bin:/bin:/usr/sbin:/sbin";

/// What of this process's environment an extension process is handed, when it is set.
const PASSED: &[&str] = &["LANG", "RUST_LOG"];

/// The key in a component's own section that turns its processes' sandbox off: `sandbox = "off"`.
const SANDBOX_KEY: &str = "sandbox";

/// The product's ends of one start's streams.
type Ends = (UnixStream, Option<UnixStream>);

/// The components whose own section turns the sandbox off: nothing but `sandbox = "off"` does,
/// and whatever else is written there is the sandbox required. Each is warned of once, here.
pub(crate) fn unsandboxed(records: &[Record]) -> BTreeSet<String> {
    records
        .iter()
        .filter(|r| r.config.get(SANDBOX_KEY).and_then(Value::as_str) == Some("off"))
        .map(|r| {
            let id = r.manifest.id().to_string();
            tracing::warn!(component = %id, "`sandbox = \"off\"`: the extension processes it starts are not sandboxed; for development only");
            id
        })
        .collect()
}

/// The sandbox a component's processes start in: `off` when its section says so, else how this
/// system applies one, or why it cannot.
pub(crate) fn sandboxing(off: bool) -> ChildSandbox {
    if off {
        return ChildSandbox::Off;
    }
    match sandbox::available() {
        Ok(Mechanism::SandboxExec) => ChildSandbox::On("sandbox-exec".into()),
        Ok(Mechanism::Landlock { abi }) => ChildSandbox::On(format!("landlock (ABI {abi})")),
        Err(why) => ChildSandbox::Unavailable(why.to_string()),
    }
}

/// Start the extension process `req` asks for, as `id`, kept by a supervisor; in its sandbox
/// unless `unsandboxed`. `replay`: where this process plays recordings back from, if it does.
pub(crate) fn spawn(children: &Arc<ChildBindings>, id: &ExtId, req: ChildRequest, unsandboxed: bool, replay: Option<&Path>) -> Rpc<ChildHandle> {
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
    let jail = match sandboxing(unsandboxed) {
        ChildSandbox::Unavailable(why) => return Err(refused(format!("it cannot be sandboxed here: {why}"))),
        ChildSandbox::Off => {
            tracing::warn!(extension = %id, "its extension process starts unsandboxed (`sandbox = \"off\"`)");
            None
        }
        // Absolute, as a sandbox's paths are; a relative one is read from where this process runs.
        _ => Some(sandbox_spec(&pkg, &run, replay.and_then(|dir| std::path::absolute(dir).ok()).as_deref())),
    };
    let env = environment(&run, |name| std::env::var(name).ok());
    let args = arguments(id, &pkg, &run, req.has_engine());
    let subject = children.bind_child(id);
    let (ends_tx, ends) = mpsc::unbounded_channel::<Ends>();
    let with_engine = req.has_engine();
    // One tail for the run: an exit's last lines are still there once it is started again.
    let tail = Tail::extension(id.to_string());
    let stderr = tail.clone();
    let make = move || {
        let (product, process) = UnixStream::pair()?;
        let mut spec = ChildSpec::new(&program)
            .args(args.iter().cloned())
            .env_clear()
            .envs(env.iter().cloned())
            .current_dir(&run)
            .inherit_fd(3, OwnedFd::from(process))
            .own_session()
            .stderr_to(stderr.clone());
        if let Some(jail) = &jail {
            spec = spec.sandbox(jail.clone());
        }
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
    Ok(ChildHandle::new(id.clone(), Box::new(Run { supervisor: Some(supervisor), ends, pending: None, tail, _subject: subject })))
}

/// What the process reaches of the state: its run directory, read and written (its data, kv, own
/// engine, `HOME`, `TMPDIR`), and its package, read. Not the directory both are in, which holds
/// its consent record. The program and what any program needs are the sandbox's to add. Its skills
/// directory (beside its package's), read, when it has one: its own engine lists it, and the rules
/// are made from the paths that exist as it starts — a package that gets skills is restarted. In
/// replay, the recordings too, read: its own engine plays them back as every other engine of the
/// product.
fn sandbox_spec(pkg: &Path, run: &Path, replay: Option<&Path>) -> SandboxSpec {
    let mut spec = SandboxSpec::new().read_write(run).read_only(pkg);
    if let Some(skills) = pkg.parent().map(|dir| dir.join("skills")).filter(|dir| dir.is_dir()) {
        spec = spec.read_only(skills);
    }
    match replay {
        Some(dir) => spec.read_only(dir),
        None => spec,
    }
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
    /// The last lines of its stderr.
    tail: Tail,
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

    fn stderr(&self) -> Vec<String> {
        self.tail.lines()
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
    fn the_process_reaches_its_run_directory_and_reads_its_package() {
        let spec = sandbox_spec(Path::new("/s/ext/acme.notes/1.0.0"), Path::new("/s/ext/acme.notes/run"), None);
        assert_eq!(spec.read_write, [Path::new("/s/ext/acme.notes/run")]);
        assert_eq!(spec.read_only, [Path::new("/s/ext/acme.notes/1.0.0")]);
        // In replay its engine reads the recordings, and writes nothing there.
        let spec = sandbox_spec(Path::new("/s/ext/acme.notes/1.0.0"), Path::new("/s/ext/acme.notes/run"), Some(Path::new("/fixtures/recordings")));
        assert_eq!(spec.read_write, [Path::new("/s/ext/acme.notes/run")]);
        assert_eq!(spec.read_only, [Path::new("/s/ext/acme.notes/1.0.0"), Path::new("/fixtures/recordings")]);
    }

    #[test]
    fn it_also_reads_its_skills_directory_when_it_has_one() {
        let state = std::env::current_exe().unwrap().ancestors().nth(3).unwrap().join("tmp");
        std::fs::create_dir_all(&state).unwrap();
        let state = tempfile::tempdir_in(state).unwrap();
        let (pkg, run) = (state.path().join("1.0.0"), state.path().join("run"));
        assert_eq!(sandbox_spec(&pkg, &run, None).read_only, std::slice::from_ref(&pkg));
        std::fs::create_dir_all(state.path().join("skills")).unwrap();
        assert_eq!(sandbox_spec(&pkg, &run, None).read_only, [pkg.clone(), state.path().join("skills")]);
    }

    #[test]
    fn only_off_turns_the_sandbox_off() {
        assert_eq!(sandboxing(true), ChildSandbox::Off);
        assert_ne!(sandboxing(false), ChildSandbox::Off);
        if cfg!(target_os = "macos") {
            assert_eq!(sandboxing(false), ChildSandbox::On("sandbox-exec".into()));
        }
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
