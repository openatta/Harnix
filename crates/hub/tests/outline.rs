//! `harnix.session.outline`: a session's turns read off its transcript, read on from the outline
//! the hub kept while the transcript only grows, read again when it changed under it; open to a
//! device for the sessions of the space it claims.
//!
//! The link here is a fake: the hub reaches an engine through `EngineLink` and nothing else.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use harnix_contract::{codes, RpcError, SpaceDecl, Subject};
use harnix_engine_link::events::EngineEvent;
use harnix_engine_link::link::{EngineControl, EngineIdentity, EngineLink, EventReceiver, InstanceCarrier, LinkStatus};
use harnix_engine_link::ops::{
    Answer, EngineOps, Entry, HistoryPage, Message, Said, SessionCreated, SessionInfo, SessionSetup, Subscribed, Tasks, TurnInput,
};
use harnix_hub::{Hub, HubCell, HubConfig, HubRow, Spaces};
use harnix_kernel::{CallContext, Handler, InstanceBook, InstanceObserver, InstanceTrust, Listed, Rpc};
use serde_json::{json, Value};
use tokio::sync::{mpsc, watch};

/// An engine whose one session's transcript is what the test writes into it.
struct FakeLink {
    events: Mutex<Option<EventReceiver>>,
    status: watch::Sender<LinkStatus>,
    log: Mutex<Vec<Message>>,
    /// Each history read: from where.
    reads: Mutex<Vec<u64>>,
    _tx: mpsc::Sender<EngineEvent>,
}

impl FakeLink {
    fn new() -> Arc<Self> {
        let (tx, rx) = mpsc::channel(16);
        let (status, _) = watch::channel(LinkStatus::Ready);
        Arc::new(Self { events: Mutex::new(Some(rx)), status, log: Mutex::default(), reads: Mutex::default(), _tx: tx })
    }

    /// A person asks `prompt` in turn `turn` and is answered `reply`, as the next messages; the row
    /// ids are `<id>-u` and `<id>-a`.
    fn turn(&self, id: &str, prompt: &str, reply: &str) {
        let mut log = self.log.lock().unwrap();
        let seq = log.len() as u64;
        log.push(Message::new(seq, "user", prompt).origin("user").turn(id).id(format!("{id}-u")));
        log.push(Message::new(seq + 1, "assistant", reply).origin("assistant").turn(id).id(format!("{id}-a")));
    }

    fn reads(&self) -> Vec<u64> {
        std::mem::take(&mut *self.reads.lock().unwrap())
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
        Ok(SessionInfo::new(json!({"session_id": session, "turn_state": "idle"})))
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
        Ok(self.log.lock().unwrap().len() as u64)
    }
    async fn transcript(&self, _: &str, _: usize) -> Answer<Vec<Said>> {
        Ok(Vec::new())
    }
    async fn history(&self, session: &str, offset: u64, limit: usize) -> Answer<HistoryPage> {
        if session != "s1" {
            return Err(RpcError::new(-32000, "no such session"));
        }
        self.reads.lock().unwrap().push(offset);
        let log = self.log.lock().unwrap();
        let page = log.iter().skip(offset as usize).take(limit).cloned().collect();
        Ok(HistoryPage::new(log.len() as u64, page))
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

/// One instance, `board`, whose sessions are the space `acme`'s.
struct Book;

#[async_trait::async_trait]
impl InstanceBook for Book {
    fn listed(&self, id: &str) -> Option<Listed> {
        (id == "board").then(|| Listed { id: id.into(), spaces: vec!["acme".into()], extension: None })
    }
    fn all(&self) -> Vec<Listed> {
        self.listed("board").into_iter().collect()
    }
    async fn ensure_for_space(&self, _: &str, _: &str, _: &str) -> Rpc<()> {
        Ok(())
    }
    fn flooding(&self, _: &str, _: u32) {}
}

struct Rig {
    link: Arc<FakeLink>,
    rows: HashMap<&'static str, Arc<dyn Handler>>,
    access: HashMap<&'static str, harnix_contract::s1::Access>,
    _hub: Arc<Hub>,
    _dir: tempfile::TempDir,
}

impl Rig {
    async fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let declared = vec![SpaceDecl::new("acme"), SpaceDecl::new("other")];
        let spaces = Spaces::load(dir.path().join("spaces.json"), declared, Default::default()).unwrap();
        let hub = Hub::new(HubConfig { instances: Arc::new(Book), spaces, turns: Default::default() });
        let cell = HubCell::default();
        let mut rows = HashMap::new();
        let mut access = HashMap::new();
        for row in Hub::rows(&cell) {
            if let HubRow::Method { name, access: a, handler: Some(h), .. } = row {
                rows.insert(name, h);
                access.insert(name, a);
            }
        }
        let _ = cell.set(hub.clone());
        let link = FakeLink::new();
        hub.added("board", link.clone(), InstanceTrust::Product).await;
        Self { link, rows, access, _hub: hub, _dir: dir }
    }

    async fn outline(&self, params: Value) -> Rpc<Value> {
        let device = CallContext::root(Subject::device("d"), None);
        self.rows["harnix.session.outline"].call(device, params).await
    }
}

fn of(outline: &Value) -> Vec<(String, u64, String, String)> {
    outline["turns"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| {
            (
                t["turn_id"].as_str().unwrap().to_string(),
                t["seq"].as_u64().unwrap(),
                t["prompt"].as_str().unwrap().to_string(),
                t["reply"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

fn row(turn: &str, seq: u64, prompt: &str, reply: &str) -> (String, u64, String, String) {
    (turn.into(), seq, prompt.into(), reply.into())
}

#[tokio::test]
async fn the_outline_is_the_transcripts_turns_and_new_turns_are_read_on_from_the_last() {
    let rig = Rig::new().await;
    let key = json!({"instance": "board", "session_id": "s1"});
    rig.link.turn("t1", "first question", "first answer");
    rig.link.turn("t2", &"long ".repeat(100), "second answer");
    let outline = rig.outline(key.clone()).await.unwrap();
    assert_eq!(outline["instance"], "board");
    assert_eq!(outline["total"], 4);
    let prompt = outline["turns"][1]["prompt"].as_str().unwrap();
    assert_eq!(prompt.chars().count(), harnix_hub::PROMPT_CHARS);
    assert!(prompt.ends_with('…'));
    assert_eq!(of(&outline)[0], row("t1", 0, "first question", "first answer"));
    assert!(outline["turns"][0].get("marks").is_none(), "{outline}");
    assert_eq!(rig.link.reads(), [0]);

    // A turn settles: read on from the start of the last turn kept, which may have grown.
    rig.link.turn("t3", "third", "third answer");
    let outline = rig.outline(key.clone()).await.unwrap();
    assert_eq!(of(&outline).len(), 3);
    assert_eq!(of(&outline)[2], row("t3", 4, "third", "third answer"));
    assert_eq!(rig.link.reads(), [2]);

    // The transcript changed under it (compacted, edited): the last turn is not where it was, so
    // the outline is read again from the start.
    {
        let mut log = rig.link.log.lock().unwrap();
        log.drain(0..2);
        for (seq, m) in log.iter_mut().enumerate() {
            m.seq = seq as u64;
        }
    }
    let outline = rig.outline(key).await.unwrap();
    assert_eq!(of(&outline).iter().map(|r| r.1).collect::<Vec<_>>(), [0, 2]);
    assert_eq!(rig.link.reads(), [4, 0]);
}

#[tokio::test]
async fn a_device_reads_the_outline_of_its_spaces_sessions_only() {
    let rig = Rig::new().await;
    rig.link.turn("t1", "q", "a");
    let access = &rig.access["harnix.session.outline"];
    assert!(access.opens_devices() && access.opens_every_extension());
    assert_eq!(access.scope(), harnix_contract::s1::RowScope::Session);

    let claiming = |space: &str| json!({"instance": "board", "session_id": "s1", "space": space});
    assert_eq!(of(&rig.outline(claiming("acme")).await.unwrap()).len(), 1);
    let refused = rig.outline(claiming("other")).await.unwrap_err();
    assert_eq!(refused.code, codes::SPACE_MISMATCH);
    // What the engine does not have is its error, and an instance nobody has is refused by name.
    assert!(rig.outline(json!({"instance": "board", "session_id": "nope"})).await.is_err());
    let missing = rig.outline(json!({"instance": "gone", "session_id": "s1"})).await.unwrap_err();
    assert_eq!(missing.code, codes::INSTANCE_NOT_FOUND);
    let unnamed = rig.outline(json!({"instance": "board"})).await.unwrap_err();
    assert_eq!(unnamed.code, codes::INVALID_PARAMS);
}
