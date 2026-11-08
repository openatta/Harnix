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
use harnix_module::{InstanceHandle, Kernel, ModuleContext, SessionRef, Tap};
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
        let dir = tempfile::tempdir().unwrap();
        let kernel = Arc::new(FakeKernel {
            data_root: dir.path().join("engine"),
            instances: vec!["system".into(), "other".into()],
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
            ..Options::default()
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
            json!({"plugin": ID, "point": "home.card", "api_version": 2, "module": "/apps/example.showcase/ui/showcase.js"})
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
    for tool in ["echo", "count", "config", "read_project", "slow", "vetoed"] {
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
    assert_eq!(plugin.tools(&project).len(), 6);
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

    // Not trusted to run unsigned code: refused before anything of it is written.
    let rig = Rig::new(false);
    let err = rig
        .install(&rig.package("unsigned", |m| m))
        .await
        .unwrap_err();
    assert!(err.message.contains("[apps] dev = true"), "{}", err.message);
    assert!(!rig.apps().join(ID).exists());
    assert!(rig.kernel.called("plugin.install").is_empty());
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
