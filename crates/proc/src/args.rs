//! The command line an extension process is started with:
//!
//! ```text
//! <program> ext-proc --ext <id> [--pkg <dir>] [--run <dir>] [--wire-fd 3] [--engine-fd 4] [--dev]
//! ```
//!
//! `ext-proc` itself is the product's to dispatch on; what reaches [`crate::entry`] is the rest.

use std::ffi::OsString;
use std::os::fd::RawFd;
use std::path::PathBuf;

use anyhow::{anyhow, bail, Context};

/// Where the extension wire is unless said otherwise: the first descriptor after stdio.
pub const WIRE_FD: RawFd = 3;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Args {
    /// The extension this process is: what `proc.hello` says, and what the product bound fd 3 to.
    pub ext: String,
    /// Its unpacked package, read-only.
    pub pkg: Option<PathBuf>,
    /// Its run directory: the working directory.
    pub run: Option<PathBuf>,
    pub wire_fd: RawFd,
    /// The wire its own engine is served on, when it has one.
    pub engine_fd: Option<RawFd>,
    /// Unsigned code runs: the profile's `dev`, until a start carries how far a package is trusted.
    pub dev: bool,
}

impl Args {
    pub fn parse(args: Vec<OsString>) -> anyhow::Result<Self> {
        let mut ext = None;
        let mut pkg = None;
        let mut run = None;
        let mut wire_fd = WIRE_FD;
        let mut engine_fd = None;
        let mut dev = false;
        let mut args = args.into_iter();
        while let Some(arg) = args.next() {
            let arg = arg.into_string().map_err(|a| anyhow!("{a:?} is not UTF-8"))?;
            let mut value = || args.next().ok_or_else(|| anyhow!("`{arg}` takes a value"));
            match arg.as_str() {
                "--ext" => ext = Some(value()?.into_string().map_err(|a| anyhow!("--ext {a:?} is not UTF-8"))?),
                "--pkg" => pkg = Some(PathBuf::from(value()?)),
                "--run" => run = Some(PathBuf::from(value()?)),
                "--wire-fd" => wire_fd = fd(&arg, value()?)?,
                "--engine-fd" => engine_fd = Some(fd(&arg, value()?)?),
                "--dev" => dev = true,
                other => bail!("`{other}` is not an argument of `ext-proc`"),
            }
        }
        let ext = ext.ok_or_else(|| anyhow!("`ext-proc` is started with `--ext <id>`"))?;
        if engine_fd == Some(wire_fd) {
            bail!("the extension wire and the engine's are two descriptors, not one ({wire_fd})");
        }
        Ok(Self {
            ext,
            pkg,
            run,
            wire_fd,
            engine_fd,
            dev,
        })
    }
}

/// A descriptor number an inherited stream is at: 3 and up.
fn fd(arg: &str, value: OsString) -> anyhow::Result<RawFd> {
    let n: RawFd = value
        .to_str()
        .and_then(|v| v.parse().ok())
        .with_context(|| format!("`{arg} {value:?}` is not a descriptor number"))?;
    if n < 3 {
        bail!("`{arg} {n}`: 0, 1 and 2 are stdio; an inherited stream is at 3 or above");
    }
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> anyhow::Result<Args> {
        Args::parse(args.iter().map(OsString::from).collect())
    }

    #[test]
    fn what_the_product_starts_it_with() {
        let args = parse(&["--ext", "example.showcase", "--pkg", "/p", "--run", "/r", "--wire-fd", "3", "--engine-fd", "4"]).unwrap();
        assert_eq!(
            args,
            Args {
                ext: "example.showcase".into(),
                pkg: Some("/p".into()),
                run: Some("/r".into()),
                wire_fd: 3,
                engine_fd: Some(4),
                dev: false,
            }
        );
        assert_eq!(parse(&["--ext", "a.b"]).unwrap().wire_fd, WIRE_FD);
    }

    #[test]
    fn what_it_refuses() {
        for (args, says) in [
            (&["--wire-fd", "3"][..], "--ext <id>"),
            (&["--ext"][..], "takes a value"),
            (&["--ext", "a.b", "--wire-fd", "1"][..], "stdio"),
            (&["--ext", "a.b", "--wire-fd", "x"][..], "not a descriptor"),
            (&["--ext", "a.b", "--engine-fd", "3"][..], "two descriptors"),
            (&["--ext", "a.b", "--profile", "p.toml"][..], "not an argument"),
        ] {
            let e = parse(args).unwrap_err().to_string();
            assert!(e.contains(says), "{args:?}: {e}");
        }
    }
}
