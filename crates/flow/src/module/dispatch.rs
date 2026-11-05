//! Carrying out what the kernel hands back.
//!
//! Every engine result comes through [`apply`]: its events are broadcast, the runs it touched
//! are published on `run {run}` and the list on `runs`, and each work item is started on its
//! own task — a method answers without waiting for a session. A work item's own engine calls
//! come back through [`apply`] again.
//!
//! A session task: a headless session in the flow's space (or the one its session key is
//! bound to), told the protocol layer and the skill; after every turn the claim is renewed and
//! the run evaluated; a turn that ends without a submission gets one nudge, and a second one
//! fails the attempt. Tasks sharing a session take turns.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use harnix_module::{OnBusy, SessionRef, SessionSpec, Tap};
use serde_json::{json, Value};

use super::shared::{session_name, session_ref, Binding, Shared, TaskKey};
use crate::kernel::def::Executor;
use crate::kernel::host::{GateResult, Outcome, WorkItem};
use crate::kernel::protocol::skill_body;
use crate::kernel::run::{Actor, RunQuery, RunStatus, TaskStatus};
use crate::kernel::{dispatched_work, FlowError};

/// How many runs the `runs` topic carries, newest first.
pub const RUNS_LISTED: usize = 200;

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

/// The newest snapshot of each run, and the list.
pub(crate) async fn publish(sh: &Arc<Shared>, runs: &BTreeSet<String>) {
    let (Some(engine), Some(cx)) = (sh.engine.get(), sh.cx.get()) else {
        return;
    };
    let _order = sh.publish.lock().await;
    let events = cx.events();
    for run in runs {
        if let Ok(view) = engine.get(run).await {
            if let Err(e) = events.publish("run", json!({"run": run}), &view).await {
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

/// The list, and every active run: what a subscriber arriving after start sees.
pub(crate) async fn publish_all(sh: &Arc<Shared>) {
    let Some(engine) = sh.engine.get() else {
        return;
    };
    let active: BTreeSet<String> = engine
        .list(RunQuery {
            status: Some(RunStatus::Active),
            ..Default::default()
        })
        .await
        .map(|l| l.into_iter().map(|s| s.id).collect())
        .unwrap_or_default();
    publish(sh, &active).await;
}

/// After a restart: every task the dispatcher still holds a live claim on is carried on with
/// (a session task waits on the session it had, a program runs again).
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
        let Ok(view) = engine.get(&s.id).await else {
            continue;
        };
        for w in dispatched_work(&view.record, now) {
            spawn_work(sh, w, true);
        }
        // Whatever a stop interrupted (a gate result that never came) is judged again.
        settle(sh, &s.id, "evaluate", engine.evaluate(&s.id).await).await;
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
                session,
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
                        session,
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
        (Some(p), Ok(view)) => p.run(step, &view.record, item).await,
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
                Err(FlowError::Invalid(m)) => fail(sh, key, token, &m).await,
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
enum Standing {
    /// Still claimed under this token: work on.
    Claimed,
    /// Handed in under this token.
    Submitted,
    /// Someone else's now, or over: stop.
    Gone,
}

async fn standing(sh: &Shared, key: &TaskKey, token: &str) -> Standing {
    let Ok(engine) = sh.engine() else {
        return Standing::Gone;
    };
    let Ok(view) = engine.get(&key.run).await else {
        return Standing::Gone;
    };
    let rec = view.record;
    if !rec.is_active() {
        return Standing::Gone;
    }
    let Some(st) = rec
        .stations
        .get(&key.station)
        .filter(|s| s.round == key.round)
    else {
        return Standing::Gone;
    };
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

struct SessionJob {
    key: TaskKey,
    token: String,
    scene: String,
    skill: Option<String>,
    skill_digest: Option<String>,
    protocol: String,
    session_key: Option<String>,
    session: Option<String>,
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

async fn session_task(sh: &Arc<Shared>, job: SessionJob) {
    let (Ok(engine), Some(cx)) = (sh.engine(), sh.cx.get()) else {
        return;
    };
    let key = &job.key;
    let lock_key = match &job.session_key {
        Some(k) => format!("{}#{k}", key.run),
        None => format!("{}/{}/{}", key.run, key.station, key.task),
    };
    let lock = sh.session_lock(&lock_key);
    let _turn = lock.lock().await;
    if standing(sh, key, &job.token).await != Standing::Claimed {
        return;
    }
    let sessions = cx.sessions();

    // The session: the one this task reported, else its key's (as bound now, by a task that
    // went before it in the queue), else a new one.
    let mut known = job.session.clone();
    if known.is_none() {
        if let (Some(k), Ok(view)) = (&job.session_key, engine.get(&key.run).await) {
            known = view.record.session_keys.get(k).cloned();
        }
    }
    let mut opened: Option<SessionRef> = None;
    if let Some(name) = &known {
        let s = session_ref(name);
        match sessions.open(&s.instance, &s.session_id).await {
            Ok(s) => opened = Some(s),
            Err(e) => tracing::info!(session = name, error = %e, "flow: session gone; a new one"),
        }
    }
    let resume = job.resume && opened.is_some() && job.session.is_some();
    let session = match opened {
        Some(s) => s,
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
    let name = session_name(&session);
    let attached = engine
        .attach_session(
            &key.run,
            &key.station,
            &key.task,
            &job.token,
            &name,
            job.session_key.as_deref(),
            dispatcher(),
        )
        .await;
    if !settle(sh, &key.run, "attach_session", attached).await {
        return;
    }
    let writes = engine
        .get(&key.run)
        .await
        .ok()
        .and_then(|v| v.record.def.station(&key.station).map(|s| s.writes.clone()))
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
    carry(sh, &job, &session, resume).await;
    sh.unbind(&session.session_id, &job.token);
}

/// Send the task, then judge after every turn until it is in, over, or failed.
async fn carry(sh: &Arc<Shared>, job: &SessionJob, session: &SessionRef, resume: bool) {
    let (Ok(engine), Some(cx)) = (sh.engine(), sh.cx.get()) else {
        return;
    };
    let key = &job.key;
    let sessions = cx.sessions();
    let mut tap = match sessions.tap(session).await {
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
    // A resumed task whose session is idle had its turn cut off: judge it as a turn that ended.
    let mut judge_now = false;
    if resume {
        let state = cx
            .call(
                "session.get",
                json!({"instance": session.instance, "session_id": session.session_id}),
            )
            .await
            .ok()
            .and_then(|v| {
                v.get("turn_state")
                    .and_then(Value::as_str)
                    .map(String::from)
            });
        judge_now = state.as_deref() != Some("running");
    } else {
        let message = task_message(sh, job);
        if let Err(e) = sessions.send(session, &message, OnBusy::Queue).await {
            fail(
                sh,
                key,
                &job.token,
                &format!("the task was not sent: {}", e.message),
            )
            .await;
            return;
        }
    }
    let mut nudged = false;
    loop {
        if !judge_now {
            match wait_turn(sh, &mut tap, key, &job.token).await {
                Waited::Settled => {}
                Waited::Superseded => return,
                Waited::Gone => {
                    if !sh.stopping() && standing(sh, key, &job.token).await == Standing::Claimed {
                        fail(sh, key, &job.token, "the session went away").await;
                    }
                    return;
                }
            }
        }
        judge_now = false;
        if sh.stopping() {
            return;
        }
        settle(sh, &key.run, "evaluate", engine.evaluate(&key.run).await).await;
        match standing(sh, key, &job.token).await {
            Standing::Submitted | Standing::Gone => return,
            Standing::Claimed => {}
        }
        let renewed = engine
            .heartbeat(
                &key.run,
                &key.station,
                &key.task,
                &job.token,
                sh.cfg.lease_ms,
            )
            .await;
        if !settle(sh, &key.run, "heartbeat", renewed).await {
            return;
        }
        if nudged {
            fail(
                sh,
                key,
                &job.token,
                "the session ended a turn twice without handing the task in",
            )
            .await;
            return;
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
            return;
        }
    }
}

/// The next settled turn, renewing the claim while it runs.
async fn wait_turn(sh: &Arc<Shared>, tap: &mut Tap, key: &TaskKey, token: &str) -> Waited {
    let Ok(engine) = sh.engine() else {
        return Waited::Gone;
    };
    let every = Duration::from_millis((sh.cfg.lease_ms / 3).max(500));
    loop {
        match tokio::time::timeout(every, tap.next_settled()).await {
            Ok(Some(_)) => return Waited::Settled,
            Ok(None) => return Waited::Gone,
            Err(_) => {
                if sh.stopping() {
                    return Waited::Superseded;
                }
                match standing(sh, key, token).await {
                    Standing::Claimed => {
                        let r = engine
                            .heartbeat(&key.run, &key.station, &key.task, token, sh.cfg.lease_ms)
                            .await;
                        if !settle(sh, &key.run, "heartbeat", r).await {
                            return Waited::Superseded;
                        }
                    }
                    // Handed in mid-turn: the turn's end is still worth waiting for.
                    Standing::Submitted => {}
                    Standing::Gone => return Waited::Superseded,
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
}
