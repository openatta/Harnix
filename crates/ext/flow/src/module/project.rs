//! A run's places in its project: the output directory its deliverables go to, and the process
//! directory where the module shows what the run is doing.
//!
//! ```text
//! <project>/flows/<flow>-<YYYYMMDD><NN>/   the output directory (the default template): the user's
//! <project>/.harnix/                      the project's, shared with the engine (its tier: settings,
//!                                         agents, rules, memory, its own worktrees)
//!   .gitignore                            runs/, added on the first task (the engine adds its own)
//!   flows/                                the project's own flows, committed with it
//!   runs/<run>/
//!     task.json                           the run as it stands (its worktree's branch and end
//!                                         too), rewritten after each change
//!     flow.yaml                           the definition and the parts pinned when it started
//!     stations/<station>/                 output.json (one task) or <task>.json each,
//!                                         gate-<gate>.log, context.json (a command gate's)
//! ```
//!
//! A worktree run's worktree is out of the project, in the module's data directory
//! (`worktree.rs`): its output directory is in there; its process directory is not. Where all of
//! these are is the engine's `ProjectLayout` ([`super::layout`]): without one, nothing is
//! projected.
//!
//! The process directory is a projection: the ledger in the module's data directory is what
//! counts, and a projection that cannot be written is logged and leaves the run as it was. A run
//! of no project has the same layout under its own directory.

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::kernel::def::{Work, DEFAULT_OUTPUT_DIR};
use crate::kernel::engine::{CreateIn, Engine, Prepared};
use crate::kernel::output::{has_seq, render, OutputVars};
use crate::kernel::refs::workspace_of;
use crate::kernel::run::{RunRecord, TaskStatus, Workspace, WorktreeRec};
use crate::kernel::host::Outcome;
use crate::kernel::layout::ProjectLayout;
use crate::kernel::run::Actor;
use crate::kernel::FlowError;

use harnix_ext::{Entry, SessionRef};

use super::shared::Shared;
use crate::wire::StartedFrom;

/// The line `<project>/.harnix/.gitignore` holds for the module: its process directories are not
/// the project's. The project's own flows (`flows/`) are, to commit; the rest there is the
/// engine's, which adds its own lines.
const IGNORED: &str = "runs/";

/// The highest `${seq}` looked for.
const MAX_SEQ: u32 = 999;

/// The local date of an instant, `YYYYMMDD`: what `${date}` is.
pub(crate) fn local_date(epoch_ms: u64) -> String {
    #[cfg(unix)]
    {
        let secs = (epoch_ms / 1000) as libc::time_t;
        // SAFETY: `tm` is plain data, zeroed, and filled by `localtime_r` alone; both pointers
        // live for the call.
        let mut tm: libc::tm = unsafe { std::mem::zeroed() };
        if !unsafe { libc::localtime_r(&secs, &mut tm) }.is_null() {
            return format!("{:04}{:02}{:02}", tm.tm_year + 1900, tm.tm_mon + 1, tm.tm_mday);
        }
    }
    crate::kernel::output::date_utc(epoch_ms)
}

/// Start a run as the module does: who may; then — for a task — the project's trust heard (its own
/// flows are the project's packs, read only while it is trusted); then, in a worktree, its
/// worktree made (`worktree.rs`); then its output directory resolved and made in the workspace;
/// then the run, and what a new run needs on disk ([`made`]). What was made for a run that is then
/// not started is taken back.
pub(crate) async fn start(sh: &Shared, engine: &Engine, mut input: CreateIn, by: Actor) -> crate::Result<Outcome> {
    engine.may_create(&input, &by)?;
    let root = root_of(&input);
    if let Some(root) = &root {
        super::project_packs::hear(sh, root).await;
        // In a project a name means the project's own flow: not run until it is trusted.
        super::project_packs::withheld(engine.packs_for(Some(root)).as_ref(), &input.flow)?;
    }
    let mut prepared = Prepared::default();
    if let Some(root) = &root {
        let def = engine.packs_for(Some(root)).flow(&input.flow).map(|(d, _)| d);
        let workspace = Workspace::chosen(input.workspace, def.as_ref().and_then(|d| d.workspace), Some(root));
        input.workspace = Some(workspace);
        if workspace == Workspace::Worktree && def.is_some() {
            let (id, w) = super::worktree::make(engine, root, &input.flow).await?;
            prepared = Prepared { id: Some(id), worktree: Some(w) };
        }
    }
    let claimed = match prepare(engine, &mut input, prepared.worktree.as_ref()) {
        Ok(c) => c,
        Err(e) => {
            if let (Some(root), Some(w)) = (&root, &prepared.worktree) {
                super::worktree::unmake(root, w).await;
            }
            return Err(e);
        }
    };
    let r = engine.create_prepared(input, by, prepared.clone()).await;
    match &r {
        Ok(o) => {
            if let Some(run) = o.value.get("run").and_then(Value::as_str) {
                made(engine, run).await;
            }
        }
        Err(_) => {
            if let Some(dir) = claimed {
                let _ = std::fs::remove_dir(dir);
            }
            if let (Some(root), Some(w)) = (&root, &prepared.worktree) {
                super::worktree::unmake(root, w).await;
            }
        }
    }
    r
}

/// The entry a conversation a task was started from gets (`flow.start`'s `from_session`): the
/// card linking to the task, drawn by the interface; the conversation does not become the task.
pub(crate) const TASK_STARTED: &str = "task_started";

/// What that card says: the run, its flow (and the flow's title), the run's title and subject,
/// and its project.
pub(crate) fn task_card(rec: &RunRecord) -> Value {
    json!({
        "run": rec.id,
        "flow": rec.flow,
        "flow_title": rec.def.title,
        "title": rec.title,
        "subject": rec.subject,
        "project_root": rec.project_root,
    })
}

/// Write the card into the conversation `run` was started from, claiming the space it belongs
/// to: the hub refuses a conversation of another space. The run is started either way; a card
/// that cannot be written is logged.
pub(crate) async fn link_from(sh: &Shared, engine: &Engine, from: &StartedFrom, run: &str) {
    let Some(cx) = sh.cx.get() else { return };
    let rec = match engine.record(run).await {
        Ok(rec) => rec,
        Err(e) => {
            tracing::warn!(run, error = %e, "no card for a run whose record cannot be read");
            return;
        }
    };
    let instance = from.instance.as_deref().unwrap_or(harnix_ext::MAIN_INSTANCE);
    let session = SessionRef::new(instance, from.session_id.as_str());
    let written = cx
        .sessions()
        .in_space(&from.space)
        .append_entry(&session, Entry::new(TASK_STARTED, task_card(&rec)))
        .await;
    if let Err(e) = written {
        tracing::warn!(run, session = %from.session_id, space = %from.space, error = %e.message, "the conversation a task was started from has no card");
    }
}

/// A start's project, as the run will hold it: trimmed, without trailing `/`s; none for none.
fn root_of(input: &CreateIn) -> Option<String> {
    input
        .project_root
        .as_deref()
        .map(str::trim)
        .filter(|r| !r.is_empty())
        .map(|r| crate::kernel::run::trim_root(r).to_string())
}

/// A start's output directory resolved and made, in place in `input`: the start's own template,
/// else its flow's, else the default; rendered against the flow, the date, the title and the
/// inputs. A task's is made in its project now (`${seq}` claimed there) — in a worktree run, at
/// the same place in its worktree — and is an absolute path; a run of no project's is left
/// relative, for the kernel to put under the run's own directory ([`made`] makes it). Returns the
/// directory made, to be taken back if the run is not.
///
/// A flow the packs do not have is the kernel's to refuse.
pub(crate) fn prepare(engine: &Engine, input: &mut CreateIn, worktree: Option<&WorktreeRec>) -> crate::Result<Option<PathBuf>> {
    let Some((def, _)) = engine.packs_for(input.project_root.as_deref()).flow(&input.flow) else {
        return Ok(None);
    };
    let template = input
        .output_dir
        .clone()
        .filter(|d| !d.trim().is_empty())
        .or_else(|| def.output_dir.clone())
        .unwrap_or_else(|| DEFAULT_OUTPUT_DIR.to_string());
    let inputs = def.apply_inputs(&input.inputs).unwrap_or_else(|_| input.inputs.clone());
    let date = local_date(engine.clock().now_ms());
    let vars = OutputVars {
        flow: &def.name,
        date: &date,
        title: input.title.as_deref().unwrap_or_default(),
        inputs: &inputs,
    };
    let root = root_of(input);
    let workspace = Workspace::chosen(input.workspace, def.workspace, root.as_deref());
    match (workspace.in_project(), root) {
        (true, Some(root)) => {
            // In a worktree: the same place relative to the project, in the worktree.
            let (base, template) = match worktree {
                Some(w) => (w.dir(), relative_in(Path::new(&root), &template).map_err(FlowError::Invalid)?),
                None => (PathBuf::from(&root), template),
            };
            let (dir, made) = claim(&base, &template, &vars).map_err(FlowError::Invalid)?;
            input.output_dir = Some(dir.to_string_lossy().into_owned());
            Ok(made)
        }
        _ => {
            if Path::new(&template).is_absolute() {
                return Err(FlowError::Invalid(format!(
                    "output_dir {template:?}: a run that works in a directory of its own takes a relative one"
                )));
            }
            // A directory of the run's own is new: nothing is numbered in it yet.
            let rel = render(&template, &vars, has_seq(&template).then_some(1))
                .map_err(|m| FlowError::Invalid(format!("output_dir: {m}")))?;
            input.output_dir = Some(rel);
            Ok(None)
        }
    }
}

/// After a run was made: its output directory where it does not exist yet (a run of no project's),
/// and the project's `.harnix/.gitignore`. Neither failing undoes the run.
pub(crate) async fn made(engine: &Engine, run: &str) {
    let Ok(rec) = engine.record(run).await else { return };
    if let Some(dir) = &rec.output_dir {
        if let Err(e) = std::fs::create_dir_all(dir) {
            tracing::warn!(run, dir, error = %e, "the run's output directory not made");
        }
    }
    if let (Some(root), Some(layout)) = (&rec.project_root, engine.layout()) {
        if let Err(e) = ensure_gitignore(Path::new(root), layout) {
            tracing::warn!(run, root, error = %e, "the project's .harnix/.gitignore not written");
        }
    }
}

/// The output directory under `base` for `template`, made: `${seq}` one more than the highest
/// already there, claimed by making it (a start at the same moment takes the next). It must stay
/// inside `base`, links followed. An absolute template is taken as a path in `base`. Returns the
/// directory, and it again when this made it.
pub(crate) fn claim(base: &Path, template: &str, vars: &OutputVars) -> Result<(PathBuf, Option<PathBuf>), String> {
    let template = relative_in(base, template)?;
    let real_base = base
        .canonicalize()
        .map_err(|e| format!("the project {} cannot be read: {e}", base.display()))?;
    let at = |seq: Option<u32>| -> Result<PathBuf, String> {
        let rel = render(&template, vars, seq).map_err(|m| format!("output_dir: {m}"))?;
        let dir = base.join(rel);
        inside(&real_base, &dir)?;
        Ok(dir)
    };
    if !has_seq(&template) {
        let dir = at(None)?;
        let existed = dir.exists();
        std::fs::create_dir_all(&dir).map_err(|e| format!("output_dir {}: {e}", dir.display()))?;
        inside(&real_base, &dir)?;
        return Ok((dir.clone(), (!existed).then_some(dir)));
    }
    let mut seq = highest(&at)? + 1;
    while seq <= MAX_SEQ {
        let dir = at(Some(seq))?;
        if let Some(parent) = dir.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("output_dir {}: {e}", parent.display()))?;
        }
        match std::fs::create_dir(&dir) {
            Ok(()) => {
                if let Err(m) = inside(&real_base, &dir) {
                    let _ = std::fs::remove_dir(&dir);
                    return Err(m);
                }
                return Ok((dir.clone(), Some(dir)));
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => seq += 1,
            Err(e) => return Err(format!("output_dir {}: {e}", dir.display())),
        }
    }
    Err(format!("output_dir {template:?}: every number up to {MAX_SEQ} is taken"))
}

/// The highest `${seq}` already there: every number to 99 looked at, and on from there while they
/// are taken.
fn highest(at: &impl Fn(Option<u32>) -> Result<PathBuf, String>) -> Result<u32, String> {
    let mut top = 0;
    for n in 1..=MAX_SEQ {
        let taken = at(Some(n))?.exists();
        if taken {
            top = n;
        } else if n > 99 {
            break;
        }
    }
    Ok(top)
}

/// A template as a path relative to `base`: an absolute one must be in it.
fn relative_in(base: &Path, template: &str) -> Result<String, String> {
    if !Path::new(template).is_absolute() {
        return Ok(template.to_string());
    }
    let norm = crate::kernel::parts::normalize(template, None);
    let root = crate::kernel::parts::normalize(&base.to_string_lossy(), None);
    match norm.strip_prefix(&format!("{}/", root.trim_end_matches('/'))) {
        Some(rel) if !rel.is_empty() => Ok(rel.to_string()),
        _ => Err(format!("output_dir {template:?} is not inside the project {}", base.display())),
    }
}

/// That `dir` is inside `real_base` (canonical), as far as it exists: its nearest existing ancestor,
/// links followed, is `real_base` or under it.
fn inside(real_base: &Path, dir: &Path) -> Result<(), String> {
    let mut at = Some(dir);
    while let Some(p) = at {
        if let Ok(real) = p.canonicalize() {
            return if real.starts_with(real_base) {
                Ok(())
            } else {
                Err(format!("output_dir {} leads outside the project", dir.display()))
            };
        }
        at = p.parent();
    }
    Err(format!("output_dir {} leads outside the project", dir.display()))
}

/// `<project>/.harnix/.gitignore` with `runs/`: written when there is none, the missing line added
/// to one that is there.
pub(crate) fn ensure_gitignore(project: &Path, layout: &ProjectLayout) -> std::io::Result<()> {
    let dir = layout.data_dir(project);
    std::fs::create_dir_all(&dir)?;
    let file = dir.join(".gitignore");
    let had = match std::fs::read_to_string(&file) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e),
    };
    if had.lines().map(str::trim).any(|l| l == IGNORED) {
        return Ok(());
    }
    let mut text = had.clone();
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    text.push_str(IGNORED);
    text.push('\n');
    std::fs::write(&file, text)
}

/// Where the module shows what a run is doing: `<project>/.harnix/runs/<run>/` — the project's
/// own, a worktree's run's too — or the same under a run of no project's own directory.
pub(crate) fn process_dir(engine: &Engine, rec: &RunRecord) -> Option<PathBuf> {
    let base = match &rec.project_root {
        Some(root) => PathBuf::from(root),
        None => engine.workspace(&rec.id)?,
    };
    Some(workspace_of(&engine.layout()?.runs_dir(&base), &rec.id))
}

/// A station's directory in the process directory.
pub(crate) fn station_dir(engine: &Engine, rec: &RunRecord, station: &str) -> Option<PathBuf> {
    Some(process_dir(engine, rec)?.join("stations").join(file_name(station)))
}

/// A name as one file name: what a path cannot hold made `-`, never `.` or `..`.
fn file_name(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') { c } else { '-' })
        .collect();
    if s.is_empty() || s.chars().all(|c| c == '.') {
        "-".repeat(s.len().max(1))
    } else {
        s
    }
}

/// Write the run's projection into its process directory, after a change. A failure is logged.
pub(crate) fn project_run(engine: &Engine, rec: &RunRecord) {
    let Some(dir) = process_dir(engine, rec) else { return };
    if let Err(e) = write_projection(&dir, rec) {
        tracing::warn!(run = rec.id, dir = %dir.display(), error = %e, "the run's process directory not written");
    }
}

fn write_projection(dir: &Path, rec: &RunRecord) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    write_if_changed(&dir.join("task.json"), &pretty(&task_json(rec)))?;
    let pinned = dir.join("flow.yaml");
    if !pinned.exists() {
        std::fs::write(&pinned, flow_yaml(rec))?;
    }
    for s in &rec.def.stations {
        let Some(r) = rec.stations.get(&s.id) else { continue };
        let one = matches!(s.work(), Work::One(_));
        for t in &r.tasks {
            let (TaskStatus::Submitted, Some(output)) = (t.status, &t.output) else { continue };
            let sdir = dir.join("stations").join(file_name(&s.id));
            std::fs::create_dir_all(&sdir)?;
            let name = if one { "output.json".to_string() } else { format!("{}.json", file_name(&t.id)) };
            write_if_changed(&sdir.join(name), &pretty(output))?;
        }
    }
    Ok(())
}

/// What `task.json` holds: the run as a person or a tool reading the project would want it.
fn task_json(rec: &RunRecord) -> Value {
    let stations: Vec<Value> = rec
        .def
        .stations
        .iter()
        .filter_map(|s| {
            let r = rec.stations.get(&s.id)?;
            Some(json!({
                "id": s.id,
                "title": s.title,
                "status": r.status,
                "round": r.round,
                "tasks": r.tasks.iter().map(|t| json!({
                    "id": t.id, "status": t.status, "attempt": t.attempt, "error": t.error,
                })).collect::<Vec<_>>(),
                "checks": r.checks.iter().filter(|c| c.round == r.round).map(|c| json!({
                    "kind": c.kind, "label": c.label, "status": c.status, "detail": c.detail,
                })).collect::<Vec<_>>(),
                "usage": r.usage(),
            }))
        })
        .collect();
    json!({
        "run": rec.id,
        "flow": rec.flow,
        "title": rec.title,
        "subject": rec.subject,
        "inputs": rec.inputs,
        "trigger": rec.trigger,
        "status": rec.status,
        "phase": rec.phase(),
        "created_at": rec.created_at,
        "finished_at": rec.finished_at,
        "project_root": rec.project_root,
        "workspace": rec.workspace,
        "output_dir": rec.output_dir,
        "worktree": rec.worktree,
        "stations": stations,
        "usage": rec.usage(),
    })
}

/// What `flow.yaml` holds: the definition the run started from, with its digest and the policies
/// and gates pinned with it.
fn flow_yaml(rec: &RunRecord) -> String {
    let doc = json!({
        "run": rec.id,
        "digest": rec.def_digest,
        "definition": rec.def,
        "parts": rec.parts,
    });
    serde_yaml::to_string(&doc).unwrap_or_default()
}

fn pretty(v: &Value) -> String {
    let mut s = serde_json::to_string_pretty(v).unwrap_or_else(|_| v.to_string());
    s.push('\n');
    s
}

/// A file written only when what it holds would change.
fn write_if_changed(path: &Path, text: &str) -> std::io::Result<()> {
    if std::fs::read_to_string(path).is_ok_and(|had| had == text) {
        return Ok(());
    }
    std::fs::write(path, text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars<'a>(inputs: &'a Value) -> OutputVars<'a> {
        OutputVars { flow: "review", date: "20261116", title: "", inputs }
    }

    #[test]
    fn seq_is_one_more_than_the_highest_already_there() {
        let p = tempfile::tempdir().unwrap();
        let none = json!({});
        let (a, made) = claim(p.path(), DEFAULT_OUTPUT_DIR, &vars(&none)).unwrap();
        assert_eq!(a, p.path().join("flows/review-2026111601"));
        assert_eq!(made.as_deref(), Some(a.as_path()));
        assert!(a.is_dir());
        let (b, _) = claim(p.path(), DEFAULT_OUTPUT_DIR, &vars(&none)).unwrap();
        assert_eq!(b, p.path().join("flows/review-2026111602"));
        // The highest counts, not the first gap.
        std::fs::create_dir_all(p.path().join("flows/review-2026111607")).unwrap();
        let (c, _) = claim(p.path(), DEFAULT_OUTPUT_DIR, &vars(&none)).unwrap();
        assert_eq!(c, p.path().join("flows/review-2026111608"));
        // Another flow, another day: numbered on its own.
        let other = OutputVars { flow: "other", ..vars(&none) };
        assert!(claim(p.path(), DEFAULT_OUTPUT_DIR, &other).unwrap().0.ends_with("flows/other-2026111601"));
    }

    #[test]
    fn a_directory_without_a_number_is_taken_as_it_is() {
        let p = tempfile::tempdir().unwrap();
        let inputs = json!({"mode": "auto"});
        let (a, made) = claim(p.path(), "out/${inputs.mode}", &vars(&inputs)).unwrap();
        assert_eq!((a.clone(), made.is_some()), (p.path().join("out/auto"), true));
        let (b, made) = claim(p.path(), "out/${inputs.mode}", &vars(&inputs)).unwrap();
        assert_eq!((b, made), (a, None), "there already: not made again, not taken back");
        // An absolute path inside the project is a path in it.
        let abs = format!("{}/abs/${{date}}", p.path().display());
        assert_eq!(claim(p.path(), &abs, &vars(&inputs)).unwrap().0, p.path().join("abs/20261116"));
    }

    #[test]
    fn nothing_leads_outside_the_project() {
        let p = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let none = json!({"x": "../.."});
        for t in ["../x", "a/../../x", "/etc/out", "out/${inputs.x}/../../.."] {
            assert!(claim(p.path(), t, &vars(&none)).is_err(), "{t}");
        }
        // A value with `..` stays one name inside.
        assert!(claim(p.path(), "out/${inputs.x}", &vars(&none)).unwrap().0.starts_with(p.path()));
        // A link out of the project is not followed out.
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(outside.path(), p.path().join("link")).unwrap();
            let e = claim(p.path(), "link/${date}${seq}", &vars(&none)).unwrap_err();
            assert!(e.contains("outside the project"), "{e}");
            assert!(std::fs::read_dir(outside.path()).unwrap().next().is_none(), "nothing made out there");
        }
    }

    #[test]
    fn the_gitignore_is_written_once_and_never_clobbered() {
        let p = tempfile::tempdir().unwrap();
        let layout = super::super::layout(p.path());
        ensure_gitignore(p.path(), &layout).unwrap();
        let file = p.path().join(".harnix/.gitignore");
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "runs/\n");
        ensure_gitignore(p.path(), &layout).unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "runs/\n");
        std::fs::write(&file, "# mine\nsettings.local.json").unwrap();
        ensure_gitignore(p.path(), &layout).unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "# mine\nsettings.local.json\nruns/\n");
    }

    /// The project's own flows are where the packs read them: in its `.harnix/`, the engine's too;
    /// its worktrees are in the module's data directory, on branches of its own.
    #[test]
    fn the_projects_flows_are_in_the_directory_it_shares_with_the_engine() {
        let root = Path::new("/p");
        let layout = super::super::layout(Path::new("/state/ext/flow/data"));
        assert_eq!(crate::packs::project_pack_dir(root, &layout), Path::new("/p/.harnix/flows"));
        assert_eq!(layout.dir, harnix_ext::engine::PROJECT_DIR);
        assert_eq!(layout.worktree_dir("f-1"), Path::new("/state/ext/flow/data/worktrees/f-1"));
        assert_eq!(layout.worktree_branch("f-1"), "harnix/flow/f-1");
    }

    #[tokio::test]
    async fn a_task_starts_with_its_output_directory_and_its_process_directory_follows_each_change() {
        use crate::{CreateIn, ManualClock, MemPacks, MemStore};
        let project = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let packs = MemPacks::new().flow(
            "{name: two, inputs: {area: {schema: {type: string}, default: core}}, stations: [\
             {id: one, executor: {kind: external}, output: {type: object}, exit: [{kind: output}]}, \
             {id: many, tasks: [{id: x, executor: {kind: external}}, {id: y, executor: {kind: external}}], output: {type: object}, exit: [{kind: output}]}]}",
        );
        let engine = Engine::new(std::sync::Arc::new(MemStore::new()), std::sync::Arc::new(packs))
            .with_clock(std::sync::Arc::new(ManualClock::new(1_794_787_200_000)))
            .with_workspaces(data.path().join("workspaces"))
            .with_layout(super::super::layout(data.path()));
        let root = project.path().to_string_lossy().into_owned();
        let by = Actor::device("d");
        let started = |output_dir: Option<&str>| CreateIn {
            flow: "two".into(),
            subject: "s".into(),
            project_root: Some(root.clone()),
            output_dir: output_dir.map(String::from),
            ..Default::default()
        };
        let run = start(&Shared::for_tests(), &engine, started(None), by.clone()).await.unwrap().value["run"].as_str().unwrap().to_string();
        let rec = engine.record(&run).await.unwrap();
        let date = local_date(1_794_787_200_000);
        let out = project.path().join(format!("flows/two-{date}01"));
        assert_eq!(rec.output_dir.as_deref(), Some(out.to_str().unwrap()));
        assert!(out.is_dir());
        assert_eq!(std::fs::read_to_string(project.path().join(".harnix/.gitignore")).unwrap(), "runs/\n");
        // The card of the conversation it might have been started from names the task.
        let card = task_card(&rec);
        assert_eq!((card["run"].as_str(), card["flow"].as_str(), card["subject"].as_str()), (Some(run.as_str()), Some("two"), Some("s")));
        assert_eq!(card["project_root"].as_str(), Some(root.as_str()));
        // The next task of the flow that day takes the next number; one of its own template, its own.
        let second = start(&Shared::for_tests(), &engine, started(None), by.clone()).await.unwrap().value["run"].as_str().unwrap().to_string();
        assert!(engine.record(&second).await.unwrap().output_dir.unwrap().ends_with(&format!("two-{date}02")));
        let own = start(&Shared::for_tests(), &engine, started(Some("out/${inputs.area}")), by.clone()).await.unwrap().value["run"].as_str().unwrap().to_string();
        assert_eq!(engine.record(&own).await.unwrap().output_dir, Some(format!("{root}/out/core")));
        // Outside the project: refused, nothing started, nothing made.
        for bad in ["../elsewhere", "/tmp/elsewhere"] {
            let e = start(&Shared::for_tests(), &engine, started(Some(bad)), by.clone()).await.unwrap_err();
            assert_eq!(e.code(), "invalid", "{bad}: {e}");
        }
        assert_eq!(engine.list(Default::default()).await.unwrap().len(), 3);
        // A flow that cannot start takes back the directory it was given.
        let mut refused = started(None);
        refused.inputs = json!({"area": 1});
        assert!(start(&Shared::for_tests(), &engine, refused, by.clone()).await.is_err());
        assert!(!project.path().join(format!("flows/two-{date}03")).exists());
        assert_eq!(std::fs::read_dir(project.path().join("flows")).unwrap().count(), 2);

        // The projection, after each change.
        let rec = engine.record(&run).await.unwrap();
        project_run(&engine, &rec);
        let dir = project.path().join(".harnix/runs").join(&run);
        let task: Value = serde_json::from_str(&std::fs::read_to_string(dir.join("task.json")).unwrap()).unwrap();
        assert_eq!((task["flow"].as_str(), task["output_dir"].as_str()), (Some("two"), out.to_str()));
        assert_eq!(task["stations"][0]["status"], "working");
        assert_eq!(task["trigger"]["kind"], "manual");
        let pinned: Value = serde_yaml::from_str(&std::fs::read_to_string(dir.join("flow.yaml")).unwrap()).unwrap();
        assert_eq!(pinned["definition"]["name"], "two");
        assert_eq!(pinned["digest"], rec.def_digest.as_str());
        let token = engine.claim(&run, "one", "one", by.clone(), 60_000).await.unwrap().value["token"].as_str().unwrap().to_string();
        engine.submit(&run, "one", "one", &token, json!({"done": 1}), by.clone()).await.unwrap();
        for t in ["x", "y"] {
            let token = engine.claim(&run, "many", t, by.clone(), 60_000).await.unwrap().value["token"].as_str().unwrap().to_string();
            engine.submit(&run, "many", t, &token, json!({"task": t}), by.clone()).await.unwrap();
        }
        let rec = engine.record(&run).await.unwrap();
        project_run(&engine, &rec);
        let read = |p: &str| serde_json::from_str::<Value>(&std::fs::read_to_string(dir.join(p)).unwrap()).unwrap();
        assert_eq!(read("stations/one/output.json"), json!({"done": 1}));
        assert_eq!(read("stations/many/x.json"), json!({"task": "x"}));
        assert_eq!(read("stations/many/y.json"), json!({"task": "y"}));
        assert_eq!(read("task.json")["status"], "finished");
        // Nothing else of the module's in the project.
        let mut top: Vec<String> = std::fs::read_dir(project.path()).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        top.sort();
        assert_eq!(top, [".harnix", "flows", "out"]);
    }

    #[tokio::test]
    async fn a_run_of_no_project_delivers_under_its_own_directory() {
        use crate::{CreateIn, ManualClock, MemPacks, MemStore};
        let data = tempfile::tempdir().unwrap();
        let packs = MemPacks::new().flow("{name: solo, stations: [{id: a, exit: [{kind: confirm}]}]}");
        let engine = Engine::new(std::sync::Arc::new(MemStore::new()), std::sync::Arc::new(packs))
            .with_clock(std::sync::Arc::new(ManualClock::new(1_794_787_200_000)))
            .with_workspaces(data.path().join("workspaces"))
            .with_layout(super::super::layout(data.path()));
        let input = CreateIn { flow: "solo".into(), subject: "s".into(), ..Default::default() };
        let run = start(&Shared::for_tests(), &engine, input, Actor::device("d")).await.unwrap().value["run"].as_str().unwrap().to_string();
        let rec = engine.record(&run).await.unwrap();
        let own = engine.workspace(&run).unwrap();
        let out = own.join(format!("flows/solo-{}01", local_date(1_794_787_200_000)));
        assert_eq!(rec.output_dir.as_deref(), Some(out.to_str().unwrap()));
        assert!(out.is_dir());
        project_run(&engine, &rec);
        assert!(own.join(".harnix/runs").join(&run).join("task.json").is_file());
        let absolute = CreateIn { flow: "solo".into(), subject: "s".into(), output_dir: Some("/tmp/x".into()), ..Default::default() };
        assert_eq!(start(&Shared::for_tests(), &engine, absolute, Actor::device("d")).await.unwrap_err().code(), "invalid");
    }

    #[test]
    fn a_local_date_is_eight_digits() {
        let d = local_date(1_794_787_200_000);
        assert_eq!(d.len(), 8);
        assert!(d.starts_with("2026111"), "{d}");
    }

    #[test]
    fn a_name_is_one_file_name() {
        assert_eq!(file_name("src/a.rs"), "src-a.rs");
        assert_eq!(file_name(".."), "--");
        assert_eq!(file_name("ok_1"), "ok_1");
    }
}
