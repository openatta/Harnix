//! Linux: Landlock, applied by the supervisor between fork and exec, so the child is sandboxed
//! before its first instruction and cannot leave it out.
//!
//! The ruleset is built here, in this process, before the fork: a ruleset at the highest ABI the
//! kernel has that handles every file system right it knows, so whatever a rule does not grant is
//! denied — the spec's paths, the program, and what any program needs to run here ([`SYSTEM`]).
//! From ABI 6 it also scopes signals and abstract unix sockets to the child's own domain, so the
//! child cannot signal this process. The network is not handled: it stays open. In the child,
//! [`restrict`] only sets `PR_SET_NO_NEW_PRIVS` (Landlock asks for it, and nothing the child
//! starts gains privileges by exec) and restricts itself to the ruleset: two system calls, as
//! between fork and exec allows. What the child starts inherits all of it.

use std::ffi::CString;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

use super::SandboxSpec;

/// What a path is open to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Access {
    ReadWrite,
    ReadOnly,
    /// Read, and run: the program, the system's libraries and tools.
    ReadExec,
}

/// What any program needs to run here, besides what the spec names: the system's libraries and
/// tools (the shells it starts), the loader's cache, TLS roots and name resolution for the model
/// its own engine calls, `/dev/null` and `/dev/urandom`. What this system lacks is left out.
/// `/proc` is not among them: through it a process reaches every other of its user's.
const SYSTEM: &[(Access, &str)] = &[
    (Access::ReadExec, "/usr"),
    (Access::ReadExec, "/lib"),
    (Access::ReadExec, "/lib64"),
    (Access::ReadExec, "/lib32"),
    (Access::ReadExec, "/bin"),
    (Access::ReadOnly, "/etc/ld.so.cache"),
    (Access::ReadOnly, "/etc/ssl"),
    (Access::ReadOnly, "/etc/resolv.conf"),
    (Access::ReadOnly, "/etc/hosts"),
    (Access::ReadOnly, "/etc/nsswitch.conf"),
    (Access::ReadWrite, "/dev/null"),
    (Access::ReadWrite, "/dev/urandom"),
];

// <linux/landlock.h>
const RULE_PATH_BENEATH: libc::c_int = 1;
const FS_EXECUTE: u64 = 1 << 0;
const FS_WRITE_FILE: u64 = 1 << 1;
const FS_READ_FILE: u64 = 1 << 2;
const FS_READ_DIR: u64 = 1 << 3;
const FS_MAKE_CHAR: u64 = 1 << 6;
const FS_MAKE_BLOCK: u64 = 1 << 11;
/// ABI 1's thirteen rights, `EXECUTE` through `MAKE_SYM`.
const FS_ABI1: u64 = (1 << 13) - 1;
const FS_REFER: u64 = 1 << 13;
const FS_TRUNCATE: u64 = 1 << 14;
const FS_IOCTL_DEV: u64 = 1 << 15;
/// The rights that apply to a file that is not a directory; a rule on one may grant no others.
const FS_ON_A_FILE: u64 = FS_EXECUTE | FS_WRITE_FILE | FS_READ_FILE | FS_TRUNCATE | FS_IOCTL_DEV;
const SCOPE_ABSTRACT_UNIX_SOCKET: u64 = 1 << 0;
const SCOPE_SIGNAL: u64 = 1 << 1;

#[repr(C)]
struct RulesetAttr {
    handled_access_fs: u64,
    handled_access_net: u64,
    scoped: u64,
}

#[repr(C, packed)]
struct PathBeneathAttr {
    allowed_access: u64,
    parent_fd: i32,
}

/// A ruleset made for one spawn, held until the child is started. Its descriptor is close-on-exec
/// (the kernel opens every ruleset so): the child restricts itself to it before the exec, and
/// what it runs never holds it.
pub(crate) struct Ruleset(OwnedFd);

impl Ruleset {
    pub(crate) fn fd(&self) -> RawFd {
        self.0.as_raw_fd()
    }
}

/// The ruleset `program`, run under `spec`, is restricted to, at Landlock ABI `abi`.
pub(crate) fn ruleset(spec: &SandboxSpec, program: &Path, abi: u32) -> io::Result<Ruleset> {
    let handled = handled(abi);
    let attr = RulesetAttr {
        handled_access_fs: handled,
        handled_access_net: 0,
        scoped: if abi >= 6 { SCOPE_ABSTRACT_UNIX_SOCKET | SCOPE_SIGNAL } else { 0 },
    };
    // An ABI before 6 knows no `scoped`, and one before 4 no `handled_access_net`: it is handed
    // only the part it knows.
    let size = match abi {
        0..=3 => 8,
        4 | 5 => 16,
        _ => std::mem::size_of::<RulesetAttr>(),
    };
    // SAFETY: `attr` outlives the call, and `size` is no more than its size.
    let fd = unsafe { libc::syscall(libc::SYS_landlock_create_ruleset, &attr as *const RulesetAttr, size, 0u32) };
    if fd < 0 {
        return Err(landlock("no ruleset", io::Error::last_os_error()));
    }
    // SAFETY: a descriptor the call just returned, owned by nothing else.
    let ruleset = Ruleset(unsafe { OwnedFd::from_raw_fd(fd as RawFd) });
    let named = std::iter::once((Access::ReadExec, program))
        .chain(spec.read_write.iter().map(|p| (Access::ReadWrite, p.as_path())))
        .chain(spec.read_only.iter().map(|p| (Access::ReadOnly, p.as_path())));
    for (access, path) in named {
        add(&ruleset, access, path, handled).map_err(|e| landlock(&path.display().to_string(), e))?;
    }
    for &(access, path) in SYSTEM {
        let path = Path::new(path);
        if path.exists() {
            add(&ruleset, access, path, handled).map_err(|e| landlock(&path.display().to_string(), e))?;
        }
    }
    Ok(ruleset)
}

/// In the child, between fork and exec: no new privileges, then the ruleset. Only system calls,
/// which is all that may run there.
///
/// # Safety
///
/// `ruleset` is a Landlock ruleset descriptor open in the calling process.
pub(crate) unsafe fn restrict(ruleset: RawFd) -> io::Result<()> {
    if libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) != 0 {
        return Err(io::Error::last_os_error());
    }
    if libc::syscall(libc::SYS_landlock_restrict_self, ruleset, 0u32) != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn landlock(what: &str, e: io::Error) -> io::Error {
    io::Error::new(e.kind(), format!("Landlock: {what}: {e}"))
}

/// Every file system right ABI `abi` knows.
fn handled(abi: u32) -> u64 {
    let mut all = FS_ABI1;
    if abi >= 2 {
        all |= FS_REFER;
    }
    if abi >= 3 {
        all |= FS_TRUNCATE;
    }
    if abi >= 5 {
        all |= FS_IOCTL_DEV;
    }
    all
}

/// What `access` grants, of the rights `handled`. Read and write is everything but running a
/// file and making device nodes.
fn rights(access: Access, handled: u64) -> u64 {
    let rights = match access {
        Access::ReadOnly => FS_READ_FILE | FS_READ_DIR,
        Access::ReadExec => FS_READ_FILE | FS_READ_DIR | FS_EXECUTE,
        Access::ReadWrite => !(FS_EXECUTE | FS_MAKE_CHAR | FS_MAKE_BLOCK),
    };
    rights & handled
}

fn add(ruleset: &Ruleset, access: Access, path: &Path, handled: u64) -> io::Result<()> {
    let c = CString::new(path.as_os_str().as_bytes()).map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
    // SAFETY: a path the CString keeps NUL-terminated; O_PATH opens nothing but a reference.
    let fd = unsafe { libc::open(c.as_ptr(), libc::O_PATH | libc::O_CLOEXEC) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: just opened, owned by nothing else; closed when this returns.
    let fd = unsafe { OwnedFd::from_raw_fd(fd) };
    let mut allowed = rights(access, handled);
    if !path.is_dir() {
        allowed &= FS_ON_A_FILE;
    }
    let rule = PathBeneathAttr { allowed_access: allowed, parent_fd: fd.as_raw_fd() };
    // SAFETY: `rule` and `fd` outlive the call.
    let added = unsafe {
        libc::syscall(libc::SYS_landlock_add_rule, ruleset.fd(), RULE_PATH_BENEATH, &rule as *const PathBeneathAttr, 0u32)
    };
    if added != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_right_the_kernel_does_not_know_is_not_asked_for() {
        assert_eq!(rights(Access::ReadWrite, handled(1)) & (FS_REFER | FS_TRUNCATE), 0);
        assert_ne!(rights(Access::ReadWrite, handled(3)) & FS_TRUNCATE, 0);
        assert_eq!(rights(Access::ReadWrite, handled(6)) & FS_EXECUTE, 0);
        assert_eq!(rights(Access::ReadOnly, handled(6)), FS_READ_FILE | FS_READ_DIR);
    }
}
