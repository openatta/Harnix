//! The runtime against the real showcase application (`examples/apps/showcase`): install it, use
//! its tools the way a session does, switch it off and on, take it out — and every way an install
//! can fail, undone.
//!
//! The process around it is a stand-in kernel that answers the engine methods the runtime calls
//! (`plugin.install` and friends) and records them, with one instance, `system`, whose data root
//! is a temporary directory.
#![cfg(feature = "compile")]

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use base::interface::tool::{PermissionDecision, ProgressCallback, ProgressSender, ToolContext};
use base::interface::tool_middleware::{dispatch_through, ToolAnswer, ToolCall};
use harnix_apps::{AppsPlugin, Options, Runtime};
use harnix_builtin::apps::{AppsRuntime, InstallRecord};
use harnix_contract::{RpcError, Subject};
use harnix_module::apps::AppRuntime;
use harnix_module::{
    AppSpec, AppSummary, AppWithdrawn, InstanceHandle, Kernel, ModuleContext, SessionRef, Tap,
};
use serde_json::{json, Value};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The showcase's component, built once per test binary (and cached across runs by cargo).
fn showcase_wasm() -> Vec<u8> {
    static BUILT: OnceLock<Vec<u8>> = OnceLock::new();
    BUILT
        .get_or_init(|| {
            let component = root().join("examples/apps/showcase/component");
            let target = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("showcase");
            let status = std::process::Command::new(env!("CARGO"))
                .args(["build", "--release", "--target", "wasm32-wasip2"])
                .env("CARGO_TARGET_DIR", &target)
                .current_dir(&component)
                .status()
                .expect("cargo runs");
            assert!(
                status.success(),
                "the showcase component did not build (rustup target add wasm32-wasip2)"
            );
            std::fs::read(target.join("wasm32-wasip2/release/showcase_tools.wasm")).unwrap()
        })
        .clone()
}

/// The engine methods called, and which should fail.
#[derive(Default)]
struct FakeKernel {
    data_root: PathBuf,
    calls: Mutex<Vec<(String, Value)>>,
    /// `(method, instance)` that answers an error.
    fail: Mutex<Vec<(String, String)>>,
    instances: Vec<String>,
    /// The application host, for the rigs that have one.
    apps: Option<Arc<FakeApps>>,
}

/// A stand-in for the kernel's application host: it keeps what was registered, answers the
/// application's own methods with their handlers, and lets the application call only what its
/// `hub` lists — what `crates/app`'s real host does through the authorizer.
#[derive(Default)]
struct FakeApps {
    specs: Mutex<Vec<AppSpec>>,
    published: Mutex<Vec<(String, Value)>>,
    /// What the application called as itself and was let through.
    called: Mutex<Vec<String>>,
    withdrawn: Mutex<Vec<String>>,
}

impl FakeApps {
    fn spec(&self, id: &str) -> Option<AppSpec> {
        self.specs.lock().unwrap().iter().find(|s| s.id == id).cloned()
    }

    async fn device_call(&self, id: &str, method: &str, params: Value) -> Result<Value, RpcError> {
        let spec = self.spec(id).ok_or_else(|| RpcError::not_found("withdrawn"))?;
        let m = spec.methods.iter().find(|m| m.name == method).unwrap().clone();
        m.handler.call(&Subject::Device { id: "d1".into() }, params).await
    }
}

#[async_trait::async_trait]
impl AppRuntime for FakeApps {
    async fn register(&self, by: &str, spec: AppSpec) -> Result<AppSummary, RpcError> {
        let summary = AppSummary {
            id: spec.id.clone(),
            by: by.into(),
            methods: spec.methods.iter().map(|m| format!("app.{}.{}", spec.id, m.name)).collect(),
            topics: spec.topics.iter().map(|t| format!("app.{}.{}", spec.id, t.name)).collect(),
            spaces: spec.spaces.iter().map(|s| s.id.clone()).collect(),
            hub: spec.hub.clone(),
            since_ms: 0,
        };
        self.specs.lock().unwrap().push(spec);
        Ok(summary)
    }
    async fn withdraw(&self, _by: &str, id: &str) -> Result<AppWithdrawn, RpcError> {
        let mut specs = self.specs.lock().unwrap();
        let before = specs.len();
        specs.retain(|s| s.id != id);
        if specs.len() == before {
            return Err(RpcError::invalid_params("not registered"));
        }
        self.withdrawn.lock().unwrap().push(id.into());
        Ok(AppWithdrawn {
            id: id.into(),
            methods: vec![],
            topics: vec![],
            spaces: vec![],
            subscriptions_ended: 0,
        })
    }
    async fn publish(&self, _by: &str, id: &str, topic: &str, _params: Value, value: Value) -> Result<(), RpcError> {
        self.published.lock().unwrap().push((format!("app.{id}.{topic}"), value));
        Ok(())
    }
    fn registered(&self) -> Vec<AppSummary> {
        vec![]
    }
    async fn call_as(&self, _by: &str, id: &str, method: &str, params: Value) -> Result<Value, RpcError> {
        let spec = self.spec(id).ok_or_else(|| RpcError::invalid_params("not registered"))?;
        if let Some(own) = method.strip_prefix(&format!("app.{id}.")) {
            let m = spec.methods.iter().find(|m| m.name == own).unwrap().clone();
            return m.handler.call(&Subject::Module { id: id.into() }, params).await;
        }
        if !spec.hub.iter().any(|h| h == method) {
            return Err(RpcError::refused(format!("`module:{id}` may not call `{method}`: not in its `hub`")));
        }
        self.called.lock().unwrap().push(method.into());
        Ok(json!({"session_id": "s-from-the-backend"}))
    }
    async fn subscribe_as(&self, _by: &str, id: &str, topic: &str, _params: Value) -> Result<Value, RpcError> {
        let spec = self.spec(id).unwrap();
        if !spec.hub.contains(&format!("topic:{topic}")) {
            return Err(RpcError::refused("not in its `hub`"));
        }
        // A value now, as the hub gives a subscriber, so the backend's `on-topic` sees one.
        Ok(json!({"topic": topic, "value": {"sessions": [{"session_id": "a"}]}}))
    }
}

#[async_trait::async_trait]
impl Kernel for FakeKernel {
    async fn call(
        &self,
        _subject: &Subject,
        method: &str,
        params: Value,
    ) -> Result<Value, RpcError> {
        self.calls
            .lock()
            .unwrap()
            .push((method.to_string(), params.clone()));
        let instance = params["instance"].as_str().unwrap_or("system").to_string();
        if self
            .fail
            .lock()
            .unwrap()
            .iter()
            .any(|(m, i)| m == method && *i == instance)
        {
            return Err(RpcError::invalid_params(format!(
                "{method} failed in {instance}"
            )));
        }
        if method == "plugin.install" {
            let url = params["download_url"].as_str().unwrap();
            let zip = url.strip_prefix("file://").unwrap();
            assert!(
                Path::new(zip).is_file(),
                "plugin.install was handed {url}, which is not a file"
            );
        }
        Ok(json!({}))
    }
    async fn publish(&self, _topic: &str, _params: Value, _value: Value) {}
    fn instance(&self, id: &str) -> Option<InstanceHandle> {
        self.instances
            .iter()
            .any(|i| i == id)
            .then(|| InstanceHandle {
                id: id.to_string(),
                data_root: self.data_root.join(id),
                recordings_root: self.data_root.join("recordings"),
                history: None,
                active_scenes: vec![],
                model: String::new(),
            })
    }
    async fn tap(
        &self,
        _subject: &Subject,
        _session: &SessionRef,
        _capacity: usize,
    ) -> Result<Tap, RpcError> {
        Err(RpcError::refused("no taps here"))
    }
    fn apps(&self, module: &str) -> Option<Arc<dyn AppRuntime>> {
        (module == "apps")
            .then(|| self.apps.clone().map(|a| a as Arc<dyn AppRuntime>))
            .flatten()
    }
}

impl FakeKernel {
    fn called(&self, method: &str) -> Vec<Value> {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(m, _)| m == method)
            .map(|(_, p)| p.clone())
            .collect()
    }
}

struct Rig {
    dir: tempfile::TempDir,
    kernel: Arc<FakeKernel>,
    cx: ModuleContext,
    runtime: Arc<Runtime>,
}

impl Rig {
    fn new(dev: bool) -> Self {
        Self::with_apps(dev, None)
    }

    fn with_apps(dev: bool, apps: Option<Arc<FakeApps>>) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let kernel = Arc::new(FakeKernel {
            data_root: dir.path().join("engine"),
            instances: vec!["system".into(), "other".into()],
            apps,
            ..Default::default()
        });
        let cx = ModuleContext::new(
            "apps",
            "harnix.apps",
            Value::Null,
            dir.path().join("module"),
            Arc::new(harnix_module::EnvCredentials),
            kernel.clone(),
            vec![],
        );
        let runtime = Runtime::new(Options {
            dev,
            config: [(
                "example.showcase".to_string(),
                toml::Value::Table(toml::toml! { greeting = "set by the profile" }),
            )]
            .into(),
        })
        .unwrap();
        Rig {
            dir,
            kernel,
            cx,
            runtime,
        }
    }

    fn apps(&self) -> PathBuf {
        self.dir.path().join("apps")
    }

    /// The showcase packed for this engine, with `edit` applied to its manifest.
    fn package(&self, name: &str, edit: impl Fn(String) -> String) -> PathBuf {
        let source = root().join("examples/apps/showcase");
        let app = self.dir.path().join("src").join(name);
        for part in ["ui", "agent"] {
            copy_tree(&source.join(part), &app.join(part));
        }
        let manifest = std::fs::read_to_string(source.join("harnix-app.toml")).unwrap();
        std::fs::write(app.join("harnix-app.toml"), edit(manifest)).unwrap();
        let wasm = showcase_wasm();
        let cwasm = self.runtime.engine().compile(&wasm).unwrap();
        let out = self.dir.path().join(format!("{name}.hxapp"));
        let artifact = harnix_apps::package::Artifact {
            fingerprint: self.runtime.engine().fingerprint(),
            cwasm: &cwasm,
            wasm: Some(&wasm),
        };
        harnix_apps::package::pack(&app, Some(artifact), &out).unwrap();
        out
    }

    fn skill(&self, instance: &str) -> PathBuf {
        self.dir
            .path()
            .join("engine")
            .join(instance)
            .join("skills/showcase-tour")
    }

    /// Install as the `apps` module does: the runtime's part, then the record.
    async fn install(&self, package: &Path) -> Result<InstallRecord, RpcError> {
        let info = self.runtime.inspect(package)?;
        let app_dir = self.apps().join(&info.id);
        let (parts, _summary) = self.runtime.install(&self.cx, package, &app_dir).await?;
        Ok(InstallRecord {
            id: info.id,
            version: info.version,
            title: info.title,
            enabled: true,
            parts,
        })
    }
}

fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() {
            copy_tree(&path, &to.join(entry.file_name()));
        } else {
            std::fs::copy(&path, to.join(entry.file_name())).unwrap();
        }
    }
}

#[derive(Default)]
struct Progress(Mutex<Vec<String>>);

impl ProgressCallback for Progress {
    fn on_progress(&self, _id: &str, data: &str) {
        self.0.lock().unwrap().push(data.to_string());
    }
}

const ID: &str = "example.showcase";

#[tokio::test(flavor = "multi_thread")]
async fn installed_the_showcase_gives_a_session_its_tools_and_they_run() {
    let rig = Rig::new(true);
    let record = rig.install(&rig.package("good", |m| m)).await.unwrap();
    assert_eq!(record.parts["agent"]["package"], json!("example-showcase"));
    assert_eq!(record.parts["agent"]["instances"], json!(["system"]));
    assert_eq!(record.parts["skills"]["system"], json!(["showcase-tour"]));
    let installs = rig.kernel.called("plugin.install");
    assert_eq!(installs.len(), 1);
    assert_eq!(installs[0]["name"], json!("example-showcase"));
    assert!(
        rig.skill("system").join("SKILL.md").is_file(),
        "the skill was not placed"
    );
    assert!(
        !rig.kernel.called("config.reload").is_empty(),
        "the instance was not told to re-read"
    );
    assert_eq!(rig.runtime.status(ID)["state"], json!("running"));
    assert_eq!(
        rig.runtime.ui_contributions(),
        [
            json!({"plugin": ID, "point": "home.card", "api_version": 2, "module": "/apps/example.showcase/ui/showcase.js"}),
            json!({"plugin": ID, "point": "space", "api_version": 2, "module": "/apps/example.showcase/ui/showcase.js"}),
        ]
    );
    assert!(
        rig.runtime.mcp_servers().is_empty(),
        "no listener yet, so nothing to point a daemon at"
    );
    rig.runtime.set_listening("http://127.0.0.1:4999/");
    let servers = rig.runtime.mcp_servers();
    assert_eq!(
        servers["app-example_showcase"]["url"],
        json!("http://127.0.0.1:4999/apps/example.showcase/mcp")
    );

    // A session's view: every tool, namespaced, and every one asks.
    let plugin = AppsPlugin::new(rig.runtime.registry());
    let project = rig.dir.path().join("project");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(project.join("NOTES.md"), "the project's notes").unwrap();
    let tools = plugin.tools(&project);
    let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
    for tool in ["echo", "count", "config", "read_project", "slow", "vetoed", "board"] {
        assert!(
            names.contains(&format!("app__example_showcase__{tool}").as_str()),
            "{names:?}"
        );
    }
    let tool = |name: &str| {
        tools
            .iter()
            .find(|t| t.name() == format!("app__example_showcase__{name}"))
            .unwrap()
            .clone()
    };
    let ctx = || ToolContext::for_test(project.clone());
    assert!(matches!(
        tool("echo").check_permissions(&json!({}), &ctx()).await,
        PermissionDecision::Ask { .. }
    ));
    let run = |name: &'static str, input: Value| {
        let tool = tool(name);
        let ctx = ctx();
        async move {
            tool.call(input, ctx, ProgressSender::noop("t"))
                .await
                .unwrap()
        }
    };
    let echoed = run("echo", json!({"text": "hi"})).await;
    assert_eq!(echoed.structured_content, Some(json!({"echoed": "hi"})));
    assert!(!echoed.is_error);
    assert!(format!("{:?}", run("count", json!({})).await.content).contains("count = 1"));
    assert!(
        format!("{:?}", run("count", json!({})).await.content).contains("count = 2"),
        "the store did not outlive the call"
    );
    let config = run("config", json!({})).await.structured_content.unwrap();
    assert_eq!(config["greeting"], json!("set by the profile"));
    assert_eq!(
        config["blocked_tool"],
        json!("app__example_showcase__vetoed")
    );
    let read = run("read_project", json!({"path": "NOTES.md"})).await;
    assert!(
        format!("{:?}", read.content).contains("the project's notes"),
        "{read:?}"
    );

    // Progress, and a cancelled call that ends.
    let progress = Arc::new(Progress::default());
    let out = tool("slow")
        .call(
            json!({"steps": 3, "ms": 10}),
            ctx(),
            ProgressSender::with_callback("p", progress.clone()),
        )
        .await
        .unwrap();
    assert!(!out.is_error);
    assert_eq!(
        *progress.0.lock().unwrap(),
        ["step 1 of 3", "step 2 of 3", "step 3 of 3"]
    );
    let cancelled = ctx();
    let token = cancelled.cancel.clone();
    let slow = tool("slow");
    let call = tokio::spawn(async move {
        slow.call(
            json!({"steps": 100, "ms": 200}),
            cancelled,
            ProgressSender::noop("c"),
        )
        .await
    });
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    let started = std::time::Instant::now();
    token.cancel();
    let ended = call.await.unwrap();
    assert!(
        matches!(ended, Err(base::error::ToolError::Cancelled)),
        "{ended:?}"
    );
    assert!(started.elapsed() < std::time::Duration::from_secs(2));

    // The event hook refuses the tool its configuration names, and lets the rest through.
    let middleware = plugin.middleware("s1", 0);
    assert_eq!(middleware.len(), 1);
    let ran = Arc::new(Mutex::new(Vec::new()));
    for name in ["app__example_showcase__vetoed", "Bash"] {
        let call = ToolCall {
            name: name.into(),
            input: json!({}),
        };
        let seen = ran.clone();
        let outcome = dispatch_through(
            &middleware,
            call,
            tokio_util::sync::CancellationToken::new(),
            move |_cancel| {
                seen.lock().unwrap().push(name.to_string());
                async { Ok(ToolAnswer::text("ran")) }
            },
        )
        .await
        .unwrap();
        if name == "Bash" {
            assert_eq!(outcome.text, "ran");
        } else {
            assert!(
                outcome.reported_failure && outcome.text.contains("event hook"),
                "{outcome:?}"
            );
        }
    }
    assert_eq!(*ran.lock().unwrap(), ["Bash"], "the refused call ran");

    // Off: no tools, no modules, no MCP server, the agent package and the skill switched off.
    rig.runtime.unload(&rig.cx, &record).await;
    assert!(plugin.tools(&project).is_empty());
    assert!(rig.runtime.ui_contributions().is_empty() && rig.runtime.mcp_servers().is_empty());
    assert_eq!(rig.kernel.called("plugin.disable").len(), 1);
    assert!(!rig.skill("system").exists());
    // On again, from the record.
    rig.runtime
        .load(&rig.cx, &record, &rig.apps().join(ID))
        .await
        .unwrap();
    assert_eq!(plugin.tools(&project).len(), 7);
    assert!(rig.skill("system").is_dir());
    // Out: what was placed elsewhere is taken back.
    rig.runtime.unload(&rig.cx, &record).await;
    rig.runtime
        .uninstall(&rig.cx, &record, &rig.apps().join(ID))
        .await
        .unwrap();
    assert_eq!(rig.kernel.called("plugin.uninstall").len(), 1);
    assert!(!rig.skill("system").exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn the_backend_is_registered_answers_its_methods_and_acts_only_as_far_as_its_hub() {
    let apps = Arc::new(FakeApps::default());
    let rig = Rig::with_apps(true, Some(apps.clone()));
    let record = rig.install(&rig.package("backend", |m| m)).await.unwrap();

    // Registered before the VM started: its methods, its topics, its space and its `hub`.
    let spec = apps.spec(ID).expect("registered");
    let names: Vec<&str> = spec.methods.iter().map(|m| m.name.as_str()).collect();
    for name in ["notes.add", "notes.list", "state", "session.start", "overreach"] {
        assert!(names.contains(&name), "{names:?}");
    }
    let topics: Vec<&str> = spec.topics.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(topics, ["notes", "heartbeat"]);
    assert_eq!(spec.spaces[0].id, ID);
    assert!(spec.hub.contains(&"topic:harnix.sessions.status".to_string()));
    let state = rig.runtime.status(ID);
    assert_eq!(state["world"], json!("backend"));
    assert_eq!(state["health"]["subscriptions"], json!(1), "init subscribed");

    // A method: checked against its schema, answered by the backend, its topic published in order.
    let added = apps.device_call(ID, "notes.add", json!({"text": "one"})).await.unwrap();
    assert_eq!(added, json!({"count": 1}));
    apps.device_call(ID, "notes.add", json!({"text": "two"})).await.unwrap();
    let bad = apps.device_call(ID, "notes.add", json!({"text": 3})).await.unwrap_err();
    assert_eq!(bad.code, harnix_contract::codes::INVALID_PARAMS);
    assert!(bad.message.contains("schema"), "{}", bad.message);
    let listed = apps.device_call(ID, "notes.list", json!({})).await.unwrap();
    assert_eq!(listed["notes"][0]["by"], json!("device:d1"), "the backend is told who called");
    for _ in 0..50 {
        if apps.published.lock().unwrap().len() >= 2 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    let counts: Vec<Value> = apps.published.lock().unwrap().iter().map(|(_, v)| v["count"].clone()).collect();
    assert_eq!(counts, [json!(1), json!(2)]);

    // What it subscribed to in `init` reached `on-topic`, once the VM was up to take it.
    let mut state = Value::Null;
    for _ in 0..50 {
        state = apps.device_call(ID, "state", json!({})).await.unwrap();
        if state["status_seen"] == json!(1) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert_eq!((state["status_seen"].clone(), state["running_sessions"].clone()), (json!(1), json!(1)));

    // As itself: what its `hub` lists goes through; anything else is refused, and kept.
    let started = apps
        .device_call(ID, "session.start", json!({"message": "hello"}))
        .await
        .unwrap();
    assert_eq!(started["session_id"], json!("s-from-the-backend"));
    assert_eq!(*apps.called.lock().unwrap(), ["session.create", "harnix.send"]);
    let over = apps.device_call(ID, "overreach", json!({})).await.unwrap();
    assert_eq!(over["refused"], json!(true));
    let status = rig.runtime.status(ID);
    assert_eq!(status["calls"]["refused"][0]["method"], json!("harnix.apps.uninstall"));

    // A tool cannot see the backend's memory; it asks, as the application, and is answered.
    let plugin = AppsPlugin::new(rig.runtime.registry());
    let project = rig.dir.path().join("project");
    std::fs::create_dir_all(&project).unwrap();
    let board = plugin
        .tools(&project)
        .into_iter()
        .find(|t| t.name() == "app__example_showcase__board")
        .unwrap();
    let out = board
        .call(json!({}), ToolContext::for_test(project.clone()), ProgressSender::noop("b"))
        .await
        .unwrap();
    assert!(format!("{:?}", out.content).contains("the board has 2 notes"), "{out:?}");
    drop(board);

    // Stopped: the registration goes with it, and a method still held by the kernel answers that
    // the application is not running, not something that looks like an answer.
    let held = spec.methods.iter().find(|m| m.name == "notes.list").unwrap().clone();
    drop(spec);
    rig.runtime.unload(&rig.cx, &record).await;
    assert_eq!(*apps.withdrawn.lock().unwrap(), [ID]);
    let gone = held.handler.call(&Subject::Kernel, json!({})).await.unwrap_err();
    assert_eq!(gone.code, harnix_contract::codes::INTERNAL_ERROR);
    let failure = gone.data.unwrap()["failure"].clone();
    assert!(failure == json!("stopped") || failure == json!("unavailable"), "{failure}: {}", gone.message);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_failed_install_is_undone_and_leaves_nothing_behind() {
    // The second instance refuses the agent package: the first is uninstalled again.
    let rig = Rig::new(true);
    rig.kernel
        .fail
        .lock()
        .unwrap()
        .push(("plugin.install".into(), "other".into()));
    let package = rig.package("two", |m| {
        m.replace(
            "install_into = [\"system\"]",
            "install_into = [\"system\", \"other\"]",
        )
    });
    let err = rig.install(&package).await.unwrap_err();
    assert!(
        err.message.contains("did not install into `other`"),
        "{}",
        err.message
    );
    let uninstalled = rig.kernel.called("plugin.uninstall");
    assert_eq!(uninstalled.len(), 1);
    assert_eq!(uninstalled[0]["instance"], json!("system"));
    assert!(
        !rig.apps().join(ID).exists(),
        "the unpacked package was left behind"
    );
    assert!(!rig.skill("system").exists());
    assert_eq!(rig.runtime.status(ID)["state"], json!("stopped"));

    // An instance that is not there: refused before anything reaches the engine.
    let rig = Rig::new(true);
    let package = rig.package("nowhere", |m| {
        m.replace(
            "install_into = [\"system\"]",
            "install_into = [\"nowhere\"]",
        )
    });
    let err = rig.install(&package).await.unwrap_err();
    assert!(err.message.contains("no such instance"), "{}", err.message);
    assert!(rig.kernel.called("plugin.install").is_empty());

    // A skill the instance has of its own is not replaced, and the install is undone.
    let rig = Rig::new(true);
    std::fs::create_dir_all(rig.skill("system")).unwrap();
    std::fs::write(rig.skill("system").join("SKILL.md"), "mine").unwrap();
    let err = rig.install(&rig.package("skill", |m| m)).await.unwrap_err();
    assert!(err.message.contains("of its own"), "{}", err.message);
    assert_eq!(
        std::fs::read_to_string(rig.skill("system").join("SKILL.md")).unwrap(),
        "mine"
    );
    assert_eq!(
        rig.kernel.called("plugin.uninstall").len(),
        1,
        "the agent package stayed"
    );

    // Not trusted to run unsigned code: the component is not loaded, and the install is undone.
    let rig = Rig::new(false);
    let err = rig
        .install(&rig.package("unsigned", |m| m))
        .await
        .unwrap_err();
    assert!(err.message.contains("[apps] dev = true"), "{}", err.message);
    assert!(!rig.apps().join(ID).exists());
    assert_eq!(rig.kernel.called("plugin.uninstall").len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_artifact_for_another_engine_is_refused_by_name() {
    let rig = Rig::new(true);
    let good = rig.package("other-engine", |m| m);
    // Rename the artifact as if compiled for another engine.
    let unpacked = rig.dir.path().join("unpacked");
    harnix_apps::package::extract(&good, &unpacked).unwrap();
    let fingerprint = rig.runtime.engine().fingerprint().to_string();
    std::fs::rename(
        unpacked.join(format!("wasm/tools.{fingerprint}.cwasm")),
        unpacked.join("wasm/tools.engine-0.0.1+elsewhere+abcdefabcdef.cwasm"),
    )
    .unwrap();
    let repacked = rig.dir.path().join("repacked.hxapp");
    harnix_apps::package::zip_dir(&unpacked, &repacked).unwrap();
    let err = rig.runtime.inspect(&repacked).unwrap_err();
    assert!(
        err.message.contains("engine-0.0.1+elsewhere+abcdefabcdef"),
        "{}",
        err.message
    );
    assert!(err.message.contains(&fingerprint), "{}", err.message);
}
