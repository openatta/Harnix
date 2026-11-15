//! A session created in a scene layer: the hub sets it up from the layer the instance's link
//! carries, records which layer it was, and says so wherever it describes the session; keeps the
//! caller's own prompt text behind the layer's; and lists, describes and activates a layer as the
//! engine's scenes are.
//!
//! The link here is a fake: the hub reaches an engine through `EngineLink` and nothing else.

use std::sync::{Arc, Mutex};

use harnix_contract::{RpcError, SpaceDecl, Subject};
use harnix_engine_api::events::EngineEvent;
use harnix_engine_api::link::{EngineControl, EngineIdentity, EngineLink, EventReceiver, InstanceCarrier, LinkStatus};
use harnix_engine_api::ops::{codes, Answer, EngineOps, Entry, Said, SessionCreated, SessionInfo, SessionSetup, Subscribed, Tasks, TurnInput};
use harnix_engine_api::{PromptText, SceneBase, SceneLayer, SceneLayers, Tool, ToolCx, ToolError, ToolOutput, ToolSpec};
use harnix_hub::{Hub, HubCell, HubConfig, HubRow, Spaces};
use harnix_kernel::{CallContext, Handler, InstanceBook, InstanceObserver, InstanceTrust, Listed, Rpc};
use serde_json::{json, Value};
use tokio::sync::{mpsc, watch};

struct Probe(ToolSpec);

#[async_trait::async_trait]
impl Tool for Probe {
    fn spec(&self) -> &ToolSpec {
        &self.0
    }
    async fn call(&self, _: ToolCx, _: Value) -> Result<ToolOutput, ToolError> {
        Ok(ToolOutput::text("probed"))
    }
}

/// An engine that remembers what it was asked to create and to prompt, and has `harnix`'s scene.
struct FakeLink {
    events: Mutex<Option<EventReceiver>>,
    status: watch::Sender<LinkStatus>,
    layers: SceneLayers,
    created: Mutex<Vec<(Value, SessionSetup)>>,
    prompted: Mutex<Vec<(Option<Option<String>>, Option<Option<String>>)>>,
    passed: Mutex<Vec<(String, Value)>>,
    busy: Mutex<bool>,
    _tx: mpsc::Sender<EngineEvent>,
}

impl FakeLink {
    fn new(layers: SceneLayers) -> Arc<Self> {
        let (tx, rx) = mpsc::channel(16);
        let (status, _) = watch::channel(LinkStatus::Ready);
        Arc::new(Self {
            events: Mutex::new(Some(rx)),
            status,
            layers,
            created: Mutex::default(),
            prompted: Mutex::default(),
            passed: Mutex::default(),
            busy: Mutex::new(false),
            _tx: tx,
        })
    }
}

fn described(id: &str) -> Value {
    json!({"session_id": id, "scene": "harnix", "turn_state": "idle"})
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
    fn scene_layers(&self) -> SceneLayers {
        self.layers.clone()
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
    async fn create(&self, params: Value, setup: SessionSetup) -> Answer<SessionCreated> {
        let mut created = self.created.lock().unwrap();
        created.push((params, setup));
        let id = format!("s{}", created.len());
        Ok(SessionCreated::new(described(&id)))
    }
    async fn get(&self, session: &str) -> Answer<SessionInfo> {
        Ok(SessionInfo::new(described(session)))
    }
    async fn list(&self) -> Answer<Vec<SessionInfo>> {
        Ok(vec![SessionInfo::new(described("s1")), SessionInfo::new(described("s2"))])
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
        Ok(SessionCreated::new(described("forked")))
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
    async fn set_prompt(&self, session: &str, append: Option<Option<String>>, override_text: Option<Option<String>>) -> Answer<Value> {
        if *self.busy.lock().unwrap() {
            return Err(RpcError {
                code: codes::SESSION_BUSY,
                message: "a turn is running".into(),
                data: Some(json!({"reason": "turn_running"})),
            });
        }
        self.prompted.lock().unwrap().push((append.clone(), override_text));
        Ok(json!({"session_id": session, "append": append.flatten(), "applies": "next_turn"}))
    }
    async fn passthrough(&self, method: &str, params: Value) -> Answer<Value> {
        self.passed.lock().unwrap().push((method.to_string(), params.clone()));
        Ok(match method {
            "engine.scene.list" => json!({"scenes": [{"scene": "harnix", "name": "Harnix", "description": "the main one", "active": true, "sessions": 1, "requires_project": false}]}),
            "engine.scene.describe" | "engine.scene.activate" => json!({"scene": params["scene"], "name": "Harnix", "active": true, "capabilities": {"requires_project": false}}),
            "engine.session.list" => json!({"sessions": [described("s1"), described("s2")]}),
            _ => described(params["session_id"].as_str().unwrap_or("?")),
        })
    }
}

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

struct Rig {
    hub: Arc<Hub>,
    link: Arc<FakeLink>,
    rows: std::collections::HashMap<&'static str, Arc<dyn Handler>>,
    _dir: tempfile::TempDir,
}

const LAYER_TEXT: &str = "the probe's own block";

fn layers() -> SceneLayers {
    let tool: Arc<dyn Tool> = Arc::new(Probe(ToolSpec::new("ProbeFixed", "p", json!({"type": "object"}))));
    SceneLayers::new(&[
        SceneLayer::new("probe", SceneBase::builtin("harnix"))
            .name("{base} probe")
            .prompt(PromptText::plain(LAYER_TEXT))
            .tool(tool)
            .auto_name(false),
        SceneLayer::new("strict", SceneBase::builtin("harnix")).requires_project(true),
    ])
    .unwrap()
}

impl Rig {
    async fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let spaces = Spaces::load(dir.path().join("spaces.json"), vec![SpaceDecl::new("lab").scene("probe")], Default::default()).unwrap();
        let hub = Hub::new(HubConfig { instances: Arc::new(Book), spaces, turns: Default::default() });
        let cell = HubCell::default();
        let rows = Hub::rows(&cell)
            .into_iter()
            .filter_map(|row| match row {
                HubRow::Method { name, handler: Some(h), .. } => Some((name, h)),
                _ => None,
            })
            .collect();
        let _ = cell.set(hub.clone());
        let link = FakeLink::new(layers());
        hub.added("main", link.clone(), InstanceTrust::Product).await;
        Self { hub, link, rows, _dir: dir }
    }

    async fn call(&self, method: &str, params: Value) -> Rpc<Value> {
        let cx = CallContext::root(Subject::Kernel, None);
        self.rows.get(method).unwrap_or_else(|| panic!("no row `{method}`")).call(cx, params).await
    }

    fn last_setup(&self) -> (Value, SessionSetup) {
        self.link.created.lock().unwrap().last().cloned().unwrap()
    }
}

#[tokio::test]
async fn a_session_in_a_layer_is_set_up_from_it_and_says_so() {
    let rig = Rig::new().await;
    let created = rig.call("harnix.session.create", json!({"scene": "probe"})).await.unwrap();
    assert_eq!((&created["scene"], &created["scene_layer"]), (&json!("harnix"), &json!("probe")));
    let (_, setup) = rig.last_setup();
    let scene = setup.scene.expect("set up from the layer");
    assert_eq!((scene.id.as_str(), scene.base.as_str(), scene.auto_name), ("probe", "harnix", Some(false)));
    assert_eq!(scene.tools.len(), 1);

    // The space's default scene is a layer too.
    let created = rig.call("harnix.session.create", json!({"space": "lab"})).await.unwrap();
    assert_eq!(created["scene_layer"], "probe");
    // One of the engine's own scenes is set up from nothing.
    let plain = rig.call("harnix.session.create", json!({"scene": "harnix"})).await.unwrap();
    assert!(plain.get("scene_layer").is_none(), "{plain}");
    assert!(rig.last_setup().1.scene.is_none());

    // Wherever the session is described.
    let got = rig.call("engine.session.get", json!({"session_id": "s1"})).await.unwrap();
    assert_eq!(got["scene_layer"], "probe");
    let listed = rig.call("engine.session.list", json!({})).await.unwrap();
    assert_eq!(listed["sessions"][0]["scene_layer"], "probe");
    let forked = rig.call("harnix.session.fork", json!({"session_id": "s1"})).await.unwrap();
    assert_eq!(forked["scene_layer"], "probe", "a fork is in its parent's layer");
}

#[tokio::test]
async fn a_layer_that_needs_a_project_refuses_a_session_with_none() {
    let rig = Rig::new().await;
    let refused = rig.call("harnix.session.create", json!({"scene": "strict", "project_root": null})).await.unwrap_err();
    assert_eq!(refused.code, codes::PROJECT_REQUIRED);
    assert!(rig.link.created.lock().unwrap().is_empty(), "the engine was asked");
    assert!(rig.call("harnix.session.create", json!({"scene": "strict"})).await.is_ok());
}

#[tokio::test]
async fn the_callers_prompt_goes_behind_the_layers() {
    let rig = Rig::new().await;
    rig.call("harnix.session.create", json!({"scene": "probe"})).await.unwrap();
    let set = rig.call("harnix.session.setPrompt", json!({"session_id": "s1", "append": "mine"})).await.unwrap();
    assert_eq!((&set["append"], &set["scene_append"]), (&json!("mine"), &json!(LAYER_TEXT)));
    let prompted = |n: usize| rig.link.prompted.lock().unwrap()[n].clone();
    assert_eq!(prompted(0), (Some(Some(format!("{LAYER_TEXT}\n\nmine"))), None));
    let got = rig.call("engine.session.get", json!({"session_id": "s1"})).await.unwrap();
    assert_eq!(got["prompt_append"], "mine");

    // Only the override: the text is left as it is.
    rig.call("harnix.session.setPrompt", json!({"session_id": "s1", "override": "all of it"})).await.unwrap();
    assert_eq!(prompted(1), (None, Some(Some("all of it".into()))));
    // Cleared: the layer's text stays.
    rig.call("harnix.session.setPrompt", json!({"session_id": "s1", "append": null})).await.unwrap();
    assert_eq!(prompted(2).0, Some(Some(LAYER_TEXT.into())));
    let got = rig.call("engine.session.get", json!({"session_id": "s1"})).await.unwrap();
    assert!(got.get("prompt_append").is_none());

    // A session in no layer, cleared: back to the settings'.
    rig.call("harnix.session.create", json!({"scene": "harnix"})).await.unwrap();
    rig.call("harnix.session.setPrompt", json!({"session_id": "s2", "append": "x"})).await.unwrap();
    assert_eq!(prompted(3).0, Some(Some("x".into())));
    rig.call("harnix.session.setPrompt", json!({"session_id": "s2", "append": null})).await.unwrap();
    assert_eq!(prompted(4).0, Some(None));

    assert_eq!(rig.call("harnix.session.setPrompt", json!({"session_id": "s1"})).await.unwrap_err().code, -32602);
}

#[tokio::test]
async fn a_busy_session_is_refused_as_the_engine_refuses_it_and_nothing_changes() {
    let rig = Rig::new().await;
    rig.call("harnix.session.create", json!({"scene": "probe"})).await.unwrap();
    rig.call("harnix.session.setPrompt", json!({"session_id": "s1", "append": "first"})).await.unwrap();
    *rig.link.busy.lock().unwrap() = true;
    let busy = rig.call("harnix.session.setPrompt", json!({"session_id": "s1", "append": "second"})).await.unwrap_err();
    assert_eq!((busy.code, busy.data), (codes::SESSION_BUSY, Some(json!({"reason": "turn_running"}))));
    let got = rig.call("engine.session.get", json!({"session_id": "s1"})).await.unwrap();
    assert_eq!(got["prompt_append"], "first");
}

#[tokio::test]
async fn the_engines_own_set_prompt_and_prompt_options_are_the_hubs() {
    let rig = Rig::new().await;
    assert!(!rig.rows.contains_key("engine.session.setPrompt"), "it has a handler");
    let refused = rig.call("harnix.session.create", json!({"options": {"prompt_types": ["question"]}})).await.unwrap_err();
    assert!(refused.message.contains("prompt_types"), "{}", refused.message);
}

#[tokio::test]
async fn a_layer_is_listed_described_and_activated_as_its_base() {
    let rig = Rig::new().await;
    let listed = rig.call("engine.scene.list", json!({})).await.unwrap();
    let rows = listed["scenes"].as_array().unwrap();
    let probe = rows.iter().find(|r| r["scene"] == "probe").expect("the layer is listed");
    assert_eq!((&probe["name"], &probe["description"], &probe["base"]), (&json!("Harnix probe"), &json!("the main one"), &json!("harnix")));
    assert_eq!(probe["scene_layer"], true);
    let hello = rig.hub.passthrough("main", "engine.scene.list", json!({})).await.unwrap();
    assert!(hello["scenes"].as_array().unwrap().iter().any(|r| r["scene"] == "strict"), "{hello}");

    let activated = rig.call("engine.scene.activate", json!({"scene": "probe"})).await.unwrap();
    assert_eq!((&activated["scene"], &activated["base"]), (&json!("probe"), &json!("harnix")));
    let sent = rig.link.passed.lock().unwrap().iter().rev().find(|(m, _)| m == "engine.scene.activate").cloned().unwrap();
    assert_eq!(sent.1, json!({"scene": "harnix"}), "the engine is asked of the base");
    let described = rig.call("engine.scene.describe", json!({"scene": "strict"})).await.unwrap();
    assert_eq!(described["capabilities"]["requires_project"], true);
}
