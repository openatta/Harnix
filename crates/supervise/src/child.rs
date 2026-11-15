//! One child: how it is spawned and how it is ended.

use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::path::PathBuf;
use std::process::{ExitStatus, Stdio};
use std::time::Duration;

use tokio::process::Command;

use crate::sandbox::{self, SandboxSpec};
use crate::tail::Tail;

/// What to run and what it is given. Its stdin is a pipe held by [`Child`]: the child is meant to
/// exit when the pipe closes, which is also what happens when this process is killed outright —
/// nothing else reliably outlives a SIGKILL to tell it. Its stdout goes nowhere; its stderr is
/// this process's, or read into a [`Tail`] and this process's log when it is given one.
#[derive(Debug)]
#[non_exhaustive]
pub struct ChildSpec {
    pub program: PathBuf,
    pub args: Vec<String>,
    /// Start from an empty environment rather than this process's: only `env` reaches the child.
    pub env_clear: bool,
    pub env: Vec<(String, String)>,
    /// Where the child starts; this process's own directory when unset.
    pub current_dir: Option<PathBuf>,
    /// Descriptors the child gets, each at the number it is to have there. Moved in: this process
    /// closes its copies once the child is started.
    pub inherit: Vec<(RawFd, OwnedFd)>,
    /// The child leads a session of its own, so that [`Child::reap`] can end whatever it leaves.
    pub own_session: bool,
    /// Started inside this sandbox, or not at all ([`crate::sandbox`]).
    pub sandbox: Option<SandboxSpec>,
    /// Where its stderr goes, a line at a time: kept, the last few, and logged.
    pub stderr: Option<Tail>,
}

impl ChildSpec {
    pub fn new(program: impl Into<PathBuf>) -> Self {
        Self {
            program: program.into(),
            args: Vec::new(),
            env_clear: false,
            env: Vec::new(),
            current_dir: None,
            inherit: Vec::new(),
            own_session: false,
            sandbox: None,
            stderr: None,
        }
    }

    pub fn args(mut self, args: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.args.extend(args.into_iter().map(Into::into));
        self
    }

    pub fn envs(mut self, env: impl IntoIterator<Item = (String, String)>) -> Self {
        self.env.extend(env);
        self
    }

    /// The child gets no variable of this process's environment, only the ones given with
    /// [`ChildSpec::envs`].
    pub fn env_clear(mut self) -> Self {
        self.env_clear = true;
        self
    }

    pub fn current_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.current_dir = Some(dir.into());
        self
    }

    /// Hand the child `fd` as its descriptor number `at` (3 and up; 0, 1 and 2 are the child's
    /// stdio).
    pub fn inherit_fd(mut self, at: RawFd, fd: OwnedFd) -> Self {
        self.inherit.push((at, fd));
        self
    }

    pub fn own_session(mut self) -> Self {
        self.own_session = true;
        self
    }

    pub fn sandbox(mut self, spec: SandboxSpec) -> Self {
        self.sandbox = Some(spec);
        self
    }

    /// Read its stderr into `tail`, and on into this process's log, rather than hand it this
    /// process's.
    pub fn stderr_to(mut self, tail: Tail) -> Self {
        self.stderr = Some(tail);
        self
    }

    /// Fork + exec. Every descriptor this process has open, apart from the ones in `inherit` and
    /// stdio, is closed in the child before the exec, whether or not it was opened `CLOEXEC`.
    /// With a sandbox that cannot be had here, nothing is started: the error says why.
    pub fn spawn(self) -> io::Result<Child> {
        let wrapped = match &self.sandbox {
            Some(spec) => Some(sandbox::wrap(spec, &self.program, &self.args)?),
            None => None,
        };
        let (program, args) = match &wrapped {
            Some(w) => (&w.program, &w.args),
            None => (&self.program, &self.args),
        };
        // Held open until the child is started: it restricts itself to it before the exec.
        #[cfg(target_os = "linux")]
        let ruleset = wrapped.as_ref().and_then(|w| w.ruleset.as_ref()).map(|r| r.fd());
        let mut targets: Vec<RawFd> = self.inherit.iter().map(|(at, _)| *at).collect();
        targets.sort_unstable();
        if targets.first().is_some_and(|&t| t < 3) {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "inherited descriptors start at 3"));
        }
        if targets.windows(2).any(|w| w[0] == w[1]) {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "two descriptors for one number"));
        }
        // Sources are moved above every target first, so that placing one cannot overwrite
        // another that has yet to be placed.
        let floor = targets.last().map_or(3, |t| t + 1);
        let mut moves: Vec<(RawFd, RawFd)> = Vec::new();
        let mut lifted: Vec<OwnedFd> = Vec::new();
        for (at, fd) in &self.inherit {
            // SAFETY: F_DUPFD_CLOEXEC on a descriptor we own; the new one is owned below.
            let high = unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_DUPFD_CLOEXEC, floor) };
            if high < 0 {
                return Err(io::Error::last_os_error());
            }
            // SAFETY: `high` was just returned by fcntl and nothing else owns it.
            lifted.push(unsafe { OwnedFd::from_raw_fd(high) });
            moves.push((high, *at));
        }
        let open = open_fds();
        let mut command = Command::new(program);
        if self.env_clear {
            command.env_clear();
        }
        if let Some(dir) = &self.current_dir {
            command.current_dir(dir);
        }
        command
            .args(args)
            .envs(self.env.iter().cloned())
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(if self.stderr.is_some() { Stdio::piped() } else { Stdio::inherit() })
            .kill_on_drop(true);
        if self.own_session {
            own_session_for(&mut command);
        }
        // SAFETY: the closure only calls prctl, the Landlock syscall, dup2 and close, all
        // async-signal-safe, and allocates nothing: `moves`, `targets`, `open` and the ruleset are
        // made before the fork.
        unsafe {
            command.pre_exec(move || {
                // First, before a move could put an inherited descriptor at the ruleset's number.
                #[cfg(target_os = "linux")]
                if let Some(ruleset) = ruleset {
                    sandbox::linux::restrict(ruleset)?;
                }
                for &(from, to) in &moves {
                    if libc::dup2(from, to) < 0 {
                        return Err(io::Error::last_os_error());
                    }
                }
                for &fd in &open {
                    if fd > 2 && targets.binary_search(&fd).is_err() {
                        libc::close(fd);
                    }
                }
                Ok(())
            });
        }
        let spawned = command.spawn();
        drop(wrapped);
        drop(lifted);
        drop(self.inherit);
        let mut child = spawned?;
        let stdin = child.stdin.take();
        let pid = child.id();
        let stderr = match (self.stderr, child.stderr.take()) {
            (Some(tail), Some(from)) => Some(tokio::spawn(tail.follow(from, pid.unwrap_or(0)))),
            _ => None,
        };
        Ok(Child { inner: child, stdin, pid, own_session: self.own_session, stderr })
    }
}

/// Descriptors open in this process now.
fn open_fds() -> Vec<RawFd> {
    let Ok(dir) = std::fs::read_dir("/dev/fd") else {
        return Vec::new();
    };
    dir.filter_map(|e| e.ok()?.file_name().to_str()?.parse().ok()).collect()
}

/// Make the child about to be spawned the leader of a session of its own.
pub(crate) fn own_session_for(command: &mut Command) {
    // SAFETY: setsid(2) is async-signal-safe, which is all pre_exec asks of the closure.
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
}

/// A running child. Dropping it kills it.
pub struct Child {
    inner: tokio::process::Child,
    stdin: Option<tokio::process::ChildStdin>,
    pid: Option<u32>,
    own_session: bool,
    /// What reads its stderr into its tail, until the last writer closes it.
    stderr: Option<tokio::task::JoinHandle<()>>,
}

impl Child {
    /// Kept after the child is waited for, when the process no longer has an id of its own.
    pub fn id(&self) -> Option<u32> {
        self.pid
    }

    pub fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        self.inner.try_wait()
    }

    pub async fn wait(&mut self) -> io::Result<ExitStatus> {
        self.inner.wait().await
    }

    /// Kill it outright (SIGKILL), and wait for it.
    pub async fn kill(&mut self) -> io::Result<()> {
        self.inner.kill().await
    }

    /// Wait up to `within` for its stderr to be read to the end, so that its tail has its last
    /// lines: once it is gone, and what it left in its session with it.
    pub async fn drained(&mut self, within: Duration) {
        if let Some(reading) = self.stderr.as_mut() {
            if tokio::time::timeout(within, reading).await.is_ok() {
                self.stderr = None;
            }
        }
    }

    /// End whatever the child left in its session. Nothing when it did not lead one.
    pub async fn reap(&self) -> usize {
        match self.pid {
            Some(pid) if self.own_session => crate::reap::reap_session(pid as i32, crate::reap::REAP_GRACE).await,
            _ => 0,
        }
    }

    /// Ask the child to finish by closing its stdin and wait up to `grace`; then TERM, a shorter
    /// grace ([`TERM_GRACE`]), KILL; then reap its session.
    pub async fn stop(mut self, grace: Duration) -> Option<ExitStatus> {
        drop(self.stdin.take());
        let status = match tokio::time::timeout(grace, self.inner.wait()).await {
            Ok(status) => status.ok(),
            Err(_) => self.terminate(TERM_GRACE).await,
        };
        self.reap().await;
        status
    }

    /// TERM, up to `grace` for it to go, KILL.
    async fn terminate(&mut self, grace: Duration) -> Option<ExitStatus> {
        // Not waited for yet, so the pid is still this child's.
        if let Some(pid) = self.inner.id() {
            // SAFETY: kill(2) on our own unreaped child; the worst case is ESRCH.
            unsafe {
                libc::kill(pid as i32, libc::SIGTERM);
            }
        }
        match tokio::time::timeout(grace, self.inner.wait()).await {
            Ok(status) => status.ok(),
            Err(_) => {
                let _ = self.inner.kill().await;
                self.inner.wait().await.ok()
            }
        }
    }
}

/// How long a child that did not finish when asked has between TERM and KILL: what the processes
/// left in its session get too.
pub const TERM_GRACE: Duration = crate::reap::REAP_GRACE;
