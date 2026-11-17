//! A session shown in another space (`harnix.session.expose`, concept §5.4): what a call claiming
//! that space may then do — look, or talk too — and what stays the owner's; who may expose; how
//! a space's list tells a lent session; that an exposure outlives a restart and goes with the
//! session.
//!
//! The link here is a fake: the hub reaches an engine through `EngineLink` and nothing else.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use harnix_contract::{codes, ExtId, RpcError, SpaceDecl, Subject};
use harnix_engine_link::events::EngineEvent;
use harnix_engine_link::link::{EngineControl, EngineIdentity, EngineLink, EventReceiver, InstanceCarrier, LinkStatus};
use harnix_engine_link::ops::{Answer, EngineOps, Entry, HistoryPage, Said, SessionCreated, SessionInfo, SessionSetup, Subscribed, Tasks, TurnInput};
use harnix_hub::{Hub, HubCell, HubConfig, HubRow, Spaces};
use harnix_kernel::{CallContext, Handler, InstanceBook, InstanceObserver, InstanceTrust, Listed, Rpc};
use serde_json::{json, Value};
use tokio::sync::{mpsc, watch};

struct FakeLink {
    events: Mutex<Option<EventReceiver>>,
    status: watch::Sender<LinkStatus>,
    /// The sessions it has, in creation order.
    sessions: Mutex<Vec<String>>,
    /// Every session a call interrupted or closed.
    acted: Mutex<Vec<(&'static str, String)>>,
    _tx: mpsc::Sender<EngineEvent>,
}

impl FakeLink {
    fn new() -> Arc<Self> {
        let (tx, rx) = mpsc::channel(16);
        let (status, _) = watch::channel(LinkStatus::Ready);
        Arc::new(Self { events: Mutex::new(Some(rx)), status, sessions: Mutex::default(), acted: Mutex::default(), _tx: tx })
    }

    fn session(&self, id: &str) -> Value {
        json!({"session_id": id, "name": "n", "turn_state": "idle", "status": "active", "scene": "harnix", "session_kind": "primary"})
    }

    fn has(&self, id: &str) -> Answer<()> {
        if self.sessions.lock().unwrap().iter().any(|s| s == id) {
            Ok(())
        } else {
            Err(RpcError::new(codes::NOT_FOUND, format!("no session {id}")))
        }
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
        let mut sessions = self.sessions.lock().unwrap();
        let id = format!("s{}", sessions.len() + 1);
        sessions.push(id.clone());
        Ok(SessionCreated::new(json!({"session_id": id})))
    }
    async fn get(&self, session: &str) -> Answer<SessionInfo> {
        self.has(session)?;
        Ok(SessionInfo::new(self.session(session)))
    }
    async fn list(&self) -> Answer<Vec<SessionInfo>> {
        Ok(self.sessions.lock().unwrap().iter().map(|s| SessionInfo::new(self.session(s))).collect())
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
    async fn interrupt(&self, session: &str) -> Answer<Value> {
        self.acted.lock().unwrap().push(("interrupt", session.to_string()));
        Ok(json!({}))
    }
    async fn close(&self, params: Value, _: bool) -> Answer<Value> {
        let id = params["session_id"].as_str().unwrap_or_default().to_string();
        self.acted.lock().unwrap().push(("close", id.clone()));
        self.sessions.lock().unwrap().retain(|s| *s != id);
        Ok(json!({}))
    }
    async fn fork(&self, _: Value) -> Answer<SessionCreated> {
        Ok(SessionCreated::new(json!({"session_id": "forked"})))
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
    async fn set_model(&self, session: &str, _: &harnix_contract::wire::ModelChoice) -> Answer<Value> {
        Ok(self.session(session))
    }
    async fn set_prompt(&self, session: &str, _: Option<Option<String>>, _: Option<Option<String>>) -> Answer<Value> {
        Ok(json!({"session_id": session}))
    }
    async fn set_skill_dirs(&self, session: &str, _: &[PathBuf]) -> Answer<Value> {
        Ok(json!({"session_id": session}))
    }
    async fn passthrough(&self, method: &str, params: Value) -> Answer<Value> {
        match method {
            "engine.session.get" => {
                let id = params["session_id"].as_str().unwrap_or_default();
                self.has(id)?;
                Ok(self.session(id))
            }
            "engine.session.history" => Ok(json!({"messages": []})),
            _ => Ok(json!({})),
        }
    }
}

/// `own` is the product's, no space's; `side` was built for the space `gamma`.
struct Book;

#[async_trait::async_trait]
impl InstanceBook for Book {
    fn listed(&self, id: &str) -> Option<Listed> {
        let id = id.to_string();
        match id.as_str() {
            "own" => Some(Listed { id, spaces: Vec::new(), extension: None }),
            "side" => Some(Listed { id, spaces: vec!["gamma".into()], extension: None }),
            _ => None,
        }
    }
    fn all(&self) -> Vec<Listed> {
        ["own", "side"].iter().filter_map(|id| self.listed(id)).collect()
    }
    async fn ensure_for_space(&self, _: &str, _: &str, _: &str) -> Rpc<()> {
        Ok(())
    }
    fn flooding(&self, _: &str, _: u32) {}
}

struct Rig {
    hub: Arc<Hub>,
    own: Arc<FakeLink>,
    side: Arc<FakeLink>,
    rows: HashMap<&'static str, Arc<dyn Handler>>,
}

const ACME: &str = "acme";
const OTHER: &str = "other";

fn ext(id: &str) -> Subject {
    Subject::extension(ExtId::native(id).unwrap())
}

impl Rig {
    /// A hub over the ledger in `dir`: the same directory again is a restart.
    async fn new(dir: &std::path::Path, own: Arc<FakeLink>, side: Arc<FakeLink>) -> Self {
        let declared = vec![
            SpaceDecl::new("alpha").instance("own").default_space(),
            SpaceDecl::new("beta").instance("own"),
            SpaceDecl::new("gamma").instance("side"),
            SpaceDecl::new("sys").instance("own").cross_space(),
            SpaceDecl::new("overview").instance("own").no_sessions(),
        ];
        let spaces = Spaces::load(dir.join("spaces.json"), declared, ["off".to_string()].into()).unwrap();
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
        hub.added("own", own.clone(), InstanceTrust::Product).await;
        hub.added("side", side.clone(), InstanceTrust::Product).await;
        Self { hub, own, side, rows }
    }

    async fn call_as(&self, subject: Subject, method: &str, params: Value) -> Rpc<Value> {
        let cx = CallContext::root(subject, None);
        self.rows.get(method).unwrap_or_else(|| panic!("no row `{method}`")).call(cx, params).await
    }

    /// As the extension `acme`.
    async fn acme(&self, method: &str, params: Value) -> Rpc<Value> {
        self.call_as(ext(ACME), method, params).await
    }

    /// As a device claiming `space`.
    async fn device(&self, space: &str, method: &str, mut params: Value) -> Rpc<Value> {
        params["space"] = json!(space);
        self.call_as(Subject::device("d"), method, params).await
    }

    /// `acme`'s new session in `space`, on `instance`.
    async fn made(&self, space: &str, instance: &str) -> Value {
        let created = self.acme("harnix.session.create", json!({"instance": instance, "space": space, "project_root": null})).await.unwrap();
        json!({"instance": instance, "session_id": created["session_id"]})
    }

    async fn ids_in(&self, space: &str) -> Vec<(String, Option<String>)> {
        let listed = self.device(space, "harnix.sessions.list", json!({})).await.unwrap();
        listed["sessions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| (r["session_id"].as_str().unwrap().to_string(), r.get("exposed_from").and_then(Value::as_str).map(str::to_string)))
            .collect()
    }
}

fn code<T: std::fmt::Debug>(r: Rpc<T>) -> i32 {
    r.unwrap_err().code
}

fn with(mut session: Value, more: Value) -> Value {
    session.as_object_mut().unwrap().extend(more.as_object().unwrap().clone());
    session
}

/// Exposed to look at: a call claiming the space reads it and attaches, but may not talk to it nor
/// close it; exposed to talk to: it may interrupt, and still not close, fork or rename — those are
/// the owner's, from its own space. Unexposed, nothing reaches it from there.
#[tokio::test]
async fn an_exposed_session_is_reached_as_far_as_the_exposure_goes() {
    let dir = tempfile::tempdir().unwrap();
    let rig = Rig::new(dir.path(), FakeLink::new(), FakeLink::new()).await;
    let s = rig.made("beta", "own").await;
    assert_eq!(code(rig.device("alpha", "engine.session.get", s.clone()).await), codes::SPACE_MISMATCH, "beta's, not alpha's");

    let shown = rig.acme("harnix.session.expose", with(s.clone(), json!({"to": "alpha"}))).await.unwrap();
    assert_eq!(shown, json!({"instance": "own", "session_id": "s1", "space": "beta", "to": "alpha", "exposed": true, "interact": false, "changed": true}));
    assert_eq!(rig.acme("harnix.session.expose", with(s.clone(), json!({"to": "alpha"}))).await.unwrap()["changed"], false);
    // Looking: the pass-through and the hub's own rows about one session.
    rig.device("alpha", "engine.session.get", s.clone()).await.unwrap();
    rig.device("alpha", "engine.session.history", s.clone()).await.unwrap();
    rig.device("alpha", "harnix.session.tasks", s.clone()).await.unwrap();
    // Talking: not yet.
    let refused = rig.device("alpha", "harnix.session.interrupt", s.clone()).await.unwrap_err();
    assert_eq!(refused.code, codes::SPACE_MISMATCH);
    assert!(refused.message.contains("to look at, not to talk to"), "{}", refused.message);
    assert_eq!(refused.data.as_ref().and_then(|d| d["space"].as_str()), Some("beta"), "the error names its own space");
    // Still beta's own, where everything works, and not gamma's.
    rig.device("beta", "harnix.session.interrupt", s.clone()).await.unwrap();
    assert_eq!(code(rig.device("gamma", "engine.session.get", s.clone()).await), codes::SPACE_MISMATCH);

    let shown = rig.acme("harnix.session.expose", with(s.clone(), json!({"to": "alpha", "interact": true}))).await.unwrap();
    assert_eq!((shown["interact"].as_bool(), shown["changed"].as_bool()), (Some(true), Some(true)));
    rig.device("alpha", "harnix.session.interrupt", s.clone()).await.unwrap();
    assert_eq!(rig.own.acted.lock().unwrap().len(), 2);
    // Its life is the owner's, whatever the exposure.
    for method in ["harnix.session.close", "harnix.session.delete", "harnix.session.rename", "harnix.session.fork"] {
        let refused = rig.device("alpha", method, with(s.clone(), json!({"name": "x"}))).await.unwrap_err();
        assert_eq!(refused.code, codes::SPACE_MISMATCH, "{method}");
        assert!(refused.message.contains("is done from there"), "{method}: {}", refused.message);
    }
    assert_eq!(rig.own.sessions.lock().unwrap().len(), 1, "nothing closed, nothing forked");

    let taken = rig.acme("harnix.session.unexpose", with(s.clone(), json!({"to": "alpha"}))).await.unwrap();
    assert_eq!(taken, json!({"instance": "own", "session_id": "s1", "space": "beta", "to": "alpha", "exposed": false, "interact": false, "changed": true}));
    assert_eq!(rig.acme("harnix.session.unexpose", with(s.clone(), json!({"to": "alpha"}))).await.unwrap()["changed"], false);
    assert_eq!(code(rig.device("alpha", "engine.session.get", s.clone()).await), codes::SPACE_MISMATCH);
}

/// A space with no sessions of its own takes none: not created there, by a device or an extension,
/// not imported there, not moved there; it lists none, and the spaces list says so.
#[tokio::test]
async fn a_space_with_no_sessions_takes_none() {
    let dir = tempfile::tempdir().unwrap();
    let rig = Rig::new(dir.path(), FakeLink::new(), FakeLink::new()).await;
    let refused = rig.device("overview", "harnix.session.create", json!({"project_root": null})).await.unwrap_err();
    assert_eq!(refused.code, codes::SPACE_MISMATCH);
    assert!(refused.message.contains("holds no sessions"), "{}", refused.message);
    assert_eq!(code(rig.acme("harnix.session.create", json!({"instance": "own", "space": "overview", "project_root": null})).await), codes::SPACE_MISMATCH);
    assert_eq!(code(rig.device("overview", "harnix.session.import", json!({"path": "/b.zip"})).await), codes::SPACE_MISMATCH);
    let s = rig.made("beta", "own").await;
    assert_eq!(code(rig.acme("harnix.session.move", with(s.clone(), json!({"space": "overview"}))).await), codes::SPACE_MISMATCH);
    assert_eq!(rig.own.sessions.lock().unwrap().len(), 1, "only beta's");
    assert!(rig.ids_in("overview").await.is_empty());
    let listed = rig.call_as(Subject::device("d"), "harnix.spaces.list", json!({})).await.unwrap();
    let row = |id: &str| listed["spaces"].as_array().unwrap().iter().find(|r| r["id"] == id).cloned().unwrap();
    assert_eq!(row("overview")["no_sessions"], json!(true));
    assert!(row("beta").get("no_sessions").is_none());
}

/// A session exposed to a space is in that space's list, saying where it is from, and in its own
/// without; the topic held to a space says the same. A session on an instance built for one space,
/// exposed to another, is reached there too.
#[tokio::test]
async fn a_spaces_list_has_the_sessions_lent_to_it_and_says_so() {
    let dir = tempfile::tempdir().unwrap();
    let rig = Rig::new(dir.path(), FakeLink::new(), FakeLink::new()).await;
    let s = rig.made("beta", "own").await;
    let g = rig.made("gamma", "side").await;
    assert_eq!(rig.ids_in("alpha").await, Vec::<(String, Option<String>)>::new());

    rig.acme("harnix.session.expose", with(s.clone(), json!({"to": "alpha"}))).await.unwrap();
    rig.acme("harnix.session.expose", with(g.clone(), json!({"to": "alpha", "interact": true}))).await.unwrap();
    assert_eq!(rig.ids_in("alpha").await, [("s1".to_string(), Some("beta".to_string())), ("s1".to_string(), Some("gamma".to_string()))]);
    assert_eq!(rig.ids_in("beta").await, [("s1".to_string(), None)]);
    assert_eq!(rig.ids_in("gamma").await, [("s1".to_string(), None)]);
    // The row says every space it is shown in, in its own list too.
    let listed = rig.device("beta", "harnix.sessions.list", json!({})).await.unwrap();
    assert_eq!(listed["sessions"][0]["exposed_to"], json!([{"space": "alpha", "interact": false}]));
    assert_eq!(listed["sessions"][0]["space"], "beta");
    // The instance built for gamma answers a call claiming alpha about the session shown there,
    // and about nothing else of its.
    rig.device("alpha", "engine.session.get", g.clone()).await.unwrap();
    assert_eq!(code(rig.device("alpha", "engine.session.get", json!({"instance": "side", "session_id": "nope"})).await), codes::SPACE_MISMATCH);
    assert_eq!(rig.side.sessions.lock().unwrap().len(), 1);

    rig.acme("harnix.session.unexpose", with(g.clone(), json!({"to": "alpha"}))).await.unwrap();
    assert_eq!(rig.ids_in("alpha").await, [("s1".to_string(), Some("beta".to_string()))]);
}

/// Only the extension that created a session exposes it: not a device, not another extension,
/// and a device's session is nobody's. The target is a space this application has (one with only
/// an interface included: the hub knows no space by name), not one it turned off, not the
/// session's own and not one reaching across spaces.
#[tokio::test]
async fn only_the_owner_exposes_a_session_and_only_to_a_space_there_is() {
    let dir = tempfile::tempdir().unwrap();
    let rig = Rig::new(dir.path(), FakeLink::new(), FakeLink::new()).await;
    let s = rig.made("beta", "own").await;
    let expose = |to: &str| with(s.clone(), json!({"to": to}));

    assert_eq!(code(rig.call_as(Subject::device("d"), "harnix.session.expose", expose("alpha")).await), codes::REFUSED);
    let refused = rig.call_as(ext(OTHER), "harnix.session.expose", expose("alpha")).await.unwrap_err();
    assert_eq!(refused.code, codes::REFUSED);
    assert!(refused.message.contains("`acme`'s to expose, not `other`'s"), "{}", refused.message);
    assert_eq!(code(rig.call_as(ext(OTHER), "harnix.session.unexpose", expose("alpha")).await), codes::REFUSED);
    for to in ["beta", "sys", "off", "bad space"] {
        assert_eq!(code(rig.acme("harnix.session.expose", expose(to)).await), codes::SPACE_MISMATCH, "{to}");
    }
    // A space with only an interface is declared nowhere in the hub, and exists all the same.
    rig.acme("harnix.session.expose", expose("front-only")).await.unwrap();
    assert_eq!(code(rig.acme("harnix.session.expose", s.clone()).await), codes::INVALID_PARAMS, "no `to`");
    assert_eq!(code(rig.acme("harnix.session.expose", json!({"instance": "own", "session_id": "nope", "to": "alpha"})).await), codes::REFUSED, "not acme's: never made");
    // A device's own session: the kernel may still expose it, as it may anything.
    let theirs = rig.device("alpha", "harnix.session.create", json!({"instance": "own", "project_root": null})).await.unwrap();
    let theirs = json!({"instance": "own", "session_id": theirs["session_id"]});
    let refused = rig.acme("harnix.session.expose", with(theirs.clone(), json!({"to": "beta"}))).await.unwrap_err();
    assert!(refused.message.contains("nobody's to expose"), "{}", refused.message);
    rig.call_as(Subject::Kernel, "harnix.session.expose", with(theirs, json!({"to": "beta"}))).await.unwrap();
}

/// An exposure outlives a restart — the ledger has it — and goes with the session it was about.
#[tokio::test]
async fn an_exposure_outlives_a_restart_and_goes_with_the_session() {
    let dir = tempfile::tempdir().unwrap();
    let (own, side) = (FakeLink::new(), FakeLink::new());
    let s = {
        let rig = Rig::new(dir.path(), own.clone(), side.clone()).await;
        let s = rig.made("beta", "own").await;
        rig.acme("harnix.session.expose", with(s.clone(), json!({"to": "alpha", "interact": true}))).await.unwrap();
        rig.hub.shutdown().await;
        s
    };
    let rig = Rig::new(dir.path(), own.clone(), side).await;
    rig.device("alpha", "engine.session.get", s.clone()).await.unwrap();
    rig.device("alpha", "harnix.session.interrupt", s.clone()).await.unwrap();
    assert_eq!(rig.ids_in("alpha").await, [("s1".to_string(), Some("beta".to_string()))]);
    // Still acme's after the restart: another extension cannot take the exposure back.
    assert_eq!(code(rig.call_as(ext(OTHER), "harnix.session.unexpose", with(s.clone(), json!({"to": "alpha"}))).await), codes::REFUSED);
    // Closed from its own space: forgotten, exposure and all.
    rig.device("beta", "harnix.session.delete", s.clone()).await.unwrap();
    assert_eq!(own.sessions.lock().unwrap().len(), 0);
    assert_eq!(rig.ids_in("alpha").await, Vec::<(String, Option<String>)>::new());
    let ledger: Value = serde_json::from_str(&std::fs::read_to_string(dir.path().join("spaces.json")).unwrap()).unwrap();
    assert!(ledger.get("exposures").is_none_or(|e| e.as_object().is_some_and(|o| o.is_empty())), "{ledger}");
}
