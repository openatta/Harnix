#![allow(dead_code)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use harnix_flow::*;
use serde_json::{json, Value};

pub const T0: u64 = 1_000_000;

/// Packs with a catalog covering every name the test flows use.
pub fn packs() -> MemPacks {
    MemPacks::new()
        .scene("chat")
        .step("collect")
        .step("work")
        .step("check")
        .gate("lint")
        .tool("count")
        .action("notify")
        .skill("analyze", "---\nname: analyze\n---\nAnalyze it.")
        .skill("report", "Write the report.")
}

/// Tool results set by the test; an unset tool fails.
#[derive(Default)]
pub struct FakeTools {
    pub results: Mutex<BTreeMap<String, Value>>,
    pub calls: Mutex<Vec<(String, Value)>>,
}

impl FakeTools {
    pub fn set(&self, tool: &str, v: Value) {
        self.results.lock().unwrap().insert(tool.into(), v);
    }
}

#[async_trait]
impl Tools for FakeTools {
    async fn call(&self, tool: &str, args: &Value) -> std::result::Result<Value, String> {
        self.calls.lock().unwrap().push((tool.into(), args.clone()));
        self.results
            .lock()
            .unwrap()
            .get(tool)
            .cloned()
            .ok_or_else(|| format!("no result for {tool}"))
    }
}

pub struct H {
    pub eng: Engine,
    pub clock: Arc<ManualClock>,
    pub store: Arc<MemStore>,
    pub packs: Arc<MemPacks>,
    pub tools: Arc<FakeTools>,
}

pub fn harness(packs: MemPacks) -> H {
    let store = Arc::new(MemStore::new());
    harness_on(packs, store)
}

pub fn harness_on(packs: MemPacks, store: Arc<MemStore>) -> H {
    let clock = Arc::new(ManualClock::new(T0));
    let packs = Arc::new(packs);
    let tools = Arc::new(FakeTools::default());
    let eng = Engine::new(store.clone(), packs.clone())
        .with_clock(clock.clone())
        .with_tools(tools.clone())
        .with_lease_ms(60_000);
    H {
        eng,
        clock,
        store,
        packs,
        tools,
    }
}

pub fn me() -> Actor {
    Actor::new("user", "alice")
}

/// A dispatched task, as the module would see it.
#[derive(Debug, Clone)]
pub struct Job {
    pub run: String,
    pub station: String,
    pub task: String,
    pub token: String,
    pub attempt: u32,
    pub item: Option<Value>,
    pub protocol: Option<String>,
    pub session_key: Option<String>,
}

pub fn jobs(o: &Outcome) -> Vec<Job> {
    o.work
        .iter()
        .filter_map(|w| match w {
            WorkItem::Task {
                run,
                station,
                task,
                token,
                attempt,
                item,
                protocol,
                session_key,
                ..
            } => Some(Job {
                run: run.clone(),
                station: station.clone(),
                task: task.clone(),
                token: token.clone(),
                attempt: *attempt,
                item: item.clone(),
                protocol: protocol.clone(),
                session_key: session_key.clone(),
            }),
            _ => None,
        })
        .collect()
}

pub fn kinds(o: &Outcome) -> Vec<String> {
    o.events
        .iter()
        .map(|e| match (&e.station, &e.task) {
            (Some(s), Some(t)) => format!("{}:{s}/{t}", e.kind),
            (Some(s), None) => format!("{}:{s}", e.kind),
            _ => e.kind.clone(),
        })
        .collect()
}

pub fn has(o: &Outcome, kind: &str) -> bool {
    kinds(o).iter().any(|k| k == kind)
}

pub async fn start(h: &H, flow: &str) -> (String, Outcome) {
    start_with(h, flow, json!({})).await
}

pub async fn start_with(h: &H, flow: &str, inputs: Value) -> (String, Outcome) {
    let o = h
        .eng
        .create(
            CreateIn {
                flow: flow.into(),
                subject: "repo".into(),
                inputs,
                ..Default::default()
            },
            me(),
        )
        .await
        .expect("create");
    let id = o.value["run"].as_str().unwrap().to_string();
    (id, o)
}

pub async fn submit(h: &H, j: &Job, output: Value) -> Outcome {
    h.eng
        .submit(
            &j.run,
            &j.station,
            &j.task,
            &j.token,
            output,
            Actor::flow("dispatch"),
        )
        .await
        .unwrap_or_else(|e| panic!("submit {}/{}: {e}", j.station, j.task))
}

/// Act as the module: submit every dispatched task with `f`'s output, and keep going with
/// the work that produces, until no task is handed out. Returns every outcome seen.
pub async fn drive(h: &H, first: Outcome, f: impl Fn(&Job) -> Value) -> Vec<Outcome> {
    let mut seen = vec![];
    let mut queue = jobs(&first);
    seen.push(first);
    while let Some(j) = queue.first().cloned() {
        queue.remove(0);
        let o = submit(h, &j, f(&j)).await;
        queue.extend(jobs(&o));
        seen.push(o);
    }
    seen
}

pub async fn status(h: &H, run: &str, station: &str) -> StationStatus {
    h.eng.get(run).await.unwrap().record.stations[station].status
}

pub async fn run_status(h: &H, run: &str) -> RunStatus {
    h.eng.get(run).await.unwrap().record.status
}

pub async fn task(h: &H, run: &str, station: &str, task: &str) -> TaskRec {
    h.eng.get(run).await.unwrap().record.stations[station]
        .task(task)
        .cloned()
        .unwrap()
}
