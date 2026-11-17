//! The `git` command line, for runs that work in a worktree of their project (`workspace:
//! worktree`): whether a project can have one, making it, and the three ends of it — merged into
//! the project's current branch, kept as a branch, discarded.
//!
//! Every call is `git -C <dir> …` with prompts off and messages in English; a failure answers what
//! git said. Nothing here leaves the project's own tree half-changed: a merge that is not a
//! fast-forward is made in a worktree of its own first, and only a fast-forward to its result
//! touches the project's tree (git refuses it whole when it would overwrite local changes).

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use crate::kernel::layout::FLOWS_DIR;

/// What a project is to git: whether a worktree can be made of it, and where it stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Repo {
    /// The project's place in its repository (`sub/dir`, no trailing `/`; empty at the top).
    pub prefix: String,
    /// The branch checked out; `None` on a detached `HEAD`.
    pub branch: Option<String>,
    /// The commit `HEAD` is at.
    pub head: String,
}

/// Why a project cannot have a worktree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum NoRepo {
    /// `git` is not there to run.
    NoGit(String),
    /// The directory is in no git repository.
    NotARepository,
    /// A bare repository: no tree to work in.
    Bare,
    /// No commit yet: nothing to branch from.
    NoCommit,
}

impl NoRepo {
    /// The reason's name on the wire (`flow.workspace_info`).
    pub fn code(&self) -> &'static str {
        match self {
            Self::NoGit(_) => "no_git",
            Self::NotARepository => "not_a_repository",
            Self::Bare => "bare",
            Self::NoCommit => "no_commit",
        }
    }
}

impl std::fmt::Display for NoRepo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoGit(e) => write!(f, "git cannot be run here: {e}"),
            Self::NotARepository => f.write_str("the project is not in a git repository"),
            Self::Bare => f.write_str("the project is a bare git repository"),
            Self::NoCommit => f.write_str("the project's git repository has no commit yet"),
        }
    }
}

/// How a merge went.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Merged {
    /// The project's branch it went into.
    pub into: String,
    /// The commit that branch is at now.
    pub commit: String,
    /// `fast_forward`, `merge` or `up_to_date`.
    pub how: &'static str,
}

/// Why a merge did not happen. Either way the project's tree is as it was.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum MergeFailed {
    /// The two sides change the same lines: these files.
    Conflict(Vec<String>),
    /// Anything else: what git said.
    Other(String),
}

impl std::fmt::Display for MergeFailed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Conflict(files) => write!(f, "the merge conflicts in {}", files.join(", ")),
            Self::Other(m) => f.write_str(m),
        }
    }
}

fn git(dir: &Path) -> Command {
    let mut c = Command::new("git");
    c.arg("-C").arg(dir);
    c.env("GIT_TERMINAL_PROMPT", "0").env("LC_ALL", "C").env("LANG", "C");
    c
}

fn output(mut c: Command) -> Result<Output, String> {
    c.output().map_err(|e| format!("git cannot be run: {e}"))
}

/// What git said on failure, in a line or few.
fn said(args: &[&str], o: &Output) -> String {
    let err = String::from_utf8_lossy(&o.stderr);
    let out = String::from_utf8_lossy(&o.stdout);
    let text = if err.trim().is_empty() { out.trim() } else { err.trim() };
    let text: Vec<&str> = text.lines().take(6).collect();
    format!("git {} failed: {}", args.join(" "), text.join(" / "))
}

/// `git -C dir args…`: its output, trimmed; what it said when it fails.
fn run(dir: &Path, args: &[&str]) -> Result<String, String> {
    let mut c = git(dir);
    c.args(args);
    let o = output(c)?;
    if o.status.success() {
        Ok(String::from_utf8_lossy(&o.stdout).trim().to_string())
    } else {
        Err(said(args, &o))
    }
}

/// Whether `git -C dir args…` exits 0 (its output unread).
fn holds(dir: &Path, args: &[&str]) -> Result<bool, String> {
    let mut c = git(dir);
    c.args(args);
    Ok(output(c)?.status.success())
}

/// A committing command with an identity when the repository has none: the user's own
/// otherwise.
fn committing(dir: &Path) -> Command {
    let mut c = git(dir);
    let has = |key: &str| run(dir, &["config", "--get", key]).is_ok_and(|v| !v.is_empty());
    if !has("user.name") {
        c.env("GIT_AUTHOR_NAME", "Harnix").env("GIT_COMMITTER_NAME", "Harnix");
    }
    if !has("user.email") {
        c.env("GIT_AUTHOR_EMAIL", "harnix@localhost").env("GIT_COMMITTER_EMAIL", "harnix@localhost");
    }
    c
}

/// What `project` is to git, or why no worktree can be made of it.
pub(crate) fn probe(project: &Path) -> Result<Repo, NoRepo> {
    let mut c = git(project);
    c.args(["rev-parse", "--is-bare-repository", "--is-inside-work-tree", "--show-prefix"]);
    let o = c.output().map_err(|e| NoRepo::NoGit(e.to_string()))?;
    if !o.status.success() {
        return Err(NoRepo::NotARepository);
    }
    let text = String::from_utf8_lossy(&o.stdout).to_string();
    let mut lines = text.lines();
    let bare = lines.next().map(str::trim) == Some("true");
    let inside = lines.next().map(str::trim) == Some("true");
    let prefix = lines.next().unwrap_or("").trim().trim_end_matches('/').to_string();
    if bare {
        return Err(NoRepo::Bare);
    }
    if !inside {
        return Err(NoRepo::NotARepository);
    }
    let head = run(project, &["rev-parse", "--verify", "-q", "HEAD^{commit}"]).map_err(|_| NoRepo::NoCommit)?;
    let branch = run(project, &["symbolic-ref", "--short", "-q", "HEAD"]).ok().filter(|b| !b.is_empty());
    Ok(Repo { prefix, branch, head })
}

/// Make a worktree of `project` at `path` on a new branch `branch` from the commit `head`.
pub(crate) fn add_worktree(project: &Path, path: &Path, branch: &str, head: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    let p = path.to_string_lossy();
    run(project, &["worktree", "add", "-q", "-b", branch, &p, head]).map(drop)
}

/// Whether `branch` exists in `project`'s repository.
pub(crate) fn has_branch(project: &Path, branch: &str) -> Result<bool, String> {
    holds(project, &["show-ref", "--verify", "-q", &format!("refs/heads/{branch}")])
}

/// What the worktree at `dir` holds that is not committed, committed on its branch: every change,
/// new files too, but what is in the project's data directory (`own`, `.harnix`) besides the
/// project's flows (the engine's own, its delegates' worktrees among it). Answers the new commit,
/// `None` when there was nothing to commit or no worktree any more.
pub(crate) fn snapshot(dir: &Path, own: Option<&str>, message: &str) -> Result<Option<String>, String> {
    if !dir.is_dir() {
        return Ok(None);
    }
    let mut args = vec!["add".to_string(), "-A".into(), "--".into(), ".".into()];
    if let Some(own) = own {
        for entry in std::fs::read_dir(dir.join(own)).into_iter().flatten().flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name != FLOWS_DIR {
                args.push(format!(":(exclude,literal){own}/{name}"));
            }
        }
    }
    run(dir, &args.iter().map(String::as_str).collect::<Vec<_>>())?;
    if holds(dir, &["diff", "--cached", "--quiet"])? {
        return Ok(None);
    }
    let args = ["commit", "-q", "--no-verify", "-m", message];
    let mut c = committing(dir);
    c.args(args);
    let o = output(c)?;
    if !o.status.success() {
        return Err(said(&args, &o));
    }
    run(dir, &["rev-parse", "HEAD"]).map(Some)
}

/// Remove the worktree at `path` of `project`'s repository, whatever it still holds, and forget
/// worktrees whose directories are gone (a delegate's inside it among them). A worktree already
/// gone is fine.
pub(crate) fn remove_worktree(project: &Path, path: &Path) -> Result<(), String> {
    if path.exists() {
        let p = path.to_string_lossy();
        if let Err(e) = run(project, &["worktree", "remove", "--force", "--force", &p]) {
            // Not a worktree git knows any more: the directory is only ours, under the module's `worktrees`.
            if !path.components().any(|c| c.as_os_str() == "worktrees") {
                return Err(e);
            }
            std::fs::remove_dir_all(path).map_err(|io| format!("{e}; {}: {io}", path.display()))?;
        }
    }
    run(project, &["worktree", "prune"]).map(drop)
}

/// Delete `branch`: only once it is merged unless `force`. A branch already gone is fine.
pub(crate) fn delete_branch(project: &Path, branch: &str, force: bool) -> Result<(), String> {
    if !has_branch(project, branch)? {
        return Ok(());
    }
    run(project, &["branch", "-q", if force { "-D" } else { "-d" }, branch]).map(drop)
}

/// The commit `branch` is at.
pub(crate) fn tip(project: &Path, branch: &str) -> Result<String, String> {
    run(project, &["rev-parse", "--verify", "-q", &format!("refs/heads/{branch}^{{commit}}")])
}

/// Merge `branch` into the branch `project` has checked out: nothing when it is there already, a
/// fast-forward when it can be, else a merge commit made in a scratch worktree at `scratch` and
/// the project's branch fast-forwarded to it. On a conflict, or local changes in the way, nothing
/// in the project changes.
pub(crate) fn merge(project: &Path, branch: &str, scratch: &Path, message: &str) -> Result<Merged, MergeFailed> {
    let other = MergeFailed::Other;
    let into = run(project, &["symbolic-ref", "--short", "-q", "HEAD"])
        .ok()
        .filter(|b| !b.is_empty())
        .ok_or_else(|| other("the project has no branch checked out (a detached HEAD): check one out to merge into".into()))?;
    let head = run(project, &["rev-parse", "HEAD"]).map_err(other)?;
    let theirs = tip(project, branch).map_err(|_| other(format!("the branch {branch} is not there any more")))?;
    let done = |how, commit| Merged { into: into.clone(), commit, how };
    if holds(project, &["merge-base", "--is-ancestor", &theirs, &head]).map_err(other)? {
        return Ok(done("up_to_date", head));
    }
    if holds(project, &["merge-base", "--is-ancestor", &head, &theirs]).map_err(other)? {
        run(project, &["merge", "--ff-only", "-q", &theirs]).map_err(other)?;
        return Ok(done("fast_forward", theirs));
    }
    // A merge commit: made beside the project, then the project's branch moved to it.
    let _ = remove_worktree(project, scratch);
    let s = scratch.to_string_lossy();
    run(project, &["worktree", "add", "-q", "--detach", &s, &head]).map_err(other)?;
    let made = merge_in(scratch, &theirs, message);
    let _ = remove_worktree(project, scratch);
    let commit = made?;
    run(project, &["merge", "--ff-only", "-q", &commit]).map_err(|e| {
        other(format!("{e} (the project's branch was left as it was)"))
    })?;
    Ok(done("merge", commit))
}

/// In the scratch worktree at `dir` (on the project's commit): merge `theirs` with a commit, or
/// abort and say which files conflict.
fn merge_in(dir: &Path, theirs: &str, message: &str) -> Result<String, MergeFailed> {
    let args = ["merge", "--no-ff", "--no-verify", "-q", "-m", message, theirs];
    let mut c = committing(dir);
    c.args(args);
    let o = output(c).map_err(MergeFailed::Other)?;
    if o.status.success() {
        return run(dir, &["rev-parse", "HEAD"]).map_err(MergeFailed::Other);
    }
    let files: Vec<String> = run(dir, &["diff", "--name-only", "--diff-filter=U"])
        .unwrap_or_default()
        .lines()
        .map(String::from)
        .filter(|l| !l.is_empty())
        .collect();
    let _ = run(dir, &["merge", "--abort"]);
    if files.is_empty() {
        Err(MergeFailed::Other(said(&args, &o)))
    } else {
        Err(MergeFailed::Conflict(files))
    }
}

/// The scratch worktree a merge commit for the worktree at `path` is made in: beside it.
pub(crate) fn scratch_for(path: &Path) -> PathBuf {
    let mut name = path.file_name().map(|n| n.to_os_string()).unwrap_or_default();
    name.push(".merging");
    path.with_file_name(name)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Whether `git` can be run here: the tests that need it pass quietly without.
    pub(crate) fn have_git() -> bool {
        Command::new("git").arg("--version").output().is_ok_and(|o| o.status.success())
    }

    /// A repository with one commit (`a.txt`), its own identity, on `main`.
    pub(crate) fn repo() -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        let p = d.path();
        run(p, &["init", "-q", "-b", "main"]).unwrap();
        run(p, &["config", "user.name", "T"]).unwrap();
        run(p, &["config", "user.email", "t@example.com"]).unwrap();
        run(p, &["config", "commit.gpgsign", "false"]).unwrap();
        std::fs::write(p.join("a.txt"), "one\ntwo\nthree\n").unwrap();
        commit_all(p, "first");
        d
    }

    /// Everything but `.harnix/` committed: a person's commit in the project.
    pub(crate) fn commit_all(dir: &Path, message: &str) {
        run(dir, &["add", "-A", "--", ".", ":(exclude).harnix"]).unwrap();
        run(dir, &["commit", "-q", "-m", message]).unwrap();
    }

    #[test]
    fn a_project_is_probed_for_what_a_worktree_needs() {
        if !have_git() {
            return;
        }
        let r = repo();
        let at = probe(r.path()).unwrap();
        assert_eq!((at.prefix.as_str(), at.branch.as_deref()), ("", Some("main")));
        assert_eq!(at.head.len(), 40);
        std::fs::create_dir_all(r.path().join("sub/dir")).unwrap();
        assert_eq!(probe(&r.path().join("sub/dir")).unwrap().prefix, "sub/dir");

        let plain = tempfile::tempdir().unwrap();
        assert_eq!(probe(plain.path()), Err(NoRepo::NotARepository));
        let empty = tempfile::tempdir().unwrap();
        run(empty.path(), &["init", "-q"]).unwrap();
        assert_eq!(probe(empty.path()), Err(NoRepo::NoCommit));
        let bare = tempfile::tempdir().unwrap();
        run(bare.path(), &["init", "-q", "--bare"]).unwrap();
        assert_eq!(probe(bare.path()), Err(NoRepo::Bare));
    }

    #[test]
    fn a_worktree_is_made_snapshotted_and_removed_keeping_its_branch() {
        if !have_git() {
            return;
        }
        let r = repo();
        let p = r.path();
        let out = tempfile::tempdir().unwrap();
        let wt = out.path().join("f-1");
        let head = probe(p).unwrap().head;
        add_worktree(p, &wt, "harnix/flow/f-1", &head).unwrap();
        assert!(wt.join("a.txt").is_file());
        assert_eq!(probe(&wt).unwrap().branch.as_deref(), Some("harnix/flow/f-1"));
        // Nothing changed: nothing committed.
        assert_eq!(snapshot(&wt, Some(".harnix"), "m").unwrap(), None);
        std::fs::write(wt.join("new.txt"), "new\n").unwrap();
        std::fs::create_dir_all(wt.join(".harnix/worktrees/x")).unwrap();
        std::fs::write(wt.join(".harnix/settings.local.json"), "{}\n").unwrap();
        std::fs::create_dir_all(wt.join(".harnix/flows")).unwrap();
        std::fs::write(wt.join(".harnix/flows/f.yaml"), "name: f\n").unwrap();
        let c = snapshot(&wt, Some(".harnix"), "the task's work").unwrap().unwrap();
        assert_eq!(tip(p, "harnix/flow/f-1").unwrap(), c);
        let files = run(p, &["ls-tree", "-r", "--name-only", "harnix/flow/f-1"]).unwrap();
        assert_eq!(
            files.lines().collect::<Vec<_>>(),
            [".harnix/flows/f.yaml", "a.txt", "new.txt"],
            "the engine's own left out, the project's flows in"
        );
        remove_worktree(p, &wt).unwrap();
        assert!(!wt.exists());
        assert!(has_branch(p, "harnix/flow/f-1").unwrap());
        assert!(!p.join("new.txt").exists(), "the project's tree untouched");
        // A worktree already gone is fine.
        remove_worktree(p, &wt).unwrap();
        delete_branch(p, "harnix/flow/f-1", true).unwrap();
        assert!(!has_branch(p, "harnix/flow/f-1").unwrap());
        delete_branch(p, "harnix/flow/f-1", true).unwrap();
    }

    #[test]
    fn a_branch_ahead_is_fast_forwarded_one_beside_is_merged_with_a_commit() {
        if !have_git() {
            return;
        }
        let r = repo();
        let p = r.path();
        let head = probe(p).unwrap().head;
        // Nothing done on it: up to date.
        let out = tempfile::tempdir().unwrap();
        let wt = out.path().join("f-1");
        add_worktree(p, &wt, "harnix/flow/f-1", &head).unwrap();
        let m = merge(p, "harnix/flow/f-1", &scratch_for(&wt), "merge").unwrap();
        assert_eq!((m.how, m.into.as_str(), m.commit.as_str()), ("up_to_date", "main", head.as_str()));
        // Ahead: a fast-forward brings the file.
        std::fs::write(wt.join("b.txt"), "b\n").unwrap();
        let c = snapshot(&wt, Some(".harnix"), "b").unwrap().unwrap();
        let m = merge(p, "harnix/flow/f-1", &scratch_for(&wt), "merge").unwrap();
        assert_eq!((m.how, m.commit.as_str()), ("fast_forward", c.as_str()));
        assert_eq!(std::fs::read_to_string(p.join("b.txt")).unwrap(), "b\n");
        // The project moved on meanwhile, elsewhere: a merge commit, both sides kept.
        let wt2 = out.path().join("f-2");
        add_worktree(p, &wt2, "harnix/flow/f-2", &probe(p).unwrap().head).unwrap();
        std::fs::write(wt2.join("c.txt"), "c\n").unwrap();
        snapshot(&wt2, Some(".harnix"), "c").unwrap().unwrap();
        std::fs::write(p.join("d.txt"), "d\n").unwrap();
        commit_all(p, "d");
        let m = merge(p, "harnix/flow/f-2", &scratch_for(&wt2), "Merge harnix/flow/f-2").unwrap();
        assert_eq!(m.how, "merge");
        assert_eq!(run(p, &["rev-parse", "HEAD"]).unwrap(), m.commit);
        assert_eq!(run(p, &["log", "-1", "--format=%P"]).unwrap().split(' ').count(), 2);
        assert!(p.join("c.txt").is_file() && p.join("d.txt").is_file());
        assert!(!scratch_for(&wt2).exists(), "the scratch worktree is gone");
        assert_eq!(run(p, &["status", "--porcelain", "--untracked-files=no"]).unwrap(), "");
    }

    #[test]
    fn a_conflict_or_local_changes_in_the_way_leave_the_project_as_it_was() {
        if !have_git() {
            return;
        }
        let r = repo();
        let p = r.path();
        let out = tempfile::tempdir().unwrap();
        let wt = out.path().join("f-1");
        add_worktree(p, &wt, "harnix/flow/f-1", &probe(p).unwrap().head).unwrap();
        std::fs::write(wt.join("a.txt"), "one\nTWO (task)\nthree\n").unwrap();
        snapshot(&wt, Some(".harnix"), "task").unwrap().unwrap();
        std::fs::write(p.join("a.txt"), "one\nTWO (person)\nthree\n").unwrap();
        commit_all(p, "person");
        let before = run(p, &["rev-parse", "HEAD"]).unwrap();
        let e = merge(p, "harnix/flow/f-1", &scratch_for(&wt), "merge").unwrap_err();
        assert_eq!(e, MergeFailed::Conflict(vec!["a.txt".into()]));
        assert_eq!(run(p, &["rev-parse", "HEAD"]).unwrap(), before);
        assert_eq!(run(p, &["status", "--porcelain"]).unwrap(), "", "no merge state, no conflict markers");
        assert_eq!(std::fs::read_to_string(p.join("a.txt")).unwrap(), "one\nTWO (person)\nthree\n");
        assert!(!p.join(".git/MERGE_HEAD").exists());
        assert!(!scratch_for(&wt).exists());

        // A fast-forward over a local change to the same file: refused, the change kept.
        let wt2 = out.path().join("f-2");
        add_worktree(p, &wt2, "harnix/flow/f-2", &before).unwrap();
        std::fs::write(wt2.join("a.txt"), "task\n").unwrap();
        snapshot(&wt2, Some(".harnix"), "task").unwrap().unwrap();
        std::fs::write(p.join("a.txt"), "uncommitted\n").unwrap();
        let e = merge(p, "harnix/flow/f-2", &scratch_for(&wt2), "merge").unwrap_err();
        assert!(matches!(e, MergeFailed::Other(ref m) if m.contains("overwritten")), "{e}");
        assert_eq!(run(p, &["rev-parse", "HEAD"]).unwrap(), before);
        assert_eq!(std::fs::read_to_string(p.join("a.txt")).unwrap(), "uncommitted\n");
    }
}
