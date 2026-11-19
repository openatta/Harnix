//! A session's permission mode: a device changes it for a session of the space it claims, the
//! answer says the mode it is in and whether the settings narrowed it, and the mode the hub last
//! heard — read when the session is opened, answered, or changed by the engine on its own (the
//! model's plan mode) — is on the session's row and its attach.
//!
//! The link here is a fake: the hub reaches an engine through `EngineLink` and nothing else.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use harnix_contract::{codes, Frame, FrameSink, RpcError, SpaceDecl, Subject};
use harnix_engine_link::events::{EngineEvent, SessionEventKind};
use harnix_engine_link::link::{EngineControl, EngineIdentity, EngineLink, EventReceiver, InstanceCarrier, LinkStatus};
use harnix_engine_link::ops::{
    Answer, EngineOps, Entry, HistoryPage, PermissionModeChoice, Said, SessionCreated, SessionInfo, SessionSetup, Subscribed, Tasks, TurnInput,
};
use harnix_hub::{Hub, HubCell, HubConfig, HubRow, Spaces};
use harnix_kernel::{Access, CallContext, Handler, InstanceBook, InstanceObserver, InstanceTrust, Listed, RowScope, Rpc};
use serde_json::{json, Value};
use tokio::sync::{mpsc, watch};

/// An engine whose settings allow nothing wider than `acceptEdits`, and whose sessions start in
/// `default`.
struct FakeLink {
    events: Mutex<Option<EventReceiver>>,
    status: watch::Sender<LinkStatus>,
    set: Mutex<Vec<(String, String)>>,
    reads: Mutex<usize>,
    tx: mpsc::Sender<EngineEvent>,
}

impl FakeLink {
    fn new() -> Arc<Self> {
        let (tx, rx) = mpsc::channel(16);
        let (status, _) = watch::channel(LinkStatus::Ready);
        Arc::new(Self { events: Mutex::new(Some(rx)), status, set: Mutex::default(), reads: Mutex::default(), tx })
    }

    fn set(&self) -> Vec<(String, String)> {
        self.set.lock().unwrap().clone()
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
        Ok(0)
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
    async fn append_entry(&self, _: &str, _: Entry) -> Answer<Value> {
        Ok(json!({}))
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
    async fn set_permission_mode(&self, session: &str, mode: &str) -> Answer<PermissionModeChoice> {
        self.set.lock().unwrap().push((session.to_string(), mode.to_string()));
        Ok(match mode {
            "bypassPermissions" => PermissionModeChoice::new("acceptEdits", mode, true),
            _ => PermissionModeChoice::new(mode, mode, false),
        })
    }
    async fn permission_mode(&self, _: &str, _: &SessionInfo) -> Answer<Option<String>> {
        *self.reads.lock().unwrap() += 1;
        Ok(Some("default".into()))
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

    async fn device(&self, method: &str, params: Value) -> Rpc<Value> {
        self.call(CallContext::root(Subject::device("d"), None), method, params).await
    }

    /// The session's row in its instance's list.
    async fn row(&self) -> Value {
        let listed = self.device("harnix.sessions.list", json!({"instance": "main"})).await.unwrap();
        listed["sessions"].as_array().unwrap().iter().find(|r| r["session_id"] == "s1").cloned().unwrap()
    }
}

async fn until(what: &str, mut done: impl FnMut() -> bool) {
    let waited = tokio::time::timeout(Duration::from_secs(10), async {
        while !done() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await;
    assert!(waited.is_ok(), "{what} never happened");
}

#[tokio::test]
async fn a_device_sets_a_sessions_mode_and_is_told_what_it_is_in() {
    let rig = Rig::new().await;
    let set = rig.device("harnix.session.setPermissionMode", json!({"session_id": "s1", "mode": "plan", "space": "sessions"})).await.unwrap();
    assert_eq!(set, json!({"instance": "main", "session_id": "s1", "mode": "plan", "requested": "plan", "clamped": false}));
    assert_eq!(rig.row().await["permission_mode"], "plan");

    // Wider than the settings allow: held to what they do, and said so.
    let set = rig.device("harnix.session.setPermissionMode", json!({"session_id": "s1", "mode": "bypassPermissions"})).await.unwrap();
    assert_eq!((&set["mode"], &set["requested"], &set["clamped"]), (&json!("acceptEdits"), &json!("bypassPermissions"), &json!(true)));
    assert_eq!(rig.row().await["permission_mode"], "acceptEdits");
    assert_eq!(rig.link.set(), [("s1".to_string(), "plan".to_string()), ("s1".to_string(), "bypassPermissions".to_string())]);
}

#[tokio::test]
async fn only_a_session_of_the_space_claimed_and_only_a_mode_named() {
    let rig = Rig::new().await;
    let err = rig.device("harnix.session.setPermissionMode", json!({"session_id": "s1", "mode": "plan", "space": "other"})).await.unwrap_err();
    assert_eq!(err.code, codes::SPACE_MISMATCH);
    for params in [json!({"session_id": "s1", "mode": " "}), json!({"session_id": "s1"})] {
        let err = rig.device("harnix.session.setPermissionMode", params.clone()).await.unwrap_err();
        assert_eq!(err.code, codes::INVALID_PARAMS, "{params}");
    }
    assert!(rig.link.set().is_empty(), "nothing reached the engine");

    // A person's choice: a device's alone, about one session.
    let access = &rig.access["harnix.session.setPermissionMode"];
    assert!(access.opens_devices());
    assert!(!access.opens_every_extension() && access.extensions_listed().is_empty());
    assert_eq!(access.scope(), RowScope::Session);
}

#[tokio::test]
async fn the_mode_is_read_once_when_opened_and_follows_the_engines_changes() {
    let rig = Rig::new().await;
    assert!(rig.row().await.get("permission_mode").is_none(), "nothing heard yet");
    let sink = Arc::new(Sink::default());
    let client = rig.hub.add_client(sink.clone()).await;
    let cx = || CallContext::root(Subject::device("d"), Some(client));
    let attached = rig.call(cx(), "harnix.session.attach", json!({"session_id": "s1"})).await.unwrap();
    assert_eq!(attached["session"]["permission_mode"], "default");
    rig.call(cx(), "harnix.session.attach", json!({"session_id": "s1"})).await.unwrap();
    assert_eq!(*rig.link.reads.lock().unwrap(), 1, "read once, then followed");

    // The model went into plan mode: the event reaches the watcher as the engine said it, and the
    // row and the next attach say the mode it is in now.
    let event = json!({"kind": "permission_mode_changed", "mode": "plan", "source": "model"});
    let frame = EngineEvent::Session { session: "s1".into(), turn_id: String::new(), kind: SessionEventKind::PermissionModeChanged { mode: "plan".into() }, event: event.clone() };
    rig.link.tx.send(frame).await.unwrap();
    until("the change reaching the watcher", || sink.frames("harnix.event").iter().any(|f| f["params"]["event"] == event)).await;
    assert_eq!(rig.row().await["permission_mode"], "plan");
    let attached = rig.call(cx(), "harnix.session.attach", json!({"session_id": "s1"})).await.unwrap();
    assert_eq!(attached["session"]["permission_mode"], "plan");
}
