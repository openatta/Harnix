//! Linux: Landlock, applied by the process to itself.
//!
//! `PR_SET_NO_NEW_PRIVS` first (Landlock asks for it, and nothing this process starts gains
//! privileges by exec), then a ruleset at the highest ABI the kernel has that handles every file
//! system right it knows, so whatever a rule does not grant is denied: the product's paths, and
//! what any program needs to run here ([`SYSTEM`]). From ABI 6 the ruleset also scopes signals
//! and abstract unix sockets to this process's own domain, so it cannot signal the product. The
//! network is not handled: it stays open. What this process starts inherits all of it.

use std::ffi::CString;
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

use anyhow::{anyhow, bail, Context};

use super::{Access, Spec};

/// What any program needs to run here, besides what the product named: the system's libraries
/// and tools (the shells it starts), the loader's cache, TLS roots and name resolution for the
/// model its own engine calls, `/dev/null` and `/dev/urandom`. What this system lacks is left out.
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
const CREATE_RULESET_VERSION: u32 = 1 << 0;
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

pub(super) fn apply(spec: &Spec) -> anyhow::Result<()> {
    one_thread()?;
    // SAFETY: prctl with PR_SET_NO_NEW_PRIVS takes plain integers.
    if unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) } != 0 {
        bail!("PR_SET_NO_NEW_PRIVS: {}", io::Error::last_os_error());
    }
    let abi = abi()?;
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
    let ruleset = unsafe { libc::syscall(libc::SYS_landlock_create_ruleset, &attr as *const RulesetAttr, size, 0u32) };
    if ruleset < 0 {
        bail!("Landlock: no ruleset: {}", io::Error::last_os_error());
    }
    let ruleset = ruleset as libc::c_int;
    let added = (|| {
        for (access, path) in &spec.0 {
            add(ruleset, *access, path, handled).with_context(|| format!("Landlock: {}", path.display()))?;
        }
        for (access, path) in SYSTEM {
            let path = Path::new(path);
            if path.exists() {
                add(ruleset, *access, path, handled).with_context(|| format!("Landlock: {}", path.display()))?;
            }
        }
        // SAFETY: a ruleset descriptor this function made; no flags.
        if unsafe { libc::syscall(libc::SYS_landlock_restrict_self, ruleset, 0u32) } != 0 {
            bail!("Landlock: not applied: {}", io::Error::last_os_error());
        }
        Ok(())
    })();
    // SAFETY: closed once, here; nothing else holds it.
    unsafe { libc::close(ruleset) };
    added
}

/// The process is sandboxed by its thread: one that ran before would be left out.
fn one_thread() -> anyhow::Result<()> {
    let threads = std::fs::read_dir("/proc/self/task").map(|d| d.count()).unwrap_or(1);
    if threads > 1 {
        bail!("it has {threads} threads before it is sandboxed, and only the one sandboxing itself would be");
    }
    Ok(())
}

/// The highest ABI this kernel has; none is a refusal.
fn abi() -> anyhow::Result<u32> {
    // SAFETY: with a null attribute, size 0 and the VERSION flag, the call reads nothing.
    let abi = unsafe {
        libc::syscall(libc::SYS_landlock_create_ruleset, std::ptr::null::<libc::c_void>(), 0usize, CREATE_RULESET_VERSION)
    };
    if abi < 1 {
        return Err(anyhow!("this kernel has no Landlock, or has it turned off: {}", io::Error::last_os_error()));
    }
    Ok(abi as u32)
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

fn add(ruleset: libc::c_int, access: Access, path: &Path, handled: u64) -> anyhow::Result<()> {
    let c = CString::new(path.as_os_str().as_bytes())?;
    // SAFETY: a path the CString keeps NUL-terminated; O_PATH opens nothing but a reference.
    let fd = unsafe { libc::open(c.as_ptr(), libc::O_PATH | libc::O_CLOEXEC) };
    if fd < 0 {
        bail!("{}", io::Error::last_os_error());
    }
    let mut allowed = rights(access, handled);
    if !path.is_dir() {
        allowed &= FS_ON_A_FILE;
    }
    let rule = PathBeneathAttr { allowed_access: allowed, parent_fd: fd };
    // SAFETY: `rule` outlives the call; `fd` is closed right after.
    let added = unsafe {
        libc::syscall(libc::SYS_landlock_add_rule, ruleset, RULE_PATH_BENEATH, &rule as *const PathBeneathAttr, 0u32)
    };
    let e = io::Error::last_os_error();
    // SAFETY: opened above, closed once.
    unsafe { libc::close(fd) };
    if added != 0 {
        bail!("{e}");
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
