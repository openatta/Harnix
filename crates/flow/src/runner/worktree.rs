//! A task in a worktree of its project (`workspace: worktree`, `docs/glossary.md` 项目任务):
//!
//! ```text
//! <worktrees_root>/<run>/          the worktree, on the branch <branch_prefix>/<run> from HEAD at
//!                                  start: the run's sessions, gates and output directory are here
//! <project>/<dir>/runs/<run>/      its process directory stays in the project's own tree, with
//!                                  every other run's
//! ```
//!
//! The worktree is out of the project, where the host's `ProjectLayout` says (Harnix: its flow
//! extension's state, apart from its engine's own worktrees in the repository's `.wkts/`, which
//! the engine takes away what it does not know in, and from its branches `harnix/worktree-*`).
//!
//! Once the run is over a person decides: merge the branch into the project's current branch,
//! keep the branch, or discard both ([`end`]). The worktree's uncommitted changes are committed
//! on its branch first, so keeping or merging loses nothing the task wrote; a merge never leaves
//! the project's tree half-merged (`git.rs`). What was decided is recorded on the run
//! (`RunRecord::worktree`).

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use super::git::{self, MergeFailed, NoRepo};
use crate::kernel::engine::{worktree_may_end, Engine};
use crate::kernel::host::Outcome;
use crate::kernel::run::{Actor, RunRecord, WorktreeEnd, WorktreeOutcome, WorktreeRec};
use crate::kernel::FlowError;

/// One piece of the runner's git work at a time: two ends of one worktree never race, nor a start
/// and an end in one repository.
static GIT: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// How many ids a start tries before giving up, when a worktree or branch of the name is there
/// already (left from a ledger that is gone).
const TRIES: usize = 8;

/// `f` off the async threads, holding [`GIT`].
async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Result<T, String> {
    tokio::task::spawn_blocking(move || {
        let _one = GIT.lock().unwrap_or_else(|e| e.into_inner());
        f()
    })
    .await
    .map_err(|e| format!("the git work stopped: {e}"))
}

/// Whether a worktree can be made of `project`: `{git: true, branch, prefix}`, or `{git: false,
/// reason, detail}` — `reason` one of `not_a_repository`, `bare`, `no_commit`, `no_git`.
pub async fn info(project: PathBuf) -> Value {
    match blocking(move || git::probe(&project)).await {
        Ok(Ok(repo)) => json!({"git": true, "branch": repo.branch, "prefix": repo.prefix}),
        Ok(Err(no)) => json!({"git": false, "reason": no.code(), "detail": no.to_string()}),
        Err(e) => json!({"git": false, "reason": NoRepo::NoGit(e.clone()).code(), "detail": e}),
    }
}

/// The refusal of a start in a worktree of a project that cannot have one.
fn needs_git(no: &NoRepo) -> FlowError {
    FlowError::Invalid(format!("workspace worktree needs a git repository with a commit: {no}"))
}

/// Make the worktree a run of `flow` in `root` will work in, from the project's `HEAD`, under an id
/// allocated for it now: `<worktrees_root>/<id>` on `<branch_prefix>/<id>`. The project's
/// `<dir>/.gitignore` is written first, so the run's process directory is not something the
/// project's tree shows. Returns the id and the worktree.
pub(crate) async fn make(engine: &Engine, root: &str, flow: &str) -> crate::Result<(String, WorktreeRec)> {
    let project = PathBuf::from(root);
    let probed = {
        let project = project.clone();
        blocking(move || git::probe(&project)).await.map_err(FlowError::Invalid)?
    };
    let repo = probed.map_err(|no| needs_git(&no))?;
    let layout = engine
        .layout()
        .ok_or_else(|| FlowError::Invalid("this host makes no worktrees".into()))?;
    if let Err(e) = super::project::ensure_gitignore(&project, layout) {
        tracing::warn!(root, error = %e, "the project's data directory's .gitignore not written");
    }
    for _ in 0..TRIES {
        let id = engine.alloc_id(flow, Some(root)).await?;
        let path = layout.worktree_dir(&id);
        let branch = layout.worktree_branch(&id);
        let (p, b, head, project) = (path.clone(), branch.clone(), repo.head.clone(), project.clone());
        let made = blocking(move || {
            if p.exists() || git::has_branch(&project, &b)? {
                return Ok(false);
            }
            git::add_worktree(&project, &p, &b, &head).map(|()| true)
        })
        .await
        .and_then(|r| r)
        .map_err(|e| FlowError::Invalid(format!("the run's worktree could not be made: {e}")))?;
        if made {
            let w = WorktreeRec {
                path: path.to_string_lossy().into_owned(),
                prefix: repo.prefix.clone(),
                branch,
                base: repo.head.clone(),
                outcome: None,
            };
            // A project in a directory of its repository that git does not track yet: made, so
            // the run has its place in the worktree.
            if let Err(e) = std::fs::create_dir_all(w.dir()) {
                unmake(root, &w).await;
                return Err(FlowError::Invalid(format!("the run's worktree has no {}: {e}", w.dir().display())));
            }
            return Ok((id, w));
        }
    }
    Err(FlowError::Invalid(format!(
        "the run's worktree could not be made: {TRIES} names under {} were taken",
        layout.worktrees_root.display()
    )))
}

/// Take back a worktree made for a run that was not started: the directory and the branch.
pub(crate) async fn unmake(root: &str, w: &WorktreeRec) {
    let (project, path, branch) = (PathBuf::from(root), PathBuf::from(&w.path), w.branch.clone());
    let r = blocking(move || {
        git::remove_worktree(&project, &path)?;
        git::delete_branch(&project, &branch, true)
    })
    .await
    .and_then(|r| r);
    if let Err(e) = r {
        tracing::warn!(root, worktree = w.path, error = %e, "a worktree of a run not started not taken back");
    }
}

/// The commit message for what a task left in its worktree.
fn work_message(rec: &RunRecord) -> String {
    let what = if rec.title.is_empty() { &rec.subject } else { &rec.title };
    let line = what.lines().next().unwrap_or_default().trim();
    format!("Harnix task {} ({}): {line}", rec.id, rec.flow)
}

/// Why a worktree could not be ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EndError {
    /// The kernel's: no such run, not over yet, no worktree, decided already.
    Flow(FlowError),
    /// Merging conflicts in `files`: nothing in the project changed; the task's work is committed
    /// on `branch` and its worktree is still at `worktree`.
    Conflict { files: Vec<String>, branch: String, worktree: String },
    /// git refused or failed otherwise.
    Git { message: String, branch: String },
    /// The git work could not be run at all.
    Lost(String),
}

impl std::fmt::Display for EndError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EndError::Flow(e) => e.fmt(f),
            EndError::Conflict { files, branch, worktree } => write!(
                f,
                "merging {branch} into the project conflicts in {}: nothing in the project changed; the task's work is committed on {branch} and its worktree is still at {worktree}",
                files.join(", ")
            ),
            EndError::Git { message, .. } => f.write_str(message),
            EndError::Lost(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for EndError {}

impl From<FlowError> for EndError {
    fn from(e: FlowError) -> Self {
        EndError::Flow(e)
    }
}

/// Decide `run`'s worktree: `how` carried out in git, then recorded on the run. `value`:
/// `{run, worktree}`. A merge that conflicts answers [`EndError::Conflict`] with the files, the
/// project's tree untouched.
pub(crate) async fn end(engine: &Engine, run: &str, how: WorktreeEnd, by: Actor) -> Result<Outcome, EndError> {
    let rec = engine.record(run).await?;
    let w = worktree_may_end(&rec)?.clone();
    let Some(root) = rec.project_root.clone() else {
        return Err(FlowError::Invalid(format!("run {run} has no project")).into());
    };
    let message = work_message(&rec);
    let merge_message = format!("Merge {} (Harnix task {})", w.branch, rec.id);
    let (project, path, branch) = (PathBuf::from(&root), PathBuf::from(&w.path), w.branch.clone());
    // The project's data directory in the worktree is the engine's, but for the project's flows.
    let own = engine.layout().map(|l| l.dir.clone());
    let done = blocking(move || carry_out(&project, &path, &branch, own.as_deref(), how, &message, &merge_message))
        .await
        .map_err(EndError::Lost)?;
    let (into, commit, merged_how) = match done {
        Ok(d) => d,
        Err(MergeFailed::Conflict(files)) => {
            return Err(EndError::Conflict { files, branch: w.branch.clone(), worktree: w.path.clone() });
        }
        Err(MergeFailed::Other(m)) => return Err(EndError::Git { message: m, branch: w.branch.clone() }),
    };
    let outcome = WorktreeOutcome {
        kind: how,
        at: engine.clock().now_ms(),
        by: by.clone(),
        into,
        commit,
        how: merged_how.map(String::from),
    };
    Ok(engine.worktree_ended(run, outcome, by).await?)
}

type Done = (Option<String>, Option<String>, Option<&'static str>);

/// The git side of [`end`]: what to record — the branch merged into, a commit, how it merged.
fn carry_out(
    project: &Path,
    path: &Path,
    branch: &str,
    own: Option<&str>,
    how: WorktreeEnd,
    message: &str,
    merge_message: &str,
) -> Result<Done, MergeFailed> {
    let other = MergeFailed::Other;
    match how {
        WorktreeEnd::Kept => {
            git::snapshot(path, own, message).map_err(other)?;
            git::remove_worktree(project, path).map_err(other)?;
            let tip = git::tip(project, branch).ok();
            Ok((None, tip, None))
        }
        WorktreeEnd::Discarded => {
            git::remove_worktree(project, path).map_err(other)?;
            git::delete_branch(project, branch, true).map_err(other)?;
            Ok((None, None, None))
        }
        WorktreeEnd::Merged => {
            git::snapshot(path, own, message).map_err(other)?;
            let m = git::merge(project, branch, &git::scratch_for(path), merge_message)?;
            // Merged: the worktree and the branch have nothing left the project lacks.
            if let Err(e) = git::remove_worktree(project, path).and_then(|()| git::delete_branch(project, branch, false)) {
                tracing::warn!(branch, error = %e, "a merged worktree or branch not removed");
            }
            Ok((Some(m.into), Some(m.commit), Some(m.how)))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use crate::kernel::engine::CreateIn;
    use crate::kernel::run::Workspace;
    use crate::runner::git::tests::{commit_all, have_git, repo};
    use crate::runner::{Runner, ScriptedHost};
    use crate::{ManualClock, MemPacks, MemStore, ProjectLayout};

    fn engine(data: &Path) -> Arc<Engine> {
        let packs = MemPacks::new()
            .flow("{name: f, stations: [{id: a, exit: [{kind: confirm}]}]}")
            .flow("{name: w, workspace: worktree, stations: [{id: a, exit: [{kind: confirm}]}]}");
        Arc::new(
            Engine::new(Arc::new(MemStore::new()), Arc::new(packs))
                .with_clock(Arc::new(ManualClock::new(1_794_787_200_000)))
                .with_workspaces(data.join("workspaces"))
                .with_layout(ProjectLayout::new(".harnix", data.join("worktrees"), "harnix/flow")),
        )
    }

    fn by() -> Actor {
        Actor::device("d")
    }

    async fn started(e: &Arc<Engine>, flow: &str, root: &Path, ws: Option<Workspace>) -> crate::Result<String> {
        let input = CreateIn {
            flow: flow.into(),
            subject: "the subject".into(),
            project_root: Some(root.to_string_lossy().into_owned()),
            workspace: ws,
            ..Default::default()
        };
        let rn = Runner::for_tests(e.clone(), ScriptedHost::new());
        let o = super::super::project::start(&rn, input, by()).await?;
        Ok(o.value["run"].as_str().unwrap().to_string())
    }

    fn invalid(e: &EndError) -> bool {
        matches!(e, EndError::Flow(FlowError::Invalid(_)))
    }

    fn git_out(dir: &Path, args: &[&str]) -> String {
        let o = std::process::Command::new("git").arg("-C").arg(dir).args(args).output().unwrap();
        String::from_utf8_lossy(&o.stdout).trim().to_string()
    }

    #[tokio::test]
    async fn a_worktree_task_works_in_its_worktree_and_shows_its_progress_in_the_project() {
        if !have_git() {
            return;
        }
        let r = repo();
        let p = r.path();
        let data = tempfile::tempdir().unwrap();
        let e = engine(data.path());
        let run = started(&e, "f", p, Some(Workspace::Worktree)).await.unwrap();
        let rec = e.record(&run).await.unwrap();
        let w = rec.worktree.clone().unwrap();
        // Out of the project, apart from the engine's worktrees and their branches.
        let wt = data.path().join("worktrees").join(&run);
        assert_eq!((rec.workspace, Path::new(&w.path)), (Workspace::Worktree, wt.as_path()));
        assert_eq!(w.branch, format!("harnix/flow/{run}"));
        assert_eq!(w.base, git_out(p, &["rev-parse", "HEAD"]));
        assert_eq!(git_out(&wt, &["symbolic-ref", "--short", "HEAD"]), w.branch);
        // Its sessions and gates work there; its deliverables go there.
        assert_eq!(e.workspace_for(&rec), Some(wt.clone()));
        let out = PathBuf::from(rec.output_dir.as_deref().unwrap());
        assert!(out.starts_with(&wt) && out.is_dir(), "{}", out.display());
        // Its process directory is the project's, with every other run's; the project's tree
        // shows none of it.
        super::super::project::project_run(&e, &rec);
        assert!(p.join(".harnix/runs").join(&run).join("task.json").is_file());
        assert!(!wt.join(".harnix/runs").exists());
        assert_eq!(git_out(p, &["status", "--porcelain"]), "?? .harnix/");
        // A summary says the branch, for a row.
        assert_eq!(rec.summary().worktree.unwrap().branch, w.branch);
        // Not over: nothing to decide yet.
        let early = end(&e, &run, WorktreeEnd::Kept, by()).await.unwrap_err();
        assert!(invalid(&early), "{early}");
    }

    #[tokio::test]
    async fn a_project_in_a_directory_of_its_repository_works_in_the_same_directory_of_the_worktree() {
        if !have_git() {
            return;
        }
        let r = repo();
        let sub = r.path().join("apps/one");
        std::fs::create_dir_all(&sub).unwrap();
        let data = tempfile::tempdir().unwrap();
        let e = engine(data.path());
        let run = started(&e, "f", &sub, Some(Workspace::Worktree)).await.unwrap();
        let rec = e.record(&run).await.unwrap();
        let w = rec.worktree.clone().unwrap();
        assert_eq!(w.prefix, "apps/one");
        let at = data.path().join("worktrees").join(&run).join("apps/one");
        assert_eq!(e.workspace_for(&rec), Some(at.clone()));
        assert!(at.is_dir());
        assert!(rec.output_dir.unwrap().starts_with(at.to_str().unwrap()));
        assert_eq!(git_out(r.path(), &["status", "--porcelain"]), "?? apps/");
        assert!(!sub.join(".harnix/worktrees").exists());
    }

    #[tokio::test]
    async fn a_flow_may_ask_for_a_worktree_and_a_start_may_say_otherwise() {
        if !have_git() {
            return;
        }
        let r = repo();
        let data = tempfile::tempdir().unwrap();
        let e = engine(data.path());
        let run = started(&e, "w", r.path(), None).await.unwrap();
        assert_eq!(e.record(&run).await.unwrap().workspace, Workspace::Worktree);
        let run = started(&e, "w", r.path(), Some(Workspace::Project)).await.unwrap();
        let rec = e.record(&run).await.unwrap();
        assert_eq!((rec.workspace, rec.worktree), (Workspace::Project, None));
    }

    #[tokio::test]
    async fn a_project_that_is_no_git_repository_cannot_have_a_worktree_task() {
        let plain = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let e = engine(data.path());
        let err = started(&e, "f", plain.path(), Some(Workspace::Worktree)).await.unwrap_err();
        assert_eq!(err.code(), "invalid");
        assert!(err.to_string().contains("needs a git repository"), "{err}");
        assert!(e.list(Default::default()).await.unwrap().is_empty());
        assert!(std::fs::read_dir(plain.path()).unwrap().next().is_none(), "nothing made");
        let info = info(plain.path().to_path_buf()).await;
        assert_eq!(info["git"], false);
        if have_git() {
            assert_eq!(info["reason"], "not_a_repository");
            let r = repo();
            let yes = info_of(r.path()).await;
            assert_eq!((yes["git"].clone(), yes["branch"].clone()), (json!(true), json!("main")));
        }
    }

    async fn info_of(p: &Path) -> Value {
        info(p.to_path_buf()).await
    }

    #[tokio::test]
    async fn a_start_that_fails_takes_its_worktree_back() {
        if !have_git() {
            return;
        }
        let r = repo();
        let p = r.path();
        let data = tempfile::tempdir().unwrap();
        let e = engine(data.path());
        let input = CreateIn {
            flow: "f".into(),
            subject: "s".into(),
            project_root: Some(p.to_string_lossy().into_owned()),
            workspace: Some(Workspace::Worktree),
            output_dir: Some("../outside".into()),
            ..Default::default()
        };
        let rn = Runner::for_tests(e.clone(), ScriptedHost::new());
        assert!(super::super::project::start(&rn, input, by()).await.is_err());
        assert_eq!(std::fs::read_dir(data.path().join("worktrees")).map(|d| d.count()).unwrap_or(0), 0);
        assert_eq!(git_out(p, &["branch", "--list", "harnix/*"]), "");
    }

    /// A run of `f` in a worktree, a file written there, the run over.
    async fn finished_with(e: &Arc<Engine>, p: &Path, file: &str, text: &str) -> (String, PathBuf) {
        let run = started(e, "f", p, Some(Workspace::Worktree)).await.unwrap();
        let wt = PathBuf::from(e.record(&run).await.unwrap().worktree.unwrap().path);
        std::fs::write(wt.join(file), text).unwrap();
        e.cancel(&run, "done", by()).await.unwrap();
        (run, wt)
    }

    #[tokio::test]
    async fn merged_the_work_is_in_the_project_and_the_worktree_and_branch_are_gone() {
        if !have_git() {
            return;
        }
        let r = repo();
        let p = r.path();
        let data = tempfile::tempdir().unwrap();
        let e = engine(data.path());
        // Ahead of the project: a fast-forward.
        let (run, wt) = finished_with(&e, p, "b.txt", "b\n").await;
        let o = end(&e, &run, WorktreeEnd::Merged, by()).await.unwrap();
        let outcome = e.record(&run).await.unwrap().worktree.unwrap().outcome.unwrap();
        assert_eq!((outcome.kind, outcome.how.as_deref(), outcome.into.as_deref()), (WorktreeEnd::Merged, Some("fast_forward"), Some("main")));
        assert_eq!(o.value["worktree"]["outcome"]["kind"], "merged");
        assert_eq!(std::fs::read_to_string(p.join("b.txt")).unwrap(), "b\n");
        assert!(!wt.exists());
        assert_eq!(git_out(p, &["branch", "--list", "harnix/*"]), "");
        assert_eq!(git_out(p, &["log", "-1", "--format=%s"]), format!("Harnix task {run} (f): the subject"));
        // Decided: nothing more to do.
        assert!(invalid(&end(&e, &run, WorktreeEnd::Discarded, by()).await.unwrap_err()));
        let history = e.history(&run).await.unwrap();
        assert_eq!(history.last().unwrap().op, "worktree");

        // The project moved on meanwhile: a merge commit.
        let (run, _) = finished_with(&e, p, "c.txt", "c\n").await;
        std::fs::write(p.join("d.txt"), "d\n").unwrap();
        commit_all(p, "d");
        end(&e, &run, WorktreeEnd::Merged, by()).await.unwrap();
        let outcome = e.record(&run).await.unwrap().worktree.unwrap().outcome.unwrap();
        assert_eq!(outcome.how.as_deref(), Some("merge"));
        assert_eq!(outcome.commit.unwrap(), git_out(p, &["rev-parse", "HEAD"]));
        assert!(p.join("c.txt").is_file());
    }

    #[tokio::test]
    async fn a_merge_that_conflicts_changes_nothing_and_records_nothing() {
        if !have_git() {
            return;
        }
        let r = repo();
        let p = r.path();
        let data = tempfile::tempdir().unwrap();
        let e = engine(data.path());
        let (run, wt) = finished_with(&e, p, "a.txt", "one\ntask\nthree\n").await;
        std::fs::write(p.join("a.txt"), "one\nperson\nthree\n").unwrap();
        commit_all(p, "person");
        let head = git_out(p, &["rev-parse", "HEAD"]);
        let err = end(&e, &run, WorktreeEnd::Merged, by()).await.unwrap_err();
        let EndError::Conflict { files, .. } = &err else { panic!("{err}") };
        assert_eq!(files, &["a.txt"]);
        assert_eq!(git_out(p, &["rev-parse", "HEAD"]), head);
        assert_eq!(std::fs::read_to_string(p.join("a.txt")).unwrap(), "one\nperson\nthree\n");
        assert_eq!(git_out(p, &["status", "--porcelain", "--untracked-files=no"]), "");
        // The task's work is committed on its branch, its worktree still there to resolve it in.
        assert!(wt.is_dir());
        assert_eq!(git_out(&wt, &["status", "--porcelain"]), "");
        assert_eq!(e.record(&run).await.unwrap().worktree.unwrap().outcome, None);
    }

    #[tokio::test]
    async fn kept_the_branch_holds_the_work_and_may_be_merged_or_discarded_later() {
        if !have_git() {
            return;
        }
        let r = repo();
        let p = r.path();
        let data = tempfile::tempdir().unwrap();
        let e = engine(data.path());
        let (run, wt) = finished_with(&e, p, "kept.txt", "k\n").await;
        let branch = format!("harnix/flow/{run}");
        end(&e, &run, WorktreeEnd::Kept, by()).await.unwrap();
        let outcome = e.record(&run).await.unwrap().worktree.unwrap().outcome.unwrap();
        assert_eq!(outcome.kind, WorktreeEnd::Kept);
        assert_eq!(outcome.commit.unwrap(), git_out(p, &["rev-parse", &branch]));
        assert!(!wt.exists());
        assert!(!p.join("kept.txt").exists());
        assert_eq!(git_out(p, &["show", &format!("{branch}:kept.txt")]), "k");
        // Later, merged after all.
        end(&e, &run, WorktreeEnd::Merged, by()).await.unwrap();
        assert!(p.join("kept.txt").is_file());

        // Discarded: the worktree and the branch, work and all.
        let (run, wt) = finished_with(&e, p, "gone.txt", "g\n").await;
        end(&e, &run, WorktreeEnd::Discarded, by()).await.unwrap();
        assert!(!wt.exists());
        assert_eq!(git_out(p, &["branch", "--list", &format!("harnix/flow/{run}")]), "");
        assert!(!p.join("gone.txt").exists());
        assert_eq!(e.record(&run).await.unwrap().worktree.unwrap().outcome.unwrap().kind, WorktreeEnd::Discarded);
    }

    #[tokio::test]
    async fn a_run_of_no_worktree_has_nothing_to_decide() {
        let data = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        let e = engine(data.path());
        let run = started(&e, "f", project.path(), None).await.unwrap();
        e.cancel(&run, "done", by()).await.unwrap();
        let err = end(&e, &run, WorktreeEnd::Kept, by()).await.unwrap_err();
        assert!(err.to_string().contains("does not work in a worktree"), "{err}");
    }
}
