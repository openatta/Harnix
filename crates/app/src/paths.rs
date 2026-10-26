//! Where Harnix reads and writes.
//!
//! Two directories are configurable, because two are what a user actually
//! moves:
//!
//! - **`--agix-dir`** — the engine's root, which Harnix points Agix at:
//!   settings tiers, transcripts, memory, skills. Default `~/.agix`, the same
//!   tree `agixd` and Agix use, so an existing install keeps its
//!   sessions.
//! - **`--data-dir`** — the projects root: where the picker starts and where
//!   "new project" creates a directory. Default `~/Documents`. It is a
//!   starting point, not a fence — an existing project anywhere under `$HOME`
//!   can still be opened.
//!
//! Harnix's own bookkeeping (workspaces, session titles, view preferences,
//! token, uploads) is data too — the program's own — so it lives inside the
//! data directory as `<data-dir>/.harnix`, and moving `--data-dir` moves it
//! along with the projects it describes. It gets no flag of its own because
//! nobody chooses it independently; `HARNIX_STATE_DIR` is there for the case
//! that proves otherwise.
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
    /// The engine's root — what `AGIX_CONFIG_HOME` names.
    pub engine: PathBuf,
    /// Where projects live: the picker's starting point, and where a new
    /// project directory is created.
    pub projects: PathBuf,
    /// Harnix's own state.
    pub state: PathBuf,
}

#[derive(Debug, Default, Clone)]
pub struct RootArgs {
    pub agix_dir: Option<PathBuf>,
    pub data_dir: Option<PathBuf>,
}

impl Roots {
    /// Resolve against the process environment.
    pub fn resolve(args: &RootArgs) -> Self {
        Self::resolve_with(args, &|key: &str| std::env::var_os(key))
    }

    /// Resolve against a given environment.
    ///
    /// The lookup is a parameter so the precedence rules can be tested without
    /// touching process-global state — three tests mutating `AGIX_CONFIG_HOME`
    /// in parallel threads is a race, and the race is the test harness's, not
    /// the code's.
    pub fn resolve_with(args: &RootArgs, env: &dyn Fn(&str) -> Option<OsString>) -> Self {
        let home = env("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));

        let engine = args
            .agix_dir
            .clone()
            .or_else(|| env("AGIX_CONFIG_HOME").map(PathBuf::from))
            .unwrap_or_else(|| home.join(".agix"));

        let projects = args
            .data_dir
            .clone()
            .or_else(|| env("HARNIX_DATA_DIR").map(PathBuf::from))
            .unwrap_or_else(|| home.join("Documents"));

        let state = env("HARNIX_STATE_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| projects.join(".harnix"));

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
    fn defaults_keep_the_engine_where_it_already_is() {
        let roots = Roots::resolve_with(&RootArgs::default(), &env(&[("HOME", "/home/x")]));
        assert_eq!(roots.engine, PathBuf::from("/home/x/.agix"));
        assert_eq!(roots.projects, PathBuf::from("/home/x/Documents"));
        assert_eq!(roots.state, PathBuf::from("/home/x/Documents/.harnix"));
    }

    #[test]
    fn both_directories_move_independently() {
        let roots = Roots::resolve_with(
            &RootArgs {
                agix_dir: Some(PathBuf::from("/srv/agix")),
                data_dir: Some(PathBuf::from("/srv/projects")),
            },
            &env(&[("HOME", "/home/x")]),
        );
        assert_eq!(roots.engine, PathBuf::from("/srv/agix"));
        assert_eq!(roots.projects, PathBuf::from("/srv/projects"));
        // Harnix's own data rides along with the projects it describes.
        assert_eq!(roots.state, PathBuf::from("/srv/projects/.harnix"));
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
        assert_eq!(roots.engine, PathBuf::from("/env/agix"));
        assert_eq!(roots.projects, PathBuf::from("/env/projects"));
        assert_eq!(roots.state, PathBuf::from("/env/projects/.harnix"));
    }

    #[test]
    fn a_flag_outranks_the_environment() {
        let roots = Roots::resolve_with(
            &RootArgs {
                agix_dir: Some(PathBuf::from("/flag/agix")),
                data_dir: None,
            },
            &env(&[("HOME", "/home/x"), ("AGIX_CONFIG_HOME", "/env/agix")]),
        );
        assert_eq!(roots.engine, PathBuf::from("/flag/agix"));
    }

    #[test]
    fn harnix_state_can_be_moved_on_its_own() {
        let roots = Roots::resolve_with(
            &RootArgs::default(),
            &env(&[("HOME", "/home/x"), ("HARNIX_STATE_DIR", "/var/lib/harnix")]),
        );
        assert_eq!(roots.projects, PathBuf::from("/home/x/Documents"));
        assert_eq!(roots.state, PathBuf::from("/var/lib/harnix"));
    }
}
