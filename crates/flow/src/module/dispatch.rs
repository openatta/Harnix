//! Carrying out what the kernel hands back.
//!
//! Every engine result comes through [`apply`]: its events are broadcast, the runs it touched
//! are published on `run {run}` (lease tokens removed) and the list on `runs`, bindings of
//! claims that are over are dropped, and each work item is started on its own task — a method
//! answers without waiting for a session. A work item's own engine calls come back through
//! [`apply`] again.
//!
//! A session task: a headless session where the station places it — its space (the flow's by
//! default), its instance or an instance of its template made ready first, its model — or the
//! one its session key is bound to, told the protocol layer, the rules and the skill. The
//! station's contract is the session's `output_schema` when it hands in by structured output; the
//! session `ask`s about every guarded tool and every tool a permission names, and the module
//! answers each question from the permissions of the task the turn works on, whatever carries
//! the session (xops-04 §3). After every
//! turn, the turn's structured output (if any) is submitted, the run is evaluated and the claim
//! renewed; an output the station still judges unmet is answered with what is missing; a turn
//! that ends with nothing handed in gets one nudge, and a second one fails the attempt. Tasks sharing a session take turns, and a task waiting for its turn keeps
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
use crate::kernel::def::{Executor, MAIN_SESSION_KEY};
use crate::kernel::engine::view;
use crate::kernel::host::{Delivery, GateResult, Outcome, Placement, WorkItem};
use crate::kernel::parts::{asked_tools, GateKind};
use crate::kernel::protocol::{ask_rules, task_message, STRUCTURED_OUTPUT_TOOL};
use crate::kernel::run::{
    Actor, CheckStatus, RunQuery, RunRecord, RunStatus, StationStatus, TaskStatus,
};
use crate::kernel::FlowError;

/// How many runs the `runs` topic carries, newest first.
pub const RUNS_LISTED: usize = 200;

/// How long a task that is over waits for the turn it interrupted to settle.
const SETTLE_AFTER_INTERRUPT: Duration = Duration::from_secs(30);

/// What a session is told when a turn ends and its task is still not handed in (`flow_submit`).
pub const NUDGE: &str = "Your flow task is not handed in yet. Call `flow_submit` with an output \
     that satisfies the contract in the flow protocol above (`flow_state` shows what is still \
     missing). If you cannot do the task, say why and use `flow_propose`.";

/// The same, for a session that hands in by structured output.
pub const NUDGE_STRUCTURED: &str = "Your flow task is not handed in yet. Call the \
     `StructuredOutput` tool with an output that satisfies the contract in the flow protocol \
     above. If you cannot do the task, say why.";

/// How many times an output the station still judges unmet is sent back to be fixed.
const CORRECTIONS: u32 = 2;

/// The permission mode a task's session asks for: the most permissive one that still goes
/// through the session's permission rules without a classifier. The engine only ever tightens
/// what a client asks for, so an instance set stricter keeps its own mode; one set to
/// `bypassPermissions` — which ignores a session's rules — is held to this one instead, so the
/// flow's answers to its questions still hold (xops-02 review #7).
pub const TASK_PERMISSION_MODE: &str = "acceptEdits";

/// The entry a task's session is marked with when it is opened: what it was opened with
/// (`settings_of`). A task that finds its session key bound to a session opened otherwise fails
/// instead of working under another station's contract.
pub const SESSION_SETTINGS: &str = "session_settings";

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
        "model": p.model, "provider": p.provider,
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
    keys.into_iter()
        .filter(|k| a.get(*k) != b.get(*k))
        .map(|k| k.to_string())
        .collect()
}

/// A flow session's permission rules: `ask` for every guarded tool and every tool a permission
/// of the run's flow names, so each call of them comes to the module to be judged (xops-04 §3).
/// The same rules whatever carries the session.
pub(crate) fn flow_rules(sh: &Shared, rec: &RunRecord) -> Value {
    ask_rules(&asked_tools(&rec.def, &rec.parts.policies, &sh.cfg.guarded))
}

fn dispatcher() -> Actor {
    Actor::flow("dispatch")
}

fn run_of(w: &WorkItem) -> Option<&str> {
    match w {
        WorkItem::Task { run, .. } | WorkItem::Gate { run, .. } => Some(run),
        WorkItem::CloseSession { .. } => None,
    }
}

/// Carry out an engine result.
pub(crate) async fn apply(sh: &Arc<Shared>, hint: Option<&str>, o: Outcome) {
    for e in &o.events {
        let _ = sh.events.send(e.clone());
        if matches!(e.kind.as_str(), "run.finished" | "run.cancelled") {
            let (sh, run) = (sh.clone(), e.run.clone());
            tokio::spawn(async move { stop_run_instances(&sh, &run).await });
        }
        // Every run has its main session from the start (xops-03 §1), whatever its first
        // station is: the conversation a person opens to watch and talk to it.
        if e.kind == "run.created" {
            let (sh, run) = (sh.clone(), e.run.clone());
            tokio::spawn(async move {
                if let Err(why) = super::main::ensure_main(&sh, &run).await {
                    tracing::warn!(run, why, "flow: no main session");
                }
            });
        }
    }
    let mut runs: BTreeSet<String> = o.events.iter().map(|e| e.run.clone()).collect();
    runs.extend(o.work.iter().filter_map(run_of).map(String::from));
    if let Some(h) = hint {
        runs.insert(h.to_string());
    }
    publish(sh, &runs).await;
    super::hooks::notify(sh, &o.events).await;
    for w in o.work {
        spawn_work(sh, w, false);
    }
}

/// [`settle`] for bookkeeping: what it changed is published, a refusal only logged. Boxed:
/// it is reached from work [`apply`] starts, and the future must not contain itself.
pub(crate) fn settle_quiet<'a>(
    sh: &'a Arc<Shared>,
    run: &'a str,
    r: crate::Result<Outcome>,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'a>> {
    Box::pin(async move {
        settle(sh, run, "bookkeeping", r).await;
    })
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

/// The flows on offer, as the `defs` topic carries them: `[{name, title, digest, origin, valid}]`.
pub(crate) fn defs_list(packs: &dyn crate::kernel::host::Packs) -> Vec<Value> {
    packs
        .flows()
        .into_iter()
        .filter_map(|n| {
            let (def, digest) = packs.flow(&n)?;
            let valid = crate::kernel::validate::validate(&def, packs).is_empty();
            Some(json!({
                "name": n, "title": def.title, "digest": digest,
                "origin": packs.origin(&n), "valid": valid,
            }))
        })
        .collect()
}

/// The `defs` topic, after the packs changed.
pub(crate) async fn publish_defs(sh: &Arc<Shared>) {
    let (Some(engine), Some(cx)) = (sh.engine.get(), sh.cx.get()) else {
        return;
    };
    let list = defs_list(engine.packs().as_ref());
    if let Err(e) = cx.events().publish("defs", json!({}), &list).await {
        tracing::warn!(error = %e, "flow: defs not published");
    }
}

/// The `focus {instance, session_id}` topic: `{flow}` (`null` when nothing is selected).
pub(crate) async fn publish_focus(sh: &Arc<Shared>, instance: &str, session_id: &str) {
    let Some(cx) = sh.cx.get() else { return };
    let flow = sh.focus(instance, session_id);
    let params = json!({"instance": instance, "session_id": session_id});
    if let Err(e) = cx
        .events()
        .publish("focus", params, json!({"flow": flow}))
        .await
    {
        tracing::warn!(error = %e, "flow: focus not published");
    }
}

/// A dry run's runs on `run {run}` (with their previews) and the `dry_runs` list.
pub(crate) async fn publish_dry(sh: &Arc<Shared>, run: Option<&str>) {
    let (Some(sandbox), Some(cx)) = (sh.sandbox.get(), sh.cx.get()) else {
        return;
    };
    let _order = sh.publish.lock().await;
    let events = cx.events();
    if let Some(root) = match run {
        Some(r) => sandbox.root_of(r).await,
        None => None,
    } {
        for id in sandbox.runs_of(&root).await {
            if let Ok(v) = sandbox.get(&id).await {
                if let Err(e) = events.publish("run", json!({"run": id}), v).await {
                    tracing::warn!(run = id, error = %e, "flow: dry run not published");
                }
            }
        }
    }
    let list = sandbox.list().await;
    if let Err(e) = events.publish("dry_runs", json!({}), &list).await {
        tracing::warn!(error = %e, "flow: dry runs not listed");
    }
}

/// Stop the instances the module made for `run` under its own names (xops-02 review #6): a
/// spawned daemon per run would otherwise outlive it, and a template's `max` run out.
pub(crate) async fn stop_run_instances(sh: &Arc<Shared>, run: &str) {
    let Some(cx) = sh.cx.get() else { return };
    for name in sh.take_instances(run) {
        match cx.instances().stop(&name).await {
            Ok(_) => tracing::debug!(run, instance = name, "flow: the run's instance stopped"),
            Err(e) => {
                tracing::info!(run, instance = name, error = %e, "flow: the run's instance not stopped")
            }
        }
    }
}

/// After a restart: the instances of runs that ended while the module was down.
pub(crate) async fn stop_stale_instances(sh: &Arc<Shared>) {
    let Some(engine) = sh.engine.get().cloned() else {
        return;
    };
    for run in sh.instance_runs() {
        let over = match engine.record(&run).await {
            Ok(rec) => !rec.is_active(),
            Err(FlowError::NotFound(_)) => true,
            Err(_) => false,
        };
        if over {
            stop_run_instances(sh, &run).await;
        }
    }
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
        for w in engine.dispatched_work(&rec, now) {
            spawn_work(sh, w, true);
        }
        // Its main session watched again (opened, if it never was).
        let (sh, run) = (sh.clone(), s.id.clone());
        tokio::spawn(async move {
            if let Err(why) = super::main::ensure_main(&sh, &run).await {
                tracing::warn!(run, why, "flow: no main session after a restart");
            }
        });
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
                rules,
                permissions,
                item,
                protocol,
                session_key,
                output_schema,
                placement,
                delivery,
                scene,
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
                Executor::Session { .. } | Executor::Agent { .. } => {
                    let main = session_key.as_deref() == Some(MAIN_SESSION_KEY);
                    let scene = match (scene, executor) {
                        (Some(s), _) => s,
                        (None, Executor::Session { scene, .. }) if !scene.is_empty() => scene,
                        _ => {
                            fail(
                                &sh,
                                &key,
                                &token,
                                "the flow names no main scene and the module has none",
                            )
                            .await;
                            sh.end(&key, &token);
                            return;
                        }
                    };
                    let job = SessionJob {
                        main,
                        key: key.clone(),
                        token: token.clone(),
                        scene,
                        skill,
                        skill_digest,
                        rules,
                        permissions,
                        protocol: protocol.unwrap_or_default(),
                        session_key,
                        recording,
                        resume,
                        output_schema,
                        placement: placement.unwrap_or_default(),
                        delivery: delivery.unwrap_or(Delivery::StructuredOutput),
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
            let rec = match engine.record(&run).await {
                Ok(r) => r,
                Err(e) => {
                    tracing::warn!(run, error = %e, "flow: gate not run");
                    return;
                }
            };
            let result = match rec.parts.gates.get(&gate) {
                Some(g) if g.kind == GateKind::Command => command_gate(&sh, &engine, &rec, &station, g).await,
                Some(_) => match &sh.cfg.gates {
                    Some(gs) => gs.run(&rec, &station, &gate).await,
                    None => GateResult {
                        pass: false,
                        detail: format!("the application runs no checks: {gate} cannot run"),
                    },
                },
                None => GateResult {
                    pass: false,
                    detail: format!("gate {gate} was not in the packs when the run started"),
                },
            };
            let r = engine
                .gate_result(&run, &station, round, &gate, result)
                .await;
            settle(&sh, &run, "gate_result", r).await;
        }
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
    /// The rules it works under, with the digests pinned on station entry.
    rules: Vec<(String, Option<String>)>,
    /// What it works under (xops-04 §3).
    permissions: Vec<crate::kernel::parts::Granted>,
    protocol: String,
    session_key: Option<String>,
    recording: Option<String>,
    resume: bool,
    output_schema: Option<Value>,
    placement: Placement,
    delivery: Delivery,
    /// A turn of the run's main session (xops-03 §1).
    main: bool,
}

/// Where a new session for `job` goes: the station's space (the module's when it names none),
/// its instance, or an instance of its template made ready first; the module's own space
/// without an instance means `system`, another space its own default. `Err`: why there is
/// nowhere to go, as the task's error.
async fn place(sh: &Shared, job: &SessionJob) -> Result<(String, Option<String>), String> {
    place_at(sh, &job.placement, &job.key.run).await
}

/// [`place`] for a placement of `run`'s (a station's, or the main session's).
pub(crate) async fn place_at(
    sh: &Shared,
    p: &Placement,
    run: &str,
) -> Result<(String, Option<String>), String> {
    let cx = sh.cx.get().ok_or("the flow module has not started")?;
    let space = p.space.clone().unwrap_or_else(|| sh.cfg.space.clone());
    if let (Some(template), Some(name)) = (&p.template, p.template_instance(run)) {
        let params = p
            .params
            .iter()
            .filter(|(k, _)| k.as_str() != "name")
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        // Named by the module, the instance is the run's: recorded before it is made, stopped
        // when the run is over (`stop_run_instances`).
        if p.names_its_instance() {
            sh.own_instance(&name, run);
        }
        cx.instances()
            .ensure_ready(
                template,
                &name,
                params,
                Duration::from_millis(sh.cfg.ensure_ms),
            )
            .await
            .map_err(|e| {
                format!(
                    "template {template} gave no instance `{name}`: {}",
                    e.message
                )
            })?;
        return Ok((space, Some(name)));
    }
    let instance = match &p.instance {
        Some(i) => Some(i.clone()),
        None if space == sh.cfg.space => Some(sh.cfg.instance.clone()),
        None => None,
    };
    Ok((space, instance))
}

/// How the wait for a turn ended.
enum Waited {
    /// The turn's settlement (`harnix.turn_settled` params).
    Settled(Value),
    /// The claim is no longer ours.
    Superseded,
    /// The session is gone from the hub.
    Gone,
}

/// The session lock's key: tasks sharing a session share it. A task with a session of its
/// own (`new`) shares it with nobody — not even its own next round or attempt, which gets a
/// new session and must not wait for the old one's turn to end.
fn lock_key(job: &SessionJob) -> String {
    match &job.session_key {
        Some(k) => format!("{}#{k}", job.key.run),
        None => format!(
            "{}/{}/{}@{}",
            job.key.run, job.key.station, job.key.task, job.token
        ),
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
    // Paused while it waited for its session: nothing was sent, so it goes back to ready and is
    // dispatched again on resume (xops-03 §3: what is not begun does not begin).
    if rec.is_paused() && !job.resume {
        let r = engine
            .requeue(&key.run, &key.station, &key.task, &job.token)
            .await;
        settle(sh, &key.run, "requeue", r).await;
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
    let instance_of = |job: &SessionJob| job.placement.template_instance(&job.key.run);
    let wanted = settings_of(
        &job.scene,
        job.delivery,
        job.output_schema.as_ref(),
        &job.placement,
        instance_of(job).as_deref(),
    );
    let session = match opened {
        Some((s, own)) => {
            // Validation keeps tasks that share a session alike; this is the net under it — a
            // definition from before that rule, or keys that resolve alike by chance. The main
            // session keeps no station's settings.
            if !own && !job.main {
                if let Some(opened_with) = recorded_settings(sh, &s).await {
                    let d = settings_differ(&opened_with, &wanted);
                    if !d.is_empty() {
                        let why = format!(
                            "the run's shared session {} was opened with other settings ({}) and \
                             keeps them: this task cannot work in it",
                            session_name(&s),
                            d.join(", ")
                        );
                        fail(sh, key, &job.token, &why).await;
                        return;
                    }
                }
            }
            s
        }
        None if job.main => {
            match super::main::open_or_make(sh, &key.run).await {
                Ok(s) => s,
                Err(why) => {
                    fail(sh, key, &job.token, &why).await;
                    return;
                }
            }
        }
        None => {
            let (space, instance) = match place(sh, job).await {
                Ok(p) => p,
                Err(why) => {
                    fail(sh, key, &job.token, &why).await;
                    return;
                }
            };
            let spec = SessionSpec {
                space: Some(space),
                instance,
                scene: Some(job.scene.clone()),
                model: job.placement.model.clone(),
                provider: job.placement.provider.clone(),
                output_schema: match job.delivery {
                    Delivery::StructuredOutput => job.output_schema.clone(),
                    Delivery::FlowSubmit => None,
                },
                permission_mode: Some(TASK_PERMISSION_MODE.into()),
                permission_rules: Some(flow_rules(sh, &rec)),
                recording: job.recording.clone(),
                ..SessionSpec::default()
            };
            match sessions.create(spec).await {
                Ok(s) => {
                    let entry = harnix_module::Entry::new(SESSION_SETTINGS, wanted.clone());
                    if let Err(e) = sessions.append_entry(&s, entry).await {
                        tracing::warn!(error = %e, "flow: a session's settings not recorded");
                    }
                    s
                }
                Err(e) => {
                    fail(sh, key, &job.token, &format!("no session: {}", e.message)).await;
                    return;
                }
            }
        }
    };
    // Its questions are answered from the task its turn works on (xops-04 §3); a main session's
    // watch also counts its turns.
    super::main::watch(sh, &key.run, &session, job.main).await;
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
            &session_name(&session),
            job.session_key.as_deref(),
            dispatcher(),
        )
        .await;
    if !settle(sh, &key.run, "attach_session", attached).await {
        return;
    }
    sh.bind(
        &session.session_id,
        Binding {
            instance: session.instance.clone(),
            run: key.run.clone(),
            station: key.station.clone(),
            round: key.round,
            task: key.task.clone(),
            token: job.token.clone(),
            permissions: job.permissions.clone(),
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
    // In the main session a person may talk too: only the turns this task's messages start are
    // its own (review #2). A resumed task's turn still running is taken as its own.
    let mut own = OwnTurns {
        main: job.main,
        texts: vec![],
        turns: Default::default(),
        any_next: resumed_running,
    };
    let binding = sh.binding(&session.session_id);
    let send = |text: String| {
        if let Some(b) = binding.clone() {
            sh.note_sent(&session.session_id, &text, b);
        }
        let sessions = cx.sessions();
        async move { sessions.send(session, &text, OnBusy::Queue).await }
    };
    // A resumed task whose session is idle had its turn cut off: judge it as a turn that ended.
    let mut running = resumed_running;
    if !resume {
        let message = task_message_of(sh, job);
        own.texts.push(message.clone());
        if let Err(e) = send(message).await {
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
    let mut corrections = 0;
    loop {
        let mut handed: Option<Value> = None;
        if running {
            match wait_own_turn(sh, tap, key, &job.token, &mut own).await {
                Waited::Settled(params) => {
                    handed = params
                        .pointer("/outcome/structured_output")
                        .filter(|v| !v.is_null())
                        .cloned();
                    // A main session's turns are counted by its watch (xops-03 §4).
                    if let (false, Some(u)) = (
                        job.main,
                        params.pointer("/outcome/usage").filter(|u| u.is_object()),
                    ) {
                        let r = engine
                            .account(
                                &key.run,
                                Some((&key.station, &key.task)),
                                crate::kernel::run::Usage::of_turn(u),
                            )
                            .await;
                        settle_quiet(sh, &key.run, r).await;
                    }
                }
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
        // A structured output is the turn's hand-in, whichever way the session was told to hand
        // in: the dispatcher submits it under the task's claim.
        let mut problem: Option<String> = None;
        if let Some(output) = handed {
            let r = engine
                .submit(
                    &key.run,
                    &key.station,
                    &key.task,
                    &job.token,
                    output,
                    dispatcher(),
                )
                .await;
            match r {
                Ok(o) => apply(sh, Some(&key.run), o).await,
                Err(FlowError::Contract(m)) => problem = Some(m),
                Err(e) => {
                    tracing::debug!(run = key.run, error = %e, "flow: structured output not taken");
                    return false;
                }
            }
        }
        settle(sh, &key.run, "evaluate", engine.evaluate(&key.run).await).await;
        match standing(sh, key, &job.token).await {
            Standing::Gone => return false,
            Standing::Submitted => {
                // Handed in, and judged: a station unmet on a check that failed (a `tool`
                // exit, say) is the output's to fix — the session is told what, and hands in
                // again. One waiting on a gate, a person or other tasks is not.
                if job.delivery == Delivery::StructuredOutput && corrections < CORRECTIONS {
                    if let Some(missing) = failed_checks(sh, key).await {
                        corrections += 1;
                        let text = format!(
                            "The flow took your output, but station `{}` is not passed: {missing}. \
                             Fix the output and hand it in again with `{STRUCTURED_OUTPUT_TOOL}`.",
                            key.station
                        );
                        own.texts.push(text.clone());
                        if renew(sh, key, &job.token).await && send(text).await.is_ok() {
                            running = true;
                            continue;
                        }
                    }
                }
                return false;
            }
            Standing::Claimed => {}
        }
        if let Some(m) = problem {
            // Not taken: what is wrong goes back, as a turn of its own (it does not count as
            // the nudge).
            if corrections < CORRECTIONS && renew(sh, key, &job.token).await {
                corrections += 1;
                let text = format!(
                    "The flow did not take your output: {m}. Fix it and hand it in again with \
                     `{STRUCTURED_OUTPUT_TOOL}`."
                );
                own.texts.push(text.clone());
                if send(text).await.is_ok() {
                    running = true;
                    continue;
                }
            }
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
        let nudge = match job.delivery {
            Delivery::FlowSubmit => NUDGE,
            Delivery::StructuredOutput => NUDGE_STRUCTURED,
        };
        own.texts.push(nudge.to_string());
        if let Err(e) = send(nudge.to_string()).await {
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

/// Which turns of a session are a task's: those its messages started (in the main session), or
/// every one (a session of its own).
struct OwnTurns {
    main: bool,
    /// The texts sent and not seen start yet.
    texts: Vec<String>,
    turns: std::collections::HashSet<String>,
    /// The next settlement is the task's whatever started it (a resumed task's running turn).
    any_next: bool,
}

/// [`wait_turn`] for a task: in the main session, the next settlement of a turn one of its own
/// messages started — a person's turns in between are not the task's.
async fn wait_own_turn(
    sh: &Arc<Shared>,
    tap: &mut Tap,
    key: &TaskKey,
    token: &str,
    own: &mut OwnTurns,
) -> Waited {
    if !own.main {
        return wait_turn(sh, tap, key, token).await;
    }
    let every = renew_every(sh);
    loop {
        let frame = match tokio::time::timeout(every, tap.next()).await {
            Ok(Some(harnix_module::TapEvent::Frame(f))) => f,
            Ok(Some(harnix_module::TapEvent::Lagged { .. })) => {
                let _ = tap.catch_up().await;
                continue;
            }
            Ok(None) => return Waited::Gone,
            Err(_) => {
                if sh.stopping() || !renew(sh, key, token).await {
                    return Waited::Superseded;
                }
                continue;
            }
        };
        let turn = frame
            .pointer("/params/turn_id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        match frame.get("method").and_then(Value::as_str) {
            Some("harnix.event") => {
                let text = frame
                    .pointer("/params/event")
                    .filter(|e| e.get("kind").and_then(Value::as_str) == Some("user_message"))
                    .and_then(|e| e.get("text"))
                    .and_then(Value::as_str);
                if let Some(i) = text.and_then(|t| own.texts.iter().position(|x| x == t)) {
                    own.texts.remove(i);
                    own.turns.insert(turn);
                }
            }
            Some("harnix.turn_settled")
                if own.turns.remove(&turn) || std::mem::take(&mut own.any_next) =>
            {
                return Waited::Settled(frame.get("params").cloned().unwrap_or(Value::Null));
            }
            _ => {}
        }
    }
}

/// The next settled turn, renewing the claim while it runs.
async fn wait_turn(sh: &Arc<Shared>, tap: &mut Tap, key: &TaskKey, token: &str) -> Waited {
    let every = renew_every(sh);
    loop {
        match tokio::time::timeout(every, tap.next_settled()).await {
            Ok(Some(p)) => return Waited::Settled(p),
            Ok(None) => return Waited::Gone,
            Err(_) => {
                if sh.stopping() || !renew(sh, key, token).await {
                    return Waited::Superseded;
                }
            }
        }
    }
}

/// What a session task last opened its session with, from the session's own entry.
async fn recorded_settings(sh: &Shared, s: &SessionRef) -> Option<Value> {
    let cx = sh.cx.get()?;
    let entries = cx.sessions().entries(s).await.ok()?;
    entries
        .into_iter()
        .rev()
        .find(|e| e.get("event").and_then(Value::as_str) == Some(SESSION_SETTINGS))
        .and_then(|e| e.get("payload").cloned())
}

/// The station's failed checks that are this task's output to fix: `None` when nothing failed
/// that the session can fix (it waits for a gate, a person or other tasks, or it passed).
async fn failed_checks(sh: &Shared, key: &TaskKey) -> Option<String> {
    let engine = sh.engine().ok()?;
    let v = engine.get(&key.run).await.ok()?;
    corrections_for(&v, &key.station, &key.task)
}

/// What a task's session is told to fix, from its station's view: the parts of a failed `output`
/// check about this task, and a failed `tool` check when the task is its station's only one
/// (the tool judges the station, so with several tasks it is no one task's to fix). A gate, a
/// confirmation, another task's failure is never the session's.
pub(crate) fn corrections_for(v: &crate::RunView, station: &str, task: &str) -> Option<String> {
    let st = v.record.stations.get(station)?;
    if st.status != StationStatus::Unmet {
        return None;
    }
    let only = v
        .record
        .def
        .station(station)
        .is_some_and(|s| matches!(s.work(), crate::Work::One(_)));
    let mine = format!("task {task}: ");
    let mut out = vec![];
    for c in v
        .unmet
        .get(station)?
        .iter()
        .filter(|c| c.status == CheckStatus::Fail)
    {
        match c.kind.as_str() {
            "output" => {
                let parts: Vec<&str> = c
                    .detail
                    .split("; ")
                    .filter(|p| p.starts_with(&mine))
                    .collect();
                if !parts.is_empty() {
                    out.push(format!("{} ({})", c.label, parts.join("; ")));
                }
            }
            "gate" if only => out.push(if c.detail.is_empty() {
                c.label.clone()
            } else {
                format!("{} ({})", c.label, c.detail)
            }),
            _ => {}
        }
    }
    (!out.is_empty()).then(|| out.join("; "))
}

/// The protocol layer, the rules, then the skill.
fn task_message_of(sh: &Shared, job: &SessionJob) -> String {
    let mut rules = vec![];
    if let Ok(engine) = sh.engine() {
        for (name, pinned) in &job.rules {
            match engine.packs().rule(name) {
                Some((text, digest)) => {
                    if pinned.as_ref().is_some_and(|d| *d != digest) {
                        tracing::warn!(rule = name, "flow: the rule changed since the station was entered");
                    }
                    rules.push((name.clone(), text));
                }
                None => tracing::warn!(rule = name, "flow: rule not found"),
            }
        }
    }
    let mut skill = None;
    if let (Some(name), Ok(engine)) = (&job.skill, sh.engine()) {
        match engine.packs().skill(name) {
            Some((body, digest)) => {
                if job.skill_digest.as_ref().is_some_and(|d| *d != digest) {
                    tracing::warn!(
                        skill = name,
                        "flow: the skill changed since the station was entered"
                    );
                }
                skill = Some((name.clone(), body));
            }
            None => tracing::warn!(skill = name, "flow: skill not found"),
        }
    }
    task_message(
        &job.protocol,
        &rules,
        skill.as_ref().map(|(n, b)| (n.as_str(), b.as_str())),
    )
}

/// Run a `command` gate where the run works (xops-04 §4): `sh -c` in its `cwd` (references
/// resolved; a relative one under the run's workspace), else the run's workspace; passes on exit
/// status 0. Refused unless the application turned command gates on.
///
/// The command sees none of this process's environment but the basics (`PATH`, `HOME`, the
/// locale, `TMPDIR`…): no provider key, no hook secret. The run reaches it there, never in the
/// command line — `FLOW_RUN`, `FLOW_STATION`, `FLOW_WORKSPACE`, `FLOW_CONTEXT_FILE` (the reference
/// context as JSON, written under the workspace's `.flow/`) and, when it is small,
/// `FLOW_CONTEXT` itself. It runs in a process group of its own, killed whole when it runs out of
/// time; only the end of its output is kept.
async fn command_gate(sh: &Shared, engine: &crate::Engine, rec: &RunRecord, station: &str, g: &crate::GateDef) -> GateResult {
    use tokio::io::AsyncReadExt;
    let fail = |detail: String| GateResult { pass: false, detail };
    if !sh.cfg.command_gates {
        return fail(format!("command gates are not turned on here: {} cannot run", g.name));
    }
    let Some(command) = g.command.clone().filter(|c| !c.trim().is_empty()) else {
        return fail(format!("gate {} has no command", g.name));
    };
    let Some(ws) = engine.workspace(&rec.id) else {
        return fail(format!("gate {}: the run has no workspace", g.name));
    };
    let ctx = crate::kernel::refs::context_in(rec, None, None, ws.to_str());
    let cwd = match &g.cwd {
        Some(c) => match crate::kernel::refs::resolve_str(c, &ctx) {
            Value::String(s) if !s.is_empty() => {
                let p = std::path::PathBuf::from(&s);
                if p.is_absolute() {
                    p
                } else {
                    ws.join(p)
                }
            }
            _ => return fail(format!("gate {}: cwd {c} resolves to nothing", g.name)),
        },
        None => ws.clone(),
    };
    let context = ctx.to_string();
    let context_file = ws.join(".flow").join(format!("context-{station}.json"));
    let written = std::fs::create_dir_all(&cwd)
        .and_then(|_| std::fs::create_dir_all(ws.join(".flow")))
        .and_then(|_| std::fs::write(&context_file, &context));
    if let Err(e) = written {
        return fail(format!("gate {}: {}: {e}", g.name, cwd.display()));
    }
    let mut cmd = tokio::process::Command::new("sh");
    cmd.arg("-c")
        .arg(&command)
        .current_dir(&cwd)
        .env_clear()
        .env("FLOW_RUN", &rec.id)
        .env("FLOW_STATION", station)
        .env("FLOW_WORKSPACE", &ws)
        .env("FLOW_CONTEXT_FILE", &context_file)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    for k in GATE_ENV {
        if let Some(v) = std::env::var_os(k) {
            cmd.env(k, v);
        }
    }
    if context.len() <= GATE_CONTEXT_INLINE {
        cmd.env("FLOW_CONTEXT", &context);
    }
    #[cfg(unix)]
    cmd.process_group(0);
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return fail(format!("`{command}` did not start: {e}")),
    };
    let pid = child.id();
    let tail = |r: Option<Box<dyn tokio::io::AsyncRead + Unpin + Send>>| async move {
        let mut kept: Vec<u8> = vec![];
        let Some(mut r) = r else { return kept };
        let mut buf = [0u8; 8192];
        while let Ok(n) = r.read(&mut buf).await {
            if n == 0 {
                break;
            }
            kept.extend_from_slice(&buf[..n]);
            if kept.len() > 2 * GATE_OUTPUT_READ {
                kept.drain(..kept.len() - GATE_OUTPUT_READ);
            }
        }
        kept
    };
    let out = tail(child.stdout.take().map(|o| Box::new(o) as _));
    let err = tail(child.stderr.take().map(|o| Box::new(o) as _));
    let waited = tokio::time::timeout(Duration::from_secs(g.timeout_seconds()), async {
        let (o, e, status) = tokio::join!(out, err, child.wait());
        (o, e, status)
    })
    .await;
    let (o, e, status) = match waited {
        Ok(v) => v,
        Err(_) => {
            // The whole group: a test runner's children too.
            #[cfg(unix)]
            if let Some(pid) = pid {
                let _ = std::process::Command::new("kill").arg("-KILL").arg(format!("-{pid}")).status();
            }
            let _ = child.kill().await;
            return fail(format!("`{command}` ran longer than {}s", g.timeout_seconds()));
        }
    };
    let status = match status {
        Ok(s) => s,
        Err(e) => return fail(format!("`{command}` was lost: {e}")),
    };
    let mut text = String::from_utf8_lossy(&o).into_owned();
    text.push_str(&String::from_utf8_lossy(&e));
    let tail: String = {
        let chars: Vec<char> = text.trim().chars().collect();
        chars[chars.len().saturating_sub(GATE_OUTPUT_KEPT)..].iter().collect()
    };
    match status.code() {
        Some(0) => GateResult { pass: true, detail: tail },
        code => fail(format!(
            "`{command}` exited with {}{}",
            code.map_or("a signal".to_string(), |c| c.to_string()),
            if tail.is_empty() { String::new() } else { format!(": {tail}") }
        )),
    }
}

/// What a command gate's process keeps of this one's environment.
const GATE_ENV: &[&str] = &["PATH", "HOME", "USER", "LOGNAME", "LANG", "LC_ALL", "LC_CTYPE", "TMPDIR", "TERM", "SHELL"];
/// The largest context also given inline (`FLOW_CONTEXT`); a larger one only by its file.
const GATE_CONTEXT_INLINE: usize = 32 * 1024;
/// How much of each output stream is read and held at most.
const GATE_OUTPUT_READ: usize = 64 * 1024;

/// How much of a command gate's output its result keeps (the end of it).
const GATE_OUTPUT_KEPT: usize = 2000;

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
                scene: "flow-harnix".into(),
                session: Some(SessionBinding::New),
                model: None,
                provider: None,
                instance: None,
                template: None,
                params: Default::default(),
                space: None,
            },
            skill: None,
            skill_digest: None,
            rules: vec![],
            permissions: vec![],
            item: None,
            output_schema: None,
            protocol: None,
            session_key: None,
            session: None,
            placement: None,
            scene: None,
            delivery: None,
        };
        assert_eq!(run_of(&w), Some("r-1"));
        assert_eq!(
            run_of(&WorkItem::CloseSession {
                session: "s".into()
            }),
            None
        );
    }

    fn job(round: u32, token: &str, session_key: Option<&str>) -> SessionJob {
        SessionJob {
            main: false,
            key: TaskKey {
                run: "r".into(),
                station: "s".into(),
                round,
                task: "t".into(),
            },
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
        let model = Placement {
            model: Some("m".into()),
            ..Default::default()
        };
        let m = settings_of("harnix", Delivery::StructuredOutput, Some(&schema), &model, None);
        assert_eq!(settings_differ(&a, &m), ["model"]);
    }

    #[test]
    fn only_what_the_session_can_fix_is_sent_back_to_it() {
        use crate::{CheckRec, CheckStatus, RunView};
        let view = |work: &str, checks: Vec<CheckRec>| -> RunView {
            let rec: RunRecord = serde_json::from_value(json!({
                "id": "r", "flow": "f", "def_digest": "d", "subject": "s", "status": "active",
                "rev": 1, "created_at": 0, "created_by": {"kind": "x", "id": "y"},
                "def": serde_yaml::from_str::<Value>(work).unwrap(),
                "stations": {"a": {"status": "unmet", "round": 1}}
            }))
            .unwrap();
            RunView {
                record: rec,
                unmet: [("a".to_string(), checks)].into(),
                phase: None,
            }
        };
        let check = |kind: &str, detail: &str| CheckRec {
            cond: Some(0),
            kind: kind.into(),
            label: kind.into(),
            round: 1,
            status: CheckStatus::Fail,
            detail: detail.into(),
            at: 0,
        };
        let one = "{name: f, stations: [{id: a, executor: {kind: session, scene: harnix}, skill: s, output: {type: object}, exit: [{kind: output}]}]}";
        let many = "{name: f, stations: [{id: a, tasks: [{id: x, executor: {kind: session, scene: harnix}, skill: s}, {id: y, executor: {kind: external}}], output: {type: object}, exit: [{kind: output}]}]}";
        // Its own output: yes; another task's part of the same check: no.
        let v = view(
            many,
            vec![check("output", "task x: missing n; task y: no output")],
        );
        assert_eq!(
            corrections_for(&v, "a", "x").unwrap(),
            "output (task x: missing n)"
        );
        assert_eq!(
            corrections_for(&v, "a", "y").unwrap(),
            "output (task y: no output)"
        );
        // A confirmation, another task failing: never the session's.
        let v = view(
            one,
            vec![
                check("task", "failed"),
                check("confirm", ""),
            ],
        );
        assert_eq!(corrections_for(&v, "a", "a"), None);
        // A gate judges the station: its only task's to fix, nobody's among several.
        assert!(corrections_for(&view(one, vec![check("gate", "total is 2")]), "a", "a").is_some());
        assert_eq!(
            corrections_for(&view(many, vec![check("gate", "total is 2")]), "a", "x"),
            None
        );
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
            lock_key(&job(1, "a", None)),
            lock_key(&job(2, "b", None)),
            "a rework's new round has a new session: it does not wait for the old turn"
        );
        assert_eq!(
            lock_key(&job(1, "a", Some("run"))),
            lock_key(&job(1, "b", Some("run")))
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
            permissions: vec![],
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
