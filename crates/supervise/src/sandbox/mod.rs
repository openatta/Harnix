//! The operating-system sandbox a child is started in.
//!
//! A [`SandboxSpec`] names the child's own directories — read and write, or read only. The
//! mechanism adds what any program needs to run on the system: the program itself, the system's
//! libraries, `/bin` and `/usr/bin` for the shells it starts (which inherit the sandbox), what TLS
//! and DNS reach, `/dev/null` and `/dev/urandom`. Nothing else of the file system is reachable, and
//! the network stays open.
//!
//! | system | mechanism | applied |
//! |---|---|---|
//! | macOS | `sandbox-exec` with a profile made from the spec ([`macos::profile`]) | by the exec: `sandbox-exec -p <profile> <program> …` |
//! | Linux | Landlock and `PR_SET_NO_NEW_PRIVS` | by the child, first thing: the spec reaches it in [`ENV`] |
//! | anything else | — | never: a spec is refused |
//!
//! A child that cannot be sandboxed is not started unconfined: [`available`] says why, and
//! spawning with a spec fails with the same reason.

pub mod macos;

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

/// Where a child that sandboxes itself (Linux) finds its spec, as [`SandboxSpec::to_env`] writes
/// it. `harnix-proc` reads it under the same name.
pub const ENV: &str = "HARNIX_SANDBOX";

/// The child's own part of the file system. Paths are absolute; a directory covers what is under it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct SandboxSpec {
    pub read_write: Vec<PathBuf>,
    pub read_only: Vec<PathBuf>,
}

impl SandboxSpec {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn read_write(mut self, path: impl Into<PathBuf>) -> Self {
        self.read_write.push(path.into());
        self
    }

    pub fn read_only(mut self, path: impl Into<PathBuf>) -> Self {
        self.read_only.push(path.into());
        self
    }

    /// The spec as [`ENV`] carries it to a child that applies it itself: a line a path, its access
    /// (`rw`, `ro`, or `rx` — read and execute, which only `program` gets), a tab, the path.
    pub fn to_env(&self, program: &Path) -> io::Result<String> {
        let mut out = String::new();
        let lines = std::iter::once(("rx", program))
            .chain(self.read_write.iter().map(|p| ("rw", p.as_path())))
            .chain(self.read_only.iter().map(|p| ("ro", p.as_path())));
        for (access, path) in lines {
            out.push_str(access);
            out.push('\t');
            out.push_str(text(path)?);
            out.push('\n');
        }
        Ok(out)
    }

    /// Every path is absolute, UTF-8 and on one line: what a profile and [`ENV`] can carry.
    fn check(&self) -> io::Result<()> {
        for path in self.read_write.iter().chain(&self.read_only) {
            text(path)?;
        }
        Ok(())
    }
}

/// `path` as text a profile or [`ENV`] can carry.
pub(crate) fn text(path: &Path) -> io::Result<&str> {
    let refuse = |why: &str| io::Error::new(io::ErrorKind::InvalidInput, format!("sandbox path {}: {why}", path.display()));
    let s = path.to_str().ok_or_else(|| refuse("not UTF-8"))?;
    if !path.is_absolute() {
        return Err(refuse("not absolute"));
    }
    if s.contains(['\n', '\r', '\0']) {
        return Err(refuse("holds a line break"));
    }
    Ok(s)
}

/// How children are sandboxed here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mechanism {
    /// `/usr/bin/sandbox-exec`.
    SandboxExec,
    /// The highest Landlock ABI the kernel has.
    Landlock { abi: u32 },
}

/// Why a child cannot be sandboxed here: said to whoever asked for the sandbox, in a refusal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unavailable(pub String);

impl fmt::Display for Unavailable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Unavailable {}

impl From<Unavailable> for io::Error {
    fn from(e: Unavailable) -> Self {
        io::Error::new(io::ErrorKind::Unsupported, e.0)
    }
}

/// Whether children can be sandboxed here, and how. Linux also asks Yama's `ptrace_scope` to be
/// 1 or more: at 0 a process may trace any other of its user's, the product included.
pub fn available() -> Result<Mechanism, Unavailable> {
    #[cfg(target_os = "macos")]
    {
        macos::available().map(|()| Mechanism::SandboxExec)
    }
    #[cfg(target_os = "linux")]
    {
        let abi = landlock_abi()?;
        match ptrace_scope() {
            Some(scope) if scope >= 1 => Ok(Mechanism::Landlock { abi }),
            Some(_) => Err(Unavailable(
                "`kernel.yama.ptrace_scope` is 0: a sandboxed process could still trace the product (set it to 1)".into(),
            )),
            None => Err(Unavailable(
                "no Yama `ptrace_scope` on this kernel: a sandboxed process could still trace the product".into(),
            )),
        }
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        Err(Unavailable("there is no sandbox for child processes on this system".into()))
    }
}

/// Yama's `kernel.yama.ptrace_scope`; `None` where there is none (any system but Linux, or a kernel
/// without Yama).
pub fn ptrace_scope() -> Option<u32> {
    std::fs::read_to_string("/proc/sys/kernel/yama/ptrace_scope").ok()?.trim().parse().ok()
}

/// The highest Landlock ABI this kernel has; refused when it has none, or has it turned off.
#[cfg(target_os = "linux")]
fn landlock_abi() -> Result<u32, Unavailable> {
    const LANDLOCK_CREATE_RULESET_VERSION: libc::c_uint = 1 << 0;
    // SAFETY: with a null attribute, size 0 and the VERSION flag, the call reads nothing and only
    // answers the ABI version.
    let abi = unsafe {
        libc::syscall(
            libc::SYS_landlock_create_ruleset,
            std::ptr::null::<libc::c_void>(),
            0usize,
            LANDLOCK_CREATE_RULESET_VERSION,
        )
    };
    if abi < 1 {
        let e = io::Error::last_os_error();
        return Err(Unavailable(match e.raw_os_error() {
            Some(libc::ENOSYS) => "this kernel has no Landlock (5.13 or later has it)".into(),
            Some(libc::EOPNOTSUPP) => "Landlock is turned off on this kernel (`lsm=` on its command line)".into(),
            _ => format!("Landlock: {e}"),
        }));
    }
    Ok(abi as u32)
}

/// What a spawn runs instead of the bare program: the program, its arguments and what to add to
/// its environment.
pub(crate) struct Wrapped {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
}

/// Put `program args` under `spec`, or say why it cannot be.
pub(crate) fn wrap(spec: &SandboxSpec, program: &Path, args: &[String]) -> io::Result<Wrapped> {
    spec.check()?;
    let mechanism = available()?;
    // The program is named as the sandbox sees it: by its real path.
    let program = std::fs::canonicalize(program)
        .map_err(|e| io::Error::new(e.kind(), format!("the program to sandbox, {}: {e}", program.display())))?;
    match mechanism {
        Mechanism::SandboxExec => {
            let mut wrapped = vec!["-p".to_string(), macos::profile(spec, &program)?, text(&program)?.to_string()];
            wrapped.extend(args.iter().cloned());
            Ok(Wrapped { program: macos::SANDBOX_EXEC.into(), args: wrapped, env: Vec::new() })
        }
        Mechanism::Landlock { .. } => Ok(Wrapped {
            env: vec![(ENV.to_string(), spec.to_env(&program)?)],
            program,
            args: args.to_vec(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_spec_reaches_a_child_a_line_a_path() {
        let spec = SandboxSpec::new().read_write("/s/ext/a.b/run").read_only("/s/ext/a.b/0.1.0");
        assert_eq!(
            spec.to_env(Path::new("/bin/product")).unwrap(),
            "rx\t/bin/product\nrw\t/s/ext/a.b/run\nro\t/s/ext/a.b/0.1.0\n"
        );
    }

    #[test]
    fn what_a_spec_cannot_carry_is_refused() {
        for (path, says) in [("run", "not absolute"), ("/a\nrw\t/", "line break")] {
            let e = SandboxSpec::new().read_write(path).check().unwrap_err();
            assert_eq!(e.kind(), io::ErrorKind::InvalidInput);
            assert!(e.to_string().contains(says), "{path:?}: {e}");
        }
    }
}
