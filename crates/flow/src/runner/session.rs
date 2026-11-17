//! A session task: a session the host opens where the station places it — its placement (space,
//! instance, template, model) handed to the host to interpret — or the one its session key is
//! bound to, told the protocol layer, the rules and the skill. The station's contract is the
//! session's output schema when it hands in by structured output; the session asks about every
//! guarded tool and every tool a permission names, and the runner answers each question from the
//! permissions of the task the turn works on, whatever carries the session. After every turn, the
//! turn's structured output (if any) is submitted, the run is evaluated and the claim renewed; an
//! output the station still judges unmet is answered with what is missing; a turn that ends with
//! nothing handed in gets one nudge, and a second one fails the attempt. Tasks sharing a session
//! take turns, and a task waiting for its turn keeps its claim alive. A session is bound to a
//! task only while no turn of anyone else's is running in it, and a task that is over while its
//! turn still runs interrupts that turn and waits for it to settle before the session goes to the
//! next task.

use std::pin::Pin;
use std::sync::Arc;

use futures::{Stream, StreamExt};
use serde_json::{json, Value};

use super::dispatch::{dispatcher, fail, renew, renew_every, settle};
use super::host::{Frame, OpenSpec, SessionId};
use super::state::{Binding, TaskKey};
use super::turns::{carry, wait_turn, HostSession, Turns, Waited};
use super::Runner;
use crate::kernel::host::{Delivery, Placement};
use crate::kernel::parts::asked_tools;
use crate::kernel::protocol::{skill_with_output_dir, task_message};
use crate::kernel::run::{RunRecord, StationStatus, TaskStatus};

/// What a session is opened with and keeps for good.
pub(crate) fn settings_of(
    job_scene: &str,
    delivery: Delivery,
    output: Option<&Value>,
    p: &Placement,
    instance: Option<&str>,
) -> Value {
    json!({
        "scene": job_scene,
        "delivery": delivery,
        "output_schema": match delivery {
            Delivery::StructuredOutput => output.cloned().unwrap_or(Value::Null),
            Delivery::FlowSubmit => Value::Null,
        },
        "model": p.model, "provider": p.provider, "effort": p.effort,
        "space": p.space, "template": p.template, "instance": instance.map(String::from).or_else(|| p.instance.clone()),
    })
}

/// The fields of two sessions' settings that differ.
pub(crate) fn settings_differ(opened: &Value, wanted: &Value) -> Vec<String> {
    let (Some(a), Some(b)) = (opened.as_object(), wanted.as_object()) else {
        return vec![];
    };
    let mut keys: Vec<&String> = a.keys().chain(b.keys()).collect();
    keys.sort();
    keys.dedup();
    keys.into_iter().filter(|k| a.get(*k) != b.get(*k)).map(|k| k.to_string()).collect()
}

/// The tool patterns a flow session asks about: every guarded tool and every tool a permission of
/// the run's flow names, so each call of them comes to the runner to be judged. The same whatever
/// carries the session.
pub(crate) fn asks_of(rn: &Runner, rec: &RunRecord) -> Vec<String> {
    asked_tools(&rec.def, &rec.parts.policies, &rn.cfg.guarded)
}

/// Where a task stands, as far as one claim is concerned.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Standing {
    /// Still claimed under this token: work on.
    Claimed,
    /// Handed in under this token.
    Submitted,
    /// Someone else's now, or over: stop.
    Gone,
}

/// A claim is its token: a partial rework that starts a new round keeps the tasks it did not
/// name claimed under their tokens, and they are still to be worked on.
pub(crate) fn standing_of(rec: &RunRecord, key: &TaskKey, token: &str) -> Standing {
    if !rec.is_active() {
        return Standing::Gone;
    }
    let Some(st) = rec.stations.get(&key.station) else {
        return Standing::Gone;
    };
    if !matches!(st.status, StationStatus::Working | StationStatus::Unmet) {
        return Standing::Gone;
    }
    let Some(t) = st.task(&key.task) else {
        return Standing::Gone;
    };
    let ours = t.lease.as_ref().is_some_and(|l| l.token == token);
    match t.status {
        TaskStatus::Claimed if ours => Standing::Claimed,
        TaskStatus::Submitted if ours => Standing::Submitted,
        _ => Standing::Gone,
    }
}

pub(crate) async fn standing(rn: &Runner, key: &TaskKey, token: &str) -> Standing {
    match rn.engine().record(&key.run).await {
        Ok(rec) => standing_of(&rec, key, token),
        Err(_) => Standing::Gone,
    }
}

pub(crate) struct SessionJob {
    pub(crate) key: TaskKey,
    pub(crate) token: String,
    pub(crate) scene: String,
    pub(crate) skill: Option<String>,
    pub(crate) skill_digest: Option<String>,
    /// The rules it works under, with the digests pinned on station entry.
    pub(crate) rules: Vec<(String, Option<String>)>,
    /// What it works under.
    pub(crate) permissions: Vec<crate::kernel::parts::Granted>,
    pub(crate) protocol: String,
    pub(crate) session_key: Option<String>,
    pub(crate) recording: Option<String>,
    pub(crate) resume: bool,
    pub(crate) output_schema: Option<Value>,
    pub(crate) placement: Placement,
    pub(crate) delivery: Delivery,
    /// A turn of the run's main session.
    pub(crate) main: bool,
}

/// The project a flow session of `rec` is opened in: a task's own (where the run works, its
/// worktree when it has one); for a run of no project, the run's own directory when the profile
/// needs a project (made now), else none. Nothing of the runner's is written into a task's project
/// here.
pub(crate) fn session_root(rn: &Runner, rec: &RunRecord, scene: &str) -> Result<Option<std::path::PathBuf>, String> {
    let engine = rn.engine();
    if rec.workspace.in_project() && rec.project_root.is_some() {
        return Ok(engine.workspace_for(rec));
    }
    if !rn.cfg.project_profiles.contains(scene) {
        return Ok(None);
    }
    let dir = engine
        .workspace(&rec.id)
        .ok_or("the runner has no directory for runs of no project (Engine::with_workspaces)")?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("no workspace for the run: {e}"))?;
    Ok(Some(dir))
}

/// The key a task's session is opened under, and that tasks sharing a session take turns on: tasks
/// with a session key share it; a task with a session of its own shares it with nobody — not even
/// its own next round or attempt, which gets a new session and must not wait for the old one's
/// turn to end.
pub(crate) fn session_key_of(job: &SessionJob) -> String {
    match &job.session_key {
        Some(k) => format!("flow:{}#{k}", job.key.run),
        None => format!("flow:{}/{}/{}@{}", job.key.run, job.key.station, job.key.task, job.token),
    }
}

pub(crate) async fn session_task(rn: &Arc<Runner>, job: SessionJob) {
    let name = session_key_of(&job);
    let lock = rn.state.session_lock(&name);
    let acquired = wait_turn_on_session(rn, &lock, &job).await;
    if let Some(guard) = acquired {
        with_session(rn, &job).await;
        drop(guard);
    }
    rn.state.release_lock(&name, lock);
}

/// The session's lock, renewing the claim while other tasks have it.
async fn wait_turn_on_session(
    rn: &Arc<Runner>,
    lock: &Arc<tokio::sync::Mutex<()>>,
    job: &SessionJob,
) -> Option<tokio::sync::OwnedMutexGuard<()>> {
    let acquire = lock.clone().lock_owned();
    tokio::pin!(acquire);
    let every = renew_every(rn);
    loop {
        tokio::select! {
            g = &mut acquire => return Some(g),
            _ = tokio::time::sleep(every) => {
                if !renew(rn, &job.key, &job.token).await {
                    return None;
                }
            }
            _ = rn.cancelled().cancelled() => return None,
        }
    }
}

/// With the session's lock held: find or open the session, bind it, carry the task.
async fn with_session(rn: &Arc<Runner>, job: &SessionJob) {
    let engine = rn.engine();
    let host = rn.host();
    let key = &job.key;
    let Ok(rec) = engine.record(&key.run).await else {
        return;
    };
    if standing_of(&rec, key, &job.token) != Standing::Claimed {
        return;
    }
    // Paused while it waited for its session: nothing was sent, so it goes back to ready and is
    // dispatched again on resume (what is not begun does not begin).
    if rec.is_paused() && !job.resume {
        let r = engine.requeue(&key.run, &key.station, &key.task, &job.token).await;
        settle(rn, &key.run, "requeue", r).await;
        return;
    }

    // The task's own session (it reported one before a restart), else its key's (bound by a
    // task that went before it), else a new one. Only its own is resumed: a task that never
    // told its session anything is told everything.
    let own = rec.stations.get(&key.station).and_then(|s| s.task(&key.task)).and_then(|t| t.session.clone());
    // A key's session, and what it was opened with (`settings_of`): a task that finds its key
    // bound to a session opened otherwise fails instead of working under another station's
    // contract.
    let bound = job.session_key.as_ref().and_then(|k| rec.session_keys.get(k).cloned());
    let keyed = bound.as_ref().map(|b| b.id.clone());
    let mut opened: Option<(SessionId, bool)> = None;
    for (id, is_own) in [(own.clone(), true), (keyed, false)] {
        let Some(id) = id else { continue };
        match host.lookup(&id).await {
            Ok(Some(_)) => {
                opened = Some((id, is_own));
                break;
            }
            Ok(None) => tracing::info!(session = id, "session gone"),
            Err(e) => tracing::info!(session = id, error = %e, "session not looked up"),
        }
    }
    let resume = job.resume && opened.as_ref().is_some_and(|(_, own)| *own);
    let wanted = settings_of(
        &job.scene,
        job.delivery,
        job.output_schema.as_ref(),
        &job.placement,
        job.placement.template_instance(&job.key.run).as_deref(),
    );
    let session = match opened {
        Some((s, own)) => {
            // Validation keeps tasks that share a session alike; this is the net under it — a
            // definition from before that rule, or keys that resolve alike by chance. The main
            // session keeps no station's settings.
            if !own && !job.main {
                if let Some(opened_with) = bound.as_ref().map(|b| &b.opened_with).filter(|v| !v.is_null()) {
                    let d = settings_differ(opened_with, &wanted);
                    if !d.is_empty() {
                        let why = format!(
                            "the run's shared session {s} was opened with other settings ({}) and \
                             keeps them: this task cannot work in it",
                            d.join(", ")
                        );
                        fail(rn, key, &job.token, &why).await;
                        return;
                    }
                }
            }
            s
        }
        None if job.main => match super::main::open_or_make(rn, &key.run).await {
            Ok(s) => s,
            Err(why) => {
                fail(rn, key, &job.token, &why).await;
                return;
            }
        },
        None => {
            let project_root = match session_root(rn, &rec, &job.scene) {
                Ok(r) => r,
                Err(why) => {
                    fail(rn, key, &job.token, &why).await;
                    return;
                }
            };
            let mut spec = OpenSpec::new(session_key_of(job), key.run.clone(), job.scene.clone(), job.delivery);
            spec.project_root = project_root;
            spec.placement = job.placement.clone();
            spec.asks = asks_of(rn, &rec);
            spec.output_schema = match job.delivery {
                Delivery::StructuredOutput => job.output_schema.clone(),
                Delivery::FlowSubmit => None,
            };
            spec.recording = job.recording.clone();
            spec.tools = rn.tools().specs().to_vec();
            match host.open(spec).await {
                Ok(o) => o.id,
                Err(e) => {
                    fail(rn, key, &job.token, &format!("no session: {e}")).await;
                    return;
                }
            }
        }
    };
    // Its questions are answered from the task its turn works on; a main session's watch also
    // counts its turns.
    super::main::watch(rn, &key.run, &session, job.main).await;
    let watch = match host.watch(&session).await {
        Ok(w) => w,
        Err(e) => {
            fail(rn, key, &job.token, &format!("session not watched: {e}")).await;
            return;
        }
    };
    let running = watch.info.turn_running;
    // A main session's turns are told apart by the messages that start them, so its task reads
    // every frame; a session of the task's own needs only how each turn ended.
    let mut read: Pin<Box<dyn Stream<Item = Frame> + Send>> = if job.main {
        watch.frames
    } else {
        Box::pin(watch.frames.filter(|f| std::future::ready(matches!(f, Frame::TurnSettled { .. }))))
    };
    if running && !resume {
        // Someone else's turn — the task before, a person: it ends before this task binds,
        // and its settlement is read here, not taken for this task's.
        match wait_turn(rn, &mut read, key, &job.token).await {
            Waited::Settled(_) => {}
            Waited::Superseded | Waited::Gone => return,
        }
    }
    let attached = engine
        .attach_session(
            &key.run,
            &key.station,
            &key.task,
            &job.token,
            &session,
            job.session_key.as_deref(),
            &wanted,
            dispatcher(),
        )
        .await;
    if !settle(rn, &key.run, "attach_session", attached).await {
        return;
    }
    rn.state.bind(
        &session,
        Binding {
            run: key.run.clone(),
            station: key.station.clone(),
            round: key.round,
            task: key.task.clone(),
            token: job.token.clone(),
            permissions: job.permissions.clone(),
        },
    );
    let turns = Turns {
        main: job.main,
        delivery: job.delivery,
        // The run's project's rules and skill: its own packs over the host's.
        message: (!resume).then(|| {
            let packs = engine.packs_for(rec.project_root.as_deref());
            task_message_of(job, packs.as_ref(), rec.output_dir.as_deref())
        }),
        resumed_running: resume && running,
    };
    let in_session = HostSession { rn, host: host.as_ref(), id: &session, binding: rn.state.binding(&session) };
    carry(rn, key, &job.token, turns, &in_session, &mut read).await;
    // Handed in: the binding stays, so the session can hand in again until the station
    // passes (the next change drops it then). Otherwise it goes now.
    if standing(rn, key, &job.token).await != Standing::Submitted {
        rn.state.unbind(&session, &job.token);
    }
}

/// The protocol layer, the rules, then the skill (`${output_dir}` in it put in), from the packs
/// of the run's project.
fn task_message_of(job: &SessionJob, packs: &dyn crate::kernel::host::Packs, output_dir: Option<&str>) -> String {
    let mut rules = vec![];
    for (name, pinned) in &job.rules {
        match packs.rule(name) {
            Some((text, digest)) => {
                if pinned.as_ref().is_some_and(|d| *d != digest) {
                    tracing::warn!(rule = name, "the rule changed since the station was entered");
                }
                rules.push((name.clone(), text));
            }
            None => tracing::warn!(rule = name, "rule not found"),
        }
    }
    let mut skill = None;
    if let Some(name) = &job.skill {
        match packs.skill(name) {
            Some((body, digest)) => {
                if job.skill_digest.as_ref().is_some_and(|d| *d != digest) {
                    tracing::warn!(skill = name, "the skill changed since the station was entered");
                }
                skill = Some((name.clone(), skill_with_output_dir(&body, output_dir)));
            }
            None => tracing::warn!(skill = name, "skill not found"),
        }
    }
    task_message(&job.protocol, &rules, skill.as_ref().map(|(n, b)| (n.as_str(), b.as_str())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::host::{Outcome, WorkItem};
    use crate::kernel::protocol::ask_rules;
    use crate::kernel::run::{Actor, Workspace};
    use crate::runner::{RunnerConfig, ScriptedHost};
    use crate::{CreateIn, Engine, ManualClock, MemPacks, MemStore, ProjectLayout};

    fn job(round: u32, token: &str, session_key: Option<&str>) -> SessionJob {
        SessionJob {
            main: false,
            key: TaskKey { run: "r".into(), station: "s".into(), round, task: "t".into() },
            token: token.into(),
            scene: "flow-harnix".into(),
            skill: None,
            skill_digest: None,
            rules: vec![],
            permissions: vec![],
            protocol: String::new(),
            session_key: session_key.map(String::from),
            recording: None,
            resume: false,
            output_schema: None,
            placement: Placement::default(),
            delivery: Delivery::StructuredOutput,
        }
    }

    #[test]
    fn a_shared_session_names_what_it_was_opened_with_differently() {
        let p = Placement::default();
        let schema = json!({"type": "object"});
        let a = settings_of("harnix", Delivery::StructuredOutput, Some(&schema), &p, None);
        assert!(settings_differ(&a, &a.clone()).is_empty());
        let other = json!({"type": "object", "required": ["x"]});
        let contract = settings_of("harnix", Delivery::StructuredOutput, Some(&other), &p, None);
        assert_eq!(settings_differ(&a, &contract), ["output_schema"]);
        let model = Placement { model: Some("m".into()), ..Default::default() };
        let m = settings_of("harnix", Delivery::StructuredOutput, Some(&schema), &model, None);
        assert_eq!(settings_differ(&a, &m), ["model"]);
        let effort = Placement { effort: Some("high".into()), ..Default::default() };
        let e = settings_of("harnix", Delivery::StructuredOutput, Some(&schema), &effort, None);
        assert_eq!(settings_differ(&a, &e), ["effort"]);
    }

    #[test]
    fn a_flow_session_asks_about_every_guarded_tool_and_every_tool_a_permission_names() {
        let def = crate::FlowDef::parse(
            "{name: f, permissions: [{effect: allow, tools: [mcp__desk__emit_row]}], stations: [{id: a, policies: [p], exit: [{kind: confirm}]}]}",
        )
        .unwrap();
        let policies = [(
            "p".to_string(),
            serde_yaml::from_str::<crate::PolicyDef>("{name: p, permissions: [{effect: deny, tools: [Read], paths: ['/etc/**']}]}").unwrap(),
        )]
        .into();
        let g: Vec<String> = ["Bash", "mcp__db__*"].map(String::from).to_vec();
        assert_eq!(
            ask_rules(&asked_tools(&def, &policies, &g)),
            json!([
                {"source": "session", "behavior": "ask", "tool_name": "Bash"},
                {"source": "session", "behavior": "ask", "tool_name": "mcp__db"},
                {"source": "session", "behavior": "ask", "tool_name": "mcp__desk__emit_row"},
                {"source": "session", "behavior": "ask", "tool_name": "Read"},
            ])
        );
    }

    #[test]
    fn only_tasks_sharing_a_session_wait_for_each_other() {
        assert_ne!(
            session_key_of(&job(1, "a", None)),
            session_key_of(&job(2, "b", None)),
            "a rework's new round has a new session: it does not wait for the old turn"
        );
        assert_eq!(session_key_of(&job(1, "a", Some("run"))), session_key_of(&job(1, "b", Some("run"))));
        assert_eq!(session_key_of(&job(1, "a", Some("run"))), "flow:r#run");
        assert_eq!(session_key_of(&job(1, "a", None)), "flow:r/s/t@a");
    }

    const MULTI: &str = r#"
name: multi
stations:
  - {id: collect, executor: {kind: program, step: collect}, output: {type: object}, exit: [{kind: output}]}
  - id: analyze
    foreach: ${stations.collect.output.modules}
    executor: {kind: program, step: work}
    output: {type: object}
    exit: [{kind: output}]
  - {id: next, exit: [{kind: confirm}]}
"#;

    fn engine() -> (Engine, Arc<ManualClock>) {
        let clock = Arc::new(ManualClock::new(1_000_000));
        let packs = MemPacks::new().flow(MULTI).step("collect").step("work");
        let eng = Engine::new(Arc::new(MemStore::new()), Arc::new(packs)).with_clock(clock.clone()).with_lease_ms(60_000);
        (eng, clock)
    }

    fn tasks(o: &Outcome) -> Vec<(String, u32, String)> {
        o.work
            .iter()
            .filter_map(|w| match w {
                WorkItem::Task { task, round, token, .. } => Some((task.clone(), *round, token.clone())),
                _ => None,
            })
            .collect()
    }

    #[tokio::test]
    async fn a_tasks_sessions_work_in_its_project_and_a_run_of_none_in_its_own_directory() {
        let project = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let engine = Arc::new(
            Engine::new(
                Arc::new(MemStore::new()),
                Arc::new(MemPacks::new().flow("{name: f, stations: [{id: a, exit: [{kind: confirm}]}]}")),
            )
            .with_workspaces(data.path().join("workspaces"))
            .with_layout(ProjectLayout::new(".harnix", data.path().join("worktrees"), "harnix/flow")),
        );
        let cfg = RunnerConfig { project_profiles: ["flow-code".to_string()].into(), ..Default::default() };
        let rn = Runner::for_tests_with(engine.clone(), ScriptedHost::new(), cfg);
        let root = project.path().to_string_lossy().into_owned();
        let started = |workspace: Option<Workspace>, project_root: Option<String>| CreateIn {
            flow: "f".into(),
            subject: "s".into(),
            project_root,
            workspace,
            ..Default::default()
        };
        let run_of = |o: Outcome| o.value["run"].as_str().unwrap().to_string();
        for ws in [None, Some(Workspace::Project), Some(Workspace::Worktree)] {
            let run = run_of(engine.create(started(ws, Some(root.clone())), Actor::device("d")).await.unwrap());
            let rec = engine.record(&run).await.unwrap();
            // Whatever the scene: the main session, a station's own, a gate — all in the project,
            // or in its worktree.
            let at = match ws {
                Some(Workspace::Worktree) => data.path().join("worktrees").join(&run),
                _ => project.path().to_path_buf(),
            };
            for scene in ["flow-harnix", "flow-code"] {
                assert_eq!(session_root(&rn, &rec, scene).unwrap(), Some(at.clone()), "{ws:?} {scene}");
            }
        }
        assert!(std::fs::read_dir(project.path()).unwrap().next().is_none(), "nothing written into the project");
        // A run of no project: its own directory for a scene that needs one, none otherwise.
        let run = run_of(engine.create(started(None, None), Actor::device("d")).await.unwrap());
        let rec = engine.record(&run).await.unwrap();
        assert_eq!(session_root(&rn, &rec, "flow-harnix").unwrap(), None);
        let own = session_root(&rn, &rec, "flow-code").unwrap().unwrap();
        assert_eq!(own, engine.workspace(&run).unwrap());
        assert!(own.is_dir());
        // A task that works in a directory of its own works there too.
        let run = run_of(engine.create(started(Some(Workspace::Scratch), Some(root.clone())), Actor::device("d")).await.unwrap());
        let rec = engine.record(&run).await.unwrap();
        assert_eq!(session_root(&rn, &rec, "flow-harnix").unwrap(), None);
    }

    /// A partial rework starts a new round of the station but keeps the claims it did not
    /// name: the dispatcher must go on working them (the claim is the token, not the round).
    #[tokio::test]
    async fn a_partial_rework_leaves_the_other_claims_standing() {
        let (eng, _) = engine();
        let o = eng
            .create(CreateIn { flow: "multi".into(), subject: "s".into(), ..Default::default() }, Actor::device("d"))
            .await
            .unwrap();
        let run = o.value["run"].as_str().unwrap().to_string();
        let (_, _, collect) = tasks(&o).remove(0);
        let o = eng
            .submit(&run, "collect", "collect", &collect, json!({"modules": ["a", "b"]}), dispatcher())
            .await
            .unwrap();
        let fan = tasks(&o);
        let (a, b) = (fan[0].clone(), fan[1].clone());
        eng.rework(&run, "analyze", Some(vec!["a".into()]), "again", Actor::device("d")).await.unwrap();
        let rec = eng.record(&run).await.unwrap();
        assert_eq!(rec.stations["analyze"].round, 2);
        let key = |(task, round, _): &(String, u32, String)| TaskKey {
            run: run.clone(),
            station: "analyze".into(),
            round: *round,
            task: task.clone(),
        };
        assert_eq!(standing_of(&rec, &key(&b), &b.2), Standing::Claimed);
        assert_eq!(standing_of(&rec, &key(&a), &a.2), Standing::Gone);
    }
}
