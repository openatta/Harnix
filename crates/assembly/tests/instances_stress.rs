//! The instance table under concurrent use: many `ensure`s of one name, sessions created on an
//! instance the moment it is listed, attaches, and sends while the instance restarts.
//!
//! What it holds the hub and the registry to:
//!
//! - **One name, one instance.** Concurrent `ensure`s of a name build it once and all answer
//!   with it; the ledger records it once.
//! - **Visible means routed.** An instance anyone can see is one whose events the hub already
//!   reads: a session created on it the moment it is listed settles its turn, which it cannot do
//!   unless the hub routes the engine's frames (a turn settles behind a barrier that goes through
//!   the router).
//! - **A restart does not strand a send.** A send during a restart answers; the instance that comes
//!   back routes the next turn.
//! - **Nothing panics**, the lock-order assertions of a debug build included, in a task or out:
//!   the registry calls the hub holding none of its locks, and the hub the registry none of its.
//!
//! The engines are real and built in this process; no model is called: the hub is started in
//! replay with no recordings, so each turn ends at its first model call.

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use harnix_contract::{Frame, FrameSink, Subject};
use harnix_hub::{Hub, HubCell, HubConfig, HubRow};
use harnix_instance::{
    EngineProfile, Factory, InstanceRegistry, InstanceRow, InstanceTemplate, Ledger, Owner, Recording,
    RegistryCell, RegistryConfig, INSTANCES_TOPIC,
};
use harnix_kernel::{CallContext, ClientId, Handler};
use serde_json::{json, Value};

/// How long anything here may take before it counts as stuck.
const BOUND: Duration = Duration::from_secs(30);

/// Set by the panic hook: a panic in a spawned task does not reach the test otherwise.
static PANICKED: AtomicBool = AtomicBool::new(false);

/// Once per run: watch for panics, and work in a directory of the test's own — an engine keeps
/// its project data (`.harnix/`) in the working directory.
fn once() {
    static ONCE: OnceLock<()> = OnceLock::new();
    ONCE.get_or_init(|| {
        let cwd = Path::new(env!("CARGO_TARGET_TMPDIR")).join("instances_stress");
        std::fs::create_dir_all(&cwd).unwrap();
        std::env::set_current_dir(&cwd).unwrap();
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            PANICKED.store(true, Ordering::SeqCst);
            previous(info);
        }));
    });
}

/// Every frame a client was sent, as text.
#[derive(Default)]
struct Sink(Mutex<Vec<String>>);

#[async_trait::async_trait]
impl FrameSink for Sink {
    async fn deliver(&self, frame: Frame) -> bool {
        self.0.lock().unwrap().push(frame.payload.to_string());
        true
    }
}

impl Sink {
    /// Whether a frame of `method` about `session` arrived.
    fn saw(&self, method: &str, session: &str) -> bool {
        self.0.lock().unwrap().iter().any(|f| {
            let v: Value = serde_json::from_str(f).unwrap_or(Value::Null);
            v["method"] == method && v["params"]["session_id"] == session
        })
    }
}

/// A registry with one template, `work`, whose instances are engines built in this process; the
/// hub observing it; and their rows by name.
struct Rig {
    hub: Arc<Hub>,
    instances: Arc<InstanceRegistry>,
    rows: HashMap<&'static str, Arc<dyn Handler>>,
    _dir: tempfile::TempDir,
}

impl Rig {
    async fn new() -> Self {
        once();
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let replay = root.join("recordings");
        std::fs::create_dir_all(&replay).unwrap();
        let mut factory = Factory::new(Arc::new(|_: &str| None)).unwrap();
        factory.recording = Recording::new(Some(replay));
        let mut config = RegistryConfig {
            templates: Default::default(),
            ledger: Ledger::load(root.join("state/instances.json")).unwrap(),
            factory,
            instances_dir: root.join("instances"),
            disabled: Default::default(),
            fixed: Default::default(),
            grants: Arc::new(harnix_kernel::GrantTable::new()),
        };
        config
            .add_template(InstanceTemplate::in_process("work", EngineProfile::new()).max(64), Owner::Kernel)
            .unwrap();
        let instances = InstanceRegistry::new(config);
        let hub = Hub::new(HubConfig {
            instances: instances.clone(),
            spaces: harnix_hub::Spaces::load(root.join("state/spaces.json"), Vec::new(), Default::default()).unwrap(),
            turns: Default::default(),
        });
        instances.observe(hub.clone());
        let (hub_cell, instance_cell): (HubCell, RegistryCell) = Default::default();
        let mut rows = handlers(Hub::rows(&hub_cell));
        rows.extend(InstanceRegistry::rows(&instance_cell).into_iter().filter_map(|row| match row {
            InstanceRow::Method { name, handler, .. } => Some((name, handler)),
            InstanceRow::Topic { .. } => None,
        }));
        let _ = hub_cell.set(hub.clone());
        let _ = instance_cell.set(instances.clone());
        hub.serve_topic(INSTANCES_TOPIC, Vec::new(), InstanceRegistry::topic(&instance_cell)).await;
        Self { hub, instances, rows, _dir: dir }
    }

    async fn shutdown(&self) {
        self.hub.shutdown().await;
        self.instances.shutdown().await;
    }

    async fn call(&self, cx: CallContext, method: &str, params: Value) -> Result<Value, harnix_contract::RpcError> {
        let row = self.rows.get(method).unwrap_or_else(|| panic!("no row `{method}`"));
        tokio::time::timeout(BOUND, row.call(cx, params))
            .await
            .unwrap_or_else(|_| panic!("`{method}` did not answer within {BOUND:?}"))
    }

    async fn kernel(&self, method: &str, params: Value) -> Result<Value, harnix_contract::RpcError> {
        self.call(CallContext::root(Subject::Kernel, None), method, params).await
    }

    /// A connected device: its context, and what it is sent.
    async fn client(&self) -> (CallContext, Arc<Sink>) {
        let sink = Arc::new(Sink::default());
        let id: ClientId = self.hub.add_client(sink.clone()).await;
        (CallContext::root(Subject::Device { id: format!("d{}", id.0) }, Some(id)), sink)
    }

    async fn ids(&self) -> Vec<String> {
        let list = self.kernel("harnix.instances.list", json!({})).await.unwrap();
        list["instances"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["id"].as_str().unwrap().to_string())
            .collect()
    }

    /// Create a session on `instance`, attach `client` to it, send one message, and wait for the
    /// turn to settle: what only a routed instance can do.
    async fn turn(&self, cx: &CallContext, sink: &Sink, instance: &str) -> String {
        let created = self
            .call(cx.clone(), "harnix.session.create", json!({"instance": instance}))
            .await
            .unwrap_or_else(|e| panic!("harnix.session.create on `{instance}`: {}", e.message));
        let sid = created["session_id"].as_str().unwrap().to_string();
        let at = json!({"instance": instance, "session_id": sid});
        self.call(cx.clone(), "harnix.session.attach", at.clone()).await.unwrap();
        let mut send = at.clone();
        send["message"] = json!("hello");
        self.call(cx.clone(), "harnix.session.send", send).await.unwrap();
        settled(sink, &sid).await;
        sid
    }

    fn assert_no_panic(&self) {
        assert!(!PANICKED.load(Ordering::SeqCst), "something panicked (see the output above)");
    }
}

fn handlers(rows: Vec<HubRow>) -> HashMap<&'static str, Arc<dyn Handler>> {
    rows.into_iter()
        .filter_map(|row| match row {
            HubRow::Method { name, handler: Some(h), .. } => Some((name, h)),
            _ => None,
        })
        .collect()
}

/// Wait for `harnix.turn_settled` on `session`.
async fn settled(sink: &Sink, session: &str) {
    let waited = tokio::time::timeout(BOUND, async {
        while !sink.saw("harnix.turn_settled", session) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
    assert!(waited.is_ok(), "the turn on `{session}` did not settle within {BOUND:?}");
}

fn ledger_ids(dir: &Path) -> Vec<String> {
    let text = std::fs::read_to_string(dir.join("state/instances.json")).unwrap_or_else(|_| "[]".into());
    let entries: Vec<Value> = serde_json::from_str(&text).unwrap();
    entries.iter().map(|e| e["id"].as_str().unwrap().to_string()).collect()
}

/// Sixteen `ensure`s of one name, at once, build one instance and all answer with it; eight
/// distinct names at the same time are all built.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_ensures_build_each_name_once() {
    let rig = Arc::new(Rig::new().await);
    let mut calls = Vec::new();
    for n in 0..24 {
        let rig = rig.clone();
        let name = if n < 16 { "shared".to_string() } else { format!("own-{n}") };
        calls.push(tokio::spawn(async move {
            rig.kernel("harnix.instances.ensure", json!({"template": "work", "name": name})).await
        }));
    }
    for call in calls {
        let answered = call.await.unwrap().unwrap_or_else(|e| panic!("ensure: {}", e.message));
        assert!(answered["id"].is_string(), "{answered}");
    }
    let ids = rig.ids().await;
    assert_eq!(ids.iter().filter(|id| *id == "shared").count(), 1, "{ids:?}");
    assert_eq!(ids.len(), 9, "{ids:?}");
    let ledger = ledger_ids(rig._dir.path());
    assert_eq!(ledger.iter().filter(|id| *id == "shared").count(), 1, "{ledger:?}");
    assert_eq!(ledger.len(), 9, "{ledger:?}");
    rig.shutdown().await;
    rig.assert_no_panic();
}

/// A session created on an instance the moment it is listed runs a turn to its end: whatever
/// can see an instance can use it, because the hub was reading it first.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_instance_is_routed_as_soon_as_it_is_listed() {
    let rig = Arc::new(Rig::new().await);
    let names: Vec<String> = (0..6).map(|n| format!("fresh-{n}")).collect();
    let mut watchers = Vec::new();
    for name in &names {
        let (rig, name) = (rig.clone(), name.clone());
        watchers.push(tokio::spawn(async move {
            // Poll the list, and use the instance the first time it is there.
            let deadline = tokio::time::Instant::now() + BOUND;
            while !rig.ids().await.contains(&name) {
                assert!(tokio::time::Instant::now() < deadline, "`{name}` never listed");
                tokio::task::yield_now().await;
            }
            let (cx, sink) = rig.client().await;
            rig.turn(&cx, &sink, &name).await;
        }));
    }
    let mut builds = Vec::new();
    for name in &names {
        let (rig, name) = (rig.clone(), name.clone());
        builds.push(tokio::spawn(async move {
            rig.kernel("harnix.instances.ensure", json!({"template": "work", "name": name})).await
        }));
    }
    for b in builds {
        b.await.unwrap().unwrap();
    }
    for w in watchers {
        w.await.unwrap();
    }
    rig.shutdown().await;
    rig.assert_no_panic();
}

/// Many sessions created and attached on one instance at once, each running a turn.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_sessions_on_one_instance_all_settle() {
    let rig = Arc::new(Rig::new().await);
    rig.kernel("harnix.instances.ensure", json!({"template": "work", "name": "busy"})).await.unwrap();
    let mut turns = Vec::new();
    for _ in 0..12 {
        let rig = rig.clone();
        turns.push(tokio::spawn(async move {
            let (cx, sink) = rig.client().await;
            rig.turn(&cx, &sink, "busy").await
        }));
    }
    for t in turns {
        t.await.unwrap();
    }
    rig.shutdown().await;
    rig.assert_no_panic();
}

/// Sends while the instance restarts answer, ensures of the same name meanwhile answer with it,
/// and the instance that comes back routes the next turn.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn sends_during_a_restart_answer_and_the_instance_comes_back_routed() {
    let rig = Arc::new(Rig::new().await);
    rig.kernel("harnix.instances.ensure", json!({"template": "work", "name": "phoenix"})).await.unwrap();
    let (cx, sink) = rig.client().await;
    let sid = rig.turn(&cx, &sink, "phoenix").await;

    for round in 0..3 {
        let mut sends = Vec::new();
        for n in 0..6 {
            let (rig, cx, sid) = (rig.clone(), cx.clone(), sid.clone());
            sends.push(tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(n * 5)).await;
                let send = json!({"instance": "phoenix", "session_id": sid, "message": format!("m{n}")});
                // Answered, one way or another: a turn, a queued item, or the instance's refusal.
                let _ = rig.call(cx, "harnix.session.send", send).await;
            }));
        }
        let mut ensures = Vec::new();
        for _ in 0..4 {
            let rig = rig.clone();
            ensures.push(tokio::spawn(async move {
                rig.kernel("harnix.instances.ensure", json!({"template": "work", "name": "phoenix"})).await
            }));
        }
        let restarted = rig.kernel("harnix.instances.restart", json!({"instance": "phoenix"})).await;
        assert!(restarted.is_ok(), "round {round}: {restarted:?}");
        for s in sends {
            s.await.unwrap();
        }
        for e in ensures {
            let answered = e.await.unwrap().unwrap_or_else(|e| panic!("round {round}: ensure: {}", e.message));
            assert_eq!(answered["id"], "phoenix");
        }
        let ids = rig.ids().await;
        assert_eq!(ids.iter().filter(|id| *id == "phoenix").count(), 1, "round {round}: {ids:?}");
        // The instance that came back is routed: a new session on it settles its turn.
        let (cx, sink) = rig.client().await;
        rig.turn(&cx, &sink, "phoenix").await;
    }
    rig.shutdown().await;
    rig.assert_no_panic();
}

/// Stops and ensures of one name, interleaved: each answers, and at the end the name is listed
/// at most once and the ledger agrees with the list.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stops_and_ensures_of_one_name_keep_the_list_and_the_ledger_in_step() {
    let rig = Arc::new(Rig::new().await);
    rig.kernel("harnix.instances.ensure", json!({"template": "work", "name": "flicker"})).await.unwrap();
    let mut calls = Vec::new();
    for n in 0..16u64 {
        let rig = rig.clone();
        calls.push(tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(n * 3)).await;
            if n % 2 == 0 {
                let _ = rig.kernel("harnix.instances.stop", json!({"instance": "flicker"})).await;
            } else {
                let _ = rig.kernel("harnix.instances.ensure", json!({"template": "work", "name": "flicker"})).await;
            }
        }));
    }
    for c in calls {
        c.await.unwrap();
    }
    let listed = rig.ids().await.iter().filter(|id| *id == "flicker").count();
    assert!(listed <= 1);
    let ledger = ledger_ids(rig._dir.path());
    assert_eq!(ledger.iter().filter(|id| *id == "flicker").count(), listed, "{ledger:?}");
    if listed == 1 {
        let (cx, sink) = rig.client().await;
        rig.turn(&cx, &sink, "flicker").await;
    }
    rig.shutdown().await;
    rig.assert_no_panic();
}
