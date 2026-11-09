//! Judging a task's turns: the loop a session task runs once its session is bound to it.
//!
//! The task's message goes first — unless the task was taken up after a restart: it was sent
//! then. After every turn of the task's own, the turn's structured output (if any) is submitted,
//! the run is evaluated and the claim renewed. An output the contract refuses, or one taken that
//! leaves its station unmet on a check the output can fix, is sent back with what is wrong, at
//! most [`CORRECTIONS`] times; a turn that ends with nothing handed in gets one nudge, and a
//! second one fails the attempt. A task that is over while its turn still runs stops that turn
//! and waits for it to settle.
//!
//! The loop reads its session as a stream of frames and acts on it through [`TaskSession`]: over
//! a hub tap in the module (`session.rs`), over a script in the tests below. The kernel side is
//! the module's engine either way.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use futures::{Stream, StreamExt};
use serde_json::Value;

use super::dispatch::{apply, dispatcher, fail, renew, renew_every, settle, settle_quiet};
use super::session::{standing, Standing};
use super::shared::{Shared, TaskKey};
use crate::kernel::host::Delivery;
use crate::kernel::protocol::STRUCTURED_OUTPUT_TOOL;
use crate::kernel::run::{CheckStatus, StationStatus};
use crate::kernel::FlowError;

/// A session's frame as a tap reads it: `{method, params}`.
pub(crate) type Frame = Value;

/// What the loop does to its session besides reading it.
#[async_trait::async_trait]
pub(crate) trait TaskSession: Send + Sync {
    /// Send `text` as a message, queued behind a turn that is running.
    async fn send(&self, text: &str) -> Result<(), String>;
    /// Stop the turn that is running.
    async fn interrupt(&self);
}

/// How a task's turns go.
pub(crate) struct Turns {
    /// A turn of the run's main session: only the turns its own messages start are the task's.
    pub main: bool,
    pub delivery: Delivery,
    /// What is sent first: the task. `None` for a task taken up after a restart, which was sent
    /// it before.
    pub message: Option<String>,
    /// Taken up after a restart with a turn still running: that turn is the task's.
    pub resumed_running: bool,
}

/// How long a task that is over waits for the turn it interrupted to settle.
const SETTLE_AFTER_INTERRUPT: Duration = Duration::from_secs(30);

/// What a session is told when a turn ends and its task is still not handed in (`flow_submit`).
pub(crate) const NUDGE: &str = "Your flow task is not handed in yet. Call `flow_submit` with an output \
     that satisfies the contract in the flow protocol above (`flow_state` shows what is still \
     missing). If you cannot do the task, say why and use `flow_propose`.";

/// The same, for a session that hands in by structured output.
pub(crate) const NUDGE_STRUCTURED: &str = "Your flow task is not handed in yet. Call the \
     `StructuredOutput` tool with an output that satisfies the contract in the flow protocol \
     above. If you cannot do the task, say why.";

/// How many times an output the station still judges unmet is sent back to be fixed.
const CORRECTIONS: u32 = 2;

/// How the wait for a turn ended.
pub(crate) enum Waited {
    /// The turn's settlement (`harnix.turn_settled` params).
    Settled(Value),
    /// The claim is no longer ours.
    Superseded,
    /// The session is gone from the hub.
    Gone,
}

/// Carry a task in its bound session: [`judge`] its turns until it is in, over or failed, then,
/// if its turn may still be running, stop that turn and wait for it to settle.
pub(crate) async fn carry<S, F>(
    sh: &Arc<Shared>,
    key: &TaskKey,
    token: &str,
    turns: Turns,
    session: &S,
    frames: &mut F,
) where
    S: TaskSession + ?Sized,
    F: Stream<Item = Frame> + Unpin + Send + ?Sized,
{
    let still_running = judge(sh, key, token, turns, session, frames).await;
    if still_running && !sh.stopping() {
        // The task is over while its turn runs on: stop that turn before the session is
        // anyone else's, so nothing it does lands under the next task.
        session.interrupt().await;
        let _ = tokio::time::timeout(SETTLE_AFTER_INTERRUPT, next_settled(frames)).await;
    }
}

/// Send the task, then judge after every turn until it is in, over, or failed. Returns
/// whether a turn may still be running.
async fn judge<S, F>(
    sh: &Arc<Shared>,
    key: &TaskKey,
    token: &str,
    turns: Turns,
    session: &S,
    frames: &mut F,
) -> bool
where
    S: TaskSession + ?Sized,
    F: Stream<Item = Frame> + Unpin + Send + ?Sized,
{
    let Ok(engine) = sh.engine() else {
        return false;
    };
    // In the main session a person may talk too: only the turns this task's messages start are
    // its own (review #2). A resumed task's turn still running is taken as its own.
    let mut own = OwnTurns {
        main: turns.main,
        texts: vec![],
        turns: Default::default(),
        any_next: turns.resumed_running,
    };
    // A resumed task whose session is idle had its turn cut off: judge it as a turn that ended.
    let mut running = turns.resumed_running;
    if let Some(message) = turns.message {
        own.texts.push(message.clone());
        if let Err(e) = session.send(&message).await {
            fail(sh, key, token, &format!("the task was not sent: {e}")).await;
            return false;
        }
        running = true;
    }
    let mut nudged = false;
    let mut corrections = 0;
    loop {
        let mut handed: Option<Value> = None;
        if running {
            match wait_own_turn(sh, frames, key, token, &mut own).await {
                Waited::Settled(params) => {
                    handed = params
                        .pointer("/outcome/structured_output")
                        .filter(|v| !v.is_null())
                        .cloned();
                    // A main session's turns are counted by its watch (xops-03 §4).
                    if let (false, Some(u)) = (
                        turns.main,
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
                    if !sh.stopping() && standing(sh, key, token).await == Standing::Claimed {
                        fail(sh, key, token, "the session went away").await;
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
                    token,
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
        match standing(sh, key, token).await {
            Standing::Gone => return false,
            Standing::Submitted => {
                // Handed in, and judged: a station unmet on a check that failed (a `tool`
                // exit, say) is the output's to fix — the session is told what, and hands in
                // again. One waiting on a gate, a person or other tasks is not.
                if turns.delivery == Delivery::StructuredOutput && corrections < CORRECTIONS {
                    if let Some(missing) = failed_checks(sh, key).await {
                        corrections += 1;
                        let text = format!(
                            "The flow took your output, but station `{}` is not passed: {missing}. \
                             Fix the output and hand it in again with `{STRUCTURED_OUTPUT_TOOL}`.",
                            key.station
                        );
                        own.texts.push(text.clone());
                        if renew(sh, key, token).await && session.send(&text).await.is_ok() {
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
            if corrections < CORRECTIONS && renew(sh, key, token).await {
                corrections += 1;
                let text = format!(
                    "The flow did not take your output: {m}. Fix it and hand it in again with \
                     `{STRUCTURED_OUTPUT_TOOL}`."
                );
                own.texts.push(text.clone());
                if session.send(&text).await.is_ok() {
                    running = true;
                    continue;
                }
            }
        }
        if !renew(sh, key, token).await {
            return false;
        }
        if nudged {
            fail(
                sh,
                key,
                token,
                "the session ended a turn twice without handing the task in",
            )
            .await;
            return false;
        }
        nudged = true;
        let nudge = match turns.delivery {
            Delivery::FlowSubmit => NUDGE,
            Delivery::StructuredOutput => NUDGE_STRUCTURED,
        };
        own.texts.push(nudge.to_string());
        if let Err(e) = session.send(nudge).await {
            fail(sh, key, token, &format!("the reminder was not sent: {e}")).await;
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
    turns: HashSet<String>,
    /// The next settlement is the task's whatever started it (a resumed task's running turn).
    any_next: bool,
}

/// [`wait_turn`] for a task: in the main session, the next settlement of a turn one of its own
/// messages started — a person's turns in between are not the task's.
async fn wait_own_turn<F>(
    sh: &Arc<Shared>,
    frames: &mut F,
    key: &TaskKey,
    token: &str,
    own: &mut OwnTurns,
) -> Waited
where
    F: Stream<Item = Frame> + Unpin + Send + ?Sized,
{
    if !own.main {
        return wait_turn(sh, frames, key, token).await;
    }
    let every = renew_every(sh);
    loop {
        let next = tokio::select! {
            next = tokio::time::timeout(every, frames.next()) => next,
            _ = sh.cancel.cancelled() => return Waited::Superseded,
        };
        let frame = match next {
            Ok(Some(f)) => f,
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

/// The next settled turn, whoever started it, renewing the claim while it runs.
pub(crate) async fn wait_turn<F>(sh: &Arc<Shared>, frames: &mut F, key: &TaskKey, token: &str) -> Waited
where
    F: Stream<Item = Frame> + Unpin + Send + ?Sized,
{
    let every = renew_every(sh);
    loop {
        let next = tokio::select! {
            next = tokio::time::timeout(every, next_settled(frames)) => next,
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

/// The params of the next `harnix.turn_settled` frame; `None` when the session is gone.
async fn next_settled<F>(frames: &mut F) -> Option<Value>
where
    F: Stream<Item = Frame> + Unpin + Send + ?Sized,
{
    while let Some(frame) = frames.next().await {
        if frame.get("method").and_then(Value::as_str) == Some("harnix.turn_settled") {
            return Some(frame.get("params").cloned().unwrap_or(Value::Null));
        }
    }
    None
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

/// The loop against the real kernel (an engine on a memory store, its clock still) and a
/// scripted session: what the flow specs in `tests/live/ui/flow*.spec.ts` see through a whole
/// backend, a turn at a time.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::host::{RunStore, Tools, WorkItem};
    use crate::kernel::run::{Actor, RunStatus, TaskStatus};
    use crate::{CreateIn, Engine, ManualClock, MemPacks, MemStore, RunRecord};
    use futures::channel::mpsc;
    use serde_json::json;
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicI64, Ordering as AtomicOrdering};
    use std::sync::Mutex;

    /// One task in a session of its own, whose output needs an `n`.
    const FLOW: &str = "{name: f, stations: [\
        {id: a, executor: {kind: session, scene: harnix}, skill: do, output: {type: object, required: [n]}, exit: [{kind: output}]},\
        {id: b, exit: [{kind: confirm}]}]}";

    /// The same, judged by a tool too: an output that is taken can still leave the station unmet.
    const COUNTED: &str = "{name: counted, stations: [\
        {id: a, executor: {kind: session, scene: harnix}, skill: do, output: {type: object}, exit: [{kind: output}, {kind: gate, gate: zero}]},\
        {id: b, exit: [{kind: confirm}]}]}";

    fn packs() -> MemPacks {
        MemPacks::new()
            .scene("harnix")
            .skill("do", "Do it.")
            .tool("count")
            .gate_def("{name: zero, kind: tool, tool: count, args: {run: '${run}'}, assert: [{path: total, eq: 0}]}")
            .flow(FLOW)
            .flow(COUNTED)
    }

    /// `count`: what is left to fix, outside the run — what a session's work changes.
    struct Count(Arc<AtomicI64>);

    #[async_trait::async_trait]
    impl Tools for Count {
        async fn call(&self, _: &str, _: &Value) -> Result<Value, String> {
            Ok(json!({"total": self.0.load(AtomicOrdering::SeqCst)}))
        }
    }

    /// A run's one session task, claimed by the dispatcher, and the module around it.
    struct Task {
        sh: Arc<Shared>,
        engine: Arc<Engine>,
        key: TaskKey,
        token: String,
    }

    impl Task {
        async fn record(&self) -> RunRecord {
            self.engine.record(&self.key.run).await.unwrap()
        }
        async fn station(&self) -> StationStatus {
            self.record().await.stations["a"].status
        }
        async fn task(&self) -> crate::TaskRec {
            self.record().await.stations["a"].task("a").unwrap().clone()
        }
    }

    fn engine_on(store: Arc<dyn RunStore>, left: Arc<AtomicI64>) -> Arc<Engine> {
        Arc::new(
            Engine::new(store, Arc::new(packs()))
                .with_clock(Arc::new(ManualClock::new(1_000_000)))
                .with_tools(Arc::new(Count(left)))
                .with_lease_ms(60_000),
        )
    }

    /// The module on `engine`, holding the claim `work` hands out.
    fn module(engine: Arc<Engine>, work: &WorkItem) -> Task {
        let WorkItem::Task {
            run,
            station,
            round,
            task,
            token,
            ..
        } = work
        else {
            panic!("not a task: {work:?}");
        };
        let sh = Arc::new(Shared::for_tests());
        let _ = sh.engine.set(engine.clone());
        Task {
            sh,
            engine,
            key: TaskKey {
                run: run.clone(),
                station: station.clone(),
                round: *round,
                task: task.clone(),
            },
            token: token.clone(),
        }
    }

    async fn start(flow: &str) -> Task {
        start_on(Arc::new(MemStore::new()), flow, Arc::default()).await
    }

    async fn start_on(store: Arc<dyn RunStore>, flow: &str, left: Arc<AtomicI64>) -> Task {
        let engine = engine_on(store, left);
        let o = engine
            .create(
                CreateIn {
                    flow: flow.into(),
                    subject: "s".into(),
                    ..Default::default()
                },
                Actor::new("device", "d"),
            )
            .await
            .unwrap();
        module(engine, &o.work[0])
    }

    fn turns(delivery: Delivery) -> Turns {
        Turns {
            main: false,
            delivery,
            message: Some("Do the task.".into()),
            resumed_running: false,
        }
    }

    fn settled(turn: &str, output: Option<Value>) -> Frame {
        json!({"method": "harnix.turn_settled", "params": {
            "turn_id": turn, "result": "done",
            "outcome": {"stop_reason": "end_turn", "structured_output": output}}})
    }

    fn user_message(turn: &str, text: &str) -> Frame {
        json!({"method": "harnix.event", "params": {
            "turn_id": turn, "seq": 1, "event": {"kind": "user_message", "text": text}}})
    }

    /// What one message sent makes the session do.
    enum Turn {
        /// The turn ends, with this structured output (or none).
        Ends(Option<Value>),
        /// The turn runs until it is interrupted.
        Runs,
    }

    type OnSend = Box<dyn FnMut(&str) + Send>;

    /// A session played from a script: each message sent starts the next scripted turn, whose
    /// frames — the message's `user_message`, then its settlement — arrive on [`Script::frames`].
    struct Script {
        turns: Mutex<VecDeque<Turn>>,
        tx: mpsc::UnboundedSender<Frame>,
        sent: Mutex<Vec<String>>,
        running: Mutex<Option<String>>,
        interrupts: Mutex<u32>,
        /// What the session does about a message, beyond its turn.
        on_send: Mutex<Option<OnSend>>,
    }

    impl Script {
        fn new(turns: Vec<Turn>) -> (Self, mpsc::UnboundedReceiver<Frame>) {
            let (tx, rx) = mpsc::unbounded();
            let s = Self {
                turns: Mutex::new(turns.into()),
                tx,
                sent: Mutex::default(),
                running: Mutex::default(),
                interrupts: Mutex::default(),
                on_send: Mutex::default(),
            };
            (s, rx)
        }
        /// A frame from outside the script: a person's turn, one from before a restart.
        fn push(&self, frame: Frame) {
            self.tx.unbounded_send(frame).unwrap();
        }
        fn sent(&self) -> Vec<String> {
            self.sent.lock().unwrap().clone()
        }
        fn interrupts(&self) -> u32 {
            *self.interrupts.lock().unwrap()
        }
    }

    #[async_trait::async_trait]
    impl TaskSession for Script {
        async fn send(&self, text: &str) -> Result<(), String> {
            let turn = {
                let mut sent = self.sent.lock().unwrap();
                sent.push(text.to_string());
                format!("t{}", sent.len())
            };
            if let Some(f) = self.on_send.lock().unwrap().as_mut() {
                f(text);
            }
            self.push(user_message(&turn, text));
            match self.turns.lock().unwrap().pop_front() {
                Some(Turn::Ends(output)) => self.push(settled(&turn, output)),
                Some(Turn::Runs) | None => *self.running.lock().unwrap() = Some(turn),
            }
            Ok(())
        }
        async fn interrupt(&self) {
            *self.interrupts.lock().unwrap() += 1;
            if let Some(turn) = self.running.lock().unwrap().take() {
                self.push(settled(&turn, None));
            }
        }
    }

    /// [`carry`], which must end: a loop left waiting is a failure, not a hung test (time is
    /// paused, so an hour of waiting passes at once).
    async fn carry_with(t: &Task, turns: Turns, script: &Script, frames: &mut mpsc::UnboundedReceiver<Frame>) {
        let carried = carry(&t.sh, &t.key, &t.token, turns, script, frames);
        tokio::time::timeout(Duration::from_secs(3600), carried)
            .await
            .expect("the task is still being carried");
    }

    #[tokio::test(start_paused = true)]
    async fn a_turn_that_hands_in_passes_its_station() {
        let t = start("f").await;
        let (script, mut frames) = Script::new(vec![Turn::Ends(Some(json!({"n": 1})))]);
        carry_with(&t, turns(Delivery::StructuredOutput), &script, &mut frames).await;
        assert_eq!(script.sent(), ["Do the task."]);
        assert_eq!(t.station().await, StationStatus::Passed);
        assert_eq!(t.task().await.output, Some(json!({"n": 1})));
    }

    #[tokio::test(start_paused = true)]
    async fn a_turn_that_hands_in_nothing_is_nudged_once_and_a_second_fails_the_attempt() {
        for (delivery, nudge) in [(Delivery::StructuredOutput, NUDGE_STRUCTURED), (Delivery::FlowSubmit, NUDGE)] {
            let t = start("f").await;
            let (script, mut frames) = Script::new(vec![Turn::Ends(None), Turn::Ends(None)]);
            carry_with(&t, turns(delivery), &script, &mut frames).await;
            assert_eq!(script.sent(), ["Do the task.", nudge]);
            let task = t.task().await;
            assert_eq!(task.status, TaskStatus::Failed);
            assert_eq!(
                task.error.as_deref(),
                Some("the session ended a turn twice without handing the task in")
            );
        }
    }

    #[tokio::test(start_paused = true)]
    async fn a_nudged_session_that_hands_in_passes() {
        let t = start("f").await;
        let (script, mut frames) = Script::new(vec![Turn::Ends(None), Turn::Ends(Some(json!({"n": 2})))]);
        carry_with(&t, turns(Delivery::StructuredOutput), &script, &mut frames).await;
        assert_eq!(script.sent(), ["Do the task.", NUDGE_STRUCTURED]);
        assert_eq!(t.station().await, StationStatus::Passed);
    }

    #[tokio::test(start_paused = true)]
    async fn an_output_the_contract_refuses_goes_back_with_what_is_wrong() {
        let t = start("f").await;
        let (script, mut frames) = Script::new(vec![
            Turn::Ends(Some(json!({"m": 1}))),
            Turn::Ends(Some(json!({"n": 1}))),
        ]);
        carry_with(&t, turns(Delivery::StructuredOutput), &script, &mut frames).await;
        let sent = script.sent();
        assert_eq!(sent.len(), 2);
        assert!(sent[1].starts_with("The flow did not take your output: "), "{}", sent[1]);
        assert!(sent[1].contains('n') && sent[1].contains(STRUCTURED_OUTPUT_TOOL), "{}", sent[1]);
        assert_eq!(t.station().await, StationStatus::Passed);
    }

    /// Two corrections, then the nudge (a correction is not one), then the attempt fails.
    #[tokio::test(start_paused = true)]
    async fn corrections_run_out_then_the_nudge_then_the_attempt_fails() {
        let t = start("f").await;
        let bad = || Turn::Ends(Some(json!({"m": 1})));
        let (script, mut frames) = Script::new(vec![bad(), bad(), bad(), bad()]);
        carry_with(&t, turns(Delivery::StructuredOutput), &script, &mut frames).await;
        let sent = script.sent();
        assert_eq!(sent.len(), 4, "{sent:#?}");
        assert!(sent[1].starts_with("The flow did not take your output") && sent[2].starts_with("The flow did not take your output"));
        assert_eq!(sent[3], NUDGE_STRUCTURED);
        assert_eq!(t.task().await.status, TaskStatus::Failed);
    }

    /// Taken, but its station is unmet on a check the output can fix: the session is told which.
    #[tokio::test(start_paused = true)]
    async fn an_output_taken_but_failing_its_stations_check_goes_back() {
        let left = Arc::new(AtomicI64::new(2));
        let t = start_on(Arc::new(MemStore::new()), "counted", left.clone()).await;
        let (script, mut frames) = Script::new(vec![
            Turn::Ends(Some(json!({"n": 1}))),
            Turn::Ends(Some(json!({"n": 0}))),
        ]);
        // The session fixes what it is told is wrong.
        *script.on_send.lock().unwrap() = Some(Box::new(move |text: &str| {
            if text.starts_with("The flow took your output") {
                left.store(0, AtomicOrdering::SeqCst);
            }
        }));
        carry_with(&t, turns(Delivery::StructuredOutput), &script, &mut frames).await;
        let sent = script.sent();
        assert_eq!(sent.len(), 2, "{sent:#?}");
        assert!(
            sent[1].starts_with("The flow took your output, but station `a` is not passed: "),
            "{}",
            sent[1]
        );
        assert_eq!(t.station().await, StationStatus::Passed);
        assert_eq!(t.task().await.output, Some(json!({"n": 0})));
    }

    /// The run is cancelled while the task's turn runs: the claim is gone at the next renewal,
    /// and the turn is stopped and waited for before the session is anyone else's.
    #[tokio::test(start_paused = true)]
    async fn a_task_over_while_its_turn_runs_interrupts_that_turn() {
        let t = start("f").await;
        let (script, mut frames) = Script::new(vec![Turn::Runs]);
        let cancel = async {
            tokio::time::sleep(Duration::from_secs(1)).await;
            t.engine
                .cancel(&t.key.run, "no longer wanted", Actor::new("device", "d"))
                .await
                .unwrap();
        };
        tokio::join!(carry_with(&t, turns(Delivery::StructuredOutput), &script, &mut frames), cancel);
        assert_eq!(script.interrupts(), 1);
        assert_eq!(t.record().await.status, RunStatus::Cancelled);
        assert!(script.running.lock().unwrap().is_none(), "the interrupted turn settled");
    }

    /// A module that stops only stops waiting: the turn is left to finish, the claim to lapse,
    /// and the next start takes the task up again.
    #[tokio::test(start_paused = true)]
    async fn stopping_the_module_does_not_interrupt_the_turn() {
        let t = start("f").await;
        let (script, mut frames) = Script::new(vec![Turn::Runs]);
        let stop = async {
            tokio::time::sleep(Duration::from_secs(1)).await;
            t.sh.cancel.cancel();
        };
        tokio::join!(carry_with(&t, turns(Delivery::StructuredOutput), &script, &mut frames), stop);
        assert_eq!(script.interrupts(), 0);
        assert_eq!(t.task().await.status, TaskStatus::Claimed);
    }

    #[tokio::test(start_paused = true)]
    async fn a_session_that_goes_away_fails_the_attempt() {
        let t = start("f").await;
        let (script, mut frames) = Script::new(vec![Turn::Runs]);
        let gone = async {
            tokio::time::sleep(Duration::from_secs(1)).await;
            script.tx.close_channel();
        };
        tokio::join!(carry_with(&t, turns(Delivery::StructuredOutput), &script, &mut frames), gone);
        let task = t.task().await;
        assert_eq!(task.status, TaskStatus::Failed);
        assert_eq!(task.error.as_deref(), Some("the session went away"));
    }

    /// The module restarted: the work is rebuilt from the store under the same claim. A turn
    /// that was still running is the task's, whoever's message started it — in a session of the
    /// task's own and in the main session alike — and nothing is sent again.
    #[tokio::test(start_paused = true)]
    async fn after_a_restart_the_turn_still_running_is_the_tasks() {
        for main in [false, true] {
            let store = Arc::new(MemStore::new());
            let before = start_on(store.clone(), "f", Arc::default()).await;
            let engine = engine_on(store, Arc::default());
            let rec = engine.record(&before.key.run).await.unwrap();
            let work = engine.dispatched_work(&rec, engine.clock().now_ms());
            assert_eq!(work.len(), 1, "the claim is still the dispatcher's");
            let t = module(engine, &work[0]);
            assert_eq!(t.token, before.token);

            let (script, mut frames) = Script::new(vec![]);
            script.push(settled("before-restart", Some(json!({"n": 3}))));
            let resumed = Turns {
                main,
                delivery: Delivery::StructuredOutput,
                message: None,
                resumed_running: true,
            };
            carry_with(&t, resumed, &script, &mut frames).await;
            assert!(script.sent().is_empty(), "main: {main}");
            assert_eq!(t.station().await, StationStatus::Passed, "main: {main}");
        }
    }

    /// After a restart with the session idle, the turn was cut off: it is judged as one that
    /// ended without a hand-in — a nudge, not the task told again.
    #[tokio::test(start_paused = true)]
    async fn after_a_restart_an_idle_session_is_nudged_not_told_again() {
        let t = start("f").await;
        let (script, mut frames) = Script::new(vec![Turn::Ends(Some(json!({"n": 4})))]);
        let resumed = Turns {
            message: None,
            ..turns(Delivery::StructuredOutput)
        };
        carry_with(&t, resumed, &script, &mut frames).await;
        assert_eq!(script.sent(), [NUDGE_STRUCTURED]);
        assert_eq!(t.station().await, StationStatus::Passed);
    }

    /// In the main session a person talks too: a turn their message started, and what it hands
    /// in, is not the task's.
    #[tokio::test(start_paused = true)]
    async fn in_the_main_session_a_persons_turn_is_not_the_tasks() {
        let t = start("f").await;
        let (script, mut frames) = Script::new(vec![Turn::Ends(Some(json!({"n": "task"})))]);
        script.push(user_message("person", "How is it going?"));
        script.push(settled("person", Some(json!({"n": "person"}))));
        let main = Turns {
            main: true,
            ..turns(Delivery::FlowSubmit)
        };
        carry_with(&t, main, &script, &mut frames).await;
        assert_eq!(t.station().await, StationStatus::Passed);
        assert_eq!(t.task().await.output, Some(json!({"n": "task"})));
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
}
