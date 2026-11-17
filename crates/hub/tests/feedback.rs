//! Feedback on an answer: a device's, about a session it can see, filed in the session's log as
//! an entry of the hub's own — rated, changed, taken back, each a new entry, the last one standing
//! — and told on a topic only a holder of `feedback.read` may hear.
//!
//! The link here is a fake: the hub reaches an engine through `EngineLink` and nothing else.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use harnix_contract::wire::{current_feedback, Rating, FEEDBACK_EVENT, FEEDBACK_NS, FEEDBACK_TOPIC};
use harnix_contract::s1::caps;
use harnix_contract::{codes, ExtId, Frame, FrameSink, RpcError, SpaceDecl, Subject};
use harnix_engine_link::events::EngineEvent;
use harnix_engine_link::link::{EngineControl, EngineIdentity, EngineLink, EventReceiver, InstanceCarrier, LinkStatus};
use harnix_engine_link::ops::{Answer, EngineOps, Entry, HistoryPage, Said, SessionCreated, SessionInfo, SessionSetup, Subscribed, Tasks, TurnInput};
use harnix_hub::{Hub, HubCell, HubConfig, HubRow, Spaces};
use harnix_kernel::{Access, CallContext, RowScope, Handler, InstanceBook, InstanceObserver, InstanceTrust, Listed, Rpc};
use serde_json::{json, Value};
use tokio::sync::{mpsc, watch};

/// An engine that keeps the entries it is given, as the engine answers them.
struct FakeLink {
    events: Mutex<Option<EventReceiver>>,
    status: watch::Sender<LinkStatus>,
    entries: Mutex<Vec<Value>>,
    _tx: mpsc::Sender<EngineEvent>,
}

impl FakeLink {
    fn new() -> Arc<Self> {
        let (tx, rx) = mpsc::channel(16);
        let (status, _) = watch::channel(LinkStatus::Ready);
        Arc::new(Self { events: Mutex::new(Some(rx)), status, entries: Mutex::default(), _tx: tx })
    }

    fn entries(&self) -> Vec<Value> {
        self.entries.lock().unwrap().clone()
    }
}

fn described(id: &str) -> Value {
    json!({"session_id": id, "name": "n", "turn_state": "idle", "scene": "harnix"})
}

#[async_trait::async_trait]
impl EngineControl for FakeLink {
    fn status(&self) -> watch::Receiver<LinkStatus> {
        self.status.subscribe()
    }
    fn identity(&self) -> EngineIdentity {
        EngineIdentity::default()
    }
    fn carrier(&self) -> InstanceCarrier {
        InstanceCarrier::InProcess
    }
    async fn reload_settings(&self) -> Result<(), RpcError> {
        Ok(())
    }
    async fn shutdown(&self) {}
}

#[async_trait::async_trait]
impl EngineLink for FakeLink {
    fn ops(&self) -> &dyn EngineOps {
        self
    }
    fn take_events(&self) -> Option<EventReceiver> {
        self.events.lock().unwrap().take()
    }
    async fn mark(&self, _id: u64) {}
}

#[async_trait::async_trait]
impl EngineOps for FakeLink {
    async fn create(&self, _: Value, _: SessionSetup) -> Answer<SessionCreated> {
        Ok(SessionCreated::new(json!({"session_id": "s1"})))
    }
    async fn get(&self, session: &str) -> Answer<SessionInfo> {
        Ok(SessionInfo::new(described(session)))
    }
    async fn list(&self) -> Answer<Vec<SessionInfo>> {
        Ok(vec![SessionInfo::new(described("s1"))])
    }
    async fn subscribe(&self, _: &str) -> Answer<Option<Subscribed>> {
        Ok(Some(Subscribed::new(Some(0), Vec::new())))
    }
    async fn resume(&self, _: &str, _: &SessionInfo) -> Answer<()> {
        Ok(())
    }
    async fn history_len(&self, _: &str) -> Answer<u64> {
        Ok(2)
    }
    async fn transcript(&self, _: &str, _: usize) -> Answer<Vec<Said>> {
        Ok(Vec::new())
    }
    async fn history(&self, _: &str, _: u64, _: usize) -> Answer<HistoryPage> {
        Ok(HistoryPage::default())
    }
    async fn tasks(&self, _: &str) -> Answer<Tasks> {
        Ok(Tasks::new(json!({"tasks": []}), Default::default()))
    }
    async fn run_turn(&self, _: &str, _: TurnInput) -> Answer<Value> {
        Ok(json!({}))
    }
    async fn interrupt(&self, _: &str) -> Answer<Value> {
        Ok(json!({}))
    }
    async fn close(&self, _: Value, _: bool) -> Answer<Value> {
        Ok(json!({}))
    }
    async fn fork(&self, _: Value) -> Answer<SessionCreated> {
        Ok(SessionCreated::new(json!({"session_id": "s2"})))
    }
    async fn append_entry(&self, _: &str, entry: Entry) -> Answer<Value> {
        let mut entries = self.entries.lock().unwrap();
        let view = json!({
            "id": format!("e{}", entries.len() + 1),
            "ts": "2026-11-16T00:00:00Z",
            "ns": entry.ns,
            "event": entry.event,
            "payload": entry.payload,
            "at": 2,
        });
        entries.push(view.clone());
        Ok(view)
    }
    async fn respond(&self, _: Value) -> Answer<Value> {
        Ok(json!({}))
    }
    async fn rename(&self, session: &str, _: &str) -> Answer<Value> {
        Ok(described(session))
    }
    async fn set_model(&self, session: &str, _: &harnix_contract::wire::ModelChoice) -> Answer<Value> {
        Ok(described(session))
    }
    async fn set_prompt(&self, session: &str, _: Option<Option<String>>, _: Option<Option<String>>) -> Answer<Value> {
        Ok(described(session))
    }
    async fn set_skill_dirs(&self, session: &str, _: &[std::path::PathBuf]) -> Answer<Value> {
        Ok(described(session))
    }
    async fn passthrough(&self, _: &str, _: Value) -> Answer<Value> {
        Ok(json!({}))
    }
}

/// One instance, `main`, no space's: its sessions are the default space's.
struct Book;

#[async_trait::async_trait]
impl InstanceBook for Book {
    fn listed(&self, id: &str) -> Option<Listed> {
        (id == "main").then(|| Listed { id: id.into(), spaces: Vec::new(), extension: None })
    }
    fn all(&self) -> Vec<Listed> {
        self.listed("main").into_iter().collect()
    }
    async fn ensure_for_space(&self, _: &str, _: &str, _: &str) -> Rpc<()> {
        Ok(())
    }
    fn flooding(&self, _: &str, _: u32) {}
}

#[derive(Default)]
struct Sink(Mutex<Vec<Value>>);

#[async_trait::async_trait]
impl FrameSink for Sink {
    async fn deliver(&self, frame: Frame) -> bool {
        self.0.lock().unwrap().push(serde_json::from_str(&frame.payload).unwrap_or(Value::Null));
        true
    }
}

impl Sink {
    fn frames(&self, method: &str) -> Vec<Value> {
        self.0.lock().unwrap().iter().filter(|f| f["method"] == method).cloned().collect()
    }
}

struct Rig {
    hub: Arc<Hub>,
    link: Arc<FakeLink>,
    rows: HashMap<&'static str, Arc<dyn Handler>>,
    access: HashMap<&'static str, Access>,
    _dir: tempfile::TempDir,
}

impl Rig {
    async fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let declared = vec![SpaceDecl::new("sessions").default_space(), SpaceDecl::new("other")];
        let spaces = Spaces::load(dir.path().join("spaces.json"), declared, Default::default()).unwrap();
        let hub = Hub::new(HubConfig { instances: Arc::new(Book), spaces, turns: Default::default() });
        let cell = HubCell::default();
        let (mut rows, mut access) = (HashMap::new(), HashMap::new());
        for row in Hub::rows(&cell) {
            match row {
                HubRow::Method { name, access: a, handler, .. } => {
                    access.insert(name, a);
                    if let Some(h) = handler {
                        rows.insert(name, h);
                    }
                }
                HubRow::Topic { name, access: a } => {
                    access.insert(name, a);
                }
            }
        }
        let _ = cell.set(hub.clone());
        let link = FakeLink::new();
        hub.added("main", link.clone(), InstanceTrust::Product).await;
        Self { hub, link, rows, access, _dir: dir }
    }

    async fn call(&self, cx: CallContext, method: &str, params: Value) -> Rpc<Value> {
        self.rows.get(method).unwrap_or_else(|| panic!("no row `{method}`")).call(cx, params).await
    }

    async fn rate(&self, params: Value) -> Rpc<Value> {
        let mut p = json!({"session_id": "s1", "space": "sessions"});
        p.as_object_mut().unwrap().extend(params.as_object().unwrap().clone());
        self.call(CallContext::root(Subject::device("d"), None), "harnix.session.feedback", p).await
    }
}

async fn until(sink: &Sink, method: &str, n: usize) {
    let waited = tokio::time::timeout(Duration::from_secs(10), async {
        while sink.frames(method).len() < n {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await;
    assert!(waited.is_ok(), "{n} `{method}` never arrived: {:?}", sink.frames(method));
}

#[tokio::test]
async fn rated_changed_and_taken_back_each_files_an_entry_and_the_last_one_stands() {
    let rig = Rig::new().await;
    let filed = rig.rate(json!({"turn_id": "t1", "rating": "up"})).await.unwrap();
    assert_eq!(filed["feedback"]["by"], "device:d");
    assert_eq!(filed["entry"]["id"], "e1");
    assert_eq!(filed["entry"]["payload"], filed["feedback"]);

    rig.rate(json!({"turn_id": "t2", "rating": "up"})).await.unwrap();
    rig.rate(json!({"turn_id": "t1", "rating": "down", "category": "inaccurate", "note": "the wrong file"})).await.unwrap();
    let entries = rig.link.entries();
    assert!(entries.iter().all(|e| e["ns"] == FEEDBACK_NS && e["event"] == FEEDBACK_EVENT));
    let standing = current_feedback(&entries);
    assert_eq!(standing.len(), 2);
    assert_eq!((standing[0].turn_id.as_str(), standing[0].rating), ("t1", Some(Rating::Down)));
    assert_eq!(standing[0].note.as_deref(), Some("the wrong file"));

    rig.rate(json!({"turn_id": "t1", "rating": null})).await.unwrap();
    let entries = rig.link.entries();
    assert_eq!(entries.len(), 4, "the history stays in the log");
    let standing = current_feedback(&entries);
    assert_eq!(standing.iter().map(|f| f.turn_id.as_str()).collect::<Vec<_>>(), ["t2"]);
}

#[tokio::test]
async fn a_device_rates_only_a_session_of_the_space_it_claims() {
    let rig = Rig::new().await;
    let err = rig.rate(json!({"turn_id": "t1", "rating": "up", "space": "other"})).await.unwrap_err();
    assert_eq!(err.code, codes::SPACE_MISMATCH);
    assert!(rig.link.entries().is_empty(), "nothing filed for a session the device cannot see");
    let err = rig.rate(json!({"turn_id": "t1", "rating": "meh"})).await.unwrap_err();
    assert_eq!(err.code, codes::INVALID_PARAMS);
    assert!(rig.link.entries().is_empty());

    // The row: a device's alone, about one session; the topic: a holder of the capability's, never
    // a device's.
    let method = &rig.access["harnix.session.feedback"];
    assert!(method.opens_devices());
    assert!(!method.opens_every_extension() && method.extensions_listed().is_empty());
    assert_eq!(method.scope(), RowScope::Session);
    let topic = &rig.access[FEEDBACK_TOPIC];
    assert!(!topic.opens_devices());
    assert_eq!(topic.requirement(), Some(&caps::FEEDBACK_READ));
}

#[tokio::test]
async fn each_piece_of_feedback_is_told_on_the_topic_with_its_session_and_space() {
    let rig = Rig::new().await;
    let sink = Arc::new(Sink::default());
    let client = rig.hub.add_client(sink.clone()).await;
    let collector = CallContext::root(Subject::extension(ExtId::native("collector").unwrap()), Some(client));
    let subscribed = rig.call(collector, "harnix.topics.subscribe", json!({"topic": FEEDBACK_TOPIC})).await.unwrap();
    assert_eq!(subscribed["value"], Value::Null, "nothing given yet");

    rig.rate(json!({"turn_id": "t1", "rating": "down", "category": "slow"})).await.unwrap();
    until(&sink, "harnix.topic", 1).await;
    let told = &sink.frames("harnix.topic")[0]["params"];
    assert_eq!(told["topic"], FEEDBACK_TOPIC);
    let value = &told["value"];
    assert_eq!(
        (&value["instance"], &value["session_id"], &value["space"], &value["entry_id"]),
        (&json!("main"), &json!("s1"), &json!("sessions"), &json!("e1"))
    );
    assert_eq!(value["feedback"]["category"], "slow");
}
