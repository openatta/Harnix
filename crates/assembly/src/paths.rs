//! Where Harnix reads and writes.
//!
//! Two directories are configurable, because two are what a user actually
//! moves:
//!
//! - **`--engine-dir`** — `main`'s engine root: settings tiers, transcripts,
//!   memory, skills, recordings. Default `<state>/engine`, Harnix's own: an
//!   engine directory of another client's (`~/.agix`) is never read.
//! - **`--data-dir`** — the projects root: where the picker starts and where
//!   "new project" creates a directory. Default `~/Documents`. It is a
//!   starting point, not a fence — an existing project anywhere under `$HOME`
//!   can still be opened.
//!
//! The program's own configuration and state (token, devices, workspaces,
//! session titles, the space and instance ledgers, instance data roots, each
//! module's data) lives in `~/.<product_id>` — `~/.harnix` unless the profile's
//! `[brand]` names the product — so two products on one machine never share a
//! token or a ledger, and Documents holds only projects, with nothing hidden in
//! it. A project's own data goes in `<project>/.harnix/`, the engine's project
//! tier beside it.
//! `HARNIX_STATE_DIR` moves it for the case that needs to. An older
//! `~/Documents/.harnix` is not read (maintainer, 2026-10-26).
//!
//! The install artifact is the binary — the web app is compiled into it, so
//! nothing is read from an install directory at runtime (`--assets-dir` is the
//! development exception).
//!
//! Nothing here creates directories: each owner creates its own on first
//! write, so a misconfigured path fails where it is used rather than leaving
//! an empty tree somewhere surprising.

use std::ffi::OsString;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct Roots {
    /// `main`'s engine root.
    pub engine: PathBuf,
    /// Where projects live: the picker's starting point, and where a new
    /// project directory is created.
    pub projects: PathBuf,
    /// Harnix's own state.
    pub state: PathBuf,
}

#[derive(Debug, Default, Clone)]
pub struct RootArgs {
    pub engine_dir: Option<PathBuf>,
    pub data_dir: Option<PathBuf>,
    /// `[brand] product_id`; `harnix` when absent.
    pub product_id: Option<String>,
}

impl Roots {
    /// Resolve against the process environment.
    pub fn resolve(args: &RootArgs) -> Self {
        Self::resolve_with(args, &|key: &str| std::env::var_os(key))
    }

    /// Resolve against a given environment.
    ///
    /// The lookup is a parameter so the precedence rules can be tested without
    /// touching process-global state — three tests mutating `HARNIX_DATA_DIR`
    /// in parallel threads is a race, and the race is the test harness's, not
    /// the code's.
    pub fn resolve_with(args: &RootArgs, env: &dyn Fn(&str) -> Option<OsString>) -> Self {
        let home = env("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));

        let projects = args
            .data_dir
            .clone()
            .or_else(|| env("HARNIX_DATA_DIR").map(PathBuf::from))
            .unwrap_or_else(|| home.join("Documents"));

        let product = args.product_id.as_deref().unwrap_or("harnix");
        let state = env("HARNIX_STATE_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(format!(".{product}")));
        let engine = args.engine_dir.clone().unwrap_or_else(|| state.join("engine"));

        Self {
            engine,
            projects,
            state,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An environment with exactly the variables a case is about.
    fn env<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<OsString> + 'a {
        move |key| {
            pairs
                .iter()
                .find(|(name, _)| *name == key)
                .map(|(_, value)| OsString::from(*value))
        }
    }

    #[test]
    fn the_engine_is_in_harnixs_own_state_by_default() {
        let roots = Roots::resolve_with(&RootArgs::default(), &env(&[("HOME", "/home/x")]));
        assert_eq!(roots.engine, PathBuf::from("/home/x/.harnix/engine"));
        assert_eq!(roots.projects, PathBuf::from("/home/x/Documents"));
        assert_eq!(roots.state, PathBuf::from("/home/x/.harnix"));
    }

    #[test]
    fn a_product_keeps_its_own_state() {
        let roots = Roots::resolve_with(
            &RootArgs {
                product_id: Some("helm".into()),
                ..RootArgs::default()
            },
            &env(&[("HOME", "/home/x")]),
        );
        assert_eq!(roots.state, PathBuf::from("/home/x/.helm"));
        assert_eq!(roots.engine, PathBuf::from("/home/x/.helm/engine"));
        assert_eq!(roots.projects, PathBuf::from("/home/x/Documents"));
    }

    #[test]
    fn both_directories_move_independently() {
        let roots = Roots::resolve_with(
            &RootArgs {
                engine_dir: Some(PathBuf::from("/srv/engine")),
                data_dir: Some(PathBuf::from("/srv/projects")),
                product_id: None,
            },
            &env(&[("HOME", "/home/x")]),
        );
        assert_eq!(roots.engine, PathBuf::from("/srv/engine"));
        assert_eq!(roots.projects, PathBuf::from("/srv/projects"));
        // The program's state stays in the home directory, not with the projects.
        assert_eq!(roots.state, PathBuf::from("/home/x/.harnix"));
    }

    #[test]
    fn the_environment_fills_in_for_a_missing_flag() {
        let roots = Roots::resolve_with(
            &RootArgs::default(),
            &env(&[
                ("HOME", "/home/x"),
                ("AGIX_CONFIG_HOME", "/env/agix"),
                ("HARNIX_DATA_DIR", "/env/projects"),
            ]),
        );
        // The engine's own variable is not Harnix's to read.
        assert_eq!(roots.engine, PathBuf::from("/home/x/.harnix/engine"));
        assert_eq!(roots.projects, PathBuf::from("/env/projects"));
        assert_eq!(roots.state, PathBuf::from("/home/x/.harnix"));
    }

    #[test]
    fn a_flag_outranks_the_environment() {
        let roots = Roots::resolve_with(
            &RootArgs {
                data_dir: Some(PathBuf::from("/flag/projects")),
                ..RootArgs::default()
            },
            &env(&[("HOME", "/home/x"), ("HARNIX_DATA_DIR", "/env/projects")]),
        );
        assert_eq!(roots.projects, PathBuf::from("/flag/projects"));
    }

    #[test]
    fn harnix_state_can_be_moved_on_its_own() {
        let roots = Roots::resolve_with(
            &RootArgs::default(),
            &env(&[("HOME", "/home/x"), ("HARNIX_STATE_DIR", "/var/lib/harnix")]),
        );
        assert_eq!(roots.projects, PathBuf::from("/home/x/Documents"));
        assert_eq!(roots.state, PathBuf::from("/var/lib/harnix"));
        // The engine goes with it.
        assert_eq!(roots.engine, PathBuf::from("/var/lib/harnix/engine"));
    }
}
