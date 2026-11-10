//! The operating-system sandbox, as far as this process takes part in it.
//!
//! On macOS there is nothing to do: the product exec'd this process inside `sandbox-exec`, which
//! it cannot leave. On Linux the process sandboxes itself, first thing, before it reads its
//! package ([`linux`]): the product hands it what it may reach in [`ENV`], and a process that was
//! handed it and cannot apply it does not run.

#[cfg(target_os = "linux")]
mod linux;

use std::path::PathBuf;

use anyhow::{anyhow, bail};

/// Where the product puts the spec (`harnix_supervise::sandbox::ENV`, which writes it).
pub(crate) const ENV: &str = "HARNIX_SANDBOX";

/// What a path is open to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(crate) enum Access {
    ReadWrite,
    ReadOnly,
    /// Read, and run: the program, the system's libraries and tools.
    ReadExec,
}

/// The paths the product named, in the order it named them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Spec(pub(crate) Vec<(Access, PathBuf)>);

impl Spec {
    /// A line a path: `rw`, `ro` or `rx`, a tab, the absolute path.
    pub(crate) fn parse(text: &str) -> anyhow::Result<Self> {
        let mut paths = Vec::new();
        for line in text.lines().filter(|l| !l.is_empty()) {
            let (access, path) = line.split_once('\t').ok_or_else(|| anyhow!("`{ENV}`: `{line}` is not `<access>\\t<path>`"))?;
            let access = match access {
                "rw" => Access::ReadWrite,
                "ro" => Access::ReadOnly,
                "rx" => Access::ReadExec,
                other => bail!("`{ENV}`: `{other}` is not an access (`rw`, `ro`, `rx`)"),
            };
            let path = PathBuf::from(path);
            if !path.is_absolute() {
                bail!("`{ENV}`: {} is not absolute", path.display());
            }
            paths.push((access, path));
        }
        Ok(Self(paths))
    }
}

/// Sandbox this process as the product asked, or say why it cannot be. Nothing when the product
/// asked for nothing. Called while the process has one thread: what a sandbox applies to is the
/// calling thread and what it starts afterwards.
pub(crate) fn apply() -> anyhow::Result<()> {
    let Some(text) = std::env::var_os(ENV) else {
        return Ok(());
    };
    // Its own children inherit the sandbox itself, not the request for one.
    std::env::remove_var(ENV);
    let text = text.into_string().map_err(|_| anyhow!("`{ENV}` is not UTF-8"))?;
    let spec = Spec::parse(&text)?;
    #[cfg(target_os = "linux")]
    {
        linux::apply(&spec)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = spec;
        bail!("`{ENV}` asks this process to sandbox itself, which it does on Linux alone")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_spec_is_read_a_line_a_path() {
        let spec = Spec::parse("rx\t/bin/product\nrw\t/s/ext/a.b/run\nro\t/s/ext/a.b/0.1.0\n").unwrap();
        assert_eq!(
            spec.0,
            [
                (Access::ReadExec, "/bin/product".into()),
                (Access::ReadWrite, "/s/ext/a.b/run".into()),
                (Access::ReadOnly, "/s/ext/a.b/0.1.0".into()),
            ]
        );
    }

    #[test]
    fn a_spec_it_cannot_read_is_refused() {
        for (text, says) in [("rw /a", "not `<access>"), ("rwx\t/a", "not an access"), ("ro\ta", "not absolute")] {
            let e = Spec::parse(text).unwrap_err().to_string();
            assert!(e.contains(says), "{text:?}: {e}");
        }
    }
}
