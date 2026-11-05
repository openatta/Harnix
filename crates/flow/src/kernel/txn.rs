//! One attempt at an operation: the runs it loaded, what it changed, and the
//! events, work and history it produced. Nothing leaves a transaction until
//! its commit succeeds; on a conflict the engine drops it and starts over.
//!
//! [`M`] is the state machine of one run: entering stations, dispatching tasks,
//! judging exits, rework, finishing. What reaches across runs (spawning a
//! child, a child finishing, cancelling a child) is queued as [`Cross`] and
//! carried out by [`Txn::settle`], so a parent and its children change in the
//! same commit.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use serde_json::{json, Value};

use super::def::{Executor, ExitCond, FlowDef, SessionBinding, TaskSpec, Work, MAIN_SESSION_KEY};
use super::engine::{Engine, MAX_FOREACH_ITEMS, MAX_LEASE_MS, MAX_SUBFLOW_DEPTH};
use super::host::{ChangeSet, Delivery, Event, Outcome, Placement, WorkItem};
use super::protocol;
use super::refs::{asserts_fail, context, resolve, resolve_str, station_output};
use super::run::{
    Actor, CheckRec, CheckStatus, History, Lease, ParentRef, RunRecord, RunStatus, StationRec,
    StationStatus, TaskRec, TaskStatus,
};
use super::validate::validate;
use super::{FlowError, Result};

/// Leases of subflow tasks: held until the child finishes.
const FOREVER: u64 = u64::MAX;

pub(crate) enum Cross {
    Spawn {
        parent: ParentRef,
        token: String,
        flow: String,
        subject: String,
        inputs: Value,
    },
    ChildDone {
        child: String,
        parent: ParentRef,
        result: std::result::Result<Value, String>,
    },
    Cancel {
        run: String,
        reason: String,
    },
}

/// Everything an operation produced besides the records themselves.
pub(crate) struct Fx {
    pub now: u64,
    pub events: Vec<Event>,
    pub work: Vec<WorkItem>,
    pub history: Vec<History>,
    /// (run, station) whose exit should be judged once its tasks are settled.
    pub judge: BTreeSet<(String, String)>,
    pub advance: BTreeSet<String>,
    pub cross: VecDeque<Cross>,
}

struct Slot {
    rec: RunRecord,
    /// As loaded; `None` for a run created in this transaction.
    orig: Option<RunRecord>,
}

pub(crate) struct Txn<'e> {
    pub eng: &'e Engine,
    pub by: Actor,
    slots: BTreeMap<String, Slot>,
    pub fx: Fx,
}

impl<'e> Txn<'e> {
    pub fn new(eng: &'e Engine, by: Actor) -> Self {
        Self {
            eng,
            by,
            slots: BTreeMap::new(),
            fx: Fx {
                now: eng.clock.now_ms(),
                events: vec![],
                work: vec![],
                history: vec![],
                judge: BTreeSet::new(),
                advance: BTreeSet::new(),
                cross: VecDeque::new(),
            },
        }
    }

    pub async fn load(&mut self, id: &str) -> Result<()> {
        if self.slots.contains_key(id) {
            return Ok(());
        }
        let rec = self
            .eng
            .store
            .load(id)
            .await?
            .ok_or_else(|| FlowError::NotFound(format!("run {id}")))?;
        self.slots.insert(
            id.to_string(),
            Slot {
                orig: Some(rec.clone()),
                rec,
            },
        );
        Ok(())
    }

    pub fn get(&self, id: &str) -> &RunRecord {
        &self.slots[id].rec
    }

    pub fn insert_new(&mut self, rec: RunRecord) {
        self.slots.insert(rec.id.clone(), Slot { rec, orig: None });
    }

    /// The state machine of a loaded run, acting as the operation's actor.
    pub fn m(&mut self, id: &str) -> M<'_> {
        let by = self.by.clone();
        let slot = self.slots.get_mut(id).expect("run loaded");
        M::new(&mut slot.rec, &mut self.fx, self.eng, by)
    }

    /// Advance every run that asked for it and carry out cross-run work, until quiet.
    pub async fn settle(&mut self) -> Result<()> {
        // A bound against a definition that would otherwise spin; never reached in practice.
        for _ in 0..10_000 {
            if let Some(x) = self.fx.cross.pop_front() {
                self.cross(x).await?;
                continue;
            }
            let Some(id) = self.fx.advance.pop_first() else {
                return Ok(());
            };
            if self.slots.contains_key(&id) {
                self.m(&id).advance().await;
            }
        }
        Err(FlowError::Invalid("the run did not settle".into()))
    }

    async fn cross(&mut self, x: Cross) -> Result<()> {
        match x {
            Cross::Spawn {
                parent,
                token,
                flow,
                subject,
                inputs,
            } => self.spawn(parent, token, flow, subject, inputs).await,
            Cross::ChildDone {
                child,
                parent,
                result,
            } => {
                self.load(&parent.run).await?;
                let waiting = self
                    .get(&parent.run)
                    .station(&parent.station)
                    .and_then(|s| s.task(&parent.task))
                    .is_some_and(|t| {
                        t.status == TaskStatus::Claimed
                            && t.child.as_deref() == Some(child.as_str())
                    });
                if waiting {
                    let mut m = self.m(&parent.run);
                    m.by = Actor::flow("dispatch");
                    match result {
                        Ok(out) => m.task_submitted(&parent.station, &parent.task, out),
                        Err(e) => m.task_failed(&parent.station, &parent.task, e),
                    }
                    self.fx.advance.insert(parent.run);
                }
                Ok(())
            }
            Cross::Cancel { run, reason } => {
                self.load(&run).await?;
                if self.get(&run).is_active() {
                    self.m(&run).cancel_run(&reason);
                }
                Ok(())
            }
        }
    }

    async fn spawn(
        &mut self,
        parent: ParentRef,
        token: String,
        flow: String,
        subject: String,
        inputs: Value,
    ) -> Result<()> {
        self.load(&parent.run).await?;
        let waiting = self
            .get(&parent.run)
            .station(&parent.station)
            .and_then(|s| s.task(&parent.task))
            .is_some_and(|t| {
                t.status == TaskStatus::Claimed
                    && t.lease.as_ref().is_some_and(|l| l.token == token)
            });
        if !waiting {
            return Ok(());
        }
        // How deep the parent already is.
        let mut depth = 1;
        let mut up = self.get(&parent.run).parent.clone();
        while let Some(p) = up {
            depth += 1;
            if depth > MAX_SUBFLOW_DEPTH {
                break;
            }
            self.load(&p.run).await?;
            up = self.get(&p.run).parent.clone();
        }
        let made = match self.eng.packs.flow(&flow) {
            _ if depth > MAX_SUBFLOW_DEPTH => Err(format!(
                "child runs nest deeper than {MAX_SUBFLOW_DEPTH}: {flow} is not started"
            )),
            None => Err(format!("no flow {flow}")),
            Some((def, digest)) => {
                let problems = validate(&def, self.eng.packs.as_ref());
                if !problems.is_empty() {
                    Err(format!(
                        "flow {flow} is not valid: {}",
                        problems
                            .iter()
                            .map(ToString::to_string)
                            .collect::<Vec<_>>()
                            .join("; ")
                    ))
                } else {
                    match apply_inputs(&def, &inputs) {
                        Err(e) => Err(e),
                        Ok(inputs) => {
                            let id = self.eng.store.alloc_id(&flow).await?;
                            let title = format!("{} / {}", parent.run, parent.task);
                            Ok(new_record(
                                id,
                                def,
                                digest,
                                title,
                                subject,
                                inputs,
                                Actor::flow("dispatch"),
                                self.fx.now,
                                Some(parent.clone()),
                            ))
                        }
                    }
                }
            }
        };
        match made {
            Ok(child) => {
                let id = child.id.clone();
                {
                    let mut m = self.m(&parent.run);
                    if let Some(t) = m.task_mut(&parent.station, &parent.task) {
                        t.child = Some(id.clone());
                    }
                    let detail = json!({"child": id});
                    m.log("spawn", Some(&parent.station), Some(&parent.task), detail);
                }
                self.insert_new(child);
                let mut m = self.m(&id);
                m.by = Actor::flow("dispatch");
                m.created();
                self.fx.advance.insert(id);
            }
            Err(e) => {
                let mut m = self.m(&parent.run);
                m.by = Actor::flow("dispatch");
                m.task_failed(&parent.station, &parent.task, e);
                self.fx.advance.insert(parent.run);
            }
        }
        Ok(())
    }

    /// Commit every changed record with its history; only then hand out events and work.
    pub async fn commit(self, value: Value) -> Result<Outcome> {
        let mut cs = ChangeSet::default();
        for (_, mut slot) in self.slots {
            if slot.orig.as_ref() == Some(&slot.rec) {
                continue;
            }
            let expected = slot.orig.as_ref().map(|o| o.rev);
            slot.rec.rev = expected.map_or(1, |r| r + 1);
            cs.records.push((slot.rec, expected));
        }
        cs.history = self.fx.history;
        if !cs.records.is_empty() {
            self.eng.store.commit(&cs).await?;
        }
        Ok(Outcome {
            value,
            events: self.fx.events,
            work: self.fx.work,
        })
    }
}

/// A new run record, every station pending.
#[allow(clippy::too_many_arguments)]
pub(crate) fn new_record(
    id: String,
    def: FlowDef,
    digest: String,
    title: String,
    subject: String,
    inputs: Value,
    by: Actor,
    now: u64,
    parent: Option<ParentRef>,
) -> RunRecord {
    RunRecord {
        id,
        flow: def.name.clone(),
        def_digest: digest,
        title,
        subject,
        inputs,
        status: RunStatus::Active,
        rev: 0,
        seq: 0,
        created_at: now,
        created_by: by,
        finished_at: None,
        parent,
        stations: def
            .stations
            .iter()
            .map(|s| (s.id.clone(), StationRec::new()))
            .collect(),
        session_keys: BTreeMap::new(),
        paused: None,
        usage_other: Default::default(),
        def,
    }
}

/// Check given inputs against the definition and fill in defaults.
pub(crate) fn apply_inputs(def: &FlowDef, given: &Value) -> std::result::Result<Value, String> {
    let empty = serde_json::Map::new();
    let given = match given {
        Value::Null => &empty,
        Value::Object(o) => o,
        _ => return Err("inputs must be an object".into()),
    };
    if let Some(k) = given.keys().find(|k| !def.inputs.contains_key(*k)) {
        return Err(format!("unknown input {k}"));
    }
    let mut out = serde_json::Map::new();
    for (k, d) in &def.inputs {
        let v = match given.get(k).or(d.default.as_ref()) {
            Some(v) => v.clone(),
            None => return Err(format!("input {k} is required")),
        };
        let valid = jsonschema::validator_for(&d.schema)
            .map_err(|e| format!("input {k}: bad schema: {e}"))?
            .is_valid(&v);
        if !valid {
            return Err(format!("input {k}: {v} does not satisfy its schema"));
        }
        out.insert(k.clone(), v);
    }
    Ok(Value::Object(out))
}

/// Check an output against a station's contract; every violation listed.
pub(crate) fn check_output(
    schema: Option<&Value>,
    output: &Value,
) -> std::result::Result<(), String> {
    let Some(schema) = schema else {
        return Ok(());
    };
    let v = jsonschema::validator_for(schema)
        .map_err(|e| format!("the output contract is not valid: {e}"))?;
    let errs: Vec<String> = v
        .iter_errors(output)
        .map(|e| {
            let at = e.instance_path().to_string();
            if at.is_empty() {
                e.to_string()
            } else {
                format!("{at}: {e}")
            }
        })
        .collect();
    if errs.is_empty() {
        Ok(())
    } else {
        Err(errs.join("; "))
    }
}

/// Task ids for foreach items: the items themselves (strings, or an object's `id`/`name`)
/// when they are all present and distinct, otherwise positions `0`, `1`, ….
fn item_ids(items: &[Value]) -> Vec<String> {
    let key = |v: &Value| match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Object(o) => o.get("id").or_else(|| o.get("name")).and_then(|x| {
            x.as_str()
                .map(String::from)
                .or_else(|| x.as_u64().map(|n| n.to_string()))
        }),
        _ => None,
    };
    let keys: Option<Vec<String>> = items.iter().map(key).collect();
    if let Some(keys) = keys {
        let distinct: BTreeSet<&String> = keys.iter().collect();
        if distinct.len() == keys.len() && keys.iter().all(|k| !k.is_empty()) {
            return keys;
        }
    }
    (0..items.len()).map(|i| i.to_string()).collect()
}

/// Why a `foreach` list did not resolve.
fn foreach_problem(items: &str, got: &Value) -> String {
    match got {
        Value::Array(list) => format!(
            "{items} has {} items; a station fans out to at most {MAX_FOREACH_ITEMS}",
            list.len()
        ),
        other => format!("{items} is not a list: {other}"),
    }
}

pub(crate) fn new_token() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

/// The state machine of one run.
pub(crate) struct M<'a> {
    pub rec: &'a mut RunRecord,
    pub fx: &'a mut Fx,
    pub eng: &'a Engine,
    pub by: Actor,
    def: FlowDef,
}

impl<'a> M<'a> {
    fn new(rec: &'a mut RunRecord, fx: &'a mut Fx, eng: &'a Engine, by: Actor) -> Self {
        let def = rec.def.clone();
        Self {
            rec,
            fx,
            eng,
            by,
            def,
        }
    }

    // ── bookkeeping ─────────────────────────────────────────────────────────

    fn now(&self) -> u64 {
        self.fx.now
    }

    pub fn emit(&mut self, kind: &str, station: Option<&str>, task: Option<&str>, data: Value) {
        let ev = Event {
            kind: kind.to_string(),
            run: self.rec.id.clone(),
            at: self.fx.now,
            station: station.map(String::from),
            task: task.map(String::from),
            data,
        };
        let station_hooks = station
            .and_then(|s| self.def.station(s))
            .map(|s| s.hooks.as_slice())
            .unwrap_or(&[]);
        for h in self.def.hooks.iter().chain(station_hooks) {
            if h.on == kind {
                self.fx.work.push(WorkItem::Hook {
                    action: h.action.clone(),
                    event: ev.clone(),
                });
            }
        }
        self.fx.events.push(ev);
    }

    pub fn log(&mut self, op: &str, station: Option<&str>, task: Option<&str>, detail: Value) {
        self.rec.seq += 1;
        self.fx.history.push(History {
            run: self.rec.id.clone(),
            seq: self.rec.seq,
            at: self.fx.now,
            by: self.by.clone(),
            op: op.to_string(),
            station: station.map(String::from),
            task: task.map(String::from),
            detail,
        });
    }

    pub fn judge_later(&mut self, sid: &str) {
        self.fx.judge.insert((self.rec.id.clone(), sid.to_string()));
        self.fx.advance.insert(self.rec.id.clone());
    }

    pub fn advance_later(&mut self) {
        self.fx.advance.insert(self.rec.id.clone());
    }

    fn st(&mut self, sid: &str) -> &mut StationRec {
        self.rec.stations.entry(sid.to_string()).or_default()
    }

    pub fn status(&self, sid: &str) -> StationStatus {
        self.rec
            .stations
            .get(sid)
            .map_or(StationStatus::Pending, |s| s.status)
    }

    pub fn task_mut(&mut self, sid: &str, tid: &str) -> Option<&mut TaskRec> {
        self.rec.stations.get_mut(sid)?.task_mut(tid)
    }

    fn spec(&self, sid: &str, tid: &str) -> Option<TaskSpec> {
        self.def.station(sid)?.task_spec(tid)
    }

    pub fn created(&mut self) {
        let data =
            json!({"flow": self.rec.flow, "subject": self.rec.subject, "parent": self.rec.parent});
        self.emit("run.created", None, None, data.clone());
        self.log("create", None, None, data);
    }

    // ── progress ────────────────────────────────────────────────────────────

    /// Enter what can be entered, dispatch what can be dispatched, judge what asked to be
    /// judged; repeat until nothing moves. Finishes the run when every station is done.
    pub async fn advance(&mut self) {
        let ids: Vec<String> = self.def.stations.iter().map(|s| s.id.clone()).collect();
        for _ in 0..1_000 {
            if !self.rec.is_active() {
                return;
            }
            let mut moved = false;
            for sid in &ids {
                match self.status(sid) {
                    // Paused: nothing new is entered (xops-03 §3).
                    StationStatus::Pending | StationStatus::Stale if self.rec.paused.is_none() => {
                        if self
                            .def
                            .needs_of(sid)
                            .iter()
                            .all(|n| self.status(n).is_done())
                        {
                            self.enter(sid, None);
                            moved = true;
                        }
                    }
                    StationStatus::Working => {
                        self.dispatch(sid);
                        if self.take_judge(sid) {
                            moved |= self.judge(sid).await;
                        }
                    }
                    StationStatus::Unmet if self.take_judge(sid) => {
                        moved |= self.judge(sid).await;
                    }
                    _ => {}
                }
            }
            if ids.iter().all(|s| self.status(s).is_done()) {
                self.finish();
                return;
            }
            if !moved {
                return;
            }
        }
    }

    fn take_judge(&mut self, sid: &str) -> bool {
        let settled = self
            .rec
            .stations
            .get(sid)
            .is_some_and(StationRec::tasks_settled);
        settled
            && self
                .fx
                .judge
                .remove(&(self.rec.id.clone(), sid.to_string()))
    }

    /// Enter a station for a new round. `partial` (a rework of named tasks) keeps the other
    /// tasks and their outputs, and does not re-check `when`.
    pub fn enter(&mut self, sid: &str, partial: Option<&[String]>) {
        let Some(st) = self.def.station(sid).cloned() else {
            return;
        };
        let now = self.now();
        let ctx = context(self.rec, None, None);
        let skip = if partial.is_none() {
            asserts_fail(&ctx, &st.when)
        } else {
            None
        };
        let r = self.st(sid);
        r.round += 1;
        r.entered_at = Some(now);
        r.left_at = None;
        r.checks.clear();
        r.forced = None;
        let round = r.round;
        if let Some(why) = skip {
            r.status = StationStatus::Skipped;
            r.left_at = Some(now);
            r.tasks.clear();
            let data = json!({"round": round, "reason": why});
            self.emit("station.skipped", Some(sid), None, data.clone());
            self.log("skip", Some(sid), None, data);
            return;
        }
        r.rounds_total += 1;

        let work = st.work();
        let specs: Vec<TaskSpec> = match &work {
            Work::None => vec![],
            Work::One(t) | Work::Foreach { task: t, .. } => vec![t.clone()],
            Work::Tasks(ts) => ts.clone(),
        };
        for skill in specs.iter().filter_map(|t| t.skill.as_ref()) {
            if let Some((_, digest)) = self.eng.packs.skill(skill) {
                self.st(sid).skill_digest.insert(skill.clone(), digest);
            }
        }

        let fresh: Vec<String> = match partial {
            Some(names) => {
                let r = self.st(sid);
                for n in names {
                    if let Some(t) = r.task_mut(n) {
                        *t = TaskRec::new(n.clone(), t.item.take());
                    }
                }
                names.to_vec()
            }
            None => {
                let tasks = match &work {
                    Work::None => vec![],
                    Work::One(_) => vec![TaskRec::new(sid, None)],
                    Work::Tasks(ts) => ts
                        .iter()
                        .map(|t| TaskRec::new(t.id.clone(), None))
                        .collect(),
                    Work::Foreach { items, .. } => match resolve_str(items, &ctx) {
                        Value::Array(list) if list.len() <= MAX_FOREACH_ITEMS => item_ids(&list)
                            .into_iter()
                            .zip(list)
                            .map(|(id, v)| TaskRec::new(id, Some(v)))
                            .collect(),
                        other => {
                            let r = self.st(sid);
                            r.tasks.clear();
                            r.status = StationStatus::Unmet;
                            r.checks = vec![CheckRec {
                                cond: None,
                                kind: "foreach".into(),
                                label: "foreach".into(),
                                round,
                                status: CheckStatus::Fail,
                                detail: foreach_problem(items, &other),
                                at: now,
                            }];
                            self.emit("station.entered", Some(sid), None, json!({"round": round}));
                            self.emit("station.unmet", Some(sid), None, json!({"round": round}));
                            self.log(
                                "enter",
                                Some(sid),
                                None,
                                json!({"round": round, "error": "foreach"}),
                            );
                            return;
                        }
                    },
                };
                let ids = tasks.iter().map(|t| t.id.clone()).collect();
                self.st(sid).tasks = tasks;
                ids
            }
        };

        let r = self.st(sid);
        r.status = if st.hold {
            StationStatus::Held
        } else {
            StationStatus::Working
        };
        let empty = r.tasks.is_empty();
        let data = json!({"round": round, "tasks": fresh});
        self.emit("station.entered", Some(sid), None, data.clone());
        self.log("enter", Some(sid), None, data);
        if st.hold {
            self.emit("station.held", Some(sid), None, json!({"round": round}));
        }
        for t in &fresh {
            self.emit("task.ready", Some(sid), Some(t), Value::Null);
        }
        if !st.hold {
            if empty {
                self.judge_later(sid);
            }
            self.dispatch(sid);
        }
    }

    /// Claim ready session and program tasks for the dispatcher and spawn subflow children,
    /// within `max_parallel`. External tasks stay ready.
    pub fn dispatch(&mut self, sid: &str) {
        if self.rec.paused.is_some() {
            return;
        }
        let Some(st) = self.def.station(sid).cloned() else {
            return;
        };
        let Some(r) = self.rec.stations.get(sid) else {
            return;
        };
        if r.status != StationStatus::Working {
            return;
        }
        let max = match st.work() {
            Work::Foreach {
                max_parallel: Some(m),
                ..
            } => m as usize,
            _ => usize::MAX,
        };
        let mut busy = r
            .tasks
            .iter()
            .filter(|t| t.status == TaskStatus::Claimed)
            .count();
        let ready: Vec<String> = r
            .tasks
            .iter()
            .filter(|t| t.status == TaskStatus::Ready)
            .map(|t| t.id.clone())
            .collect();
        for tid in ready {
            if busy >= max {
                break;
            }
            let Some(spec) = st.task_spec(&tid) else {
                continue;
            };
            match &spec.executor {
                Executor::External { .. } => continue,
                Executor::Session { .. } | Executor::Agent { .. } | Executor::Program { .. } => {
                    self.claim_push(sid, &tid, &spec)
                }
                Executor::Subflow { flow, inputs } => self.claim_subflow(sid, &tid, flow, inputs),
            }
            busy += 1;
        }
    }

    /// Auto-claim a push task under a fresh token and hand it out as work.
    pub fn claim_push(&mut self, sid: &str, tid: &str, spec: &TaskSpec) {
        let now = self.now();
        let lease_ms = self.eng.lease_ms;
        let token = new_token();
        let dispatcher = Actor::flow("dispatch");
        let (round, item, attempt, digest) = {
            let r = self.st(sid);
            let round = r.round;
            let digest = spec
                .skill
                .as_ref()
                .and_then(|s| r.skill_digest.get(s).cloned());
            let Some(t) = r.task_mut(tid) else { return };
            t.status = TaskStatus::Claimed;
            t.attempt += 1;
            t.started_at = Some(now);
            t.ended_at = None;
            t.lease = Some(Lease {
                by: dispatcher.clone(),
                token: token.clone(),
                until: now.saturating_add(lease_ms.min(MAX_LEASE_MS)),
            });
            (round, t.item.clone(), t.attempt, digest)
        };
        let work = task_work(
            self.rec,
            &self.eng.guarded,
            &self.eng.submit_scenes,
            self.eng.main_scene.as_deref(),
            sid,
            tid,
            spec,
            token,
            round,
            attempt,
            item,
            digest,
        );
        self.fx.work.push(work);
        let data = json!({"by": dispatcher, "attempt": attempt});
        self.emit("task.claimed", Some(sid), Some(tid), data.clone());
        let by = std::mem::replace(&mut self.by, dispatcher);
        self.log("claim", Some(sid), Some(tid), data);
        self.by = by;
    }

    fn claim_subflow(&mut self, sid: &str, tid: &str, flow: &str, inputs: &Value) {
        let token = new_token();
        let dispatcher = Actor::flow("dispatch");
        let now = self.now();
        let item = {
            let Some(t) = self.task_mut(sid, tid) else {
                return;
            };
            t.status = TaskStatus::Claimed;
            t.attempt += 1;
            t.started_at = Some(now);
            t.ended_at = None;
            t.child = None;
            t.lease = Some(Lease {
                by: dispatcher.clone(),
                token: token.clone(),
                until: FOREVER,
            });
            t.item.clone()
        };
        let ctx = context(self.rec, item.as_ref(), Some(tid));
        let subject = match &item {
            Some(Value::String(s)) => s.clone(),
            Some(v) => v.to_string(),
            None => self.rec.subject.clone(),
        };
        self.fx.cross.push_back(Cross::Spawn {
            parent: ParentRef {
                run: self.rec.id.clone(),
                station: sid.to_string(),
                task: tid.to_string(),
            },
            token,
            flow: flow.to_string(),
            subject,
            inputs: resolve(inputs, &ctx),
        });
        self.emit(
            "task.claimed",
            Some(sid),
            Some(tid),
            json!({"by": dispatcher}),
        );
    }

    /// A task handed in its output (already checked against the contract).
    pub fn task_submitted(&mut self, sid: &str, tid: &str, output: Value) {
        let now = self.now();
        if let Some(t) = self.task_mut(sid, tid) {
            t.ended_at = Some(now);
            t.status = TaskStatus::Submitted;
            t.output = Some(output);
            t.error = None;
        }
        self.emit("task.submitted", Some(sid), Some(tid), Value::Null);
        self.log("submit", Some(sid), Some(tid), Value::Null);
        self.judge_later(sid);
    }

    /// A task's attempt failed: back to ready while attempts remain, otherwise failed.
    pub fn task_failed(&mut self, sid: &str, tid: &str, error: String) {
        let attempts = self.spec(sid, tid).map_or(1, |s| s.attempts());
        let now = self.now();
        let retrying = {
            let Some(t) = self.task_mut(sid, tid) else {
                return;
            };
            t.ended_at = Some(now);
            t.error = Some(error.clone());
            t.lease = None;
            let retrying = t.attempt < attempts;
            t.status = if retrying {
                TaskStatus::Ready
            } else {
                TaskStatus::Failed
            };
            retrying
        };
        let data = json!({"error": error, "retrying": retrying});
        self.emit("task.failed", Some(sid), Some(tid), data.clone());
        self.log("fail", Some(sid), Some(tid), data);
        if retrying {
            self.emit("task.ready", Some(sid), Some(tid), Value::Null);
            self.advance_later();
        } else {
            self.judge_later(sid);
        }
    }

    /// Cancel a task that is still to be done; close its session, cancel its child run.
    pub fn cancel_task(&mut self, sid: &str, tid: &str) {
        let binding_new = self
            .spec(sid, tid)
            .and_then(|s| s.executor.binding())
            .is_some_and(|b| b.is_new());
        let (session, child) = {
            let Some(t) = self.task_mut(sid, tid) else {
                return;
            };
            if !t.in_flight() {
                return;
            }
            t.status = TaskStatus::Cancelled;
            t.lease = None;
            (t.session.clone(), t.child.clone())
        };
        if let (Some(s), true) = (session, binding_new) {
            self.fx.work.push(WorkItem::CloseSession { session: s });
        }
        if let Some(c) = child {
            self.fx.cross.push_back(Cross::Cancel {
                run: c,
                reason: format!("task {tid} of {} was cancelled", self.rec.id),
            });
        }
        self.emit("task.cancelled", Some(sid), Some(tid), Value::Null);
    }

    fn cancel_in_flight(&mut self, sid: &str) {
        let ids: Vec<String> = self
            .rec
            .stations
            .get(sid)
            .map(|r| {
                r.tasks
                    .iter()
                    .filter(|t| t.in_flight())
                    .map(|t| t.id.clone())
                    .collect()
            })
            .unwrap_or_default();
        for t in ids {
            self.cancel_task(sid, &t);
        }
    }

    // ── judgement ───────────────────────────────────────────────────────────

    /// Judge a station whose tasks are settled. Returns whether its status changed.
    pub async fn judge(&mut self, sid: &str) -> bool {
        let Some(st) = self.def.station(sid).cloned() else {
            return false;
        };
        let now = self.now();
        let (old, round) = {
            let r = self.st(sid);
            (r.status, r.round)
        };
        let mut checks = vec![];
        let r = self.rec.stations[sid].clone();
        // A fan-out whose list never resolved has no tasks, and an `output` exit over no tasks
        // would pass: the station stays unmet until a rework enters it again.
        if let Work::Foreach { items, .. } = st.work() {
            if r.tasks.is_empty() {
                match resolve_str(&items, &context(self.rec, None, None)) {
                    Value::Array(list) if list.len() <= MAX_FOREACH_ITEMS => {}
                    other => checks.push(CheckRec {
                        cond: None,
                        kind: "foreach".into(),
                        label: "foreach".into(),
                        round,
                        status: CheckStatus::Fail,
                        detail: foreach_problem(&items, &other),
                        at: now,
                    }),
                }
            }
        }
        for t in &r.tasks {
            let detail = match t.status {
                TaskStatus::Failed => t.error.clone().unwrap_or_else(|| "failed".into()),
                TaskStatus::Cancelled => "cancelled".into(),
                _ => continue,
            };
            checks.push(CheckRec {
                cond: None,
                kind: "task".into(),
                label: format!("task {}", t.id),
                round,
                status: CheckStatus::Fail,
                detail,
                at: now,
            });
        }
        for (i, c) in st.exit.iter().enumerate() {
            use super::def::ExitCond as C;
            let mut at = now;
            let (status, detail) = match c {
                C::Output { .. } => {
                    let mut bad = vec![];
                    for t in &r.tasks {
                        match (&t.status, &t.output) {
                            (TaskStatus::Submitted, Some(o)) => {
                                if let Err(e) = check_output(st.output.as_ref(), o) {
                                    bad.push(format!("task {}: {e}", t.id));
                                }
                            }
                            _ => bad.push(format!("task {}: no output", t.id)),
                        }
                    }
                    if bad.is_empty() {
                        (CheckStatus::Pass, String::new())
                    } else {
                        (CheckStatus::Fail, bad.join("; "))
                    }
                }
                C::Tool {
                    tool, args, assert, ..
                } => {
                    let args = if args.is_null() {
                        json!({})
                    } else {
                        args.clone()
                    };
                    let args = resolve(&args, &context(self.rec, None, None));
                    match self.eng.tools.call(tool, &args).await {
                        Ok(v) => match asserts_fail(&v, assert) {
                            None => (CheckStatus::Pass, String::new()),
                            Some(d) => (CheckStatus::Fail, d),
                        },
                        Err(e) => (CheckStatus::Fail, format!("{tool} failed: {e}")),
                    }
                }
                C::Gate { gate, .. } => match r
                    .checks
                    .iter()
                    .find(|k| k.cond == Some(i) && k.round == round)
                {
                    Some(k) => {
                        at = k.at;
                        (k.status, k.detail.clone())
                    }
                    None => {
                        self.fx.work.push(WorkItem::Gate {
                            run: self.rec.id.clone(),
                            station: sid.to_string(),
                            round,
                            gate: gate.clone(),
                        });
                        (CheckStatus::Wait, "requested".into())
                    }
                },
                C::Confirm { .. } => match r.confirms.iter().rev().find(|c| c.round == round) {
                    Some(c) if c.pass => (CheckStatus::Pass, String::new()),
                    Some(c) => (
                        CheckStatus::Fail,
                        format!(
                            "rejected by {}: {}",
                            c.by,
                            c.reason.clone().unwrap_or_default()
                        ),
                    ),
                    None => (CheckStatus::Wait, "awaiting confirmation".into()),
                },
                C::Children { .. } => {
                    let open: Vec<&str> = r
                        .tasks
                        .iter()
                        .filter(|t| t.status != TaskStatus::Submitted)
                        .map(|t| t.id.as_str())
                        .collect();
                    if open.is_empty() {
                        (CheckStatus::Pass, String::new())
                    } else {
                        (
                            CheckStatus::Fail,
                            format!("children not finished: {}", open.join(", ")),
                        )
                    }
                }
            };
            checks.push(CheckRec {
                cond: Some(i),
                kind: c.kind().into(),
                label: c.label(),
                round,
                status,
                detail,
                at,
            });
        }
        let fail = checks.iter().any(|c| c.status == CheckStatus::Fail);
        let wait = checks.iter().any(|c| c.status == CheckStatus::Wait);
        self.st(sid).checks = checks;
        if fail {
            self.st(sid).status = StationStatus::Unmet;
            // Re-judging a station that is already unmet (an `evaluate`, a resubmission)
            // reports nothing new and does not rework again.
            if old == StationStatus::Unmet {
                return false;
            }
            let unmet: Vec<CheckRec> = self.rec.stations[sid]
                .checks
                .iter()
                .filter(|c| c.status == CheckStatus::Fail)
                .cloned()
                .collect();
            let data = json!({"round": round, "unmet": unmet});
            self.emit("station.unmet", Some(sid), None, data.clone());
            self.log("unmet", Some(sid), None, data);
            self.on_unmet(sid);
            true
        } else if wait {
            self.st(sid).status = StationStatus::Working;
            old != StationStatus::Working
        } else {
            self.pass(sid);
            true
        }
    }

    pub fn pass(&mut self, sid: &str) {
        let now = self.now();
        let r = self.st(sid);
        r.status = StationStatus::Passed;
        r.left_at = Some(now);
        let data = json!({"round": r.round, "forced": r.forced.is_some()});
        self.emit("station.passed", Some(sid), None, data.clone());
        self.log("pass", Some(sid), None, data);
    }

    /// `on_unmet`: rework automatically while under `max` and the rework limit.
    fn on_unmet(&mut self, sid: &str) -> bool {
        let Some(u) = self.def.station(sid).and_then(|s| s.on_unmet.clone()) else {
            return false;
        };
        if self.st(sid).auto_reworks >= u.max || self.can_rework(&u.rework_to).is_err() {
            return false;
        }
        self.st(sid).auto_reworks += 1;
        let by = std::mem::replace(&mut self.by, Actor::flow("auto"));
        self.rework(&u.rework_to, None, &format!("on_unmet of {sid}"));
        self.by = by;
        true
    }

    // ── rework ──────────────────────────────────────────────────────────────

    /// The target has run and is under the rework limit.
    pub fn can_rework(&self, to: &str) -> std::result::Result<(), String> {
        let Some(r) = self.rec.stations.get(to) else {
            return Err(format!("no station {to}"));
        };
        if matches!(r.status, StationStatus::Pending | StationStatus::Stale) {
            return Err(format!("station {to} has not run"));
        }
        let limit = self.def.rework_limit();
        if r.rounds_total >= limit {
            return Err(format!(
                "station {to} has run {} rounds; the limit is {limit}",
                r.rounds_total
            ));
        }
        Ok(())
    }

    /// Rework to `to`: its descendants go stale (their tasks cancelled), unrelated branches
    /// are kept, and `to` starts a new round — all of it, or only the `partial` tasks.
    pub fn rework(&mut self, to: &str, partial: Option<&[String]>, reason: &str) {
        let now = self.now();
        let mut swept = vec![];
        for d in self.def.descendants(to) {
            if matches!(
                self.status(&d),
                StationStatus::Pending | StationStatus::Stale
            ) {
                continue;
            }
            self.cancel_in_flight(&d);
            let r = self.st(&d);
            r.status = StationStatus::Stale;
            r.left_at = Some(now);
            let round = r.round;
            self.emit("station.stale", Some(&d), None, json!({"round": round}));
            swept.push(d);
        }
        match partial {
            Some(names) => {
                for n in names {
                    self.cancel_task(to, n);
                }
            }
            None => self.cancel_in_flight(to),
        }
        let data = json!({"reason": reason, "tasks": partial, "stale": swept});
        self.emit("station.reworked", Some(to), None, data.clone());
        self.log("rework", Some(to), None, data);
        self.enter(to, partial);
    }

    // ── run level ───────────────────────────────────────────────────────────

    fn finish(&mut self) {
        let now = self.now();
        self.rec.status = RunStatus::Finished;
        self.rec.finished_at = Some(now);
        self.close_keyed_sessions();
        self.emit("run.finished", None, None, Value::Null);
        self.log("finish", None, None, Value::Null);
        if let Some(p) = self.rec.parent.clone() {
            let stations: serde_json::Map<String, Value> = self
                .def
                .stations
                .iter()
                .map(|s| (s.id.clone(), station_output(&self.def, self.rec, &s.id)))
                .collect();
            self.fx.cross.push_back(Cross::ChildDone {
                child: self.rec.id.clone(),
                parent: p,
                result: Ok(json!({"run": self.rec.id, "stations": stations})),
            });
        }
    }

    pub fn cancel_run(&mut self, reason: &str) {
        let now = self.now();
        let ids: Vec<String> = self.def.stations.iter().map(|s| s.id.clone()).collect();
        for sid in &ids {
            self.cancel_in_flight(sid);
        }
        self.rec.status = RunStatus::Cancelled;
        self.rec.finished_at = Some(now);
        self.close_keyed_sessions();
        let data = json!({"reason": reason});
        self.emit("run.cancelled", None, None, data.clone());
        self.log("cancel", None, None, data);
        if let Some(p) = self.rec.parent.clone() {
            self.fx.cross.push_back(Cross::ChildDone {
                child: self.rec.id.clone(),
                parent: p,
                result: Err(format!("child run {} was cancelled: {reason}", self.rec.id)),
            });
        }
    }

    /// The sessions shared by key (`per_run`, `{key}`) outlive their tasks: closed with the run.
    fn close_keyed_sessions(&mut self) {
        let sessions: BTreeSet<String> = self.rec.session_keys.values().cloned().collect();
        for session in sessions {
            self.fx.work.push(WorkItem::CloseSession { session });
        }
    }

    /// Leases that have run out. A push task's attempt has failed: it is dispatched again
    /// while its retries last (a session of its own is closed first), otherwise it fails.
    /// An external task goes back to ready. Returns whether anything expired.
    pub fn expire(&mut self) -> bool {
        let now = self.now();
        let mut expired = vec![];
        for (sid, r) in &self.rec.stations {
            for t in &r.tasks {
                if t.status == TaskStatus::Claimed
                    && t.lease.as_ref().is_some_and(|l| l.until <= now)
                {
                    expired.push((sid.clone(), t.id.clone()));
                }
            }
        }
        for (sid, tid) in &expired {
            let Some(spec) = self.spec(sid, tid) else {
                continue;
            };
            self.emit("task.expired", Some(sid), Some(tid), Value::Null);
            self.log("expire", Some(sid), Some(tid), Value::Null);
            if spec.executor.is_push() {
                let own = spec.executor.binding().is_some_and(|b| b.is_new());
                let session = self.task_mut(sid, tid).and_then(|t| t.session.take());
                if let (true, Some(session)) = (own, session) {
                    self.fx.work.push(WorkItem::CloseSession { session });
                }
                self.task_failed(sid, tid, "the lease ran out".into());
            } else if let Some(t) = self.task_mut(sid, tid) {
                t.status = TaskStatus::Ready;
                t.lease = None;
                self.emit("task.ready", Some(sid), Some(tid), Value::Null);
            }
        }
        !expired.is_empty()
    }
}

/// The work item of a push task claimed under `token`: what [`M::claim_push`] hands out, and
/// what [`dispatched_work`] rebuilds after a restart.
#[allow(clippy::too_many_arguments)]
fn task_work(
    rec: &RunRecord,
    guarded: &[String],
    submit_scenes: &BTreeSet<String>,
    main_scene: Option<&str>,
    sid: &str,
    tid: &str,
    spec: &TaskSpec,
    token: String,
    round: u32,
    attempt: u32,
    item: Option<Value>,
    digest: Option<String>,
) -> WorkItem {
    let ctx = context(rec, item.as_ref(), Some(tid));
    let resolve_params = |params: &BTreeMap<String, String>| -> BTreeMap<String, String> {
        params
            .iter()
            .map(|(k, v)| {
                let r = match resolve_str(v, &ctx) {
                    Value::String(s) => s,
                    Value::Null => String::new(),
                    v => v.to_string(),
                };
                (k.clone(), r)
            })
            .collect()
    };
    let session_key = match &spec.executor {
        Executor::Agent { .. } => Some(MAIN_SESSION_KEY.to_string()),
        e @ Executor::Session { .. } => match e.binding().unwrap_or(SessionBinding::New) {
            SessionBinding::Main => Some(MAIN_SESSION_KEY.to_string()),
            SessionBinding::New => None,
            SessionBinding::PerRun => Some("run".to_string()),
            SessionBinding::Key(k) => Some(match resolve_str(&k, &ctx) {
                Value::String(s) => s,
                v => v.to_string(),
            }),
        },
        _ => None,
    };
    let session = session_key
        .as_ref()
        .and_then(|k| rec.session_keys.get(k).cloned());
    let st = rec.def.station(sid);
    let (placement, delivery, scene) = match &spec.executor {
        e if e.in_main() => {
            let (placement, scene) = main_placement(rec, main_scene, &resolve_params);
            (Some(placement), Some(Delivery::FlowSubmit), scene)
        }
        Executor::Session {
            scene,
            model,
            provider,
            instance,
            template,
            params,
            space,
            ..
        } => (
            Some(Placement {
                model: model.clone(),
                provider: provider.clone(),
                instance: instance.clone(),
                template: template.clone(),
                params: resolve_params(params),
                space: space.clone(),
            }),
            Some(if submit_scenes.contains(scene) {
                Delivery::FlowSubmit
            } else {
                Delivery::StructuredOutput
            }),
            Some(scene.clone()),
        ),
        _ => (None, None, None),
    };
    let protocol = match (&spec.executor, st) {
        (Executor::Session { .. } | Executor::Agent { .. }, Some(st)) => Some(protocol::render(&protocol::TaskContext {
            def: &rec.def,
            station: st,
            run: &rec.id,
            subject: &rec.subject,
            round,
            task: tid,
            item: item.as_ref(),
            guarded,
            delivery: delivery.unwrap_or(Delivery::StructuredOutput),
            executor: Some(&spec.executor),
        })),
        _ => None,
    };
    WorkItem::Task {
        run: rec.id.clone(),
        station: sid.to_string(),
        round,
        task: tid.to_string(),
        token,
        attempt,
        executor: spec.executor.clone(),
        skill: spec.skill.clone(),
        skill_digest: digest,
        item,
        output_schema: st.and_then(|s| s.output.clone()),
        protocol,
        session_key,
        session,
        placement,
        delivery,
        scene,
    }
}

/// [`main_placement`] with `params` resolved against the run (no item, no task).
pub(crate) fn main_of(rec: &RunRecord, main_scene: Option<&str>) -> (Placement, Option<String>) {
    let ctx = context(rec, None, None);
    let resolve_params = |params: &BTreeMap<String, String>| -> BTreeMap<String, String> {
        params
            .iter()
            .map(|(k, v)| {
                let r = match resolve_str(v, &ctx) {
                    Value::String(s) => s,
                    Value::Null => String::new(),
                    v => v.to_string(),
                };
                (k.clone(), r)
            })
            .collect()
    };
    main_placement(rec, main_scene, &resolve_params)
}

/// Where the run's main session goes and its scene: the flow's `main`, its `params` resolved,
/// and the module's main scene when it names none (`None`: there is none).
pub(crate) fn main_placement(
    rec: &RunRecord,
    main_scene: Option<&str>,
    resolve_params: &dyn Fn(&BTreeMap<String, String>) -> BTreeMap<String, String>,
) -> (Placement, Option<String>) {
    let m = rec.def.main.clone().unwrap_or_default();
    (
        Placement {
            model: m.model,
            provider: m.provider,
            instance: m.instance,
            template: m.template,
            params: resolve_params(&m.params),
            space: m.space,
        },
        m.scene.or_else(|| main_scene.map(String::from)),
    )
}

/// The work handed out for a run and not answered yet, as it was handed out — for a module that
/// restarted and must carry on with what it had been given: every session and program task the
/// dispatcher holds a live claim on (`session` is the session the task reported with
/// `attach_session`, else its key's), and every gate of the current round still waiting for its
/// result.
///
/// Every session task is taken to hand in by structured output here; [`Engine::dispatched_work`]
/// knows which scenes hand in with `flow_submit`.
pub fn dispatched_work(rec: &RunRecord, guarded: &[String], now: u64) -> Vec<WorkItem> {
    dispatched_work_with(rec, guarded, &BTreeSet::new(), None, now)
}

pub(crate) fn dispatched_work_with(
    rec: &RunRecord,
    guarded: &[String],
    submit_scenes: &BTreeSet<String>,
    main_scene: Option<&str>,
    now: u64,
) -> Vec<WorkItem> {
    let dispatcher = Actor::flow("dispatch");
    let mut out = vec![];
    if !rec.is_active() {
        return out;
    }
    for (sid, st) in &rec.stations {
        let Some(sdef) = rec.def.station(sid) else {
            continue;
        };
        if matches!(st.status, StationStatus::Working | StationStatus::Unmet) {
            for c in &st.checks {
                let gate = c.cond.and_then(|i| sdef.exit.get(i));
                if let (Some(ExitCond::Gate { gate, .. }), CheckStatus::Wait, true) =
                    (gate, c.status, c.round == st.round)
                {
                    out.push(WorkItem::Gate {
                        run: rec.id.clone(),
                        station: sid.clone(),
                        round: st.round,
                        gate: gate.clone(),
                    });
                }
            }
        }
        if st.status != StationStatus::Working {
            continue;
        }
        for t in &st.tasks {
            let Some(lease) = t.lease.as_ref() else {
                continue;
            };
            if t.status != TaskStatus::Claimed || lease.by != dispatcher || lease.until <= now {
                continue;
            }
            let Some(spec) = sdef.task_spec(&t.id) else {
                continue;
            };
            if !spec.executor.is_push() {
                continue;
            }
            let digest = spec
                .skill
                .as_ref()
                .and_then(|s| st.skill_digest.get(s).cloned());
            let mut w = task_work(
                rec,
                guarded,
                submit_scenes,
                main_scene,
                sid,
                &t.id,
                &spec,
                lease.token.clone(),
                st.round,
                t.attempt,
                t.item.clone(),
                digest,
            );
            if let (WorkItem::Task { session, .. }, Some(own)) = (&mut w, &t.session) {
                *session = Some(own.clone());
            }
            out.push(w);
        }
    }
    out
}
