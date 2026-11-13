//! An instance whose engine runs in an extension's own process is under third-party control, and
//! the hub holds it to bounds the product's own engines are not held to:
//!
//! - its engine-wide events are not broadcast to every client;
//! - its frames are bounded in size and rate: what is over is dropped, and the registry told;
//! - what it says of a session is held to what a session can say: a title of at most 200
//!   characters, a turn state the hub knows;
//! - nobody reaches its settings or its plugins, the kernel included;
//! - its sessions are marked `trust: "extension"` wherever they are shown.
//!
//! The links here are fakes: the hub reaches an engine through `EngineLink` and nothing else.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use harnix_contract::{codes, ExtId, Frame, FrameSink, RpcError, SpaceDecl, Subject};
use harnix_engine_api::events::{DaemonEventKind, EngineEvent, Fragment, Joins, SessionEventKind};
use harnix_engine_api::link::{EngineControl, EngineIdentity, EngineLink, EventReceiver, InstanceCarrier, LinkStatus};
use harnix_engine_api::ops::{Answer, EngineOps, Entry, Said, SessionCreated, SessionInfo, Subscribed, Tasks, TurnInput};
use harnix_hub::{Hub, HubCell, HubConfig, HubRow, Spaces};
use harnix_kernel::{CallContext, Handler, InstanceBook, InstanceObserver, InstanceTrust, Listed, Rpc};
use serde_json::{json, Value};
use tokio::sync::{mpsc, watch};

const LONG: usize = 500;

/// An engine that answers from fixed values and says what it is told to.
struct FakeLink {
    events: Mutex<Option<EventReceiver>>,
    status: watch::Sender<LinkStatus>,
    /// The engine methods passed through to it, by name.
    passed: Mutex<Vec<String>>,
}

impl FakeLink {
    fn new() -> (Arc<Self>, mpsc::Sender<EngineEvent>) {
        let (tx, rx) = mpsc::channel(1024);
        let (status, _) = watch::channel(LinkStatus::Ready);
        (Arc::new(Self { events: Mutex::new(Some(rx)), status, passed: Mutex::new(Vec::new()) }), tx)
    }

    fn passed(&self) -> Vec<String> {
        self.passed.lock().unwrap().clone()
    }
}

/// A session as an engine that misbehaves describes it.
fn described(id: &str) -> Value {
    json!({"session_id": id, "name": "é".repeat(LONG), "turn_state": "<script>", "scene": "harnix"})
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
    async fn mark(&self, _id: u64) {}
}

#[async_trait::async_trait]
impl EngineOps for FakeLink {
    async fn create(&self, _: Value) -> Answer<SessionCreated> {
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
    async fn set_model(&self, session: &str, _: &str, _: Option<&str>) -> Answer<Value> {
        Ok(described(session))
    }
    async fn passthrough(&self, method: &str, params: Value) -> Answer<Value> {
        self.passed.lock().unwrap().push(method.to_string());
        Ok(match params.get("session_id").and_then(Value::as_str) {
            Some(session) => described(session),
            None => json!({"sessions": [described("s1")]}),
        })
    }
}

/// The registry's side: two instances — `ext` the extension `acme`'s own engine, for the spaces it
/// declared — and what it was told of floods.
#[derive(Default)]
struct Book {
    floods: Mutex<Vec<(String, u32)>>,
    acme_spaces: Vec<String>,
}

#[async_trait::async_trait]
impl InstanceBook for Book {
    fn listed(&self, id: &str) -> Option<Listed> {
        let id = id.to_string();
        match id.as_str() {
            "ext" => Some(Listed { id, spaces: self.acme_spaces.clone(), extension: Some(ExtId::native("acme").unwrap()) }),
            "own" => Some(Listed { id, spaces: Vec::new(), extension: None }),
            _ => None,
        }
    }
    fn all(&self) -> Vec<Listed> {
        ["ext", "own"].iter().filter_map(|id| self.listed(id)).collect()
    }
    async fn ensure_for_space(&self, _: &str, _: &str, _: &str) -> Rpc<()> {
        Ok(())
    }
    fn flooding(&self, id: &str, consecutive: u32) {
        self.floods.lock().unwrap().push((id.to_string(), consecutive));
    }
}

/// Every frame a client was sent.
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
    book: Arc<Book>,
    rows: HashMap<&'static str, Arc<dyn Handler>>,
    ext: (Arc<FakeLink>, mpsc::Sender<EngineEvent>),
    own: (Arc<FakeLink>, mpsc::Sender<EngineEvent>),
    _dir: tempfile::TempDir,
}

impl Rig {
    async fn new() -> Self {
        Self::with_spaces(Vec::new()).await
    }

    async fn with_spaces(declared: Vec<SpaceDecl>) -> Self {
        Self::build(declared, &[]).await
    }

    /// With `declared` spaces, `acme`'s engine for `acme_spaces` of them.
    async fn build(declared: Vec<SpaceDecl>, acme_spaces: &[&str]) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let book = Arc::new(Book { acme_spaces: acme_spaces.iter().map(|s| s.to_string()).collect(), ..Book::default() });
        let spaces = Spaces::load(dir.path().join("spaces.json"), declared, Default::default()).unwrap();
        let hub = Hub::new(HubConfig { instances: book.clone(), spaces, turns: Default::default() });
        let cell = HubCell::default();
        let rows = Hub::rows(&cell)
            .into_iter()
            .filter_map(|row| match row {
                HubRow::Method { name, handler: Some(h), .. } => Some((name, h)),
                _ => None,
            })
            .collect();
        let _ = cell.set(hub.clone());
        let (ext, own) = (FakeLink::new(), FakeLink::new());
        let extension = InstanceTrust::Extension(ExtId::native("acme").unwrap());
        hub.added("ext", ext.0.clone(), extension).await;
        hub.added("own", own.0.clone(), InstanceTrust::Product).await;
        Self { hub, book, rows, ext, own, _dir: dir }
    }

    async fn call(&self, cx: CallContext, method: &str, params: Value) -> Rpc<Value> {
        self.rows.get(method).unwrap_or_else(|| panic!("no row `{method}`")).call(cx, params).await
    }

    /// A device, attached to `s1` on both instances.
    async fn watcher(&self) -> Arc<Sink> {
        let sink = Arc::new(Sink::default());
        let client = self.hub.add_client(sink.clone()).await;
        let cx = CallContext::root(Subject::device("d"), Some(client));
        for instance in ["ext", "own"] {
            let attached = self.call(cx.clone(), "harnix.session.attach", json!({"instance": instance, "session_id": "s1"})).await.unwrap();
            assert_eq!(attached["trust"] == "extension", instance == "ext", "{attached}");
            let title = attached["session"]["name"].as_str().unwrap();
            assert_eq!(title.chars().count(), if instance == "ext" { 200 } else { LONG });
        }
        sink
    }
}

fn text(t: &str) -> EngineEvent {
    EngineEvent::Session {
        session: "s1".into(),
        turn_id: "t".into(),
        kind: SessionEventKind::Fragment(Fragment::new(Joins::Answer, "text", t)),
        event: json!({"kind": "text_delta", "text": t}),
    }
}

/// Wait until `sink` has `n` frames of `method`.
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
async fn an_extensions_engine_wide_events_are_not_broadcast() {
    let rig = Rig::new().await;
    let sink = rig.watcher().await;
    let host = |n: u32| EngineEvent::Daemon { kind: DaemonEventKind::Other, event: json!({"kind": "note", "n": n}) };
    rig.ext.1.send(host(1)).await.unwrap();
    // Behind it on the same stream: once this arrives, the host event was routed.
    rig.ext.1.send(text("after")).await.unwrap();
    rig.own.1.send(host(2)).await.unwrap();
    until(&sink, "harnix.event", 1).await;
    until(&sink, "harnix.host_event", 1).await;
    let told = sink.frames("harnix.host_event");
    assert_eq!(told.len(), 1, "{told:?}");
    assert_eq!(told[0]["params"]["instance"], "own");
}

#[tokio::test]
async fn an_extensions_session_events_are_marked_and_the_products_are_not() {
    let rig = Rig::new().await;
    let sink = rig.watcher().await;
    rig.ext.1.send(text("a")).await.unwrap();
    rig.own.1.send(text("b")).await.unwrap();
    until(&sink, "harnix.event", 2).await;
    for frame in sink.frames("harnix.event") {
        let from_ext = frame["params"]["instance"] == "ext";
        assert_eq!(frame["params"].get("trust").is_some(), from_ext, "{frame}");
        if from_ext {
            assert_eq!(frame["params"]["trust"], "extension");
        }
    }
}

#[tokio::test]
async fn an_extensions_frames_over_the_rate_or_the_size_are_dropped_and_the_registry_told() {
    let rig = Rig::new().await;
    let sink = rig.watcher().await;
    let sent = harnix_hub::FRAMES_PER_SECOND as usize + 50;
    for n in 0..sent {
        rig.ext.1.send(text(&n.to_string())).await.unwrap();
    }
    let huge = "x".repeat(harnix_hub::FRAME_MAX_BYTES + 1);
    rig.ext.1.send(text(&huge)).await.unwrap();
    // The product's engine is not counted: as many as it sends arrive.
    for n in 0..sent {
        rig.own.1.send(text(&n.to_string())).await.unwrap();
    }
    until(&sink, "harnix.event", sent).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    let frames = sink.frames("harnix.event");
    let from_ext = frames.iter().filter(|f| f["params"]["instance"] == "ext").count();
    let from_own = frames.len() - from_ext;
    assert_eq!(from_own, sent);
    assert!(from_ext <= sent, "nothing of the extension's {} was dropped", sent + 1);
    assert!(frames.iter().all(|f| f.to_string().len() < harnix_hub::FRAME_MAX_BYTES), "the oversized frame went out");
    let floods = rig.book.floods.lock().unwrap().clone();
    assert!(!floods.is_empty() && floods.iter().all(|(id, n)| id == "ext" && *n >= 1), "{floods:?}");
}

#[tokio::test]
async fn what_an_extensions_engine_says_of_a_session_is_held_to_what_a_session_can_say() {
    let rig = Rig::new().await;
    let kernel = || CallContext::root(Subject::Kernel, None);
    for instance in ["ext", "own"] {
        let listed = rig.call(kernel(), "harnix.sessions.list", json!({"instance": instance})).await.unwrap();
        let row = &listed["sessions"][0];
        let ext = instance == "ext";
        assert_eq!(row["name"].as_str().unwrap().chars().count(), if ext { 200 } else { LONG });
        assert_eq!(row["turn_state"], if ext { Value::Null } else { json!("<script>") });
        assert_eq!(row["trust"] == "extension", ext, "{row}");

        // And passed through as it is otherwise.
        let got = rig.call(kernel(), "engine.session.get", json!({"instance": instance, "session_id": "s1"})).await.unwrap();
        assert_eq!(got["name"].as_str().unwrap().chars().count(), if ext { 200 } else { LONG });
        let all = rig.call(kernel(), "engine.session.list", json!({"instance": instance})).await.unwrap();
        assert_eq!(all["sessions"][0]["turn_state"], if ext { Value::Null } else { json!("<script>") });
    }
}

#[tokio::test]
async fn nobody_reaches_an_extensions_engines_settings_or_plugins() {
    let rig = Rig::new().await;
    let kernel = || CallContext::root(Subject::Kernel, None);
    for method in ["engine.config.get", "engine.config.reload", "engine.plugin.install", "engine.plugin.list", "engine.config.setProvider"] {
        let refused = rig.call(kernel(), method, json!({"instance": "ext"})).await.unwrap_err();
        assert_eq!(refused.code, codes::REFUSED, "{method}: {}", refused.message);
        rig.call(kernel(), method, json!({"instance": "own"})).await.unwrap_or_else(|e| panic!("{method}: {}", e.message));
    }
    assert!(rig.ext.0.passed().is_empty(), "{:?}", rig.ext.0.passed());
    assert_eq!(rig.own.0.passed().len(), 5);
    // What is not its settings passes through as for any engine.
    rig.call(kernel(), "engine.scene.list", json!({"instance": "ext"})).await.unwrap();
    assert_eq!(rig.ext.0.passed(), ["engine.scene.list"]);
}

#[tokio::test]
async fn a_caller_that_claims_a_space_is_told_of_that_spaces_sessions_only() {
    let rig = Rig::with_spaces(vec![SpaceDecl::new("acme").default_space(), SpaceDecl::new("other")]).await;
    let kernel = || CallContext::root(Subject::Kernel, None);
    let listed = |space: Option<&str>| {
        let mut params = json!({"instance": "own"});
        if let Some(space) = space {
            params["space"] = json!(space);
        }
        params
    };
    // The session is the default space's: acme sees it, another space does not, and a caller
    // that claims nothing sees what the engine lists.
    let sessions = |answer: Value| answer["sessions"].as_array().unwrap().len();
    assert_eq!(sessions(rig.call(kernel(), "engine.session.list", listed(Some("acme"))).await.unwrap()), 1);
    assert_eq!(sessions(rig.call(kernel(), "engine.session.list", listed(Some("other"))).await.unwrap()), 0);
    assert_eq!(sessions(rig.call(kernel(), "engine.session.list", listed(None)).await.unwrap()), 1);

    // The same for the values of the topics that list sessions: each subscriber is told its own.
    let mut told = Vec::new();
    for space in [Some("acme"), Some("other"), None] {
        let sink = Arc::new(Sink::default());
        let client = rig.hub.add_client(sink.clone()).await;
        let cx = CallContext::root(Subject::device("d"), Some(client));
        let mut params = json!({"instance": "own"});
        if let Some(space) = space {
            params["space"] = json!(space);
        }
        let on = rig
            .call(cx, "harnix.topics.subscribe", json!({"topic": "harnix.instances.sessions", "params": params}))
            .await
            .unwrap();
        told.push((sessions(on["value"].clone()), sink));
    }
    assert_eq!(told.iter().map(|(n, _)| *n).collect::<Vec<_>>(), [1, 0, 1]);
}

/// Who calls, claiming `space` (none: `None`), naming `acme`'s engine.
fn naming(space: Option<&str>) -> Value {
    let mut params = json!({"instance": "ext"});
    if let Some(space) = space {
        params["space"] = json!(space);
    }
    params
}

#[tokio::test]
async fn an_extensions_engine_is_used_by_it_from_its_spaces_and_across_spaces_only() {
    let declared = || vec![SpaceDecl::new("acme"), SpaceDecl::new("other"), SpaceDecl::new("system").cross_space()];
    let rig = Rig::build(declared(), &["acme"]).await;
    let device = || CallContext::root(Subject::device("d"), None);
    let as_ext = |id: &str| CallContext::root(Subject::extension(ExtId::native(id).unwrap()), None);
    let refused = |r: Rpc<Value>| r.map(|_| ()).unwrap_err().code;

    // Another space naming it, or a call claiming none: refused, and its engine never asked.
    for space in [Some("other"), None] {
        assert_eq!(refused(rig.call(device(), "harnix.session.create", naming(space)).await), codes::SPACE_MISMATCH);
        assert_eq!(refused(rig.call(device(), "engine.scene.list", naming(space)).await), codes::SPACE_MISMATCH);
    }
    assert_eq!(refused(rig.call(as_ext("other"), "engine.scene.list", naming(Some("other"))).await), codes::SPACE_MISMATCH);
    assert!(rig.ext.0.passed().is_empty(), "{:?}", rig.ext.0.passed());
    // The product's own engine is any space's, as before.
    rig.call(device(), "engine.scene.list", json!({"instance": "own", "space": "other"})).await.unwrap();

    // From its own space: a session there, and its engine reached.
    let created = rig.call(device(), "harnix.session.create", naming(Some("acme"))).await.unwrap();
    assert_eq!((created["instance"].as_str(), created["space"].as_str()), (Some("ext"), Some("acme")), "{created}");
    rig.call(device(), "engine.scene.list", naming(Some("acme"))).await.unwrap();
    // The extension itself, and a page of it in a frame, whatever they claim.
    rig.call(as_ext("acme"), "engine.scene.list", naming(None)).await.unwrap();
    let mut framed = device();
    framed.on_behalf_of = Some(ExtId::native("acme").unwrap());
    rig.call(framed, "engine.scene.list", naming(Some("other"))).await.unwrap();
    // The system space reaches across spaces.
    rig.call(device(), "engine.scene.list", naming(Some("system"))).await.unwrap();
    assert_eq!(rig.ext.0.passed().len(), 4, "{:?}", rig.ext.0.passed());

    // A session of it is not moved into another extension's space.
    let moved = rig.call(as_ext("other"), "harnix.session.move", json!({"instance": "ext", "session_id": "s1", "space": "other"})).await;
    assert_eq!(refused(moved), codes::SPACE_MISMATCH);

    // An extension that declared no space: its engine is no space's but its own and the system
    // space's.
    let rig = Rig::build(declared(), &[]).await;
    assert_eq!(refused(rig.call(device(), "harnix.session.create", naming(Some("acme"))).await), codes::SPACE_MISMATCH);
    assert_eq!(refused(rig.call(device(), "engine.scene.list", naming(Some("other"))).await), codes::SPACE_MISMATCH);
    rig.call(as_ext("acme"), "harnix.session.create", naming(Some("acme"))).await.unwrap();
    rig.call(device(), "engine.scene.list", naming(Some("system"))).await.unwrap();
}
