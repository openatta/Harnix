//! Dry runs (xops-02 §5): the kernel, for real, in a sandbox where everything outside it is
//! mocked. No session is opened, none of the application's programs, gates or tools is called,
//! no model is paid for.
//!
//! Each dry run has an [`Engine`] of its own over a [`MemStore`]: its run ids start `dry-`, its
//! child runs live beside it, and dropping it (the oldest beyond [`KEEP`]) drops them all. Work
//! the kernel hands out is queued, not carried out; [`Sandbox::step`] takes the next item,
//! [`Sandbox::run`] takes items until none is left — the run is over, or waits for a person:
//!
//! | work | in a dry run |
//! |---|---|
//! | a session or program task | handed in by the dispatcher: `mocks.stations.<id>` (one value, or a table by task id for a station of several tasks), else the contract's `examples` / `default`, else [`minimal`] of the contract; a session task first records the message its session would have been sent (`preview`) |
//! | a gate | passes, unless `mocks.gates.<gate>` is `false` |
//! | a `tool` exit | answers `mocks.tools.<tool>`, `{}` when not given |
//! | a confirmation, an external task | waits for a person, through the same methods as a real run |
//! | a subflow | a child run in the sandbox |
//!
//! A definition that names programs, gates, tools or actions the application does not have
//! still runs here: the sandbox's catalog takes in every name the pack's flows use, since none of
//! them is called.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use crate::kernel::def::{Executor, ExitCond, FlowDef, Work};
use crate::kernel::engine::{view, CreateIn, Engine};
use crate::kernel::host::{
    Catalog, ChangeSet, GateResult, Outcome, Packs, RunStore, StoreError, Tools, WorkItem,
};
use crate::kernel::protocol::task_message;
use crate::kernel::run::{Actor, History, RunQuery, RunRecord, RunStatus, RunSummary};
use crate::kernel::schema::minimal;
use crate::kernel::{FlowError, Result};
use crate::store::MemStore;

/// The prefix of every dry run's id (and its children's). No flow may be named with it
/// (validation), so a real run's id never has it.
pub const PREFIX: &str = crate::kernel::validate::DRY_PREFIX;
/// How many dry runs are kept; starting one more drops the oldest.
pub const KEEP: usize = 20;
/// The most work items one [`Sandbox::run`] takes before it stops (a flow cannot loop forever,
/// but a long one can be long).
pub const MAX_STEPS: usize = 2000;

/// Whether a run id looks like a dry run's. Routing asks [`Sandbox::holds`] as well: only a
/// run the sandbox has goes there.
pub fn is_dry(run: &str) -> bool {
    run.starts_with(PREFIX)
}

/// What a dry run's outside answers.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct Mocks {
    /// Station id → the output its tasks hand in: one value, or (for a station of several
    /// tasks) an object keyed by task id.
    pub stations: BTreeMap<String, Value>,
    /// Gate → whether it passes (`true` when absent).
    pub gates: BTreeMap<String, bool>,
    /// Tool → what a `tool` exit gets back (`{}` when absent).
    pub tools: BTreeMap<String, Value>,
}

/// Starting a dry run: a pack's flow by name, or a definition text not saved anywhere.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
pub struct StartIn {
    #[serde(default)]
    pub flow: Option<String>,
    #[serde(default)]
    pub yaml: Option<String>,
    #[serde(default)]
    pub inputs: Value,
    #[serde(default)]
    pub mocks: Mocks,
    /// `dry run` when absent.
    #[serde(default)]
    pub subject: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
}

/// One dry run in the list: `{id, flow, status, created_at}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct DryRunSummary {
    pub id: String,
    pub flow: String,
    pub status: RunStatus,
    pub created_at: u64,
}

/// One work item a dry run took: `{kind, run, station?, task?, gate?, result}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Stepped {
    /// `task`, `gate`, `hook`, `close_session`.
    pub kind: String,
    pub run: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub station: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gate: Option<String>,
    /// What came of it: `submitted`, `failed: …`, `passed`, `not passed`, `ignored`, `skipped: …`.
    pub result: String,
}

/// What a step or a run did, and what is left.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Progress {
    pub run: String,
    pub steps: Vec<Stepped>,
    /// Work items still queued.
    pub pending: usize,
    /// The root run's status.
    pub status: Option<RunStatus>,
}

/// The tools a dry run's `tool` exits call: whatever the mocks say.
struct MockTools(BTreeMap<String, Value>);

#[async_trait]
impl Tools for MockTools {
    async fn call(&self, tool: &str, _: &Value) -> std::result::Result<Value, String> {
        Ok(self.0.get(tool).cloned().unwrap_or_else(|| json!({})))
    }
}

/// A [`MemStore`] whose ids start `dry-` and never repeat across the sandbox.
struct DryStore {
    inner: MemStore,
    seq: Arc<AtomicU64>,
}

#[async_trait]
impl RunStore for DryStore {
    async fn alloc_id(&self, flow: &str) -> std::result::Result<String, StoreError> {
        let n = self.seq.fetch_add(1, Ordering::SeqCst) + 1;
        Ok(format!("{PREFIX}{flow}-{n}"))
    }
    async fn load(&self, run: &str) -> std::result::Result<Option<RunRecord>, StoreError> {
        self.inner.load(run).await
    }
    async fn list(&self, q: &RunQuery) -> std::result::Result<Vec<RunSummary>, StoreError> {
        self.inner.list(q).await
    }
    async fn commit(&self, cs: &ChangeSet) -> std::result::Result<(), StoreError> {
        self.inner.commit(cs).await
    }
    async fn history(&self, run: &str) -> std::result::Result<Vec<History>, StoreError> {
        self.inner.history(run).await
    }
}

/// The application's packs, with an unsaved definition over them and a catalog that takes in
/// every name the flows use: nothing outside the kernel is called in a dry run.
struct SandboxPacks {
    base: Arc<dyn Packs>,
    extra: Option<(String, String)>,
}

impl SandboxPacks {
    fn defs(&self) -> Vec<FlowDef> {
        let mut out: Vec<FlowDef> = self
            .base
            .flows()
            .into_iter()
            .filter_map(|n| self.flow(&n).map(|(d, _)| d))
            .collect();
        if let Some((_, text)) = &self.extra {
            if let Ok(d) = FlowDef::parse(text) {
                out.push(d);
            }
        }
        out
    }
}

/// Add every program step, gate, tool, action, scene and placement name `def` uses.
fn take_in(c: &mut Catalog, def: &FlowDef) {
    if let Some(sc) = def.main.as_ref().and_then(|m| m.scene.clone()) {
        c.scenes.insert(sc.clone());
        c.submit_scenes.insert(sc);
    }
    for h in &def.hooks {
        c.actions.insert(h.action.clone());
    }
    for s in &def.stations {
        for h in &s.hooks {
            c.actions.insert(h.action.clone());
        }
        for e in &s.exit {
            match e {
                ExitCond::Gate { gate, .. } => {
                    c.gates.insert(gate.clone());
                }
                ExitCond::Tool { tool, .. } => {
                    c.tools.insert(tool.clone());
                }
                _ => {}
            }
        }
        let specs = match s.work() {
            Work::None => vec![],
            Work::One(t) | Work::Foreach { task: t, .. } => vec![t],
            Work::Tasks(ts) => ts,
        };
        for t in specs {
            match t.executor {
                Executor::Program { step } => {
                    c.steps.insert(step);
                }
                Executor::Session {
                    scene,
                    model,
                    provider,
                    instance,
                    template,
                    space,
                    ..
                } => {
                    if !scene.is_empty() {
                        c.scenes.insert(scene);
                    }
                    let put = |set: &mut BTreeSet<String>, v: Option<String>| {
                        if let (false, Some(v)) = (set.is_empty(), v) {
                            set.insert(v);
                        }
                    };
                    put(&mut c.templates, template);
                    put(&mut c.instances, instance);
                    put(&mut c.spaces, space);
                    let m = match (provider, model) {
                        (Some(p), Some(m)) => Some(format!("{p}/{m}")),
                        (None, m) => m,
                        (Some(_), None) => None,
                    };
                    put(&mut c.models, m);
                }
                _ => {}
            }
        }
    }
}

impl Packs for SandboxPacks {
    fn flows(&self) -> Vec<String> {
        let mut names = self.base.flows();
        if let Some((n, _)) = &self.extra {
            if !names.contains(n) {
                names.push(n.clone());
            }
        }
        names
    }
    fn flow(&self, name: &str) -> Option<(FlowDef, String)> {
        match &self.extra {
            Some((n, text)) if n == name => Some((
                FlowDef::parse(text).ok()?,
                crate::kernel::host::digest(text),
            )),
            _ => self.base.flow(name),
        }
    }
    fn skill(&self, name: &str) -> Option<(String, String)> {
        self.base.skill(name)
    }
    fn catalog(&self) -> Catalog {
        let mut c = self.base.catalog();
        for d in self.defs() {
            take_in(&mut c, &d);
        }
        c
    }
    fn flow_text(&self, name: &str) -> Option<String> {
        match &self.extra {
            Some((n, text)) if n == name => Some(text.clone()),
            _ => self.base.flow_text(name),
        }
    }
    fn skills(&self) -> Vec<String> {
        self.base.skills()
    }
}

struct DryRun {
    root: String,
    flow: String,
    created_at: u64,
    engine: Arc<Engine>,
    store: Arc<DryStore>,
    mocks: Mocks,
    queue: VecDeque<WorkItem>,
    /// (run, station, task) → what its session would have been sent.
    previews: BTreeMap<(String, String, String), Value>,
    /// Driven by `run` (a person's action carries on to the next stop) or by `step` (it waits).
    auto: bool,
}

/// How the sandbox builds each dry run's engine.
#[derive(Clone, Default)]
pub struct SandboxConfig {
    /// The tools refused as writes (the protocol layer names them).
    pub guarded: Vec<String>,
    /// Scenes whose sessions hand in with `flow_submit`.
    pub submit_scenes: Vec<String>,
    /// The scene a run's main session is opened on when its flow names none.
    pub main_scene: Option<String>,
}

/// Every dry run, newest last.
pub struct Sandbox {
    packs: Arc<dyn Packs>,
    cfg: SandboxConfig,
    seq: Arc<AtomicU64>,
    runs: tokio::sync::Mutex<Vec<DryRun>>,
}

fn dispatcher() -> Actor {
    Actor::flow("dispatch")
}

impl Sandbox {
    pub fn new(packs: Arc<dyn Packs>, cfg: SandboxConfig) -> Self {
        Self {
            packs,
            cfg,
            seq: Arc::default(),
            runs: tokio::sync::Mutex::new(vec![]),
        }
    }

    /// Start a dry run; its first work is queued, not taken. `value`: the run id. Starting one
    /// beyond [`KEEP`] drops the oldest.
    pub async fn start(&self, input: StartIn, by: Actor) -> Result<String> {
        let extra = match (&input.flow, &input.yaml) {
            (_, Some(text)) => {
                let def = FlowDef::parse(text)
                    .map_err(|e| FlowError::Invalid(format!("not a flow definition: {e}")))?;
                if let Some(f) = input.flow.as_ref().filter(|f| **f != def.name) {
                    return Err(FlowError::Invalid(format!(
                        "the text defines flow {}, not {f}",
                        def.name
                    )));
                }
                Some((def.name, text.clone()))
            }
            (Some(_), None) => None,
            (None, None) => return Err(FlowError::Invalid("name a flow or give its yaml".into())),
        };
        let flow = match (&extra, &input.flow) {
            (Some((n, _)), _) => n.clone(),
            (None, Some(f)) => f.clone(),
            (None, None) => unreachable!(),
        };
        let packs: Arc<dyn Packs> = Arc::new(SandboxPacks {
            base: self.packs.clone(),
            extra,
        });
        let store = Arc::new(DryStore {
            inner: MemStore::new(),
            seq: self.seq.clone(),
        });
        let mut engine = Engine::new(store.clone(), packs)
            .with_tools(Arc::new(MockTools(input.mocks.tools.clone())))
            .with_guarded(self.cfg.guarded.clone())
            .with_submit_scenes(self.cfg.submit_scenes.clone());
        if let Some(m) = &self.cfg.main_scene {
            engine = engine.with_main_scene(m.clone());
        }
        let engine = Arc::new(engine);
        let o = engine
            .create(
                CreateIn {
                    flow: flow.clone(),
                    subject: input.subject.clone().unwrap_or_else(|| "dry run".into()),
                    inputs: input.inputs.clone(),
                    title: input.title.clone(),
                    parent: None,
                },
                by,
            )
            .await?;
        let root = o.value["run"].as_str().unwrap_or_default().to_string();
        let created_at = engine
            .record(&root)
            .await
            .map(|r| r.created_at)
            .unwrap_or_default();
        let mut runs = self.runs.lock().await;
        runs.push(DryRun {
            root: root.clone(),
            flow,
            created_at,
            engine,
            store,
            mocks: input.mocks,
            queue: o.work.into(),
            previews: BTreeMap::new(),
            auto: false,
        });
        let over = runs.len().saturating_sub(KEEP);
        runs.drain(..over);
        Ok(root)
    }

    /// The dry runs kept, newest first.
    pub async fn list(&self) -> Vec<DryRunSummary> {
        let runs = self.runs.lock().await;
        let mut out = vec![];
        for d in runs.iter().rev() {
            let status = d
                .engine
                .record(&d.root)
                .await
                .map(|r| r.status)
                .unwrap_or(RunStatus::Cancelled);
            out.push(DryRunSummary {
                id: d.root.clone(),
                flow: d.flow.clone(),
                status,
                created_at: d.created_at,
            });
        }
        out
    }

    /// The index of the dry run `run` (a root or a child) belongs to.
    async fn find(runs: &[DryRun], run: &str) -> Option<usize> {
        for (i, d) in runs.iter().enumerate() {
            if d.root == run || d.store.load(run).await.ok().flatten().is_some() {
                return Some(i);
            }
        }
        None
    }

    fn not_found(run: &str) -> FlowError {
        FlowError::NotFound(format!("dry run {run} (only the last {KEEP} are kept)"))
    }

    /// The engine a dry run's runs are in: for a person's action on one of them. Hand what it
    /// returns to [`Sandbox::absorb`].
    pub async fn engine_for(&self, run: &str) -> Result<Arc<Engine>> {
        let runs = self.runs.lock().await;
        let i = Self::find(&runs, run)
            .await
            .ok_or_else(|| Self::not_found(run))?;
        Ok(runs[i].engine.clone())
    }

    /// Whether `run` is one of the sandbox's runs (a root or a child).
    pub async fn holds(&self, run: &str) -> bool {
        is_dry(run) && {
            let runs = self.runs.lock().await;
            Self::find(&runs, run).await.is_some()
        }
    }

    /// The root of the dry run `run` belongs to.
    pub async fn root_of(&self, run: &str) -> Option<String> {
        let runs = self.runs.lock().await;
        let i = Self::find(&runs, run).await?;
        Some(runs[i].root.clone())
    }

    /// Every run of the dry run `root` (it and its children).
    pub async fn runs_of(&self, root: &str) -> Vec<String> {
        let runs = self.runs.lock().await;
        let Some(i) = Self::find(&runs, root).await else {
            return vec![];
        };
        runs[i]
            .store
            .list(&RunQuery::default())
            .await
            .map(|l| l.into_iter().map(|s| s.id).collect())
            .unwrap_or_default()
    }

    /// Queue what a person's action produced; a dry run being driven by `run` carries on.
    pub async fn absorb(&self, run: &str, o: Outcome) -> Result<Progress> {
        let mut runs = self.runs.lock().await;
        let i = Self::find(&runs, run)
            .await
            .ok_or_else(|| Self::not_found(run))?;
        let d = &mut runs[i];
        d.queue.extend(o.work);
        let limit = if d.auto { MAX_STEPS } else { 0 };
        Ok(Self::drive(d, &self.packs, limit).await)
    }

    /// Take the next work item.
    pub async fn step(&self, run: &str) -> Result<Progress> {
        let mut runs = self.runs.lock().await;
        let i = Self::find(&runs, run)
            .await
            .ok_or_else(|| Self::not_found(run))?;
        let d = &mut runs[i];
        d.auto = false;
        Ok(Self::drive(d, &self.packs, 1).await)
    }

    /// Take work items until none is left: the run is over, or waits for a person.
    pub async fn run(&self, run: &str) -> Result<Progress> {
        let mut runs = self.runs.lock().await;
        let i = Self::find(&runs, run)
            .await
            .ok_or_else(|| Self::not_found(run))?;
        let d = &mut runs[i];
        d.auto = true;
        Ok(Self::drive(d, &self.packs, MAX_STEPS).await)
    }

    /// A run of a dry run as a reader sees it (a `RunView`), each session task carrying the
    /// `preview` of what its session would have been sent.
    pub async fn get(&self, run: &str) -> Result<Value> {
        let runs = self.runs.lock().await;
        let i = Self::find(&runs, run)
            .await
            .ok_or_else(|| Self::not_found(run))?;
        let d = &runs[i];
        let rec = d.engine.record(run).await?;
        let mut v = serde_json::to_value(view(rec)).map_err(|e| FlowError::Store(e.to_string()))?;
        for ((r, station, task), preview) in &d.previews {
            if r != run {
                continue;
            }
            if let Some(tasks) = v
                .pointer_mut(&format!("/record/stations/{}/tasks", escape(station)))
                .and_then(Value::as_array_mut)
            {
                if let Some(t) = tasks
                    .iter_mut()
                    .find(|t| t.get("id").and_then(Value::as_str) == Some(task))
                {
                    t["preview"] = preview.clone();
                }
            }
        }
        Ok(v)
    }

    async fn drive(d: &mut DryRun, packs: &Arc<dyn Packs>, limit: usize) -> Progress {
        let mut steps = vec![];
        while steps.len() < limit {
            let Some(w) = d.queue.pop_front() else { break };
            let (stepped, more) = Self::take(d, packs, w).await;
            d.queue.extend(more);
            steps.push(stepped);
        }
        let status = d.engine.record(&d.root).await.ok().map(|r| r.status);
        Progress {
            run: d.root.clone(),
            steps,
            pending: d.queue.len(),
            status,
        }
    }

    /// Carry out one work item the mocked way; the work it led to.
    async fn take(d: &mut DryRun, packs: &Arc<dyn Packs>, w: WorkItem) -> (Stepped, Vec<WorkItem>) {
        let engine = d.engine.clone();
        match w {
            WorkItem::Task {
                run,
                station,
                round,
                task,
                token,
                attempt,
                executor,
                skill,
                protocol,
                output_schema,
                session_key,
                placement,
                delivery,
                scene,
                ..
            } => {
                let mut stepped = Stepped {
                    kind: "task".into(),
                    run: run.clone(),
                    station: Some(station.clone()),
                    task: Some(task.clone()),
                    gate: None,
                    result: String::new(),
                };
                if matches!(executor, Executor::Session { .. } | Executor::Agent { .. }) {
                    let body = skill
                        .as_ref()
                        .and_then(|s| packs.skill(s).map(|(b, _)| (s.clone(), b)));
                    let message = task_message(
                        protocol.as_deref().unwrap_or_default(),
                        body.as_ref().map(|(n, b)| (n.as_str(), b.as_str())),
                    );
                    d.previews.insert(
                        (run.clone(), station.clone(), task.clone()),
                        json!({
                            "round": round, "attempt": attempt, "scene": scene,
                            "message": message, "placement": placement, "delivery": delivery,
                            "output_schema": output_schema, "session_key": session_key,
                        }),
                    );
                }
                let output = mock_output(&engine, &d.mocks, &run, &station, &task).await;
                let mut work = vec![];
                match engine
                    .submit(&run, &station, &task, &token, output, dispatcher())
                    .await
                {
                    Ok(o) => {
                        stepped.result = "submitted".into();
                        work = o.work;
                    }
                    Err(FlowError::Contract(m)) => {
                        stepped.result = format!("failed: {m}");
                        if let Ok(o) = engine
                            .fail(&run, &station, &task, &token, &m, dispatcher())
                            .await
                        {
                            work = o.work;
                        }
                    }
                    Err(e) => stepped.result = format!("skipped: {e}"),
                }
                (stepped, work)
            }
            WorkItem::Gate {
                run,
                station,
                round,
                gate,
            } => {
                let pass = d.mocks.gates.get(&gate).copied().unwrap_or(true);
                let detail = if pass {
                    "mocked: passes"
                } else {
                    "mocked: does not pass"
                };
                let r = engine
                    .gate_result(
                        &run,
                        &station,
                        round,
                        &gate,
                        GateResult {
                            pass,
                            detail: detail.into(),
                        },
                    )
                    .await;
                let stepped = Stepped {
                    kind: "gate".into(),
                    run,
                    station: Some(station),
                    task: None,
                    gate: Some(gate),
                    result: if pass { "passed" } else { "not passed" }.into(),
                };
                (stepped, r.map(|o| o.work).unwrap_or_default())
            }
            WorkItem::Hook { action, event } => (
                Stepped {
                    kind: "hook".into(),
                    run: event.run.clone(),
                    station: event.station.clone(),
                    task: event.task.clone(),
                    gate: None,
                    result: format!("ignored: {action} on {}", event.kind),
                },
                vec![],
            ),
            WorkItem::CloseSession { session } => (
                Stepped {
                    kind: "close_session".into(),
                    run: String::new(),
                    station: None,
                    task: None,
                    gate: None,
                    result: format!("ignored: {session}"),
                },
                vec![],
            ),
        }
    }
}

/// A JSON Pointer segment.
fn escape(s: &str) -> String {
    s.replace('~', "~0").replace('/', "~1")
}

/// What a mocked task hands in: the station's mock (by task id when the station has several
/// tasks and the mock is a table of them), else the contract's example or default, else a
/// minimal value of the contract, else `{}`.
async fn mock_output(
    engine: &Engine,
    mocks: &Mocks,
    run: &str,
    station: &str,
    task: &str,
) -> Value {
    let rec = engine.record(run).await.ok();
    let sdef = rec.as_ref().and_then(|r| r.def.station(station).cloned());
    let several = sdef
        .as_ref()
        .is_some_and(|s| matches!(s.work(), Work::Tasks(_) | Work::Foreach { .. }));
    if let Some(m) = mocks.stations.get(station) {
        if let (true, Value::Object(table)) = (several, m) {
            if let Some(v) = table.get(task) {
                return v.clone();
            }
            // A table naming other tasks is not this task's value.
            if table_names_tasks(table, rec.as_ref(), station) {
                return contract_value(sdef.as_ref().and_then(|s| s.output.as_ref()));
            }
        }
        return m.clone();
    }
    contract_value(sdef.as_ref().and_then(|s| s.output.as_ref()))
}

fn table_names_tasks(table: &Map<String, Value>, rec: Option<&RunRecord>, station: &str) -> bool {
    let Some(st) = rec.and_then(|r| r.stations.get(station)) else {
        return false;
    };
    table.keys().any(|k| st.task(k).is_some())
}

fn contract_value(schema: Option<&Value>) -> Value {
    match schema {
        Some(s) => minimal(s),
        None => json!({}),
    }
}
