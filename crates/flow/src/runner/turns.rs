//! Judging a task's turns: the loop a session task runs once its session is bound to it.
//!
//! The task's message goes first — unless the task was taken up after a restart: it was sent
//! then. After every turn of the task's own, the turn's structured output (if any) is submitted,
//! the run is evaluated and the claim renewed. An output the contract refuses, or one taken that
//! leaves its station unmet on a check the output can fix, is sent back with what is wrong, at
//! most [`CORRECTIONS`] times; a turn that ends with nothing handed in gets one nudge, and a
//! second one fails the attempt; a turn that fails for good (no credentials) fails the attempt at
//! once. A task that is over while its turn still runs stops that turn and waits for it to
//! settle.
//!
//! The loop reads its session as a stream of [`Frame`]s and acts on it through [`TaskSession`]:
//! the host's session in the runner (`session.rs`), a scripted one in the tests below. The kernel
//! side is the runner's engine either way.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use futures::{Stream, StreamExt};
use serde_json::Value;

use super::dispatch::{apply, dispatcher, fail, renew, renew_every, settle, settle_quiet};
use super::host::{Frame, SessionHost, SessionId, TurnEnd};
use super::session::{standing, Standing};
use super::state::{Binding, TaskKey};
use super::Runner;
use crate::kernel::host::Delivery;
use crate::kernel::protocol::STRUCTURED_OUTPUT_TOOL;
use crate::kernel::run::{CheckStatus, StationStatus, Usage};
use crate::kernel::FlowError;

/// What the loop does to its session besides reading it.
#[async_trait::async_trait]
pub(crate) trait TaskSession: Send + Sync {
    /// Send `text` as a message, queued behind a turn that is running.
    async fn send(&self, text: &str) -> Result<(), String>;
    /// Stop the turn that is running.
    async fn interrupt(&self);
}

/// A session of the host's, as the loop acts on it: what is sent to a main session for a task is
/// noted first, so its turn is told from a person's.
pub(crate) struct HostSession<'a> {
    pub rn: &'a Runner,
    pub host: &'a dyn SessionHost,
    pub id: &'a SessionId,
    /// The task the session is bound to: what a main session's turns are told apart by.
    pub binding: Option<Binding>,
}

#[async_trait::async_trait]
impl TaskSession for HostSession<'_> {
    async fn send(&self, text: &str) -> Result<(), String> {
        if let Some(b) = self.binding.clone() {
            self.rn.state.note_sent(self.id, text, b);
        }
        self.host.send(self.id, text).await.map_err(|e| e.to_string())
    }
    async fn interrupt(&self) {
        let _ = self.host.interrupt(self.id).await;
    }
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
pub const NUDGE: &str = "Your flow task is not handed in yet. Call `flow_submit` with an output \
     that satisfies the contract in the flow protocol above (`flow_state` shows what is still \
     missing). If you cannot do the task, say why and use `flow_propose`.";

/// The same, for a session that hands in by structured output.
pub const NUDGE_STRUCTURED: &str = "Your flow task is not handed in yet. Call the \
     `StructuredOutput` tool with an output that satisfies the contract in the flow protocol \
     above. If you cannot do the task, say why.";

/// How many times an output the station still judges unmet is sent back to be fixed.
const CORRECTIONS: u32 = 2;

/// How a turn ended, as the loop reads it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Settled {
    pub end: TurnEnd,
    pub output: Option<Value>,
    pub usage: Option<Usage>,
}

/// How the wait for a turn ended.
pub(crate) enum Waited {
    /// The turn's settlement.
    Settled(Settled),
    /// The claim is no longer ours.
    Superseded,
    /// The session is gone from the host.
    Gone,
}

/// Carry a task in its bound session: [`judge`] its turns until it is in, over or failed, then,
/// if its turn may still be running, stop that turn and wait for it to settle.
pub(crate) async fn carry<S, F>(rn: &Arc<Runner>, key: &TaskKey, token: &str, turns: Turns, session: &S, frames: &mut F)
where
    S: TaskSession + ?Sized,
    F: Stream<Item = Frame> + Unpin + Send + ?Sized,
{
    let still_running = judge(rn, key, token, turns, session, frames).await;
    if still_running && !rn.stopping() {
        // The task is over while its turn runs on: stop that turn before the session is
        // anyone else's, so nothing it does lands under the next task.
        session.interrupt().await;
        let _ = tokio::time::timeout(SETTLE_AFTER_INTERRUPT, next_settled(frames)).await;
    }
}

/// Send the task, then judge after every turn until it is in, over, or failed. Returns
/// whether a turn may still be running.
async fn judge<S, F>(rn: &Arc<Runner>, key: &TaskKey, token: &str, turns: Turns, session: &S, frames: &mut F) -> bool
where
    S: TaskSession + ?Sized,
    F: Stream<Item = Frame> + Unpin + Send + ?Sized,
{
    let engine = rn.engine();
    // In the main session a person may talk too: only the turns this task's messages start are
    // its own. A resumed task's turn still running is taken as its own.
    let mut own = OwnTurns { main: turns.main, texts: vec![], turns: Default::default(), any_next: turns.resumed_running };
    // A resumed task whose session is idle had its turn cut off: judge it as a turn that ended.
    let mut running = turns.resumed_running;
    if let Some(message) = turns.message {
        own.texts.push(message.clone());
        if let Err(e) = session.send(&message).await {
            fail(rn, key, token, &format!("the task was not sent: {e}")).await;
            return false;
        }
        running = true;
    }
    let mut nudged = false;
    let mut corrections = 0;
    loop {
        let mut handed: Option<Value> = None;
        if running {
            match wait_own_turn(rn, frames, key, token, &mut own).await {
                Waited::Settled(s) => {
                    // A turn that failed and would fail again: the attempt is over now, not
                    // after two more turns like it.
                    if let TurnEnd::Error { retryable: false, message, .. } = &s.end {
                        fail(rn, key, token, &format!("the session's turn failed: {message}")).await;
                        return false;
                    }
                    handed = s.output.filter(|v| !v.is_null());
                    // A main session's turns are counted by its watch.
                    if let (false, Some(u)) = (turns.main, s.usage) {
                        let r = engine.account(&key.run, Some((&key.station, &key.task)), u).await;
                        settle_quiet(rn, &key.run, r).await;
                    }
                }
                Waited::Superseded => return true,
                Waited::Gone => {
                    if !rn.stopping() && standing(rn, key, token).await == Standing::Claimed {
                        fail(rn, key, token, "the session went away").await;
                    }
                    return false;
                }
            }
        }
        if rn.stopping() {
            return false;
        }
        // A structured output is the turn's hand-in, whichever way the session was told to hand
        // in: the dispatcher submits it under the task's claim.
        let mut problem: Option<String> = None;
        if let Some(output) = handed {
            let r = engine.submit(&key.run, &key.station, &key.task, token, output, dispatcher()).await;
            match r {
                Ok(o) => apply(rn, Some(&key.run), o).await,
                Err(FlowError::Contract(m)) => problem = Some(m),
                Err(e) => {
                    tracing::debug!(run = key.run, error = %e, "structured output not taken");
                    return false;
                }
            }
        }
        settle(rn, &key.run, "evaluate", engine.evaluate(&key.run).await).await;
        match standing(rn, key, token).await {
            Standing::Gone => return false,
            Standing::Submitted => {
                // Handed in, and judged: a station unmet on a check that failed (a `tool`
                // exit, say) is the output's to fix — the session is told what, and hands in
                // again. One waiting on a gate, a person or other tasks is not.
                if turns.delivery == Delivery::StructuredOutput && corrections < CORRECTIONS {
                    if let Some(missing) = failed_checks(rn, key).await {
                        corrections += 1;
                        let text = format!(
                            "The flow took your output, but station `{}` is not passed: {missing}. \
                             Fix the output and hand it in again with `{STRUCTURED_OUTPUT_TOOL}`.",
                            key.station
                        );
                        own.texts.push(text.clone());
                        if renew(rn, key, token).await && session.send(&text).await.is_ok() {
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
            if corrections < CORRECTIONS && renew(rn, key, token).await {
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
        if !renew(rn, key, token).await {
            return false;
        }
        if nudged {
            fail(rn, key, token, "the session ended a turn twice without handing the task in").await;
            return false;
        }
        nudged = true;
        let nudge = match turns.delivery {
            Delivery::FlowSubmit => NUDGE,
            Delivery::StructuredOutput => NUDGE_STRUCTURED,
        };
        own.texts.push(nudge.to_string());
        if let Err(e) = session.send(nudge).await {
            fail(rn, key, token, &format!("the reminder was not sent: {e}")).await;
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
async fn wait_own_turn<F>(rn: &Arc<Runner>, frames: &mut F, key: &TaskKey, token: &str, own: &mut OwnTurns) -> Waited
where
    F: Stream<Item = Frame> + Unpin + Send + ?Sized,
{
    if !own.main {
        return wait_turn(rn, frames, key, token).await;
    }
    let every = renew_every(rn);
    loop {
        let next = tokio::select! {
            next = tokio::time::timeout(every, frames.next()) => next,
            _ = rn.cancelled().cancelled() => return Waited::Superseded,
        };
        let frame = match next {
            Ok(Some(f)) => f,
            Ok(None) => return Waited::Gone,
            Err(_) => {
                if rn.stopping() || !renew(rn, key, token).await {
                    return Waited::Superseded;
                }
                continue;
            }
        };
        match frame {
            Frame::TurnStarted { turn, text } => {
                if let Some(i) = own.texts.iter().position(|x| *x == text) {
                    own.texts.remove(i);
                    own.turns.insert(turn);
                }
            }
            Frame::TurnSettled { turn, end, output, usage }
                if own.turns.remove(&turn) || std::mem::take(&mut own.any_next) =>
            {
                return Waited::Settled(Settled { end, output, usage });
            }
            _ => {}
        }
    }
}

/// The next settled turn, whoever started it, renewing the claim while it runs.
pub(crate) async fn wait_turn<F>(rn: &Arc<Runner>, frames: &mut F, key: &TaskKey, token: &str) -> Waited
where
    F: Stream<Item = Frame> + Unpin + Send + ?Sized,
{
    let every = renew_every(rn);
    loop {
        let next = tokio::select! {
            next = tokio::time::timeout(every, next_settled(frames)) => next,
            _ = rn.cancelled().cancelled() => return Waited::Superseded,
        };
        match next {
            Ok(Some(s)) => return Waited::Settled(s),
            Ok(None) => return Waited::Gone,
            Err(_) => {
                if rn.stopping() || !renew(rn, key, token).await {
                    return Waited::Superseded;
                }
            }
        }
    }
}

/// The next `TurnSettled`; `None` when the session is gone.
async fn next_settled<F>(frames: &mut F) -> Option<Settled>
where
    F: Stream<Item = Frame> + Unpin + Send + ?Sized,
{
    while let Some(frame) = frames.next().await {
        if let Frame::TurnSettled { end, output, usage, .. } = frame {
            return Some(Settled { end, output, usage });
        }
    }
    None
}

/// The station's failed checks that are this task's output to fix: `None` when nothing failed
/// that the session can fix (it waits for a gate, a person or other tasks, or it passed).
async fn failed_checks(rn: &Runner, key: &TaskKey) -> Option<String> {
    let v = rn.engine().get(&key.run).await.ok()?;
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
    let only = v.record.def.station(station).is_some_and(|s| matches!(s.work(), crate::Work::One(_)));
    let mine = format!("task {task}: ");
    let mut out = vec![];
    for c in v.unmet.get(station)?.iter().filter(|c| c.status == CheckStatus::Fail) {
        match c.kind.as_str() {
            "output" => {
                let parts: Vec<&str> = c.detail.split("; ").filter(|p| p.starts_with(&mine)).collect();
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
    use crate::runner::testing::{ScriptedHost, Turn};
    use crate::runner::OpenSpec;
    use crate::{CreateIn, Engine, ManualClock, MemPacks, MemStore, RunRecord};
    use serde_json::json;
    use std::pin::Pin;
    use std::sync::atomic::{AtomicI64, Ordering as AtomicOrdering};

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

    /// A run's one session task, claimed by the dispatcher; the runner around it and its
    /// scripted host, with the task's session open and watched.
    struct Task {
        rn: Arc<Runner>,
        host: Arc<ScriptedHost>,
        engine: Arc<Engine>,
        key: TaskKey,
        token: String,
        session: SessionId,
        frames: Pin<Box<dyn Stream<Item = Frame> + Send>>,
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
        fn sent(&self) -> Vec<String> {
            self.host.sent_to(&self.session)
        }
        fn host_session(&self) -> HostSession<'_> {
            HostSession { rn: &self.rn, host: self.host.as_ref(), id: &self.session, binding: None }
        }
        /// [`carry`], which must end: a loop left waiting is a failure, not a hung test (time is
        /// paused, so an hour of waiting passes at once).
        async fn carry(&mut self, turns: Turns) {
            let session = HostSession { rn: &self.rn, host: self.host.as_ref(), id: &self.session, binding: None };
            let carried = carry(&self.rn, &self.key, &self.token, turns, &session, &mut self.frames);
            tokio::time::timeout(Duration::from_secs(3600), carried).await.expect("the task is still being carried");
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

    /// The runner on `engine`, holding the claim `work` hands out, its session open on `host`.
    async fn module(engine: Arc<Engine>, host: Arc<ScriptedHost>, work: &WorkItem) -> Task {
        let WorkItem::Task { run, station, round, task, token, .. } = work else {
            panic!("not a task: {work:?}");
        };
        let rn = Runner::for_tests(engine.clone(), host.clone());
        let spec = OpenSpec::new(format!("flow:{run}/{station}/{task}@{token}"), run, "harnix", Delivery::StructuredOutput);
        let session = host.open(spec).await.unwrap().id;
        let frames = host.watch(&session).await.unwrap().frames;
        Task {
            rn,
            host,
            engine,
            key: TaskKey { run: run.clone(), station: station.clone(), round: *round, task: task.clone() },
            token: token.clone(),
            session,
            frames,
        }
    }

    async fn start(flow: &str, turns: Vec<Turn>) -> Task {
        start_on(Arc::new(MemStore::new()), flow, Arc::default(), turns).await
    }

    async fn start_on(store: Arc<dyn RunStore>, flow: &str, left: Arc<AtomicI64>, turns: Vec<Turn>) -> Task {
        let engine = engine_on(store, left);
        let o = engine
            .create(CreateIn { flow: flow.into(), subject: "s".into(), ..Default::default() }, Actor::device("d"))
            .await
            .unwrap();
        let host = ScriptedHost::new();
        host.station("a", turns);
        module(engine, host, &o.work[0]).await
    }

    fn turns(delivery: Delivery) -> Turns {
        Turns { main: false, delivery, message: Some("Do the task.".into()), resumed_running: false }
    }

    fn ends(output: Option<Value>) -> Turn {
        Turn::Ends { output }
    }

    fn settled(turn: &str, output: Option<Value>) -> Frame {
        Frame::TurnSettled { turn: turn.into(), end: TurnEnd::Done, output, usage: None }
    }

    #[tokio::test(start_paused = true)]
    async fn a_turn_that_hands_in_passes_its_station() {
        let mut t = start("f", vec![ends(Some(json!({"n": 1})))]).await;
        t.carry(turns(Delivery::StructuredOutput)).await;
        assert_eq!(t.sent(), ["Do the task."]);
        assert_eq!(t.station().await, StationStatus::Passed);
        assert_eq!(t.task().await.output, Some(json!({"n": 1})));
    }

    #[tokio::test(start_paused = true)]
    async fn a_turn_that_hands_in_nothing_is_nudged_once_and_a_second_fails_the_attempt() {
        for (delivery, nudge) in [(Delivery::StructuredOutput, NUDGE_STRUCTURED), (Delivery::FlowSubmit, NUDGE)] {
            let mut t = start("f", vec![ends(None), ends(None)]).await;
            t.carry(turns(delivery)).await;
            assert_eq!(t.sent(), ["Do the task.", nudge]);
            let task = t.task().await;
            assert_eq!(task.status, TaskStatus::Failed);
            assert_eq!(task.error.as_deref(), Some("the session ended a turn twice without handing the task in"));
        }
    }

    #[tokio::test(start_paused = true)]
    async fn a_nudged_session_that_hands_in_passes() {
        let mut t = start("f", vec![ends(None), ends(Some(json!({"n": 2})))]).await;
        t.carry(turns(Delivery::StructuredOutput)).await;
        assert_eq!(t.sent(), ["Do the task.", NUDGE_STRUCTURED]);
        assert_eq!(t.station().await, StationStatus::Passed);
    }

    #[tokio::test(start_paused = true)]
    async fn an_output_the_contract_refuses_goes_back_with_what_is_wrong() {
        let mut t = start("f", vec![ends(Some(json!({"m": 1}))), ends(Some(json!({"n": 1})))]).await;
        t.carry(turns(Delivery::StructuredOutput)).await;
        let sent = t.sent();
        assert_eq!(sent.len(), 2);
        assert!(sent[1].starts_with("The flow did not take your output: "), "{}", sent[1]);
        assert!(sent[1].contains('n') && sent[1].contains(STRUCTURED_OUTPUT_TOOL), "{}", sent[1]);
        assert_eq!(t.station().await, StationStatus::Passed);
    }

    /// Two corrections, then the nudge (a correction is not one), then the attempt fails.
    #[tokio::test(start_paused = true)]
    async fn corrections_run_out_then_the_nudge_then_the_attempt_fails() {
        let bad = || ends(Some(json!({"m": 1})));
        let mut t = start("f", vec![bad(), bad(), bad(), bad()]).await;
        t.carry(turns(Delivery::StructuredOutput)).await;
        let sent = t.sent();
        assert_eq!(sent.len(), 4, "{sent:#?}");
        assert!(sent[1].starts_with("The flow did not take your output") && sent[2].starts_with("The flow did not take your output"));
        assert_eq!(sent[3], NUDGE_STRUCTURED);
        assert_eq!(t.task().await.status, TaskStatus::Failed);
    }

    /// Taken, but its station is unmet on a check the output can fix: the session is told which.
    #[tokio::test(start_paused = true)]
    async fn an_output_taken_but_failing_its_stations_check_goes_back() {
        let left = Arc::new(AtomicI64::new(2));
        let mut t = start_on(
            Arc::new(MemStore::new()),
            "counted",
            left.clone(),
            vec![ends(Some(json!({"n": 1}))), ends(Some(json!({"n": 0})))],
        )
        .await;
        // The session fixes what it is told is wrong.
        t.host.on_send(move |_, text| {
            if text.starts_with("The flow took your output") {
                left.store(0, AtomicOrdering::SeqCst);
            }
        });
        t.carry(turns(Delivery::StructuredOutput)).await;
        let sent = t.sent();
        assert_eq!(sent.len(), 2, "{sent:#?}");
        assert!(sent[1].starts_with("The flow took your output, but station `a` is not passed: "), "{}", sent[1]);
        assert_eq!(t.station().await, StationStatus::Passed);
        assert_eq!(t.task().await.output, Some(json!({"n": 0})));
    }

    /// The run is cancelled while the task's turn runs: the claim is gone at the next renewal,
    /// and the turn is stopped and waited for before the session is anyone else's.
    #[tokio::test(start_paused = true)]
    async fn a_task_over_while_its_turn_runs_interrupts_that_turn() {
        let mut t = start("f", vec![Turn::Runs]).await;
        let (engine, run) = (t.engine.clone(), t.key.run.clone());
        let cancel = async {
            tokio::time::sleep(Duration::from_secs(1)).await;
            engine.cancel(&run, "no longer wanted", Actor::device("d")).await.unwrap();
        };
        tokio::join!(t.carry(turns(Delivery::StructuredOutput)), cancel);
        assert_eq!(t.host.interrupts().len(), 1);
        assert_eq!(t.record().await.status, RunStatus::Cancelled);
        assert!(!t.host.running(&t.session), "the interrupted turn settled");
    }

    /// A runner that stops only stops waiting: the turn is left to finish, the claim to lapse,
    /// and the next start takes the task up again.
    #[tokio::test(start_paused = true)]
    async fn stopping_the_runner_does_not_interrupt_the_turn() {
        let mut t = start("f", vec![Turn::Runs]).await;
        let rn = t.rn.clone();
        let stop = async {
            tokio::time::sleep(Duration::from_secs(1)).await;
            rn.cancelled().cancel();
        };
        tokio::join!(t.carry(turns(Delivery::StructuredOutput)), stop);
        assert!(t.host.interrupts().is_empty());
        assert_eq!(t.task().await.status, TaskStatus::Claimed);
    }

    #[tokio::test(start_paused = true)]
    async fn a_session_that_goes_away_fails_the_attempt() {
        let mut t = start("f", vec![Turn::Runs]).await;
        let (host, session) = (t.host.clone(), t.session.clone());
        let gone = async {
            tokio::time::sleep(Duration::from_secs(1)).await;
            host.close(&session).await.unwrap();
        };
        tokio::join!(t.carry(turns(Delivery::StructuredOutput)), gone);
        let task = t.task().await;
        assert_eq!(task.status, TaskStatus::Failed);
        assert_eq!(task.error.as_deref(), Some("the session went away"));
    }

    /// A turn that fails for good — no credentials — fails the attempt at once, with why; one
    /// that may go through next time (a rate limit) is nudged as a turn that handed in nothing.
    #[tokio::test(start_paused = true)]
    async fn a_turn_that_fails_for_good_fails_the_attempt_and_a_passing_failure_is_nudged() {
        let mut t = start("f", vec![Turn::Errors { retryable: false, message: "no credentials for acme".into() }]).await;
        t.carry(turns(Delivery::StructuredOutput)).await;
        let task = t.task().await;
        assert_eq!(task.status, TaskStatus::Failed);
        assert_eq!(task.error.as_deref(), Some("the session's turn failed: no credentials for acme"));
        assert_eq!(t.sent().len(), 1, "nothing more was sent");

        let mut t = start("f", vec![Turn::Errors { retryable: true, message: "429".into() }, ends(Some(json!({"n": 5})))]).await;
        t.carry(turns(Delivery::StructuredOutput)).await;
        assert_eq!(t.sent(), ["Do the task.", NUDGE_STRUCTURED]);
        assert_eq!(t.station().await, StationStatus::Passed);
    }

    /// The runner restarted: the work is rebuilt from the store under the same claim. A turn
    /// that was still running is the task's, whoever's message started it — in a session of the
    /// task's own and in the main session alike — and nothing is sent again.
    #[tokio::test(start_paused = true)]
    async fn after_a_restart_the_turn_still_running_is_the_tasks() {
        for main in [false, true] {
            let store = Arc::new(MemStore::new());
            let before = start_on(store.clone(), "f", Arc::default(), vec![]).await;
            let engine = engine_on(store, Arc::default());
            let rec = engine.record(&before.key.run).await.unwrap();
            let work = engine.dispatched_work(&rec, engine.clock().now_ms());
            assert_eq!(work.len(), 1, "the claim is still the dispatcher's");
            let host = ScriptedHost::new();
            let mut t = module(engine, host, &work[0]).await;
            assert_eq!(t.token, before.token);

            t.host.push(&t.session, settled("before-restart", Some(json!({"n": 3}))));
            let resumed = Turns { main, delivery: Delivery::StructuredOutput, message: None, resumed_running: true };
            t.carry(resumed).await;
            assert!(t.sent().is_empty(), "main: {main}");
            assert_eq!(t.station().await, StationStatus::Passed, "main: {main}");
        }
    }

    /// After a restart with the session idle, the turn was cut off: it is judged as one that
    /// ended without a hand-in — a nudge, not the task told again.
    #[tokio::test(start_paused = true)]
    async fn after_a_restart_an_idle_session_is_nudged_not_told_again() {
        let mut t = start("f", vec![ends(Some(json!({"n": 4})))]).await;
        let resumed = Turns { message: None, ..turns(Delivery::StructuredOutput) };
        t.carry(resumed).await;
        assert_eq!(t.sent(), [NUDGE_STRUCTURED]);
        assert_eq!(t.station().await, StationStatus::Passed);
    }

    /// In the main session a person talks too: a turn their message started, and what it hands
    /// in, is not the task's.
    #[tokio::test(start_paused = true)]
    async fn in_the_main_session_a_persons_turn_is_not_the_tasks() {
        let mut t = start("f", vec![ends(Some(json!({"n": "task"})))]).await;
        t.host.push(&t.session, Frame::TurnStarted { turn: "person".into(), text: "How is it going?".into() });
        t.host.push(&t.session, settled("person", Some(json!({"n": "person"}))));
        let main = Turns { main: true, ..turns(Delivery::FlowSubmit) };
        t.carry(main).await;
        assert_eq!(t.station().await, StationStatus::Passed);
        assert_eq!(t.task().await.output, Some(json!({"n": "task"})));
        let _ = t.host_session();
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
            RunView { record: rec, unmet: [("a".to_string(), checks)].into(), phase: None }
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
        let v = view(many, vec![check("output", "task x: missing n; task y: no output")]);
        assert_eq!(corrections_for(&v, "a", "x").unwrap(), "output (task x: missing n)");
        assert_eq!(corrections_for(&v, "a", "y").unwrap(), "output (task y: no output)");
        // A confirmation, another task failing: never the session's.
        let v = view(one, vec![check("task", "failed"), check("confirm", "")]);
        assert_eq!(corrections_for(&v, "a", "a"), None);
        // A gate judges the station: its only task's to fix, nobody's among several.
        assert!(corrections_for(&view(one, vec![check("gate", "total is 2")]), "a", "a").is_some());
        assert_eq!(corrections_for(&view(many, vec![check("gate", "total is 2")]), "a", "x"), None);
    }
}
