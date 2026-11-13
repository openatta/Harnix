//! A host with nothing of a product around it: a fake kernel and agent engine that keep what they
//! were told and answer what the showcase asks of them, and the port between them carried through
//! JSON both ways (`harnix_wasm_port::rpc::Loopback`), as it would be over a link.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use async_trait::async_trait;
use harnix_wasm_host::{Host, HostOptions};
use harnix_wasm_port::rpc::Loopback;
use harnix_wasm_port::*;
use harnix_contract::sync::lock;
use serde_json::{json, Value};

pub const ID: &str = "example.showcase";

pub fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The showcase's component, built once per test binary (and cached across runs by cargo).
pub fn showcase_wasm() -> Vec<u8> {
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

/// What the kernel and the agent engine were asked, in order, as `"<method> <detail>"`.
#[derive(Default)]
pub struct FakePlatform {
    pub said: Mutex<Vec<String>>,
    pub registered: Mutex<Vec<Registration>>,
    pub published: Mutex<Vec<(String, Value)>>,
    pub progress: Mutex<Vec<String>>,
    pub offers: Mutex<Vec<Offer>>,
    /// The host, as the kernel reaches it: across the loopback too.
    pub control: OnceLock<Arc<dyn Control>>,
    /// When set, a call the application makes that its `calls` lists is answered by this method of
    /// its own instead, called from another task: a product that routes it back into the
    /// application over a link, where nothing of the calling task comes along.
    pub reroute: OnceLock<String>,
    /// When set, a call the application makes into its own methods is answered from another task
    /// too, as a product that routes it over a link does.
    pub across: std::sync::atomic::AtomicBool,
    /// The metadata of every call the application made as itself, in order.
    pub metas: Mutex<Vec<CallMeta>>,
    /// What a call routed back into the application answered when it failed, and how long it took.
    pub routed_errors: Mutex<Vec<(RpcError, std::time::Duration)>>,
}

impl FakePlatform {
    fn say(&self, what: String) {
        lock(&self.said).push(what);
    }

    pub fn said(&self, prefix: &str) -> Vec<String> {
        self.said
            .lock()
            .unwrap()
            .iter()
            .filter(|s| s.starts_with(prefix))
            .cloned()
            .collect()
    }

    fn calls_of(&self, app: &str) -> Vec<String> {
        self.registered
            .lock()
            .unwrap()
            .iter()
            .find(|r| r.app == app)
            .map(|r| r.calls.clone())
            .unwrap_or_default()
    }

    /// One of the application's methods, called as a device would.
    pub async fn device_call(&self, method: &str, params: Value) -> Result<Value, RpcError> {
        self.device_call_as(CallMeta::new(next_id()), method, params).await
    }

    /// The same, with the call's metadata as given.
    pub async fn device_call_as(&self, meta: CallMeta, method: &str, params: Value) -> Result<Value, RpcError> {
        self.control
            .get()
            .unwrap()
            .call_method(
                meta,
                MethodCall {
                    app: ID.into(),
                    method: method.into(),
                    caller: Subject::Device { id: "d1".into() },
                    params,
                },
            )
            .await
    }
}

/// A call id of this product's own: never the same twice.
pub fn next_id() -> String {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    format!("p{}", NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed))
}

/// A call the product routes on, made while handling `meta`: a new id of its own, and the chain
/// the call came with — the extensions it holds, each on the call it holds them on.
pub fn routed(meta: &CallMeta) -> CallMeta {
    CallMeta::new(next_id()).chain(meta.links().iter().cloned())
}

#[async_trait]
impl Kernel for FakePlatform {
    async fn register(&self, _: CallMeta, r: Registration) -> Result<(), RpcError> {
        self.say(format!("register {}", r.app));
        lock(&self.registered).push(r);
        Ok(())
    }
    async fn withdraw(&self, _: CallMeta, app: String) -> Result<(), RpcError> {
        self.say(format!("withdraw {app}"));
        lock(&self.registered).retain(|r| r.app != app);
        Ok(())
    }
    async fn call(&self, meta: CallMeta, c: CallAs) -> Result<Value, RpcError> {
        lock(&self.metas).push(meta.clone());
        let caller = Subject::extension(harnix_contract::ExtId::package(&c.app).unwrap());
        let began = std::time::Instant::now();
        let routed_back = if let Some(own) = c.method.strip_prefix(&format!("{}.", c.app)) {
            let control = self.control.get().unwrap().clone();
            let call = MethodCall { app: c.app.clone(), method: own.into(), caller, params: c.params };
            let meta = routed(&meta);
            Some(match self.across.load(std::sync::atomic::Ordering::Relaxed) {
                true => tokio::spawn(async move { control.call_method(meta, call).await }).await.unwrap(),
                false => control.call_method(meta, call).await,
            })
        } else if let Some(own) = self.reroute.get() {
            let control = self.control.get().unwrap().clone();
            let call = MethodCall { app: c.app.clone(), method: own.clone(), caller, params: json!({}) };
            let meta = routed(&meta);
            Some(tokio::spawn(async move { control.call_method(meta, call).await }).await.unwrap())
        } else {
            None
        };
        if let Some(outcome) = routed_back {
            if let Err(e) = &outcome {
                lock(&self.routed_errors).push((e.clone(), began.elapsed()));
            }
            return outcome;
        }
        if !self.calls_of(&c.app).contains(&c.method) {
            return Err(RpcError::refused(format!("`module:{}` may not call `{}`", c.app, c.method)));
        }
        self.say(format!("call {}", c.method));
        Ok(json!({"session_id": "s-from-the-backend"}))
    }
    async fn publish(&self, _: CallMeta, p: Publish) -> Result<(), RpcError> {
        lock(&self.published).push((p.topic, p.value));
        Ok(())
    }
    async fn subscribe(&self, _: CallMeta, s: Subscription) -> Result<Value, RpcError> {
        if !self.calls_of(&s.app).contains(&format!("topic:{}", s.topic)) {
            return Err(RpcError::refused("not in its `calls`"));
        }
        self.say(format!("subscribe {}", s.topic));
        Ok(json!({"topic": s.topic, "value": {"sessions": [{"session_id": "a"}]}}))
    }
    async fn unsubscribe(&self, _: CallMeta, s: Subscription) -> Result<(), RpcError> {
        self.say(format!("unsubscribe {}", s.topic));
        Ok(())
    }
    async fn fetch(&self, _: CallMeta, f: Fetch) -> Result<Fetched, RpcError> {
        Err(RpcError::refused(format!("this product makes no request for `{}`", f.url)))
    }
    async fn turn_chain(&self, _: CallMeta, t: TurnRef) -> Result<TurnChain, RpcError> {
        Ok(TurnChain { instance: t.instance, session_id: t.session_id, chain: Vec::new() })
    }
    fn progress(&self, p: Progress) {
        lock(&self.progress).push(p.text);
    }
}

#[async_trait]
impl Agent for FakePlatform {
    async fn has_instance(&self, instance: String) -> Result<bool, RpcError> {
        Ok(instance == "main")
    }
    async fn install_agent(&self, a: AgentInstall) -> Result<Value, RpcError> {
        assert!(a.archive.is_file(), "{} is not a file", a.archive.display());
        self.say(format!("install_agent {} {}", a.name, a.instance));
        Ok(json!({"tools": []}))
    }
    async fn uninstall_agent(&self, a: AgentRef) -> Result<(), RpcError> {
        self.say(format!("uninstall_agent {} {}", a.name, a.instance));
        Ok(())
    }
    async fn switch_agent(&self, s: AgentSwitch) -> Result<(), RpcError> {
        self.say(format!("switch_agent {} {}", s.name, if s.on { "on" } else { "off" }));
        Ok(())
    }
    async fn place_skills(&self, s: Skills) -> Result<(), RpcError> {
        for name in &s.names {
            assert!(s.source.join(name).join("SKILL.md").is_file(), "no skill {name}");
        }
        self.say(format!("place_skills {}", s.names.join(",")));
        Ok(())
    }
    async fn remove_skills(&self, s: Skills) -> Result<(), RpcError> {
        self.say(format!("remove_skills {}", s.names.join(",")));
        Ok(())
    }
    async fn reload_instance(&self, instance: String) -> Result<(), RpcError> {
        self.say(format!("reload {instance}"));
        Ok(())
    }
    async fn expose_tools(&self, offer: Offer) -> Result<(), RpcError> {
        self.say(format!("expose_tools {}", offer.tools.len()));
        lock(&self.offers).push(offer);
        Ok(())
    }
}

pub struct Rig {
    pub dir: tempfile::TempDir,
    pub platform: Arc<FakePlatform>,
    pub host: Arc<Host>,
    /// The host, as the product reaches it: across the loopback.
    pub control: Arc<dyn Control>,
    /// What every start says of the application besides its record: its settings and secrets.
    pub config: Mutex<Value>,
    pub secrets: Mutex<std::collections::BTreeMap<String, String>>,
    /// The sha256 of the component the last `unpack` packed: what the product verified.
    pub cwasm_sha256: Mutex<Option<String>>,
}

impl Rig {
    pub fn new(options: impl FnOnce(&mut HostOptions)) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let platform = Arc::new(FakePlatform::default());
        let mut host_options = HostOptions::default();
        options(&mut host_options);
        let host = Host::new(host_options, Loopback::kernel(platform.clone()), Loopback::agent(platform.clone())).unwrap();
        let control: Arc<dyn Control> = Loopback::control(host.clone());
        let _ = platform.control.set(control.clone());
        Rig {
            dir,
            platform,
            host,
            control,
            config: Mutex::new(Value::Null),
            secrets: Mutex::default(),
            cwasm_sha256: Mutex::default(),
        }
    }

    /// A rig whose starts give the application `config`.
    pub fn configured(config: Value) -> Self {
        let rig = Self::new(|_| {});
        *lock(&rig.config) = config;
        rig
    }

    /// What the product starts `version` with: its record, and what it verified of it.
    pub fn start_of(&self, version: &str, parts: Value) -> Start {
        Start {
            app: ID.into(),
            version: version.into(),
            trust: "store".into(),
            cwasm_sha256: lock(&self.cwasm_sha256).clone(),
            config: lock(&self.config).clone(),
            secrets: lock(&self.secrets).clone(),
            parts,
            pkg: Some(self.app_dir().join(version)),
            run: Some(self.app_dir()),
        }
    }

    pub fn app_dir(&self) -> PathBuf {
        self.dir.path().join("ext").join(ID)
    }

    /// The showcase with `edit` applied to its manifest, packed for this engine and unpacked where
    /// an install would put it. Answers its version.
    pub fn unpack(&self, edit: impl Fn(String) -> String) -> String {
        let source = root().join("examples/apps/showcase");
        let app = self.dir.path().join("src");
        for part in ["ui", "agent"] {
            copy_tree(&source.join(part), &app.join(part));
        }
        let manifest = edit(std::fs::read_to_string(source.join("harnix-ext.toml")).unwrap());
        std::fs::write(app.join("harnix-ext.toml"), &manifest).unwrap();
        let wasm = showcase_wasm();
        let cwasm = self.host.engine().compile(&wasm).unwrap();
        *lock(&self.cwasm_sha256) = Some(harnix_package::signature::sha256(&cwasm));
        let out = self.dir.path().join("showcase.hxext");
        let artifact = harnix_package::package::Artifact {
            fingerprint: self.host.engine().fingerprint(),
            cwasm: &cwasm,
            wasm: Some(&wasm),
        };
        harnix_package::package::pack(&app, Some(artifact), &out).unwrap();
        let parsed = harnix_package::manifest::Manifest::parse(&manifest).unwrap();
        let version = parsed.app.version;
        harnix_package::package::extract(&out, &self.app_dir().join(&version)).unwrap();
        version
    }

    /// Put in place and start, as the product does on an install: answers the record's `parts`.
    pub async fn install(&self, edit: impl Fn(String) -> String) -> Value {
        let version = self.unpack(edit);
        let placement = self
            .control
            .place(
                CallMeta::default(),
                Place {
                    app: ID.into(),
                    app_dir: self.app_dir(),
                    version: version.clone(),
                    instances: None,
                },
            )
            .await
            .unwrap();
        let start = self.start_of(&version, placement.parts.clone());
        self.control.start(CallMeta::new(next_id()), start).await.unwrap();
        placement.parts
    }

    pub async fn status(&self) -> Value {
        self.control.status(CallMeta::default(), ID.into()).await.unwrap()
    }
}

pub fn copy_tree(from: &Path, to: &Path) {
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

/// Wait until `f` holds, for up to a few seconds.
pub async fn eventually(what: &str, mut f: impl AsyncFnMut() -> bool) {
    for _ in 0..200 {
        if f().await {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    panic!("never: {what}");
}
