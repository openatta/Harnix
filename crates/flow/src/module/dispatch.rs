//! Carrying out what the kernel hands back.
//!
//! Every engine result comes through [`apply`]: its events are broadcast, the runs it touched
//! are published on `run {run}` (lease tokens removed) and the list on `runs`, bindings of
//! claims that are over are dropped, and each work item is started on its own task — a method
//! answers without waiting for a session. A work item's own engine calls come back through
//! [`apply`] again.
//!
//! A session task: a headless session in the flow's space (or the one its session key is
//! bound to), told the protocol layer and the skill; after every turn the run is evaluated and
//! the claim renewed; a turn that ends without a submission gets one nudge, and a second one
//! fails the attempt. Tasks sharing a session take turns, and a task waiting for its turn keeps
//! its claim alive. A session is bound to a task only while no turn of anyone else's is
//! running in it, and a task that is over while its turn still runs interrupts that turn and
//! waits for it to settle before the session goes to the next task. A program keeps its claim
//! alive while it runs.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use harnix_module::{OnBusy, SessionRef, SessionSpec, Tap};
use serde_json::{json, Value};

use super::shared::{session_name, session_ref, Binding, Shared, TaskKey};
use crate::kernel::def::Executor;
use crate::kernel::engine::view;
use crate::kernel::host::{GateResult, Outcome, WorkItem};
use crate::kernel::protocol::skill_body;
use crate::kernel::run::{Actor, RunQuery, RunRecord, RunStatus, StationStatus, TaskStatus};
use crate::kernel::{dispatched_work, FlowError};

/// How many runs the `runs` topic carries, newest first.
pub const RUNS_LISTED: usize = 200;

/// How long a task that is over waits for the turn it interrupted to settle.
const SETTLE_AFTER_INTERRUPT: Duration = Duration::from_secs(30);

/// What a session is told when a turn ends and its task is still not handed in.
pub const NUDGE: &str = "Your flow task is not handed in yet. Call `flow_submit` with an output \
     that satisfies the contract in the flow protocol above (`flow_state` shows what is still \
     missing). If you cannot do the task, say why and use `flow_propose`.";

fn dispatcher() -> Actor {
    Actor::flow("dispatch")
}

fn run_of(w: &WorkItem) -> Option<&str> {
    match w {
        WorkItem::Task { run, .. } | WorkItem::Gate { run, .. } => Some(run),
        WorkItem::Hook { event, .. } => Some(&event.run),
        WorkItem::CloseSession { .. } => None,
    }
}

/// Carry out an engine result.
pub(crate) async fn apply(sh: &Arc<Shared>, hint: Option<&str>, o: Outcome) {
    for e in &o.events {
        let _ = sh.events.send(e.clone());
    }
    let mut runs: BTreeSet<String> = o.events.iter().map(|e| e.run.clone()).collect();
    runs.extend(o.work.iter().filter_map(run_of).map(String::from));
    if let Some(h) = hint {
        runs.insert(h.to_string());
    }
    publish(sh, &runs).await;
    for w in o.work {
        spawn_work(sh, w, false);
    }
}

/// An engine call's result: carried out when it succeeded, logged when not.
async fn settle(sh: &Arc<Shared>, run: &str, what: &str, r: crate::Result<Outcome>) -> bool {
    match r {
        Ok(o) => {
            apply(sh, Some(run), o).await;
            true
        }
        Err(e) => {
            tracing::debug!(run, what, error = %e, "flow: not applied");
            false
        }
    }
}

/// The newest snapshot of each run, and the list; bindings of claims that are over dropped.
pub(crate) async fn publish(sh: &Arc<Shared>, runs: &BTreeSet<String>) {
    let (Some(engine), Some(cx)) = (sh.engine.get(), sh.cx.get()) else {
        return;
    };
    let _order = sh.publish.lock().await;
    let events = cx.events();
    for run in runs {
        if let Ok(rec) = engine.record(run).await {
            sh.prune_bindings(run, |b| binding_current(&rec, b));
            if let Err(e) = events.publish("run", json!({"run": run}), view(rec)).await {
                tracing::warn!(run, error = %e, "flow: run not published");
            }
        }
    }
    match engine
        .list(RunQuery {
            limit: Some(RUNS_LISTED),
            ..Default::default()
        })
        .await
    {
        Ok(list) => {
            let _ = events.publish("runs", json!({}), &list).await;
        }
        Err(e) => tracing::warn!(error = %e, "flow: runs not listed"),
    }
}

/// Whether a binding's claim still counts: its station open, its task claimed or handed in
/// under the binding's token. A task handed in keeps its session bound so it can hand in
/// again until the station passes.
pub(crate) fn binding_current(rec: &RunRecord, b: &Binding) -> bool {
    rec.is_active()
        && rec.stations.get(&b.station).is_some_and(|st| {
            matches!(st.status, StationStatus::Working | StationStatus::Unmet)
                && st.task(&b.task).is_some_and(|t| {
                    matches!(t.status, TaskStatus::Claimed | TaskStatus::Submitted)
                        && t.lease.as_ref().is_some_and(|l| l.token == b.token)
                })
        })
}

/// The list, and every run in it: what a subscriber arriving after start sees.
pub(crate) async fn publish_all(sh: &Arc<Shared>) {
    let Some(engine) = sh.engine.get() else {
        return;
    };
    let listed: BTreeSet<String> = engine
        .list(RunQuery {
            limit: Some(RUNS_LISTED),
            ..Default::default()
        })
        .await
        .map(|l| l.into_iter().map(|s| s.id).collect())
        .unwrap_or_default();
    publish(sh, &listed).await;
}

/// After a restart: whatever was handed out and not answered is carried on with — a session
/// task waits on the session it had, a program runs again, a gate still waiting runs again.
pub(crate) async fn recover(sh: &Arc<Shared>) {
    let Some(engine) = sh.engine.get().cloned() else {
        return;
    };
    let active = match engine
        .list(RunQuery {
            status: Some(RunStatus::Active),
            ..Default::default()
        })
        .await
    {
        Ok(l) => l,
        Err(e) => {
            tracing::warn!(error = %e, "flow: runs to recover could not be listed");
            return;
        }
    };
    let now = engine.clock().now_ms();
    for s in active {
        let Ok(rec) = engine.record(&s.id).await else {
            continue;
        };
        for w in dispatched_work(&rec, &sh.cfg.guarded, now) {
            spawn_work(sh, w, true);
        }
    }
}

/// Lease expiry, once a tick.
pub(crate) async fn tick_loop(sh: Arc<Shared>) {
    let mut every = tokio::time::interval(Duration::from_millis(sh.cfg.tick_ms));
    every.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        every.tick().await;
        if sh.stopping() {
            return;
        }
        let Some(engine) = sh.engine.get().cloned() else {
            continue;
        };
        match engine.tick().await {
            Ok(outcomes) => {
                for o in outcomes {
                    apply(&sh, None, o).await;
                }
            }
            Err(e) => tracing::warn!(error = %e, "flow: tick failed"),
        }
    }
}

pub(crate) fn spawn_work(sh: &Arc<Shared>, w: WorkItem, resume: bool) {
    let sh = sh.clone();
    tokio::spawn(async move { run_work(sh, w, resume).await });
}

async fn run_work(sh: Arc<Shared>, w: WorkItem, resume: bool) {
    if sh.stopping() {
        return;
    }
    let Ok(engine) = sh.engine() else { return };
    match w {
        WorkItem::Task { .. } => {
            let recording = sh.cfg.recording.as_ref().and_then(|f| f(&w));
            let WorkItem::Task {
                run,
                station,
                round,
                task,
                token,
                executor,
                skill,
                skill_digest,
                item,
                protocol,
                session_key,
                ..
            } = w
            else {
                return;
            };
            let key = TaskKey {
                run: run.clone(),
                station: station.clone(),
                round,
                task: task.clone(),
            };
            if !sh.begin(&key, &token) {
                return;
            }
            match executor {
                Executor::Program { step } => {
                    program_task(&sh, &key, &token, &step, item.as_ref()).await
                }
                Executor::Session { scene, .. } => {
                    let job = SessionJob {
                        key: key.clone(),
                        token: token.clone(),
                        scene,
                        skill,
                        skill_digest,
                        protocol: protocol.unwrap_or_default(),
                        session_key,
                        recording,
                        resume,
                    };
                    session_task(&sh, job).await
                }
                Executor::External { .. } | Executor::Subflow { .. } => {}
            }
            sh.end(&key, &token);
        }
        WorkItem::Gate {
            run,
            station,
            round,
            gate,
        } => {
            let result = match (&sh.cfg.gates, engine.get(&run).await) {
                (Some(g), Ok(view)) => g.run(&view.record, &station, &gate).await,
                (None, _) => GateResult {
                    pass: false,
                    detail: format!("no gates are registered to run {gate}"),
                },
                (_, Err(e)) => {
                    tracing::warn!(run, error = %e, "flow: gate not run");
                    return;
                }
            };
            let r = engine
                .gate_result(&run, &station, round, &gate, result)
                .await;
            settle(&sh, &run, "gate_result", r).await;
        }
        WorkItem::Hook { action, event } => match &sh.cfg.hooks {
            Some(h) => h.fire(&action, &event).await,
            None => tracing::info!(action, kind = event.kind, "flow: no hooks registered"),
        },
        WorkItem::CloseSession { session } => {
            let s = session_ref(&session);
            sh.unbind_session(&s.session_id);
            if let Some(cx) = sh.cx.get() {
                let _ = cx.sessions().interrupt(&s).await;
                if let Err(e) = cx.sessions().close(&s).await {
                    tracing::debug!(session, error = %e, "flow: session not closed");
                }
            }
        }
    }
}

/// How often a claim being worked on is renewed: a third of the lease.
fn renew_every(sh: &Shared) -> Duration {
    Duration::from_millis((sh.cfg.lease_ms / 3).max(500))
}

/// Renew a claim (claimed or handed in); `false` when it is no longer ours.
async fn renew(sh: &Arc<Shared>, key: &TaskKey, token: &str) -> bool {
    let Ok(engine) = sh.engine() else {
        return false;
    };
    let r = engine
        .heartbeat(
            &key.run,
            &key.station,
            &key.task,
            token,
            sh.cfg.lease_ms,
            dispatcher(),
        )
        .await;
    settle(sh, &key.run, "heartbeat", r).await
}

async fn program_task(
    sh: &Arc<Shared>,
    key: &TaskKey,
    token: &str,
    step: &str,
    item: Option<&Value>,
) {
    let Ok(engine) = sh.engine() else { return };
    let TaskKey {
        run, station, task, ..
    } = key;
    let result = match (&sh.cfg.programs, engine.get(run).await) {
        (Some(p), Ok(view)) => {
            // The claim is kept alive while the program runs; one that is lost abandons it.
            let running = p.run(step, &view.record, item);
            tokio::pin!(running);
            let every = renew_every(sh);
            loop {
                tokio::select! {
                    r = &mut running => break r,
                    _ = tokio::time::sleep(every) => {
                        if !renew(sh, key, token).await {
                            return;
                        }
                    }
                }
            }
        }
        (None, _) => Err(format!("no programs are registered to run {step}")),
        (_, Err(e)) => {
            tracing::warn!(run, error = %e, "flow: program not run");
            return;
        }
    };
    match result {
        Ok(output) => {
            match engine
                .submit(run, station, task, token, output, dispatcher())
                .await
            {
                Ok(o) => apply(sh, Some(run), o).await,
                // Only a contract failure is the program's: a station no longer open, a claim
                // gone, is not a failed attempt.
                Err(FlowError::Contract(m)) => fail(sh, key, token, &m).await,
                Err(e) => {
                    tracing::debug!(run, station, task, error = %e, "flow: program output not taken")
                }
            }
        }
        Err(e) => fail(sh, key, token, &e).await,
    }
}

async fn fail(sh: &Arc<Shared>, key: &TaskKey, token: &str, error: &str) {
    let Ok(engine) = sh.engine() else { return };
    let r = engine
        .fail(
            &key.run,
            &key.station,
            &key.task,
            token,
            error,
            dispatcher(),
        )
        .await;
    settle(sh, &key.run, "fail", r).await;
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

async fn standing(sh: &Shared, key: &TaskKey, token: &str) -> Standing {
    let Ok(engine) = sh.engine() else {
        return Standing::Gone;
    };
    match engine.record(&key.run).await {
        Ok(rec) => standing_of(&rec, key, token),
        Err(_) => Standing::Gone,
    }
}

struct SessionJob {
    key: TaskKey,
    token: String,
    scene: String,
    skill: Option<String>,
    skill_digest: Option<String>,
    protocol: String,
    session_key: Option<String>,
    recording: Option<String>,
    resume: bool,
}

/// How the wait for a turn ended.
enum Waited {
    Settled,
    /// The claim is no longer ours.
    Superseded,
    /// The session is gone from the hub.
    Gone,
}

/// The session lock's key: tasks sharing a session share it.
fn lock_key(job: &SessionJob) -> String {
    match &job.session_key {
        Some(k) => format!("{}#{k}", job.key.run),
        None => format!("{}/{}/{}", job.key.run, job.key.station, job.key.task),
    }
}

async fn session_task(sh: &Arc<Shared>, job: SessionJob) {
    let name = lock_key(&job);
    let lock = sh.session_lock(&name);
    let acquired = wait_turn_on_session(sh, &lock, &job).await;
    if let Some(guard) = acquired {
        with_session(sh, &job).await;
        drop(guard);
    }
    sh.release_lock(&name, lock);
}

/// The session's lock, renewing the claim while other tasks have it.
async fn wait_turn_on_session(
    sh: &Arc<Shared>,
    lock: &Arc<tokio::sync::Mutex<()>>,
    job: &SessionJob,
) -> Option<tokio::sync::OwnedMutexGuard<()>> {
    let acquire = lock.clone().lock_owned();
    tokio::pin!(acquire);
    let every = renew_every(sh);
    loop {
        tokio::select! {
            g = &mut acquire => return Some(g),
            _ = tokio::time::sleep(every) => {
                if !renew(sh, &job.key, &job.token).await {
                    return None;
                }
            }
        }
    }
}

/// With the session's lock held: find or make the session, bind it, carry the task.
async fn with_session(sh: &Arc<Shared>, job: &SessionJob) {
    let (Ok(engine), Some(cx)) = (sh.engine(), sh.cx.get()) else {
        return;
    };
    let key = &job.key;
    let Ok(rec) = engine.record(&key.run).await else {
        return;
    };
    if standing_of(&rec, key, &job.token) != Standing::Claimed {
        return;
    }
    let sessions = cx.sessions();

    // The task's own session (it reported one before a restart), else its key's (bound by a
    // task that went before it), else a new one. Only its own is resumed: a task that never
    // told its session anything is told everything.
    let own = rec
        .stations
        .get(&key.station)
        .and_then(|s| s.task(&key.task))
        .and_then(|t| t.session.clone());
    let keyed = job
        .session_key
        .as_ref()
        .and_then(|k| rec.session_keys.get(k).cloned());
    let mut opened: Option<(SessionRef, bool)> = None;
    for (name, is_own) in [(own.clone(), true), (keyed, false)] {
        let Some(name) = name else { continue };
        let s = session_ref(&name);
        match sessions.open(&s.instance, &s.session_id).await {
            Ok(s) => {
                opened = Some((s, is_own));
                break;
            }
            Err(e) => tracing::info!(session = name, error = %e, "flow: session gone"),
        }
    }
    let resume = job.resume && opened.as_ref().is_some_and(|(_, own)| *own);
    let session = match opened {
        Some((s, _)) => s,
        None => {
            let spec = SessionSpec {
                space: Some(sh.cfg.space.clone()),
                instance: Some(sh.cfg.instance.clone()),
                scene: Some(job.scene.clone()),
                recording: job.recording.clone(),
                ..SessionSpec::default()
            };
            match sessions.create(spec).await {
                Ok(s) => s,
                Err(e) => {
                    fail(sh, key, &job.token, &format!("no session: {}", e.message)).await;
                    return;
                }
            }
        }
    };
    let mut tap = match sessions.tap(&session).await {
        Ok(t) => t,
        Err(e) => {
            fail(
                sh,
                key,
                &job.token,
                &format!("session not watched: {}", e.message),
            )
            .await;
            return;
        }
    };
    let running = turn_running(sh, &session).await;
    if running && !resume {
        // Someone else's turn — the task before, a person: it ends before this task binds,
        // and its settlement is read here, not taken for this task's.
        match wait_turn(sh, &mut tap, key, &job.token).await {
            Waited::Settled => {}
            Waited::Superseded | Waited::Gone => return,
        }
    }
    let attached = engine
        .attach_session(
            &key.run,
            &key.station,
            &key.task,
            &job.token,
            &session_name(&session),
            job.session_key.as_deref(),
            dispatcher(),
        )
        .await;
    if !settle(sh, &key.run, "attach_session", attached).await {
        return;
    }
    let writes = rec
        .def
        .station(&key.station)
        .map(|s| s.writes.clone())
        .unwrap_or_default();
    sh.bind(
        &session.session_id,
        Binding {
            instance: session.instance.clone(),
            run: key.run.clone(),
            station: key.station.clone(),
            round: key.round,
            task: key.task.clone(),
            token: job.token.clone(),
            writes,
        },
    );
    let still_running = carry(sh, job, &session, &mut tap, resume, resume && running).await;
    if still_running && !sh.stopping() {
        // The task is over while its turn runs on: stop that turn before the session is
        // anyone else's, so nothing it does lands under the next task.
        let _ = sessions.interrupt(&session).await;
        let _ = tokio::time::timeout(SETTLE_AFTER_INTERRUPT, tap.next_settled()).await;
    }
    // Handed in: the binding stays, so the session can hand in again until the station
    // passes (publishing drops it then). Otherwise it goes now.
    if standing(sh, key, &job.token).await != Standing::Submitted {
        sh.unbind(&session.session_id, &job.token);
    }
}

async fn turn_running(sh: &Shared, session: &SessionRef) -> bool {
    let Some(cx) = sh.cx.get() else { return false };
    cx.call(
        "session.get",
        json!({"instance": session.instance, "session_id": session.session_id}),
    )
    .await
    .ok()
    .and_then(|v| {
        v.get("turn_state")
            .and_then(Value::as_str)
            .map(String::from)
    })
    .is_some_and(|s| s != "idle")
}

/// Send the task, then judge after every turn until it is in, over, or failed. Returns
/// whether a turn may still be running.
async fn carry(
    sh: &Arc<Shared>,
    job: &SessionJob,
    session: &SessionRef,
    tap: &mut Tap,
    resume: bool,
    resumed_running: bool,
) -> bool {
    let (Ok(engine), Some(cx)) = (sh.engine(), sh.cx.get()) else {
        return false;
    };
    let key = &job.key;
    let sessions = cx.sessions();
    // A resumed task whose session is idle had its turn cut off: judge it as a turn that ended.
    let mut running = resumed_running;
    if !resume {
        let message = task_message(sh, job);
        if let Err(e) = sessions.send(session, &message, OnBusy::Queue).await {
            fail(
                sh,
                key,
                &job.token,
                &format!("the task was not sent: {}", e.message),
            )
            .await;
            return false;
        }
        running = true;
    }
    let mut nudged = false;
    loop {
        if running {
            match wait_turn(sh, tap, key, &job.token).await {
                Waited::Settled => {}
                Waited::Superseded => return true,
                Waited::Gone => {
                    if !sh.stopping() && standing(sh, key, &job.token).await == Standing::Claimed {
                        fail(sh, key, &job.token, "the session went away").await;
                    }
                    return false;
                }
            }
        }
        if sh.stopping() {
            return false;
        }
        settle(sh, &key.run, "evaluate", engine.evaluate(&key.run).await).await;
        match standing(sh, key, &job.token).await {
            Standing::Submitted | Standing::Gone => return false,
            Standing::Claimed => {}
        }
        if !renew(sh, key, &job.token).await {
            return false;
        }
        if nudged {
            fail(
                sh,
                key,
                &job.token,
                "the session ended a turn twice without handing the task in",
            )
            .await;
            return false;
        }
        nudged = true;
        if let Err(e) = sessions.send(session, NUDGE, OnBusy::Queue).await {
            fail(
                sh,
                key,
                &job.token,
                &format!("the reminder was not sent: {}", e.message),
            )
            .await;
            return false;
        }
        running = true;
    }
}

/// The next settled turn, renewing the claim while it runs.
async fn wait_turn(sh: &Arc<Shared>, tap: &mut Tap, key: &TaskKey, token: &str) -> Waited {
    let every = renew_every(sh);
    loop {
        match tokio::time::timeout(every, tap.next_settled()).await {
            Ok(Some(_)) => return Waited::Settled,
            Ok(None) => return Waited::Gone,
            Err(_) => {
                if sh.stopping() || !renew(sh, key, token).await {
                    return Waited::Superseded;
                }
            }
        }
    }
}

/// The protocol layer, then the skill.
fn task_message(sh: &Shared, job: &SessionJob) -> String {
    let mut text = job.protocol.clone();
    if let (Some(name), Ok(engine)) = (&job.skill, sh.engine()) {
        match engine.packs().skill(name) {
            Some((body, digest)) => {
                if job.skill_digest.as_ref().is_some_and(|d| *d != digest) {
                    tracing::warn!(
                        skill = name,
                        "flow: the skill changed since the station was entered"
                    );
                }
                text.push_str(&format!("\n\n## Skill `{name}`\n\n{}\n", skill_body(&body)));
            }
            None => tracing::warn!(skill = name, "flow: skill not found"),
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::def::SessionBinding;
    use crate::{CreateIn, Engine, ManualClock, MemPacks, MemStore};

    #[test]
    fn work_names_its_run() {
        let w = WorkItem::Task {
            run: "r-1".into(),
            station: "a".into(),
            round: 1,
            task: "a".into(),
            token: "t".into(),
            attempt: 1,
            executor: Executor::Session {
                scene: "flow-chat".into(),
                session: SessionBinding::New,
            },
            skill: None,
            skill_digest: None,
            item: None,
            output_schema: None,
            protocol: None,
            session_key: None,
            session: None,
        };
        assert_eq!(run_of(&w), Some("r-1"));
        assert_eq!(
            run_of(&WorkItem::CloseSession {
                session: "s".into()
            }),
            None
        );
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
        let eng = Engine::new(Arc::new(MemStore::new()), Arc::new(packs))
            .with_clock(clock.clone())
            .with_lease_ms(60_000);
        (eng, clock)
    }

    fn tasks(o: &Outcome) -> Vec<(String, u32, String)> {
        o.work
            .iter()
            .filter_map(|w| match w {
                WorkItem::Task {
                    task, round, token, ..
                } => Some((task.clone(), *round, token.clone())),
                _ => None,
            })
            .collect()
    }

    /// A partial rework starts a new round of the station but keeps the claims it did not
    /// name: the dispatcher must go on working them (the claim is the token, not the round).
    #[tokio::test]
    async fn a_partial_rework_leaves_the_other_claims_standing() {
        let (eng, _) = engine();
        let o = eng
            .create(
                CreateIn {
                    flow: "multi".into(),
                    subject: "s".into(),
                    ..Default::default()
                },
                Actor::new("device", "d"),
            )
            .await
            .unwrap();
        let run = o.value["run"].as_str().unwrap().to_string();
        let (_, _, collect) = tasks(&o).remove(0);
        let o = eng
            .submit(
                &run,
                "collect",
                "collect",
                &collect,
                json!({"modules": ["a", "b"]}),
                dispatcher(),
            )
            .await
            .unwrap();
        let fan = tasks(&o);
        let (a, b) = (fan[0].clone(), fan[1].clone());
        eng.rework(
            &run,
            "analyze",
            Some(vec!["a".into()]),
            "again",
            Actor::new("device", "d"),
        )
        .await
        .unwrap();
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

    #[test]
    fn a_binding_counts_until_its_station_passes_or_its_claim_is_superseded() {
        let mut rec: RunRecord = serde_json::from_value(json!({
            "id": "r", "flow": "f", "def_digest": "d", "subject": "s", "status": "active",
            "rev": 1, "created_at": 0, "created_by": {"kind": "x", "id": "y"},
            "def": {"name": "f", "stations": [{"id": "a", "exit": [{"kind": "confirm"}]}]},
            "stations": {"a": {"status": "working", "round": 1, "tasks": [
                {"id": "t", "status": "submitted", "attempt": 1,
                 "lease": {"by": {"kind": "flow", "id": "dispatch"}, "token": "tok", "until": 9}}
            ]}}
        }))
        .unwrap();
        let b = Binding {
            instance: "system".into(),
            run: "r".into(),
            station: "a".into(),
            round: 1,
            task: "t".into(),
            token: "tok".into(),
            writes: vec![],
        };
        assert!(
            binding_current(&rec, &b),
            "handed in, station open: it may hand in again"
        );
        let other = Binding {
            token: "old".into(),
            ..b.clone()
        };
        assert!(!binding_current(&rec, &other));
        rec.stations.get_mut("a").unwrap().status = StationStatus::Passed;
        assert!(!binding_current(&rec, &b));
    }
}
