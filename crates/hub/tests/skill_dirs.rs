//! Which skills directories a session lists: from the space it is in, when it is created, moved,
//! subscribed to, and when the packages that publish them change.
//!
//! The link here is a fake: the hub reaches an engine through `EngineLink` and nothing else.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use harnix_contract::{ExtId, RpcError, SkillSource, SpaceDecl, Subject};
use harnix_engine_api::events::EngineEvent;
use harnix_engine_api::link::{EngineControl, EngineIdentity, EngineLink, EventReceiver, InstanceCarrier, LinkStatus};
use harnix_engine_api::ops::{Answer, EngineOps, Entry, Said, SessionCreated, SessionInfo, SessionSetup, Subscribed, Tasks, TurnInput};
use harnix_hub::{Hub, HubCell, HubConfig, HubRow, Spaces};
use harnix_kernel::{CallContext, Handler, InstanceBook, InstanceObserver, InstanceTrust, Listed, Rpc};
use serde_json::{json, Value};
use tokio::sync::{mpsc, watch};

struct FakeLink {
    carrier: InstanceCarrier,
    events: Mutex<Option<EventReceiver>>,
    status: watch::Sender<LinkStatus>,
    /// What each session lists, as the engine says.
    listed: Mutex<HashMap<String, Vec<PathBuf>>>,
    /// The create calls' params, in order.
    created: Mutex<Vec<Value>>,
    /// Each `setSkillDirs`: the session and what it was given.
    set: Mutex<Vec<(String, Vec<PathBuf>)>>,
    /// The engine refuses the second directory of a session it creates.
    refuse_second: bool,
    _tx: mpsc::Sender<EngineEvent>,
}

impl FakeLink {
    fn new(carrier: InstanceCarrier, refuse_second: bool) -> Arc<Self> {
        let (tx, rx) = mpsc::channel(16);
        let (status, _) = watch::channel(LinkStatus::Ready);
        Arc::new(Self {
            carrier,
            events: Mutex::new(Some(rx)),
            status,
            listed: Mutex::default(),
            created: Mutex::default(),
            set: Mutex::default(),
            refuse_second,
            _tx: tx,
        })
    }

    fn session(&self, id: &str) -> Value {
        let dirs: Vec<Value> = self.listed.lock().unwrap().get(id).into_iter().flatten().map(|p| json!({"path": p})).collect();
        let mut v = json!({"session_id": id, "name": "n", "turn_state": "idle", "status": "active", "scene": "harnix", "session_kind": "primary"});
        if !dirs.is_empty() {
            v["skill_dirs"] = json!(dirs);
        }
        v
    }

    fn sets(&self) -> Vec<(String, Vec<String>)> {
        self.set
            .lock()
            .unwrap()
            .iter()
            .map(|(s, d)| (s.clone(), d.iter().map(|p| p.to_string_lossy().into_owned()).collect()))
            .collect()
    }
}

fn listed_in(options: &Value) -> Vec<PathBuf> {
    options["skill_dirs"].as_array().into_iter().flatten().filter_map(|d| d["path"].as_str().map(PathBuf::from)).collect()
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
        self.carrier
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
    /// What it was asked to create, with the directories it was set up with as the link writes
    /// them into the options.
    async fn create(&self, mut params: Value, setup: SessionSetup) -> Answer<SessionCreated> {
        if !setup.skill_dirs.is_empty() {
            let listed: Vec<Value> = setup.skill_dirs.iter().map(|d| json!({"path": d})).collect();
            params["options"]["skill_dirs"] = json!(listed);
        }
        self.created.lock().unwrap().push(params.clone());
        let dirs = listed_in(&params["options"]);
        if self.refuse_second && dirs.len() > 1 {
            let mut e = RpcError::invalid_params("bad options");
            e.data = Some(json!({"invalid_options": [{"field": "skill_dirs[1].path", "reason": "inside the project"}]}));
            return Err(e);
        }
        self.listed.lock().unwrap().insert("s1".into(), dirs);
        Ok(SessionCreated::new(json!({"session_id": "s1"})))
    }
    async fn get(&self, session: &str) -> Answer<SessionInfo> {
        Ok(SessionInfo::new(self.session(session)))
    }
    async fn list(&self) -> Answer<Vec<SessionInfo>> {
        Ok(vec![SessionInfo::new(self.session("s1"))])
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
        Ok(self.session(session))
    }
    async fn set_model(&self, session: &str, _: &str, _: Option<&str>) -> Answer<Value> {
        Ok(self.session(session))
    }
    async fn set_prompt(&self, session: &str, _: Option<Option<String>>, _: Option<Option<String>>) -> Answer<Value> {
        Ok(json!({"session_id": session}))
    }
    async fn set_skill_dirs(&self, session: &str, dirs: &[PathBuf]) -> Answer<Value> {
        self.set.lock().unwrap().push((session.to_string(), dirs.to_vec()));
        self.listed.lock().unwrap().insert(session.to_string(), dirs.to_vec());
        Ok(json!({"session_id": session}))
    }
    async fn passthrough(&self, _: &str, _: Value) -> Answer<Value> {
        Ok(json!({}))
    }
}

/// `own` and `att` are the product's, no space's; `ext` is the extension `acme`'s own engine.
struct Book;

#[async_trait::async_trait]
impl InstanceBook for Book {
    fn listed(&self, id: &str) -> Option<Listed> {
        let id = id.to_string();
        match id.as_str() {
            "ext" => Some(Listed { id, spaces: vec!["alpha".into()], extension: Some(ExtId::native("acme").unwrap()) }),
            "own" | "att" => Some(Listed { id, spaces: Vec::new(), extension: None }),
            _ => None,
        }
    }
    fn all(&self) -> Vec<Listed> {
        ["ext", "own", "att"].iter().filter_map(|id| self.listed(id)).collect()
    }
    async fn ensure_for_space(&self, _: &str, _: &str, _: &str) -> Rpc<()> {
        Ok(())
    }
    fn flooding(&self, _: &str, _: u32) {}
}

struct Rig {
    hub: Arc<Hub>,
    own: Arc<FakeLink>,
    att: Arc<FakeLink>,
    rows: HashMap<&'static str, Arc<dyn Handler>>,
    _dir: tempfile::TempDir,
}

fn source(ext: &str, spaces: &[&str]) -> SkillSource {
    SkillSource::new(ext, format!("/state/ext/{ext}/skills"), spaces.iter().map(|s| s.to_string()).collect())
}

fn path(ext: &str) -> String {
    format!("/state/ext/{ext}/skills")
}

impl Rig {
    async fn new(refuse_second: bool) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let declared = vec![
            SpaceDecl::new("alpha").instance("own"),
            SpaceDecl::new("beta").instance("own"),
            SpaceDecl::new("sys").instance("own").cross_space(),
        ];
        let spaces = Spaces::load(dir.path().join("spaces.json"), declared, Default::default()).unwrap();
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
        let (own, att, ext) = (
            FakeLink::new(InstanceCarrier::InProcess, refuse_second),
            FakeLink::new(InstanceCarrier::Attach, false),
            FakeLink::new(InstanceCarrier::Adopted, false),
        );
        hub.added("own", own.clone(), InstanceTrust::Product).await;
        hub.added("att", att.clone(), InstanceTrust::Product).await;
        hub.added("ext", ext, InstanceTrust::Extension(ExtId::native("acme").unwrap())).await;
        hub.set_skill_sources("packages", vec![source("p1", &["alpha"]), source("p2", &[]), source("p3", &["beta"])]);
        Self { hub, own, att, rows, _dir: dir }
    }

    async fn call(&self, method: &str, params: Value) -> Rpc<Value> {
        let cx = CallContext::root(Subject::Kernel, None);
        self.rows.get(method).unwrap_or_else(|| panic!("no row `{method}`")).call(cx, params).await
    }
}

async fn eventually(what: &str, mut ok: impl FnMut() -> bool) {
    let waited = tokio::time::timeout(Duration::from_secs(10), async {
        while !ok() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
    assert!(waited.is_ok(), "never: {what}");
}

#[tokio::test]
async fn a_session_lists_the_directories_its_space_sees() {
    let rig = Rig::new(false).await;
    rig.call("harnix.session.create", json!({"instance": "own", "space": "alpha", "project_root": null})).await.unwrap();
    let created = rig.own.created.lock().unwrap().clone();
    assert_eq!(listed_in(&created[0]["options"]), [PathBuf::from(path("p1")), PathBuf::from(path("p2"))]);
    // An attached engine may be on another machine: it lists none.
    rig.call("harnix.session.create", json!({"instance": "att", "space": "alpha", "project_root": null})).await.unwrap();
    assert!(rig.att.created.lock().unwrap()[0].get("options").is_none_or(|o| o.get("skill_dirs").is_none()));
    // The system space sees every package's.
    rig.call("harnix.session.create", json!({"instance": "own", "space": "sys", "project_root": null})).await.unwrap();
    let created = rig.own.created.lock().unwrap().clone();
    assert_eq!(listed_in(&created[1]["options"]).len(), 3);
}

#[tokio::test]
async fn a_directory_the_engine_refuses_is_left_out_and_the_session_is_made() {
    let rig = Rig::new(true).await;
    let made = rig.call("harnix.session.create", json!({"instance": "own", "space": "alpha", "project_root": null})).await.unwrap();
    assert_eq!(made["session_id"], "s1");
    let created = rig.own.created.lock().unwrap().clone();
    assert_eq!(created.len(), 2);
    assert_eq!(listed_in(&created[1]["options"]), [PathBuf::from(path("p1"))]);
}

#[tokio::test]
async fn a_moved_session_lists_the_directories_of_its_new_space() {
    let rig = Rig::new(false).await;
    rig.call("harnix.session.create", json!({"instance": "own", "space": "alpha", "project_root": null})).await.unwrap();
    let moved = rig.call("harnix.session.move", json!({"instance": "own", "session_id": "s1", "space": "beta"})).await.unwrap();
    assert_eq!(moved["moved"], true);
    assert_eq!(rig.own.sets(), [("s1".to_string(), vec![path("p3"), path("p2")])]);
}

#[tokio::test]
async fn a_session_found_listing_other_directories_is_brought_to_its_space_when_subscribed() {
    let rig = Rig::new(false).await;
    // Made by an earlier run: in `alpha`, listing a package that has since gone.
    rig.own.listed.lock().unwrap().insert("old".into(), vec![PathBuf::from(path("gone"))]);
    let sink = Arc::new(NoSink);
    let client = rig.hub.add_client(sink).await;
    let cx = CallContext::root(Subject::device("d"), Some(client));
    let attach = rig.rows["harnix.session.attach"].call(cx, json!({"instance": "own", "session_id": "old"})).await.unwrap();
    assert_eq!(attach["session"]["session_id"], "old");
    // No space of its own: the shared directories only.
    assert_eq!(rig.own.sets(), [("old".to_string(), vec![path("p2")])]);
}

#[tokio::test]
async fn sessions_in_memory_follow_the_packages_that_come_and_go() {
    let rig = Rig::new(false).await;
    rig.call("harnix.session.create", json!({"instance": "own", "space": "alpha", "project_root": null})).await.unwrap();
    let sink = Arc::new(NoSink);
    let client = rig.hub.add_client(sink).await;
    let cx = CallContext::root(Subject::device("d"), Some(client));
    rig.rows["harnix.session.attach"].call(cx, json!({"instance": "own", "session_id": "s1"})).await.unwrap();
    assert!(rig.own.sets().is_empty(), "the session already lists what it should");
    // A package is switched off: it is not published any more.
    rig.hub.set_skill_sources("packages", vec![source("p2", &[]), source("p3", &["beta"])]);
    eventually("the session lists the shared directory only", || rig.own.sets().last().is_some_and(|(_, d)| *d == [path("p2")])).await;
    // The same list again is not a change.
    let before = rig.own.sets().len();
    rig.hub.set_skill_sources("packages", vec![source("p2", &[]), source("p3", &["beta"])]);
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(rig.own.sets().len(), before);
}

struct NoSink;

#[async_trait::async_trait]
impl harnix_contract::FrameSink for NoSink {
    async fn deliver(&self, _: harnix_contract::Frame) -> bool {
        true
    }
}
