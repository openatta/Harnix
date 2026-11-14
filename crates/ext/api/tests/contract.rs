//! The extension interface, used whole by an extension that knows nothing else: every function of
//! its registrar, every service of its context, every helper of a call — on a process started
//! without its listener (`harnix::testing::Kernel`: the real router and hub, an engine that
//! replays and so never calls a model).
//!
//! What it holds the interface to is that each of them reaches the process and answers, and that
//! a service needing a capability refuses, naming it, when the extension was not granted it.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use harnix_ext::catalog::{Frequency, PointKind};
use harnix_ext::engine::{EngineProfile, InstanceTemplate, SessionFacts, Tool, ToolProvider, ToolSpec};
use harnix_ext::http::{full, Body, Request, Response, RouteContext, RouteHandler};
use harnix_ext::ui::{ModuleDecl, PointDecl, SurfaceDecl, UiEntry};
use harnix_ext::{
    caps, codes, Access, Call, Entry, Extension, ExtensionContext, Manifest, MethodHandler, MigrateContext,
    MigrationReport, OnBusy, Registrar, Rpc, SendOptions, Service, SessionSpec, SpaceDecl, Subject, TopicCompute,
    TopicValue,
};
use serde_json::{json, Value};

const ID: &str = "probe";

#[derive(Default, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    #[serde(default)]
    greeting: Option<String>,
}

struct Probe {
    files: PathBuf,
}

struct Clock;

#[async_trait::async_trait]
impl TopicCompute for Clock {
    async fn compute(&self, ext: &ExtensionContext, _params: &Value) -> Rpc<Value> {
        Ok(json!({"by": ext.id().as_str()}))
    }
}

struct Echo;

#[async_trait::async_trait]
impl MethodHandler for Echo {
    async fn call(&self, _call: Call, params: Value) -> Rpc<Value> {
        Ok(params)
    }
}

struct Api;

#[async_trait::async_trait]
impl RouteHandler for Api {
    async fn handle(&self, cx: RouteContext, _req: Request<Body>) -> Response<Body> {
        Response::new(full(cx.ext().id().to_string()))
    }
}

struct Nothing;

impl ToolProvider for Nothing {
    fn name(&self) -> &str {
        "probe"
    }

    fn tools(&self, _: &SessionFacts) -> Vec<Arc<dyn Tool>> {
        Vec::new()
    }
}

/// A service nobody provides: what asking for one is told.
struct Absent;

impl Service for Absent {
    const NAME: &'static str = "probe.absent";
    const CAPABILITY: harnix_ext::Capability = caps::EXTENSIONS_MANAGE;
    type Handle = ();
}

#[async_trait::async_trait]
impl Extension for Probe {
    type Config = Config;

    fn manifest(&self) -> Manifest {
        Manifest::new(ID, "0.1.0").title("Probe").disclosure("uses every part of the interface once").optional([
            caps::RECORDINGS_READ,
            caps::TRANSCRIPTS_READ,
            caps::PROCESS_OBSERVE,
            caps::DEVICES_MANAGE,
            caps::ENGINE_PLUGINS,
            caps::EXTENSIONS_MANAGE,
        ])
    }

    fn register(&self, r: &mut Registrar<'_, Config>) {
        assert_eq!(r.id().as_str(), ID);
        let greeting = r.config().greeting.clone().unwrap_or_default();
        r.method("greet")
            .access(Access::device())
            .schema(json!({"type": "object"}))
            .describe("says the configured greeting")
            .handler(move |_call: Call, p: Value| {
                let greeting = greeting.clone();
                async move { Ok(format!("{greeting}, {}", p["name"].as_str().unwrap_or("you"))) }
            });
        r.method("echo").access(Access::device()).with(Arc::new(Echo));
        r.method("helpers").access(Access::device()).raw(helpers);
        r.topic("ticks").param("n").published();
        r.topic("clock").access(Access::device()).computed(Some(Duration::from_secs(60)), Clock);
        r.space(SpaceDecl::new(ID));
        r.tool(ToolSpec::new("noop", "does nothing", json!({"type": "object"})));
        r.route("api", Access::device(), Api);
        r.static_dir("files", self.files.clone(), Access::device());
        r.ui()
            .bundle(ID)
            .surface(SurfaceDecl::new("main", "files/index.html").title("Probe"))
            .module(ModuleDecl::new("home.card", "files/card.js"))
            .point(PointDecl::new("probe.panel", PointKind::Render, Frequency::PerInteraction).what("a panel"));
        r.ui().inline().command(json!({"trigger": "/", "name": "probe"}));
        r.engine().admit_everywhere("probe__*");
        r.engine_everywhere().provider(Arc::new(Nothing));
        r.template(InstanceTemplate::in_process("side", EngineProfile::new()).ui());
    }

    async fn migrate(&self, cx: &MigrateContext) -> anyhow::Result<MigrationReport> {
        assert!(cx.data_dir().is_dir());
        let config: Config = cx.config()?;
        Ok(MigrationReport::applied(config.greeting.map(|_| "greeting-read")))
    }

    async fn start(&self, cx: ExtensionContext) -> anyhow::Result<()> {
        cx.provide(String::from("provided"));
        Ok(())
    }
}

/// Every helper a call has, used once.
async fn helpers(call: Call, _params: Value) -> Rpc<Value> {
    let state = call.state::<String>()?;
    let waited = call.run(async { Ok(1) }).await?;
    let cancelled = tokio::time::timeout(Duration::from_millis(10), call.cancelled()).await.is_ok();
    let echoed = call.call("probe.echo", json!({"from": "helpers"})).await?;
    Ok(json!({
        "subject": call.subject().label(),
        "device": call.device(),
        "on_behalf_of": call.on_behalf_of().map(|e| e.to_string()),
        "ext": call.ext().id().as_str(),
        "state": *state,
        "deadline": call.deadline().map(|d| d.as_ms()),
        "ran": waited,
        "cancelled": cancelled,
        "echoed": echoed,
    }))
}

#[tokio::test]
async fn an_extension_reaches_the_process_through_the_interface_alone() {
    let dir = tempfile::tempdir().unwrap();
    // An engine keeps its project data in the working directory: this test's own.
    std::env::set_current_dir(dir.path()).unwrap();
    let files = dir.path().join("files");
    std::fs::create_dir_all(&files).unwrap();
    std::fs::write(dir.path().join("profile.toml"), "").unwrap();
    let app = harnix::App::builder()
        .extension(Probe { files })
        .grant([caps::RECORDINGS_READ, caps::TRANSCRIPTS_READ, caps::PROCESS_OBSERVE, caps::DEVICES_MANAGE, caps::EXTENSIONS_MANAGE]);
    let kernel = harnix::testing::Kernel::start(app, dir.path()).await.expect("the process starts");
    let status = kernel.extensions();
    let probe = status.iter().find(|s| s.id == ID).expect("listed");
    assert!(probe.enabled, "{:?}", probe.reason);
    assert_eq!(probe.templates, ["probe.side"]);
    assert_eq!(probe.tools, ["noop"]);
    assert!(probe.routes);
    let cx = kernel.context(ID).expect("started, and seated");
    let device = Subject::device("d");

    // The methods, as a device calls them.
    let greeted = kernel.call(device.clone(), "probe.greet", json!({"name": "ann"})).await.unwrap();
    assert_eq!(greeted, json!(", ann"));
    assert_eq!(kernel.call(device.clone(), "probe.echo", json!({"x": 1})).await.unwrap(), json!({"x": 1}));
    let helped = kernel.call(device.clone(), "probe.helpers", json!({})).await.unwrap();
    assert_eq!(helped["device"], "d");
    assert_eq!(helped["ext"], ID);
    assert_eq!(helped["state"], "provided");
    assert_eq!(helped["ran"], 1);
    assert_eq!(helped["cancelled"], false);
    assert_eq!(helped["echoed"], json!({"from": "helpers"}));
    let listed = kernel.call(device.clone(), "harnix.extensions", json!({})).await.unwrap();
    assert!(listed["extensions"].as_array().unwrap().iter().any(|e| e["id"] == ID && e["trust"] == "origin"));

    // What it is, and what it holds.
    assert_eq!(cx.id().as_str(), ID);
    assert!(cx.grants().has(&caps::PROCESS_OBSERVE));
    assert!(!cx.grants().has(&caps::ENGINE_PLUGINS));
    assert_eq!(cx.grants().capabilities().count(), 5);
    assert!(cx.config::<Config>().unwrap().greeting.is_none());
    assert!(cx.data_dir().is_dir());
    assert!(cx.credentials().get("probe/none").is_err());
    assert!(*cx.state::<String>().unwrap() == "provided");
    assert!(cx.listen().is_none(), "nothing listens here");

    // A call as itself.
    let diagnostics = cx.call("harnix.diagnostics", json!({})).await.unwrap();
    assert!(diagnostics["versions"]["ext_api"].is_u64());

    // Sessions.
    let sessions = cx.sessions();
    let session = sessions.create(SessionSpec::new().space(ID)).await.expect("a session in its space");
    sessions.open(&session.instance, &session.session_id).await.unwrap();
    let mut tap = sessions.tap(&session).await.unwrap();
    assert_eq!(tap.session(), &session);
    sessions.append_entry(&session, Entry::new("probe.note", json!({"n": 1}))).await.unwrap();
    let entries = sessions.entries(&session).await.unwrap();
    assert_eq!(entries.len(), 1, "{entries:?}");
    let _ = tokio::time::timeout(Duration::from_millis(100), tap.next()).await;
    sessions.send(&session, "hi", OnBusy::Queue).await.unwrap();
    sessions.send_with(&session, "again", SendOptions::new().on_busy(OnBusy::Queue)).await.unwrap();
    sessions.interrupt(&session).await.unwrap();
    sessions.move_to(&session, ID).await.unwrap();
    sessions.close(&session).await.unwrap();
    drop(tap);

    // Instances, from its own template.
    let instances = cx.instances();
    let built = instances.ensure("probe.side", "one", BTreeMap::new()).await.unwrap();
    let one = built["id"].as_str().or(built["instance"].as_str()).unwrap_or("one").to_string();
    instances.wait_ready(&one, Duration::from_secs(30)).await.unwrap();
    instances.ensure_ready("probe.side", "one", BTreeMap::new(), Duration::from_secs(30)).await.unwrap();
    assert!(instances.create("probe.side", "one", BTreeMap::new()).await.is_err(), "it exists already");
    instances.stop(&one).await.unwrap();

    // Topics: its own to publish on, and any to watch as itself.
    let mut ticks = cx.topics().subscribe("probe.ticks", json!({"n": 1})).await.unwrap();
    assert_eq!(ticks.next().await, Some(TopicValue::Value(Value::Null)));
    cx.topics().publish("ticks", json!({"n": 1, "extra": 2}), json!({"t": 1})).await.unwrap();
    let pushed = tokio::time::timeout(Duration::from_secs(5), ticks.next()).await.unwrap();
    assert_eq!(pushed, Some(TopicValue::Value(json!({"t": 1}))));
    assert!(cx.topics().publish("nope", json!({}), json!(1)).await.is_err(), "only its own");
    cx.topics().changed("clock", json!({})).unwrap();
    let mut clock = cx.topics().subscribe("probe.clock", json!({})).await.unwrap();
    assert_eq!(clock.next().await, Some(TopicValue::Value(json!({"by": ID}))));

    // The engine, as facts.
    let main = cx.engine("main").expect("the main instance");
    assert_eq!(main.instance(), "main");
    assert!(!main.active_scenes().is_empty());
    let _ = main.model();
    assert!(!main.trust().is_extension());
    main.recordings().expect("granted recordings.read");
    main.transcripts().expect("granted transcripts.read");
    let refused = main.skills().unwrap_err();
    assert_eq!(refused.code, codes::REFUSED);
    assert_eq!(refused.data.unwrap()["capability"], "engine.plugins");
    assert!(cx.engine("nowhere").is_none());

    // The process, uploads, interface entries, devices, a service.
    assert!(cx.process().unwrap().uptime_secs().is_some());
    let grant = cx.bulk().grant(1024).unwrap();
    assert!(!grant.token().is_empty() && grant.max_bytes() == 1024 && grant.path().parent().unwrap().is_dir());
    let named = cx.bulk().grant_named("../a b.zip", 1024).unwrap();
    assert!(named.path().file_name().unwrap().to_str().unwrap().ends_with("-.._a_b.zip"), "{}", named.path().display());
    assert_eq!(named.path().parent(), grant.path().parent());
    cx.ui().publish(vec![UiEntry::new("home.card", "/ext/probe/files/card.js", harnix_ext::CONTRIB_API_VERSION)]);
    let devices = cx.devices().unwrap();
    assert!(devices.list()["devices"].is_array());
    let pairing = devices.pair_begin("phone");
    assert!(pairing["code"].is_string());
    assert!(!devices.revoke("nobody"));
    assert!(devices.pair_complete("WRONG", "key").is_err());
    assert_eq!(cx.service::<Absent>().unwrap_err().code, codes::NOT_FOUND);

    kernel.shutdown().await;
}
