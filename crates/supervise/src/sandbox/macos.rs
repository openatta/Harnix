//! macOS: the child is exec'd as `sandbox-exec -p <profile> <program> …`, so it starts inside the
//! sandbox and cannot leave it; what it starts inherits it. Descriptors it inherited survive both
//! execs.
//!
//! `sandbox-exec` is marked deprecated by Apple and still ships. Were it taken away, a sandboxed
//! spawn fails saying so, and this becomes a call to `sandbox_init`.

use std::io;
use std::path::{Path, PathBuf};

use super::{text, SandboxSpec};

/// The tool, at the one place it is ever installed.
pub const SANDBOX_EXEC: &str = "/usr/bin/sandbox-exec";

/// Where the shells and tools the child starts are: run inside its sandbox, read and executed.
const SYSTEM_TOOLS: &[&str] = &["/bin", "/usr/bin"];

/// What the system's tools read to find their way, read only: `xcode-select`'s choice of developer
/// directory, which `/usr/bin/git` and the other shims in `/usr/bin` go through, and
/// `/private/var/select`, where `/bin/sh` learns which shell it is.
const SYSTEM_READ: &[&str] = &["/private/var/db/xcode_select_link", "/private/var/select"];

/// Mach services the child looks up: certificate trust (`trustd`) and name resolution
/// (`mDNSResponder`), for the model its own engine calls.
const MACH_SERVICES: &[&str] = &[
    "com.apple.trustd",
    "com.apple.trustd.agent",
    "com.apple.dnssd.service",
    "com.apple.mDNSResponder",
];

#[cfg(target_os = "macos")]
pub(crate) fn available() -> Result<(), super::Unavailable> {
    use std::os::unix::fs::PermissionsExt;
    match std::fs::metadata(SANDBOX_EXEC) {
        Ok(m) if m.is_file() && m.permissions().mode() & 0o111 != 0 => Ok(()),
        _ => Err(super::Unavailable(format!(
            "{SANDBOX_EXEC} is not on this system, so child processes cannot be sandboxed"
        ))),
    }
}

/// The developer directory the shims in `/usr/bin` run their tools from (`xcode-select -p`, asked
/// once, with nothing of this process's environment, as the child has none of it): read and
/// executed in the sandbox. `None` when there is none, and then a shim fails as it would anywhere.
fn developer_dir() -> Option<&'static Path> {
    static DIR: std::sync::OnceLock<Option<PathBuf>> = std::sync::OnceLock::new();
    DIR.get_or_init(|| {
        if !cfg!(target_os = "macos") {
            return None;
        }
        let out = std::process::Command::new("/usr/bin/xcode-select").arg("-p").env_clear().output().ok()?;
        let dir = String::from_utf8(out.stdout).ok()?;
        let dir = Path::new(dir.trim());
        (out.status.success() && dir.is_absolute()).then(|| std::fs::canonicalize(dir).ok()).flatten()
    })
    .as_deref()
}

/// The profile `program` runs under: everything denied but what `bsd.sb` lets any program do
/// (the system's libraries, the dyld cache, `/dev/null`, `/dev/urandom`, file metadata) and what
/// is added here. A path is named by its real path, which is what the sandbox compares
/// (`/var/folders/…` is `/private/var/folders/…`); `program` is taken to be one already.
pub fn profile(spec: &SandboxSpec, program: &Path) -> io::Result<String> {
    let program = quote(text(program)?);
    let subpaths = |paths: &[PathBuf]| -> io::Result<String> {
        let mut out = String::new();
        for path in paths {
            let real = std::fs::canonicalize(path).unwrap_or_else(|_| path.clone());
            out.push_str(&format!(" (subpath {})", quote(text(&real)?)));
        }
        Ok(out)
    };
    let mut tools: String = SYSTEM_TOOLS.iter().map(|t| format!(" (subpath {})", quote(t))).collect();
    if let Some(dir) = developer_dir() {
        tools.push_str(&format!(" (subpath {})", quote(text(dir)?)));
    }
    let system: String = SYSTEM_READ.iter().map(|p| format!(" (subpath {})", quote(p))).collect();
    let read_write = subpaths(&spec.read_write)?;
    let read_only = subpaths(&spec.read_only)?;
    let services: String = MACH_SERVICES.iter().map(|s| format!(" (global-name {})", quote(s))).collect();

    let mut p = String::from("(version 1)\n(deny default)\n(import \"bsd.sb\")\n");
    p.push_str("(allow process-fork)\n");
    p.push_str(&format!("(allow process-exec (literal {program}){tools})\n"));
    p.push_str(&format!("(allow file-read* (literal {program}){tools}{system}{read_only}{read_write})\n"));
    // A rule with no filter allows everything: none at all when there is nothing to write.
    if !read_write.is_empty() {
        p.push_str(&format!("(allow file-write*{read_write})\n"));
    }
    p.push_str("(allow file-read* file-write-data (literal \"/dev/null\") (literal \"/dev/urandom\"))\n");
    p.push_str("(allow network*)\n");
    p.push_str(&format!("(allow mach-lookup{services})\n"));
    p.push_str("(allow sysctl-read)\n");
    p.push_str("(allow process-info* (target self))\n");
    // Its own children, which share this sandbox; not the product, nor anything else.
    p.push_str("(allow signal (target same-sandbox))\n");
    Ok(p)
}

/// `s` as a profile's string literal.
fn quote(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_profile_lets_the_child_reach_its_own_paths_and_no_others() {
        let spec = SandboxSpec::new().read_write("/s/ext/a.b/run").read_only("/s/ext/a.b/0.1.0");
        let p = profile(&spec, Path::new("/opt/product")).unwrap();
        assert!(p.starts_with("(version 1)\n(deny default)\n"), "{p}");
        assert!(p.contains("(allow process-exec (literal \"/opt/product\") (subpath \"/bin\") (subpath \"/usr/bin\")"), "{p}");
        assert!(p.contains("(allow file-write* (subpath \"/s/ext/a.b/run\"))"), "{p}");
        assert!(p.contains("(subpath \"/s/ext/a.b/0.1.0\") (subpath \"/s/ext/a.b/run\"))"), "{p}");
        assert!(!p.contains("file-write* (subpath \"/s/ext/a.b/0.1.0\")"), "{p}");
    }

    #[test]
    fn what_the_systems_tools_read_is_read_only() {
        let p = profile(&SandboxSpec::new().read_write("/run"), Path::new("/opt/product")).unwrap();
        let rule = |allow: &str| p.lines().find(|l| l.starts_with(allow)).unwrap_or_default().to_string();
        for path in SYSTEM_READ {
            let path = format!("(subpath \"{path}\")");
            assert!(rule("(allow file-read*").contains(&path), "{p}");
            assert!(!rule("(allow file-write*").contains(&path), "{p}");
            assert!(!rule("(allow process-exec").contains(&path), "{p}");
        }
    }

    #[test]
    fn nothing_to_write_is_no_write_rule_at_all() {
        let p = profile(&SandboxSpec::new().read_only("/pkg"), Path::new("/opt/product")).unwrap();
        assert!(!p.contains("(allow file-write*"), "{p}");
    }

    #[test]
    fn a_path_is_quoted() {
        let p = profile(&SandboxSpec::new().read_write("/a \"b\"\\c"), Path::new("/p")).unwrap();
        assert!(p.contains(r#"(subpath "/a \"b\"\\c")"#), "{p}");
    }
}
