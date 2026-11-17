//! Where the library keeps what it writes for a project, as the host says: the name of the
//! project's data directory, where runs' worktrees go, and what their branches are named under.
//!
//! ```text
//! <project>/<dir>/flows/             the project's own packs, committed with it
//! <project>/<dir>/runs/<run>/        a run's process directory (ignored: `<dir>/.gitignore`)
//! <worktrees_root>/<run>/            a run's worktree, outside every project
//!                                    on the branch <branch_prefix>/<run>
//! ```
//!
//! The data directory may be shared with others (Harnix shares `.harnix` with its engine): the
//! library uses `flows/` and `runs/` in it and nothing else. It is the host's, not a task's: a
//! session's write into it is judged as one outside the project — except into the directories the
//! host names as its sessions' own there (`agent_dirs`: the engine's scratchpad, memory and
//! worktrees in Harnix). There is no default: the library does not know whose application it is in.

use std::path::{Path, PathBuf};

/// The project's flows in its data directory.
pub const FLOWS_DIR: &str = "flows";
/// Runs' process directories in its data directory.
pub const RUNS_DIR: &str = "runs";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectLayout {
    /// The project's data directory, by name (`.harnix`): `<project>/<dir>`.
    pub dir: String,
    /// Where runs' worktrees go, in the host's state and outside every project:
    /// `<worktrees_root>/<run>`.
    pub worktrees_root: PathBuf,
    /// What a run's worktree branch is named under: `<branch_prefix>/<run>`.
    pub branch_prefix: String,
    /// Directories in the data directory a task's session writes as its own, by name
    /// (`scratchpad`, `memory` in Harnix: what its sandbox lets a session write
    /// there). The rest of the data directory is the host's, outside what the task works in.
    pub agent_dirs: Vec<String>,
}

impl ProjectLayout {
    pub fn new(dir: impl Into<String>, worktrees_root: impl Into<PathBuf>, branch_prefix: impl Into<String>) -> Self {
        Self {
            dir: dir.into(),
            worktrees_root: worktrees_root.into(),
            branch_prefix: branch_prefix.into(),
            agent_dirs: vec![],
        }
    }

    /// The directories in the data directory a task's session may write as its own.
    pub fn with_agent_dirs<I, S>(mut self, dirs: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.agent_dirs = dirs.into_iter().map(Into::into).collect();
        self
    }

    /// The project's data directory: `<project>/<dir>`.
    pub fn data_dir(&self, project: &Path) -> PathBuf {
        project.join(&self.dir)
    }

    /// Where a project keeps its own packs: `<project>/<dir>/flows`.
    pub fn pack_dir(&self, project: &Path) -> PathBuf {
        self.data_dir(project).join(FLOWS_DIR)
    }

    /// Where the process directories of runs in `base` go: `<base>/<dir>/runs`.
    pub fn runs_dir(&self, base: &Path) -> PathBuf {
        self.data_dir(base).join(RUNS_DIR)
    }

    /// Where a run's worktree goes: `<worktrees_root>/<run>` (the run's id made safe as a name).
    pub fn worktree_dir(&self, run: &str) -> PathBuf {
        super::refs::workspace_of(&self.worktrees_root, run)
    }

    /// The branch a run's worktree is on: `<branch_prefix>/<run>` (the run's id made safe).
    pub fn worktree_branch(&self, run: &str) -> String {
        let name = super::refs::workspace_of(Path::new(""), run);
        format!("{}/{}", self.branch_prefix.trim_end_matches('/'), name.to_string_lossy())
    }

    /// What a session working in `root` must not touch without a permission naming it: the data
    /// directory there (`<root>/<dir>`), which is the host's, not the task's.
    pub fn reserved(&self, root: &str) -> Vec<String> {
        vec![format!("{}/{}", root.trim_end_matches('/'), self.dir)]
    }

    /// What a session working in `root` writes as its own inside the reserved directory
    /// (`<root>/<dir>/<agent dir>`).
    pub fn agent_dirs_in(&self, root: &str) -> Vec<String> {
        let data = format!("{}/{}", root.trim_end_matches('/'), self.dir);
        self.agent_dirs.iter().map(|d| format!("{data}/{}", d.trim_matches('/'))).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn everything_is_where_the_host_says() {
        let l = ProjectLayout::new(".codix", "/state/flow/worktrees", "codix/flow");
        let p = Path::new("/work/app");
        assert_eq!(l.pack_dir(p), Path::new("/work/app/.codix/flows"));
        assert_eq!(l.runs_dir(p), Path::new("/work/app/.codix/runs"));
        assert_eq!(l.worktree_dir("f/1"), Path::new("/state/flow/worktrees/f-1"));
        assert_eq!(l.worktree_branch("f/1"), "codix/flow/f-1");
        assert_eq!(l.reserved("/work/app/"), ["/work/app/.codix"]);
        assert!(l.agent_dirs_in("/work/app").is_empty(), "nothing in it is a task's unless the host says");
        let l = l.with_agent_dirs(["scratchpad", "memory/"]);
        assert_eq!(l.agent_dirs_in("/work/app/"), ["/work/app/.codix/scratchpad", "/work/app/.codix/memory"]);
    }
}
