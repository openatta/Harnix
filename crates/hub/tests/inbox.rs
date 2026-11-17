//! `harnix.inbox`: the asks every session is waiting on a person for, with the session's space,
//! name and project; within the space a call claims, unless that one reaches across spaces; gone
//! once answered.
//!
//! The links here are fakes: the hub reaches an engine through `EngineLink` and nothing else, and
//! hears an ask as the engine's event.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use harnix_contract::wire::Waiting;
use harnix_contract::{RpcError, SpaceDecl, Subject};
use harnix_engine_link::events::{EngineEvent, Prompt, SessionEventKind};
use harnix_engine_link::link::{EngineControl, EngineIdentity, EngineLink, EventReceiver, InstanceCarrier, LinkStatus};
use harnix_engine_link::ops::{
    Answer, EngineOps, Entry, HistoryPage, Said, SessionCreated, SessionInfo, SessionSetup, Subscribed, Tasks, TurnInput,
};
use harnix_hub::{Hub, HubCell, HubConfig, HubRow, Spaces};
use harnix_kernel::{CallContext, Handler, InstanceBook, InstanceObserver, InstanceTrust, Listed, Rpc};
use serde_json::{json, Value};
use tokio::sync::{mpsc, watch};

/// An engine whose sessions are named after themselves and work in `/work/<instance>`.
struct FakeLink {
    instance: &'static str,
    events: Mutex<Option<EventReceiver>>,
    status: watch::Sender<LinkStatus>,
    tx: mpsc::Sender<EngineEvent>,
}

impl FakeLink {
    fn new(instance: &'static str) -> Arc<Self> {
        let (tx, rx) = mpsc::channel(16);
        let (status, _) = watch::channel(LinkStatus::Ready);
        Arc::new(Self { instance, events: Mutex::new(Some(rx)), status, tx })
    }

    /// The session `session` asks a person: a tool's permission, or a question.
    async fn ask(&self, session: &str, prompt_id: &str, waiting: Waiting) {
        let value = match waiting {
            Waiting::Answer => json!({"kind": "prompt", "prompt_type": "question", "prompt_id": prompt_id, "question": "which?"}),
            _ => json!({"kind": "prompt", "prompt_id": prompt_id, "tool_name": "Bash", "tool_use_id": format!("u-{prompt_id}")}),
        };
        let prompt = Prompt::new(value.clone(), Some(prompt_id.into()), Some(format!("u-{prompt_id}")), waiting);
        let event = EngineEvent::Session { session: session.into(), turn_id: "t1".into(), kind: SessionEventKind::Asked(prompt), event: value };
        self.tx.send(event).await.unwrap();
    }

    /// The ask stopped waiting, as the engine says.
    async fn resolved(&self, session: &str, prompt_id: &str) {
        let kind = SessionEventKind::AskResolved { prompt_id: prompt_id.into(), answered: true };
        let event = json!({"kind": "ask_resolved", "prompt_id": prompt_id});
        self.tx.send(EngineEvent::Session { session: session.into(), turn_id: "t1".into(), kind, event }).await.unwrap();
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
        let root = format!("/work/{}", self.instance);
        Ok(SessionInfo::new(json!({"session_id": session, "name": format!("named {session}"), "project_root": root, "turn_state": "running"})))
    }
    async fn list(&self) -> Answer<Vec<SessionInfo>> {
        Ok(Vec::new())
    }
    async fn subscribe(&self, _: &str) -> Answer<Option<Subscribed>> {
        Ok(None)
    }
    async fn resume(&self, _: &str, _: &SessionInfo) -> Answer<()> {
        Ok(())
    }
    async fn history_len(&self, _: &str) -> Answer<u64> {
        Ok(0)
    }
    async fn transcript(&self, _: &str, _: usize) -> Answer<Vec<Said>> {
        Ok(Vec::new())
    }
    async fn history(&self, _: &str, _: u64, _: usize) -> Answer<HistoryPage> {
        Ok(HistoryPage::new(0, Vec::new()))
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
    async fn append_entry(&self, _: &str, _: Entry) -> Answer<Value> {
        Ok(json!({}))
    }
    async fn respond(&self, _: Value) -> Answer<Value> {
        Ok(json!({}))
    }
    async fn rename(&self, _: &str, _: &str) -> Answer<Value> {
        Ok(json!({}))
    }
    async fn set_model(&self, _: &str, _: &str, _: Option<&str>) -> Answer<Value> {
        Ok(json!({}))
    }
    async fn set_prompt(&self, _: &str, _: Option<Option<String>>, _: Option<Option<String>>) -> Answer<Value> {
        Ok(json!({}))
    }
    async fn set_skill_dirs(&self, _: &str, _: &[std::path::PathBuf]) -> Answer<Value> {
        Ok(json!({}))
    }
    async fn passthrough(&self, _: &str, _: Value) -> Answer<Value> {
        Ok(json!({}))
    }
}

/// `board` is the space `acme`'s, `desk` the space `other`'s.
struct Book;

#[async_trait::async_trait]
impl InstanceBook for Book {
    fn listed(&self, id: &str) -> Option<Listed> {
        let space = match id {
            "board" => "acme",
            "desk" => "other",
            _ => return None,
        };
        Some(Listed { id: id.into(), spaces: vec![space.into()], extension: None })
    }
    fn all(&self) -> Vec<Listed> {
        ["board", "desk"].iter().filter_map(|id| self.listed(id)).collect()
    }
    async fn ensure_for_space(&self, _: &str, _: &str, _: &str) -> Rpc<()> {
        Ok(())
    }
    fn flooding(&self, _: &str, _: u32) {}
}

struct Rig {
    board: Arc<FakeLink>,
    desk: Arc<FakeLink>,
    rows: HashMap<&'static str, Arc<dyn Handler>>,
    access: HashMap<&'static str, harnix_contract::s1::Access>,
    topics: Vec<&'static str>,
    _hub: Arc<Hub>,
    _dir: tempfile::TempDir,
}

impl Rig {
    async fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let declared = vec![SpaceDecl::new("acme"), SpaceDecl::new("other"), SpaceDecl::new("console").cross_space()];
        let spaces = Spaces::load(dir.path().join("spaces.json"), declared, Default::default()).unwrap();
        let hub = Hub::new(HubConfig { instances: Arc::new(Book), spaces, turns: Default::default() });
        let cell = HubCell::default();
        let (mut rows, mut access, mut topics) = (HashMap::new(), HashMap::new(), Vec::new());
        for row in Hub::rows(&cell) {
            match row {
                HubRow::Method { name, access: a, handler: Some(h), .. } => {
                    rows.insert(name, h);
                    access.insert(name, a);
                }
                HubRow::Topic { name, .. } => topics.push(name),
                _ => {}
            }
        }
        let _ = cell.set(hub.clone());
        let (board, desk) = (FakeLink::new("board"), FakeLink::new("desk"));
        hub.added("board", board.clone(), InstanceTrust::Product).await;
        hub.added("desk", desk.clone(), InstanceTrust::Product).await;
        Self { board, desk, rows, access, topics, _hub: hub, _dir: dir }
    }

    async fn inbox(&self, params: Value) -> Value {
        let device = CallContext::root(Subject::device("d"), None);
        self.rows["harnix.inbox"].call(device, params).await.unwrap()
    }

    /// The inbox once it has `n` items: the events reach the hub on their own task.
    async fn inbox_of(&self, n: usize) -> Value {
        for _ in 0..200 {
            let inbox = self.inbox(json!({})).await;
            if inbox["items"].as_array().map(Vec::len) == Some(n) {
                return inbox;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("the inbox never had {n} items: {}", self.inbox(json!({})).await);
    }
}

fn sessions(inbox: &Value) -> Vec<(String, String, String)> {
    inbox["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| (i["instance"].as_str().unwrap().into(), i["session_id"].as_str().unwrap().into(), i["waiting"].as_str().unwrap().into()))
        .collect()
}

#[tokio::test]
async fn every_ask_waiting_is_listed_with_its_session_and_goes_once_resolved() {
    let rig = Rig::new().await;
    assert_eq!(rig.inbox(json!({})).await, json!({"items": []}));
    rig.board.ask("s1", "p1", Waiting::Approval).await;
    rig.board.ask("s1", "p2", Waiting::Answer).await;
    rig.desk.ask("s9", "p3", Waiting::Approval).await;
    let inbox = rig.inbox_of(3).await;
    let s = |i: &str, s: &str, w: &str| (i.to_string(), s.to_string(), w.to_string());
    assert_eq!(sessions(&inbox), [s("board", "s1", "approval"), s("board", "s1", "answer"), s("desk", "s9", "approval")]);
    let first = &inbox["items"][0];
    assert_eq!(first["space"], "acme");
    assert_eq!(first["name"], "named s1");
    assert_eq!(first["project_root"], "/work/board");
    assert_eq!(first["started_by"], "user");
    assert_eq!(first["prompt"]["prompt_id"], "p1");
    assert_eq!(first["prompt"]["tool_name"], "Bash");
    assert!(first.get("trust").is_none(), "{first}");

    // Answered: it waits on nobody.
    rig.board.resolved("s1", "p1").await;
    let inbox = rig.inbox_of(2).await;
    assert_eq!(sessions(&inbox), [s("board", "s1", "answer"), s("desk", "s9", "approval")]);
}

#[tokio::test]
async fn a_call_claiming_a_space_sees_its_asks_and_one_reaching_across_sees_all() {
    let rig = Rig::new().await;
    rig.board.ask("s1", "p1", Waiting::Approval).await;
    rig.desk.ask("s9", "p2", Waiting::Plan).await;
    rig.inbox_of(2).await;
    let only = |inbox: Value| sessions(&inbox).into_iter().map(|(i, _, _)| i).collect::<Vec<_>>();
    assert_eq!(only(rig.inbox(json!({"space": "acme"})).await), ["board"]);
    assert_eq!(only(rig.inbox(json!({"space": "other"})).await), ["desk"]);
    assert_eq!(only(rig.inbox(json!({"space": "console"})).await), ["board", "desk"]);
    assert_eq!(only(rig.inbox(json!({"space": "nowhere"})).await), Vec::<String>::new());
}

#[tokio::test]
async fn the_method_and_the_topic_are_open_to_devices_and_narrowed_to_a_space() {
    let rig = Rig::new().await;
    let access = &rig.access["harnix.inbox"];
    assert!(access.opens_devices() && access.opens_every_extension());
    assert_eq!(access.scope(), harnix_contract::s1::RowScope::Space);
    assert!(rig.topics.contains(&"harnix.inbox"));
}
