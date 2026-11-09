//! A session task: a headless session where the station places it — its space (the flow's by
//! default), its instance or an instance of its template made ready first, its model — or the
//! one its session key is bound to, told the protocol layer, the rules and the skill. The
//! station's contract is the session's `output_schema` when it hands in by structured output; the
//! session `ask`s about every guarded tool and every tool a permission names, and the module
//! answers each question from the permissions of the task the turn works on, whatever carries
//! the session (xops-04 §3). After every turn, the turn's structured output (if any) is
//! submitted, the run is evaluated and the claim renewed; an output the station still judges
//! unmet is answered with what is missing; a turn that ends with nothing handed in gets one
//! nudge, and a second one fails the attempt. Tasks sharing a session take turns, and a task
//! waiting for its turn keeps its claim alive. A session is bound to a task only while no turn of
//! anyone else's is running in it, and a task that is over while its turn still runs interrupts
//! that turn and waits for it to settle before the session goes to the next task.
use std::sync::Arc;
use std::time::Duration;

use harnix_module::{OnBusy, SessionRef, SessionSpec, Tap};
use serde_json::{json, Value};

use super::dispatch::{apply, dispatcher, fail, renew, renew_every, settle, settle_quiet};
use super::shared::{session_name, session_ref, Binding, Shared, TaskKey};
use crate::kernel::host::{Delivery, Placement};
use crate::kernel::parts::asked_tools;
use crate::kernel::protocol::{ask_rules, task_message, STRUCTURED_OUTPUT_TOOL};
use crate::kernel::run::{CheckStatus, RunRecord, StationStatus, TaskStatus};
use crate::kernel::FlowError;

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

pub(crate) struct SessionJob {
    pub(crate) key: TaskKey,
    pub(crate) token: String,
    pub(crate) scene: String,
    pub(crate) skill: Option<String>,
    pub(crate) skill_digest: Option<String>,
    /// The rules it works under, with the digests pinned on station entry.
    pub(crate) rules: Vec<(String, Option<String>)>,
    /// What it works under (xops-04 §3).
    pub(crate) permissions: Vec<crate::kernel::parts::Granted>,
    pub(crate) protocol: String,
    pub(crate) session_key: Option<String>,
    pub(crate) recording: Option<String>,
    pub(crate) resume: bool,
    pub(crate) output_schema: Option<Value>,
    pub(crate) placement: Placement,
    pub(crate) delivery: Delivery,
    /// A turn of the run's main session (xops-03 §1).
    pub(crate) main: bool,
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

pub(crate) async fn session_task(sh: &Arc<Shared>, job: SessionJob) {
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
            _ = sh.cancel.cancelled() => return None,
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
        let next = tokio::select! {
            next = tokio::time::timeout(every, tap.next()) => next,
            _ = sh.cancel.cancelled() => return Waited::Superseded,
        };
        let frame = match next {
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
        let next = tokio::select! {
            next = tokio::time::timeout(every, tap.next_settled()) => next,
            _ = sh.cancel.cancelled() => return Waited::Superseded,
        };
        match next {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::host::{Outcome, WorkItem};
    use crate::kernel::run::Actor;
    use crate::{CreateIn, Engine, ManualClock, MemPacks, MemStore};

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
}
