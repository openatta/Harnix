//! `harnix.session.rewind`: a device rewinds a session of the space it claims; going back to a
//! message is going back to where the turn before it ended; a dry run changes nothing and tells
//! nobody; a rewind voids the changes of the turns it took out, empties what the hub had drawn past
//! its point, and has every watcher read the session again — once, whether the engine's frame for
//! it or the answer arrives first. A rewind somebody else made is followed up the same way.
//!
//! The link here is a fake: the hub reaches an engine through `EngineLink` and nothing else.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use harnix_contract::s1::{caps, RowScope};
use harnix_contract::wire::{RemovedTurn, RewindParams, RewindResult, RewindTo, RewoundConversation};
use harnix_contract::{codes, Frame, FrameSink, RpcError, SpaceDecl, Subject};
use harnix_engine_link::events::{DaemonEventKind, EngineEvent, SessionEventKind};
use harnix_engine_link::link::{EngineControl, EngineIdentity, EngineLink, EventReceiver, InstanceCarrier, LinkStatus};
use harnix_engine_link::ops::{Answer, EngineOps, Entry, HistoryPage, Said, SessionCreated, SessionInfo, SessionSetup, Subscribed, Tasks, TurnInput};
use harnix_hub::{Hub, HubCell, HubConfig, HubRow, Spaces};
use harnix_kernel::{CallContext, Handler, InstanceBook, InstanceObserver, Listed, Rpc};
use serde_json::{json, Value};
use tokio::sync::{mpsc, watch};

/// An engine whose session `s1` has three live turns, and which says its rewinds as Agix does:
/// the frame to subscribers, then the answer. It goes on listing the turns a rewind took out, as
/// Agix does a turn that wrote files.
struct FakeLink {
    events: Mutex<Option<EventReceiver>>,
    tx: mpsc::Sender<EngineEvent>,
    status: watch::Sender<LinkStatus>,
    asked: Mutex<Vec<RewindParams>>,
    appended: Mutex<Vec<Entry>>,
}

impl FakeLink {
    fn new() -> Arc<Self> {
        let (tx, rx) = mpsc::channel(1024);
        let (status, _) = watch::channel(LinkStatus::Ready);
        Arc::new(Self { events: Mutex::new(Some(rx)), tx, status, asked: Mutex::default(), appended: Mutex::default() })
    }
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
        InstanceCarrier::Adopted
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
    async fn mark(&self, id: u64) {
        let _ = self.tx.send(EngineEvent::Barrier { id }).await;
    }
}

#[async_trait::async_trait]
impl EngineOps for FakeLink {
    async fn create(&self, _: Value, _: SessionSetup) -> Answer<SessionCreated> {
        Ok(SessionCreated::new(json!({"session_id": "s1"})))
    }
    async fn get(&self, session: &str) -> Answer<SessionInfo> {
        Ok(SessionInfo::new(json!({"session_id": session, "turn_state": "idle"})))
    }
    async fn list(&self) -> Answer<Vec<SessionInfo>> {
        Ok(Vec::new())
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
        self.appended.lock().unwrap().push(entry);
        Ok(json!({}))
    }
    async fn respond(&self, _: Value) -> Answer<Value> {
        Ok(json!({}))
    }
    async fn rename(&self, _: &str, _: &str) -> Answer<Value> {
        Ok(json!({}))
    }
    async fn set_model(&self, _: &str, _: &harnix_contract::wire::ModelChoice) -> Answer<Value> {
        Ok(json!({}))
    }
    async fn set_prompt(&self, _: &str, _: Option<Option<String>>, _: Option<Option<String>>) -> Answer<Value> {
        Ok(json!({}))
    }
    async fn set_skill_dirs(&self, _: &str, _: &[std::path::PathBuf]) -> Answer<Value> {
        Ok(json!({}))
    }
    async fn passthrough(&self, _: &str, _: Value) -> Answer<Value> {
        let turn = |id: &str| json!({"ns": "harnix.changes", "event": "turn", "payload": {"turn_id": id, "files": []}, "at": 2});
        let mut entries = vec![turn("t1"), turn("t2"), turn("t3")];
        let filed = self.appended.lock().unwrap().iter().map(|e| json!({"ns": e.ns, "event": e.event, "payload": e.payload, "at": 2})).collect::<Vec<_>>();
        entries.extend(filed);
        Ok(json!({"entries": entries}))
    }
    async fn rewind(&self, session: &str, ask: &RewindParams) -> Answer<RewindResult> {
        self.asked.lock().unwrap().push(ask.clone());
        if !ask.dry_run {
            let event = json!({"kind": "rewound", "seq": 2});
            let kind = SessionEventKind::Rewound { messages: 2 };
            let _ = self.tx.send(EngineEvent::Session { session: session.into(), turn_id: String::new(), kind, event }).await;
        }
        Ok(RewindResult {
            instance: String::new(),
            session_id: session.into(),
            rewind_id: "r".into(),
            dry_run: ask.dry_run,
            scope: ask.scope.unwrap_or(harnix_contract::wire::RewindScope::Both),
            to: ask.to.clone(),
            conversation: Some(RewoundConversation {
                messages_before: 6,
                messages_after: 2,
                removed_turns: vec![
                    RemovedTurn { turn_id: "t2".into(), prompt: "fix a".into() },
                    RemovedTurn { turn_id: "t3".into(), prompt: "and b".into() },
                ],
            }),
            files: None,
            fork_session_id: ask.fork_first.then(|| "s9".to_string()),
        })
    }
    async fn live_turns(&self, _: &str) -> Answer<Vec<String>> {
        Ok(vec!["t1".into(), "t2".into(), "t3".into()])
    }
}

struct Book;

#[async_trait::async_trait]
impl InstanceBook for Book {
    fn listed(&self, _: &str) -> Option<Listed> {
        None
    }
    fn all(&self) -> Vec<Listed> {
        Vec::new()
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
    rows: HashMap<&'static str, Arc<dyn Handler>>,
    access: HashMap<&'static str, harnix_kernel::Access>,
    link: Arc<FakeLink>,
    _dir: tempfile::TempDir,
}

impl Rig {
    /// Spaces `a` (the default, where `s1` is), `b`, and `system` reaching across.
    async fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let declared = vec![SpaceDecl::new("a").default_space(), SpaceDecl::new("b"), SpaceDecl::new("system").cross_space()];
        let spaces = Spaces::load(dir.path().join("spaces.json"), declared, Default::default()).unwrap();
        let hub = Hub::new(HubConfig { instances: Arc::new(Book), spaces, turns: Default::default() });
        let cell = HubCell::default();
        let (mut rows, mut access) = (HashMap::new(), HashMap::new());
        for row in Hub::rows(&cell) {
            if let HubRow::Method { name, handler: Some(h), access: a, .. } = row {
                rows.insert(name, h);
                access.insert(name, a);
            }
        }
        let _ = cell.set(hub.clone());
        let link = FakeLink::new();
        hub.added("main", link.clone(), harnix_kernel::InstanceTrust::Product).await;
        Self { hub, rows, access, link, _dir: dir }
    }

    async fn call(&self, cx: CallContext, method: &str, params: Value) -> Rpc<Value> {
        self.rows.get(method).unwrap_or_else(|| panic!("no row `{method}`")).call(cx, params).await
    }

    /// A device watching `s1`.
    async fn watcher(&self) -> (Arc<Sink>, CallContext) {
        let sink = Arc::new(Sink::default());
        let client = self.hub.add_client(sink.clone()).await;
        let cx = CallContext::root(Subject::device("d"), Some(client));
        self.call(cx.clone(), "harnix.session.attach", json!({"session_id": "s1", "space": "a"})).await.unwrap();
        (sink, cx)
    }
}

fn device() -> CallContext {
    CallContext::root(Subject::device("d"), None)
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
async fn a_device_reaches_it_and_an_extension_holding_engine_control() {
    let rig = Rig::new().await;
    let access = &rig.access["harnix.session.rewind"];
    assert!(access.opens_devices() && access.opens_every_extension());
    assert_eq!(access.requirement(), Some(&caps::ENGINE_CONTROL));
    assert_eq!(access.scope(), RowScope::Session);
}

#[tokio::test]
async fn a_device_rewinds_only_a_session_of_the_space_it_claims() {
    let rig = Rig::new().await;
    let ask = |space: &str| json!({"session_id": "s1", "space": space, "to": "start", "dry_run": true});
    rig.call(device(), "harnix.session.rewind", ask("a")).await.unwrap();
    let other = rig.call(device(), "harnix.session.rewind", ask("b")).await.unwrap_err();
    assert_eq!(other.code, codes::SPACE_MISMATCH, "{other:?}");
    let across = rig.call(device(), "harnix.session.rewind", ask("system")).await.unwrap_err();
    assert_eq!(across.code, codes::SPACE_MISMATCH, "{across:?}");
    assert_eq!(rig.link.asked.lock().unwrap().len(), 1, "nothing reached the engine for the refused two");
}

#[tokio::test]
async fn a_dry_run_goes_back_to_before_a_message_and_changes_nothing() {
    let rig = Rig::new().await;
    let (sink, _) = rig.watcher().await;
    let plan = rig
        .call(device(), "harnix.session.rewind", json!({"session_id": "s1", "space": "a", "to": {"before_turn": "t2"}, "dry_run": true}))
        .await
        .unwrap();
    let asked = rig.link.asked.lock().unwrap().clone();
    assert_eq!(asked[0].to, RewindTo::Turn("t1".into()), "the end of the turn before it");
    assert!(asked[0].dry_run);
    assert_eq!((plan["instance"].as_str(), plan["dry_run"].as_bool()), (Some("main"), Some(true)));
    assert_eq!(plan["to"], json!({"turn": "t1"}));
    assert_eq!(plan["conversation"]["removed_turns"][0], json!({"turn_id": "t2", "prompt": "fix a"}));
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(sink.frames("harnix.session_resync").is_empty());
    assert!(rig.link.appended.lock().unwrap().is_empty());

    rig.call(device(), "harnix.session.rewind", json!({"session_id": "s1", "space": "a", "to": {"before_turn": "t1"}, "dry_run": true}))
        .await
        .unwrap();
    assert_eq!(rig.link.asked.lock().unwrap()[1].to, RewindTo::Start, "before the first turn is the start");
    let gone = rig
        .call(device(), "harnix.session.rewind", json!({"session_id": "s1", "space": "a", "to": {"before_turn": "t9"}}))
        .await
        .unwrap_err();
    assert_eq!(gone.code, codes::INVALID_PARAMS);
}

#[tokio::test]
async fn a_rewind_voids_its_turns_changes_and_every_watcher_reads_the_session_again_once() {
    let rig = Rig::new().await;
    let (sink, cx) = rig.watcher().await;
    let done = rig
        .call(device(), "harnix.session.rewind", json!({"session_id": "s1", "space": "a", "to": {"before_turn": "t2"}}))
        .await
        .unwrap();
    assert_eq!(done["dry_run"], false);
    until(&sink, "harnix.session_resync", 1).await;
    // The engine's frame went to the watcher; it is not a second resync.
    until(&sink, "harnix.event", 1).await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(sink.frames("harnix.session_resync").len(), 1);
    let appended = rig.link.appended.lock().unwrap().clone();
    assert_eq!(appended.len(), 1);
    assert_eq!((appended[0].ns.as_str(), appended[0].event.as_str()), ("harnix.changes", "rewind"));
    assert_eq!(appended[0].payload, json!({"rewind_id": "r", "turn_ids": ["t2", "t3"]}));
    // Attaching again: nothing drawn past the point comes back from the buffer.
    let attached = rig.call(cx, "harnix.session.attach", json!({"session_id": "s1", "space": "a"})).await.unwrap();
    assert_eq!(attached["replay"], json!([]));
    assert_eq!(attached["history_total"], 2);
    // And the entries listed leave out the changes of the turns it took out.
    let listed = rig.call(device(), "engine.session.entries", json!({"session_id": "s1", "space": "a"})).await.unwrap();
    let turns: Vec<&str> = listed["entries"].as_array().unwrap().iter().filter_map(|e| e["payload"]["turn_id"].as_str()).collect();
    assert_eq!(turns, ["t1"]);
    // A turn it took out is no target again, though the engine still lists it; the one before is.
    let again = json!({"session_id": "s1", "space": "a", "to": {"before_turn": "t2"}, "dry_run": true});
    assert_eq!(rig.call(device(), "harnix.session.rewind", again).await.unwrap_err().code, codes::INVALID_PARAMS);
    let first = json!({"session_id": "s1", "space": "a", "to": {"before_turn": "t1"}, "dry_run": true});
    rig.call(device(), "harnix.session.rewind", first).await.unwrap();
}

#[tokio::test]
async fn a_fork_made_first_lands_in_the_sessions_space() {
    let rig = Rig::new().await;
    let created = rig.call(device(), "harnix.session.create", json!({"instance": "main", "space": "b"})).await.unwrap();
    assert_eq!(created["space"], "b");
    let done = rig
        .call(device(), "harnix.session.rewind", json!({"session_id": "s1", "space": "b", "to": "start", "fork_first": true}))
        .await
        .unwrap();
    assert_eq!(done["fork_session_id"], "s9");
    let dry = |space: &str| json!({"session_id": "s9", "space": space, "to": "start", "dry_run": true});
    rig.call(device(), "harnix.session.rewind", dry("b")).await.unwrap();
    assert_eq!(rig.call(device(), "harnix.session.rewind", dry("a")).await.unwrap_err().code, codes::SPACE_MISMATCH);
}

#[tokio::test]
async fn not_while_a_turn_runs() {
    let rig = Rig::new().await;
    let _watching = rig.watcher().await;
    let started = EngineEvent::Daemon {
        kind: DaemonEventKind::TurnStarted { session: "s1".into(), turn_id: "t4".into() },
        event: json!({"kind": "turn_started", "session_id": "s1", "turn_id": "t4"}),
    };
    rig.link.tx.send(started).await.unwrap();
    let busy = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            match rig.call(device(), "harnix.session.rewind", json!({"session_id": "s1", "space": "a", "to": "start", "dry_run": true})).await {
                Err(e) => return e,
                Ok(_) => tokio::time::sleep(Duration::from_millis(5)).await,
            }
        }
    })
    .await
    .expect("refused once the turn is heard of");
    assert_eq!(busy.code, harnix_engine_link::ops::codes::SESSION_BUSY);
}

#[tokio::test]
async fn a_rewind_somebody_else_made_is_followed_up_too() {
    let rig = Rig::new().await;
    let (sink, _) = rig.watcher().await;
    let event = json!({"kind": "rewound", "seq": 2});
    let frame = EngineEvent::Session { session: "s1".into(), turn_id: String::new(), kind: SessionEventKind::Rewound { messages: 2 }, event };
    rig.link.tx.send(frame).await.unwrap();
    until(&sink, "harnix.session_resync", 1).await;
}
