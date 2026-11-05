//! The engine: every operation on runs.
//!
//! One operation = load → compute → [`RunStore::commit`] against the revs it
//! loaded → events and work. On a conflict the whole computation is thrown
//! away and done again from a fresh load (a few times, then
//! [`FlowError::Conflict`]); nothing is returned before a commit succeeds.
//! Every change made on someone's behalf asks the [`Authorizer`] first.

use std::sync::Arc;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::def::{ExitCond, FlowDef};
use super::host::{
    AllowAll, Authorizer, Clock, GateResult, NoTools, Op, Outcome, Packs, RunStore, SystemClock,
    Tools,
};
use super::run::{
    Actor, CheckRec, CheckStatus, ConfirmRec, ForcedRec, History, ParentRef, RunQuery, RunRecord,
    RunStatus, RunSummary, RunView, StationStatus, TaskStatus,
};
use super::txn::{apply_inputs, check_output, new_record, new_token, Txn};
use super::validate::{validate, validate_yaml, Problem};
use super::{FlowError, Result};

/// The lease an auto-claimed task gets, unless [`Engine::with_lease_ms`] says otherwise.
pub const DEFAULT_LEASE_MS: u64 = 10 * 60 * 1000;
/// How often an operation is recomputed after a rev conflict before giving up.
pub const DEFAULT_RETRIES: usize = 5;
/// The longest lease anyone gets, whatever they ask for: a day.
pub const MAX_LEASE_MS: u64 = 24 * 60 * 60 * 1000;
/// How deep child runs may nest; a deeper spawn fails its task.
pub const MAX_SUBFLOW_DEPTH: usize = 8;
/// The most tasks a `foreach` station may fan out to; a longer list leaves it unmet.
pub const MAX_FOREACH_ITEMS: usize = 1000;

/// Starting a run.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CreateIn {
    pub flow: String,
    pub subject: String,
    #[serde(default)]
    pub inputs: Value,
    #[serde(default)]
    pub title: Option<String>,
    /// Recorded only. Children the kernel spawns for subflow tasks set it themselves.
    #[serde(default)]
    pub parent: Option<ParentRef>,
}

/// The kernel. Cheap to share behind an `Arc`; it holds no state of its own.
pub struct Engine {
    pub(crate) store: Arc<dyn RunStore>,
    pub(crate) packs: Arc<dyn Packs>,
    pub(crate) auth: Arc<dyn Authorizer>,
    pub(crate) tools: Arc<dyn Tools>,
    pub(crate) clock: Arc<dyn Clock>,
    pub(crate) lease_ms: u64,
    pub(crate) retries: usize,
    pub(crate) guarded: Vec<String>,
}

/// Run `$body` in a fresh transaction `$t`, commit, and retry the whole thing on a conflict.
macro_rules! transact {
    ($eng:expr, $by:expr, |$t:ident| $body:block) => {{
        let eng: &Engine = $eng;
        let by: Actor = $by;
        let mut tries = 0usize;
        loop {
            let mut $t = Txn::new(eng, by.clone());
            let value: Result<Value> = async { $body }.await;
            match $t.commit(value?).await {
                Ok(o) => break Ok(o),
                Err(FlowError::Conflict(m)) => {
                    tries += 1;
                    if tries >= eng.retries {
                        break Err(FlowError::Conflict(m));
                    }
                }
                Err(e) => break Err(e),
            }
        }
    }};
}

fn invalid<T>(m: impl Into<String>) -> Result<T> {
    Err(FlowError::Invalid(m.into()))
}

impl Engine {
    pub fn new(store: Arc<dyn RunStore>, packs: Arc<dyn Packs>) -> Self {
        Self {
            store,
            packs,
            auth: Arc::new(AllowAll),
            tools: Arc::new(NoTools),
            clock: Arc::new(SystemClock),
            lease_ms: DEFAULT_LEASE_MS,
            retries: DEFAULT_RETRIES,
            guarded: vec![],
        }
    }

    pub fn with_authorizer(mut self, a: Arc<dyn Authorizer>) -> Self {
        self.auth = a;
        self
    }

    pub fn with_tools(mut self, t: Arc<dyn Tools>) -> Self {
        self.tools = t;
        self
    }

    pub fn with_clock(mut self, c: Arc<dyn Clock>) -> Self {
        self.clock = c;
        self
    }

    /// The lease of tasks the kernel claims for its dispatcher.
    pub fn with_lease_ms(mut self, ms: u64) -> Self {
        self.lease_ms = ms.clamp(1, MAX_LEASE_MS);
        self
    }

    /// The tools the host refuses as writes unless a station lists them (`prefix*` matches a
    /// group): named in the protocol layer, so a session is told the truth about them.
    pub fn with_guarded(mut self, tools: Vec<String>) -> Self {
        self.guarded = tools;
        self
    }

    pub fn with_retries(mut self, n: usize) -> Self {
        self.retries = n.max(1);
        self
    }

    pub fn store(&self) -> &Arc<dyn RunStore> {
        &self.store
    }

    pub fn packs(&self) -> &Arc<dyn Packs> {
        &self.packs
    }

    pub fn clock(&self) -> &Arc<dyn Clock> {
        &self.clock
    }

    fn authorize(&self, op: &Op, by: &Actor, run: Option<&RunRecord>) -> Result<()> {
        self.auth.check(op, by, run).map_err(FlowError::Denied)
    }

    // ── reads ────────────────────────────────────────────────────────────────

    /// Every structural problem of a definition text.
    pub async fn validate(&self, yaml: &str) -> Vec<Problem> {
        validate_yaml(yaml, self.packs.as_ref())
    }

    /// A run with its pinned definition and, per open station, what is not met — as anyone may
    /// read it: lease tokens are removed.
    pub async fn get(&self, run: &str) -> Result<RunView> {
        Ok(view(self.record(run).await?))
    }

    /// The whole record, lease tokens included: for the host's own dispatcher, never for
    /// someone else to read.
    pub async fn record(&self, run: &str) -> Result<RunRecord> {
        self.store
            .load(run)
            .await?
            .ok_or_else(|| FlowError::NotFound(format!("run {run}")))
    }

    pub async fn list(&self, q: RunQuery) -> Result<Vec<RunSummary>> {
        Ok(self.store.list(&q).await?)
    }

    pub async fn history(&self, run: &str) -> Result<Vec<History>> {
        Ok(self.store.history(run).await?)
    }

    // ── runs ─────────────────────────────────────────────────────────────────

    /// Start a run of a pack's flow; the definition is validated and pinned.
    /// `value`: `{run}`.
    pub async fn create(&self, input: CreateIn, by: Actor) -> Result<Outcome> {
        self.authorize(
            &Op::Create {
                flow: input.flow.clone(),
                subject: input.subject.clone(),
            },
            &by,
            None,
        )?;
        let (def, digest) = self
            .packs
            .flow(&input.flow)
            .ok_or_else(|| FlowError::NotFound(format!("flow {}", input.flow)))?;
        check_def(&def, self.packs.as_ref())?;
        let inputs = apply_inputs(&def, &input.inputs).map_err(FlowError::Invalid)?;
        transact!(self, by.clone(), |t| {
            let id = self.store.alloc_id(&def.name).await?;
            let rec = new_record(
                id.clone(),
                def.clone(),
                digest.clone(),
                input.title.clone().unwrap_or_default(),
                input.subject.clone(),
                inputs.clone(),
                by.clone(),
                t.fx.now,
                input.parent.clone(),
            );
            t.insert_new(rec);
            t.m(&id).created();
            t.fx.advance.insert(id.clone());
            t.settle().await?;
            Ok(json!({"run": id}))
        })
    }

    /// Cancel a run, its in-flight tasks and its child runs.
    pub async fn cancel(&self, run: &str, reason: &str, by: Actor) -> Result<Outcome> {
        transact!(self, by.clone(), |t| {
            t.load(run).await?;
            if !t.get(run).is_active() {
                return invalid(format!("run {run} is not active"));
            }
            self.authorize(&Op::Cancel, &by, Some(t.get(run)))?;
            t.m(run).cancel_run(reason);
            t.settle().await?;
            Ok(json!({"run": run}))
        })
    }

    /// Judge again every open station whose tasks are settled (tool exits are re-called).
    pub async fn evaluate(&self, run: &str) -> Result<Outcome> {
        transact!(self, Actor::flow("evaluate"), |t| {
            t.load(run).await?;
            let rec = t.get(run);
            if !rec.is_active() {
                return Ok(json!({"run": run}));
            }
            let open: Vec<String> = rec
                .stations
                .iter()
                .filter(|(_, s)| matches!(s.status, StationStatus::Working | StationStatus::Unmet))
                .map(|(id, _)| id.clone())
                .collect();
            let mut m = t.m(run);
            for s in &open {
                m.judge_later(s);
            }
            m.advance_later();
            t.settle().await?;
            Ok(json!({"run": run}))
        })
    }

    /// Lease expiry over every active run whose earliest lease has run out. A run that fails
    /// is skipped (its error is in `Err` only when listing the runs fails); the others go on.
    pub async fn tick(&self) -> Result<Vec<Outcome>> {
        let now = self.clock.now_ms();
        let due: Vec<String> = self
            .store
            .list(&RunQuery {
                status: Some(RunStatus::Active),
                ..Default::default()
            })
            .await?
            .into_iter()
            .filter(|s| s.next_expiry.is_some_and(|u| u <= now))
            .map(|s| s.id)
            .collect();
        let mut out = vec![];
        for run in due {
            let o = transact!(self, Actor::flow("clock"), |t| {
                t.load(&run).await?;
                if t.get(&run).is_active() && t.m(&run).expire() {
                    t.fx.advance.insert(run.clone());
                    t.settle().await?;
                }
                Ok(json!({"run": run}))
            });
            if let Ok(o) = o {
                if !o.events.is_empty() {
                    out.push(o);
                }
            }
        }
        Ok(out)
    }

    // ── tasks ────────────────────────────────────────────────────────────────

    /// Claim an external task. `value`: `{token, until}`.
    pub async fn claim(
        &self,
        run: &str,
        station: &str,
        task: &str,
        by: Actor,
        lease_ms: u64,
    ) -> Result<Outcome> {
        transact!(self, by.clone(), |t| {
            t.load(run).await?;
            let rec = t.get(run);
            let (def, st) = open_station(rec, station)?;
            if st.status != StationStatus::Working {
                return invalid(format!("station {station} is {:?}, not working", st.status));
            }
            let spec = def
                .station(station)
                .and_then(|s| s.task_spec(task))
                .ok_or_else(|| FlowError::NotFound(format!("task {station}/{task}")))?;
            let who = match &spec.executor {
                super::def::Executor::External { who } => who.clone(),
                e => {
                    return invalid(format!(
                        "a {} task is dispatched by the flow, not claimed",
                        e.kind()
                    ))
                }
            };
            let tr = st
                .task(task)
                .ok_or_else(|| FlowError::NotFound(format!("task {station}/{task}")))?;
            if tr.status != TaskStatus::Ready {
                return Err(FlowError::Lease(format!(
                    "task {station}/{task} is {:?}, not ready",
                    tr.status
                )));
            }
            if let super::def::Work::Foreach {
                max_parallel: Some(m),
                ..
            } = def
                .station(station)
                .map(|s| s.work())
                .unwrap_or(super::def::Work::None)
            {
                let busy = st
                    .tasks
                    .iter()
                    .filter(|x| x.status == TaskStatus::Claimed)
                    .count();
                if busy >= m as usize {
                    return invalid(format!(
                        "station {station} already has {busy} tasks in flight (max_parallel {m})"
                    ));
                }
            }
            self.authorize(
                &Op::Claim {
                    station: station.into(),
                    task: task.into(),
                    who,
                },
                &by,
                Some(rec),
            )?;
            let token = new_token();
            let until = t.fx.now.saturating_add(lease_ms.clamp(1, MAX_LEASE_MS));
            let mut m = t.m(run);
            let attempt = {
                let Some(tr) = m.task_mut(station, task) else {
                    return invalid("task vanished");
                };
                tr.status = TaskStatus::Claimed;
                tr.attempt += 1;
                tr.lease = Some(super::run::Lease {
                    by: by.clone(),
                    token: token.clone(),
                    until,
                });
                tr.attempt
            };
            let data = json!({"by": by, "attempt": attempt, "until": until});
            m.emit("task.claimed", Some(station), Some(task), data.clone());
            m.log("claim", Some(station), Some(task), data);
            t.settle().await?;
            Ok(json!({"token": token, "until": until}))
        })
    }

    /// Extend a claim — or, after a submission, the window for submitting again. Only the
    /// current token, before its lease runs out, on a station still open. `value`: `{until}`.
    pub async fn heartbeat(
        &self,
        run: &str,
        station: &str,
        task: &str,
        token: &str,
        lease_ms: u64,
        by: Actor,
    ) -> Result<Outcome> {
        transact!(self, by.clone(), |t| {
            t.load(run).await?;
            let now = t.fx.now;
            let holder = {
                let rec = t.get(run);
                let (def, st) = open_station(rec, station)?;
                if !matches!(st.status, StationStatus::Working | StationStatus::Unmet) {
                    return invalid(format!("station {station} is {:?}", st.status));
                }
                let tr = held(st, station, task, token, now)?;
                let executor = task_executor(def, station, task)?;
                hand_in_by(&executor, &by, station, task)?;
                self.authorize(
                    &Op::Heartbeat {
                        station: station.into(),
                        task: task.into(),
                        executor: executor.kind().into(),
                    },
                    &by,
                    Some(rec),
                )?;
                tr.lease.as_ref().map(|l| l.by.clone())
            };
            let until = now.saturating_add(lease_ms.clamp(1, MAX_LEASE_MS));
            let mut m = t.m(run);
            if let Some(h) = holder {
                m.by = h;
            }
            if let Some(l) = m.task_mut(station, task).and_then(|x| x.lease.as_mut()) {
                l.until = until;
            }
            m.log(
                "heartbeat",
                Some(station),
                Some(task),
                json!({"until": until}),
            );
            Ok(json!({"until": until}))
        })
    }

    /// Hand in a task's output, checked against the station's contract. A task may hand in
    /// again under the same token until its station passes; the last output counts.
    pub async fn submit(
        &self,
        run: &str,
        station: &str,
        task: &str,
        token: &str,
        output: Value,
        by: Actor,
    ) -> Result<Outcome> {
        transact!(self, by.clone(), |t| {
            t.load(run).await?;
            let now = t.fx.now;
            let rec = t.get(run);
            let (def, st) = open_station(rec, station)?;
            if !matches!(st.status, StationStatus::Working | StationStatus::Unmet) {
                return invalid(format!("station {station} is {:?}", st.status));
            }
            held(st, station, task, token, now)?;
            let sdef = def.station(station).cloned();
            let executor = task_executor(def, station, task)?;
            hand_in_by(&executor, &by, station, task)?;
            let who = match &executor {
                super::def::Executor::External { who } => who.clone(),
                _ => None,
            };
            self.authorize(
                &Op::Submit {
                    station: station.into(),
                    task: task.into(),
                    executor: executor.kind().into(),
                    who,
                },
                &by,
                Some(rec),
            )?;
            check_output(sdef.and_then(|s| s.output).as_ref(), &output).map_err(|e| {
                FlowError::Contract(format!("output does not satisfy the contract: {e}"))
            })?;
            t.m(run).task_submitted(station, task, output.clone());
            t.settle().await?;
            Ok(json!({"run": run, "station": t.get(run).stations[station].status}))
        })
    }

    /// Report a failed attempt: dispatched again while retries remain, otherwise the task
    /// fails and its station becomes unmet.
    pub async fn fail(
        &self,
        run: &str,
        station: &str,
        task: &str,
        token: &str,
        error: &str,
        by: Actor,
    ) -> Result<Outcome> {
        transact!(self, by.clone(), |t| {
            t.load(run).await?;
            let now = t.fx.now;
            let rec = t.get(run);
            let (def, st) = open_station(rec, station)?;
            claimed(st, station, task, token, now)?;
            let executor = task_executor(def, station, task)?;
            hand_in_by(&executor, &by, station, task)?;
            self.authorize(
                &Op::Fail {
                    station: station.into(),
                    task: task.into(),
                    executor: executor.kind().into(),
                },
                &by,
                Some(rec),
            )?;
            t.m(run).task_failed(station, task, error.to_string());
            t.settle().await?;
            Ok(json!({"run": run}))
        })
    }

    /// Record the session serving a task (and its session key's binding). Module use.
    #[allow(clippy::too_many_arguments)]
    pub async fn attach_session(
        &self,
        run: &str,
        station: &str,
        task: &str,
        token: &str,
        session: &str,
        session_key: Option<&str>,
        by: Actor,
    ) -> Result<Outcome> {
        transact!(self, by.clone(), |t| {
            t.load(run).await?;
            let now = t.fx.now;
            let rec = t.get(run);
            let (_, st) = open_station(rec, station)?;
            claimed(st, station, task, token, now)?;
            self.authorize(
                &Op::AttachSession {
                    station: station.into(),
                    task: task.into(),
                    session: session.into(),
                },
                &by,
                Some(rec),
            )?;
            let mut m = t.m(run);
            if let Some(tr) = m.task_mut(station, task) {
                tr.session = Some(session.to_string());
            }
            if let Some(k) = session_key {
                m.rec
                    .session_keys
                    .insert(k.to_string(), session.to_string());
            }
            m.log(
                "attach_session",
                Some(station),
                Some(task),
                json!({"session": session}),
            );
            Ok(json!({"run": run}))
        })
    }

    /// Put a failed task back to ready with a fresh retry budget.
    pub async fn retry(&self, run: &str, station: &str, task: &str, by: Actor) -> Result<Outcome> {
        transact!(self, by.clone(), |t| {
            t.load(run).await?;
            let rec = t.get(run);
            let (_, st) = open_station(rec, station)?;
            if !matches!(st.status, StationStatus::Working | StationStatus::Unmet) {
                return invalid(format!("station {station} is {:?}", st.status));
            }
            let tr = st
                .task(task)
                .ok_or_else(|| FlowError::NotFound(format!("task {station}/{task}")))?;
            if tr.status != TaskStatus::Failed {
                return invalid(format!(
                    "task {station}/{task} is {:?}, not failed",
                    tr.status
                ));
            }
            self.authorize(
                &Op::Retry {
                    station: station.into(),
                    task: task.into(),
                },
                &by,
                Some(rec),
            )?;
            let mut m = t.m(run);
            if let Some(tr) = m.task_mut(station, task) {
                tr.status = TaskStatus::Ready;
                tr.attempt = 0;
                tr.lease = None;
            }
            if let Some(s) = m.rec.stations.get_mut(station) {
                if s.status == StationStatus::Unmet {
                    s.status = StationStatus::Working;
                }
            }
            m.emit("task.ready", Some(station), Some(task), Value::Null);
            m.log("retry", Some(station), Some(task), Value::Null);
            m.advance_later();
            t.settle().await?;
            Ok(json!({"run": run}))
        })
    }

    // ── stations ─────────────────────────────────────────────────────────────

    /// Confirm (`pass`) or reject a station with a `confirm` exit, once its tasks are in.
    pub async fn confirm(
        &self,
        run: &str,
        station: &str,
        pass: bool,
        reason: Option<&str>,
        by: Actor,
    ) -> Result<Outcome> {
        transact!(self, by.clone(), |t| {
            t.load(run).await?;
            let rec = t.get(run);
            let (def, st) = open_station(rec, station)?;
            let sdef = def.station(station).cloned();
            if !sdef
                .as_ref()
                .is_some_and(|s| s.exit.iter().any(|e| matches!(e, ExitCond::Confirm { .. })))
            {
                return invalid(format!("station {station} has no confirm exit"));
            }
            if !matches!(st.status, StationStatus::Working | StationStatus::Unmet)
                || !st.tasks_settled()
            {
                return invalid(format!(
                    "station {station} is not waiting for a confirmation"
                ));
            }
            self.authorize(
                &Op::Confirm {
                    station: station.into(),
                    pass,
                    who: sdef.and_then(|s| s.confirm_who()),
                },
                &by,
                Some(rec),
            )?;
            let now = t.fx.now;
            let mut m = t.m(run);
            let round = m.rec.stations[station].round;
            if let Some(s) = m.rec.stations.get_mut(station) {
                s.confirms.push(ConfirmRec {
                    by: by.clone(),
                    at: now,
                    round,
                    pass,
                    reason: reason.map(String::from),
                });
            }
            let data = json!({"pass": pass, "reason": reason, "round": round});
            m.emit("station.confirmed", Some(station), None, data.clone());
            m.log("confirm", Some(station), None, data);
            m.judge_later(station);
            t.settle().await?;
            Ok(json!({"run": run, "station": t.get(run).stations[station].status}))
        })
    }

    /// Pass a station whose conditions are not met. A reason is required; the conditions
    /// not met at the time are recorded with it.
    pub async fn force(
        &self,
        run: &str,
        station: &str,
        reason: &str,
        by: Actor,
    ) -> Result<Outcome> {
        transact!(self, by.clone(), |t| {
            if reason.trim().is_empty() {
                return invalid("forcing a station needs a reason");
            }
            t.load(run).await?;
            let rec = t.get(run);
            let (_, st) = open_station(rec, station)?;
            if !st.status.is_open() {
                return invalid(format!("station {station} is {:?}", st.status));
            }
            self.authorize(
                &Op::Force {
                    station: station.into(),
                },
                &by,
                Some(rec),
            )?;
            let view = view(rec.clone());
            let unmet = view.unmet.get(station).cloned().unwrap_or_default();
            let now = t.fx.now;
            let mut m = t.m(run);
            let ids: Vec<String> = m.rec.stations[station]
                .tasks
                .iter()
                .map(|x| x.id.clone())
                .collect();
            for id in ids {
                m.cancel_task(station, &id);
            }
            if let Some(s) = m.rec.stations.get_mut(station) {
                s.forced = Some(ForcedRec {
                    by: by.clone(),
                    at: now,
                    reason: reason.to_string(),
                    unmet: unmet.clone(),
                });
            }
            let data = json!({"reason": reason, "unmet": unmet});
            m.emit("station.forced", Some(station), None, data.clone());
            m.log("force", Some(station), None, data);
            m.pass(station);
            m.advance_later();
            t.settle().await?;
            Ok(json!({"run": run}))
        })
    }

    /// Rework to `to`: it starts a new round (only the named `tasks`, if given) and every
    /// station downstream of it goes stale. `to` must be open, or listed in the `rework_to`
    /// (or `on_unmet.rework_to`) of an open station.
    pub async fn rework(
        &self,
        run: &str,
        to: &str,
        tasks: Option<Vec<String>>,
        reason: &str,
        by: Actor,
    ) -> Result<Outcome> {
        transact!(self, by.clone(), |t| {
            t.load(run).await?;
            let rec = t.get(run);
            if !rec.is_active() {
                return invalid(format!("run {run} is not active"));
            }
            let def = &rec.def;
            let target = rec
                .stations
                .get(to)
                .filter(|_| def.station(to).is_some())
                .ok_or_else(|| FlowError::NotFound(format!("station {to}")))?;
            if !target.status.is_open() {
                let listed = rec.open_stations().iter().any(|o| {
                    def.station(o).is_some_and(|s| {
                        s.rework_to.iter().any(|r| r == to)
                            || s.on_unmet.as_ref().is_some_and(|u| u.rework_to == to)
                    })
                });
                if !listed {
                    return invalid(format!("no open station may rework to {to}"));
                }
            }
            if let Some(names) = &tasks {
                if names.is_empty() {
                    return invalid("a partial rework names at least one task");
                }
                if let Some(n) = names.iter().find(|n| target.task(n).is_none()) {
                    return Err(FlowError::NotFound(format!("task {to}/{n}")));
                }
            }
            self.authorize(
                &Op::Rework {
                    to: to.into(),
                    tasks: tasks.clone(),
                },
                &by,
                Some(rec),
            )?;
            let mut m = t.m(run);
            m.can_rework(to).map_err(FlowError::Invalid)?;
            m.rework(to, tasks.as_deref(), reason);
            m.advance_later();
            t.settle().await?;
            Ok(json!({"run": run}))
        })
    }

    /// Release a held station: its tasks are dispatched.
    pub async fn release(&self, run: &str, station: &str, by: Actor) -> Result<Outcome> {
        transact!(self, by.clone(), |t| {
            t.load(run).await?;
            let rec = t.get(run);
            let (_, st) = open_station(rec, station)?;
            if st.status != StationStatus::Held {
                return invalid(format!("station {station} is not held"));
            }
            self.authorize(
                &Op::Release {
                    station: station.into(),
                },
                &by,
                Some(rec),
            )?;
            let mut m = t.m(run);
            let empty = {
                let s = m.rec.stations.get_mut(station).expect("checked");
                s.status = StationStatus::Working;
                s.tasks.is_empty()
            };
            m.emit("station.released", Some(station), None, Value::Null);
            m.log("release", Some(station), None, Value::Null);
            if empty {
                m.judge_later(station);
            }
            m.dispatch(station);
            m.advance_later();
            t.settle().await?;
            Ok(json!({"run": run}))
        })
    }

    /// A gate's result for one round. A result for a round that has moved on, or for a
    /// gate nobody is waiting for, is ignored (`value.applied` = false).
    pub async fn gate_result(
        &self,
        run: &str,
        station: &str,
        round: u32,
        gate: &str,
        result: GateResult,
    ) -> Result<Outcome> {
        transact!(self, Actor::flow("gate"), |t| {
            t.load(run).await?;
            let rec = t.get(run);
            let Some(sdef) = rec.def.station(station).cloned() else {
                return Err(FlowError::NotFound(format!("station {station}")));
            };
            let open = rec.stations.get(station).is_some_and(|s| {
                s.round == round
                    && matches!(s.status, StationStatus::Working | StationStatus::Unmet)
            });
            if !rec.is_active() || !open {
                return Ok(json!({"applied": false}));
            }
            let now = t.fx.now;
            let mut m = t.m(run);
            let mut applied = false;
            if let Some(s) = m.rec.stations.get_mut(station) {
                for c in s.checks.iter_mut() {
                    let is_gate = c
                        .cond
                        .and_then(|i| sdef.exit.get(i))
                        .is_some_and(|e| matches!(e, ExitCond::Gate { gate: g, .. } if g == gate));
                    if is_gate && c.round == round && c.status == CheckStatus::Wait {
                        c.status = if result.pass {
                            CheckStatus::Pass
                        } else {
                            CheckStatus::Fail
                        };
                        c.detail = result.detail.clone();
                        c.at = now;
                        applied = true;
                    }
                }
            }
            if applied {
                m.log(
                    "gate_result",
                    Some(station),
                    None,
                    json!({"gate": gate, "round": round, "pass": result.pass, "detail": result.detail}),
                );
                m.judge_later(station);
                t.settle().await?;
            }
            Ok(json!({"applied": applied}))
        })
    }
}

/// A definition that fails validation cannot start a run.
fn check_def(def: &FlowDef, packs: &dyn Packs) -> Result<()> {
    let problems = validate(def, packs);
    if problems.is_empty() {
        return Ok(());
    }
    invalid(format!(
        "flow {} is not valid: {}",
        def.name,
        problems
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("; ")
    ))
}

/// The run is active and the station exists.
fn open_station<'r>(
    rec: &'r RunRecord,
    station: &str,
) -> Result<(&'r FlowDef, &'r super::run::StationRec)> {
    if !rec.is_active() {
        return invalid(format!("run {} is not active", rec.id));
    }
    match (rec.def.station(station), rec.stations.get(station)) {
        (Some(_), Some(s)) => Ok((&rec.def, s)),
        _ => Err(FlowError::NotFound(format!("station {station}"))),
    }
}

/// The executor of one task of a station.
fn task_executor(def: &FlowDef, station: &str, task: &str) -> Result<super::def::Executor> {
    def.station(station)
        .and_then(|s| s.task_spec(task))
        .map(|s| s.executor)
        .ok_or_else(|| FlowError::NotFound(format!("task {station}/{task}")))
}

/// Who may hand in, fail or renew a task. An external task: whoever claimed it (the token
/// says so). A session task: the kernel's dispatcher or the session itself
/// ([`Actor::agent`]); a program or subflow task: the dispatcher alone. Nobody else may,
/// even holding the token.
fn hand_in_by(
    executor: &super::def::Executor,
    by: &Actor,
    station: &str,
    task: &str,
) -> Result<()> {
    use super::def::Executor as E;
    use super::run::{AGENT_KIND, FLOW_KIND};
    let ok = match executor {
        E::External { .. } => true,
        E::Session { .. } => by.kind == FLOW_KIND || by.kind == AGENT_KIND,
        E::Program { .. } | E::Subflow { .. } => by.kind == FLOW_KIND,
    };
    if ok {
        Ok(())
    } else {
        Err(FlowError::Denied(format!(
            "{station}/{task} is a {} task: the flow hands it in, not {by}",
            executor.kind()
        )))
    }
}

/// The task's current claim is `token` and has not run out: claimed, or submitted under it
/// (a submission may be made again while the claim lasts).
fn held<'r>(
    st: &'r super::run::StationRec,
    station: &str,
    task: &str,
    token: &str,
    now: u64,
) -> Result<&'r super::run::TaskRec> {
    let tr = st
        .task(task)
        .ok_or_else(|| FlowError::NotFound(format!("task {station}/{task}")))?;
    let Some(lease) = tr
        .lease
        .as_ref()
        .filter(|_| matches!(tr.status, TaskStatus::Claimed | TaskStatus::Submitted))
    else {
        return Err(FlowError::Lease(format!(
            "task {station}/{task} is {:?}, not claimed",
            tr.status
        )));
    };
    if lease.token != token {
        return Err(FlowError::Lease(format!(
            "token is not the current one for {station}/{task}"
        )));
    }
    if lease.until <= now {
        return Err(FlowError::Lease(format!(
            "the lease on {station}/{task} has expired"
        )));
    }
    Ok(tr)
}

/// The task is claimed under `token` and its lease has not run out.
fn claimed<'r>(
    st: &'r super::run::StationRec,
    station: &str,
    task: &str,
    token: &str,
    now: u64,
) -> Result<&'r super::run::TaskRec> {
    let tr = st
        .task(task)
        .ok_or_else(|| FlowError::NotFound(format!("task {station}/{task}")))?;
    let Some(lease) = tr
        .lease
        .as_ref()
        .filter(|_| tr.status == TaskStatus::Claimed)
    else {
        return Err(FlowError::Lease(format!(
            "task {station}/{task} is not claimed"
        )));
    };
    if lease.token != token {
        return Err(FlowError::Lease(format!(
            "token is not the current one for {station}/{task}"
        )));
    }
    if lease.until <= now {
        return Err(FlowError::Lease(format!(
            "the lease on {station}/{task} has expired"
        )));
    }
    Ok(tr)
}

/// The record plus what each open station is still missing, lease tokens removed.
pub fn view(mut record: RunRecord) -> RunView {
    for s in record.stations.values_mut() {
        for t in &mut s.tasks {
            if let Some(l) = t.lease.as_mut() {
                l.token.clear();
            }
        }
    }
    let mut unmet = std::collections::BTreeMap::new();
    for (id, s) in &record.stations {
        if !s.status.is_open() {
            continue;
        }
        let mut list: Vec<CheckRec> = s
            .checks
            .iter()
            .filter(|c| c.status != CheckStatus::Pass)
            .cloned()
            .collect();
        if s.status == StationStatus::Held {
            list.push(CheckRec {
                cond: None,
                kind: "hold".into(),
                label: "held".into(),
                round: s.round,
                status: CheckStatus::Wait,
                detail: "waiting for a release".into(),
                at: s.entered_at.unwrap_or(0),
            });
        } else if !s.tasks_settled() {
            let done = s
                .tasks
                .iter()
                .filter(|t| t.status == TaskStatus::Submitted)
                .count();
            list.push(CheckRec {
                cond: None,
                kind: "tasks".into(),
                label: "tasks".into(),
                round: s.round,
                status: CheckStatus::Wait,
                detail: format!("{done} of {} tasks submitted", s.tasks.len()),
                at: s.entered_at.unwrap_or(0),
            });
        }
        unmet.insert(id.clone(), list);
    }
    RunView { record, unmet }
}
