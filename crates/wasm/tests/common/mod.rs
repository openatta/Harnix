//! What every test of a real component needs: the fixtures built, compiled
//! and loaded the way the product loads them, and a host that records what
//! the component asked of it.
//!
//! A fixture is built with `cargo build --target wasm32-wasip2` (rustc's
//! wasip2 linker produces the component; no other tooling is needed) and
//! cached under the test target directory by the hash of its sources and of
//! the WIT, so a run after the first costs nothing unless one of them changed.
#![allow(dead_code)]

use async_trait::async_trait;
use harnix_wasm::*;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn cache_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("harnix-wasm-fixtures")
}

fn hash_tree(hasher: &mut Sha256, dir: &Path) {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .map(|e| e.unwrap().path())
        .collect();
    entries.sort();
    for path in entries {
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        if name == "target" {
            continue;
        }
        if path.is_dir() {
            hash_tree(hasher, &path);
        } else {
            hasher.update(name.as_bytes());
            hasher.update(std::fs::read(&path).unwrap());
        }
    }
}

/// The fixture `name` (a directory under tests/fixtures) as a `.wasm`
/// component, built if its sources changed since the last build.
pub fn fixture_wasm(name: &str) -> Vec<u8> {
    static BUILT: OnceLock<Mutex<HashMap<String, Vec<u8>>>> = OnceLock::new();
    let mut built = BUILT.get_or_init(Default::default).lock().unwrap();
    if let Some(bytes) = built.get(name) {
        return bytes.clone();
    }
    let source = crate_dir().join("tests/fixtures").join(name);
    let mut hasher = Sha256::new();
    hash_tree(&mut hasher, &source);
    hash_tree(&mut hasher, &crate_dir().join("wit"));
    let digest = hex::encode(hasher.finalize());
    let cached = cache_dir().join(format!("{name}-{}.wasm", &digest[..16]));
    if !cached.exists() {
        let target = cache_dir().join("build");
        let status = std::process::Command::new(env!("CARGO"))
            .args(["build", "--release", "--target", "wasm32-wasip2"])
            .env("CARGO_TARGET_DIR", &target)
            .current_dir(&source)
            .status()
            .expect("cargo should be runnable");
        assert!(
            status.success(),
            "could not build the fixture `{name}`. If the target is missing: \
             rustup target add wasm32-wasip2"
        );
        let out = target
            .join("wasm32-wasip2/release")
            .join(format!("{}.wasm", name.replace('-', "_")));
        std::fs::copy(&out, &cached).unwrap();
    }
    let bytes = std::fs::read(&cached).unwrap();
    built.insert(name.to_string(), bytes.clone());
    bytes
}

/// One engine per test binary, as the product has one per process.
pub fn engine() -> &'static WasmEngine {
    static ENGINE: OnceLock<WasmEngine> = OnceLock::new();
    ENGINE.get_or_init(|| WasmEngine::new().unwrap())
}

/// The fixture compiled to an artifact, the way the store's build tool
/// compiles it. Once per test binary.
#[cfg(feature = "compile")]
pub fn fixture_artifact(name: &str) -> Vec<u8> {
    static COMPILED: OnceLock<Mutex<HashMap<String, Vec<u8>>>> = OnceLock::new();
    let mut compiled = COMPILED.get_or_init(Default::default).lock().unwrap();
    compiled
        .entry(name.to_string())
        .or_insert_with(|| engine().compile(&fixture_wasm(name)).unwrap())
        .clone()
}

/// The fixture loaded through the deserialize path, as the product loads it.
#[cfg(feature = "compile")]
pub fn component(name: &str) -> AppComponent {
    let artifact = fixture_artifact(name);
    // SAFETY: compiled by this process a moment ago.
    unsafe { engine().load(&artifact, engine().fingerprint()) }.unwrap()
}

/// A host that records everything a component asks of it.
#[derive(Default)]
pub struct MockHost {
    pub calls: Mutex<Vec<(String, String, Value)>>,
    pub published: Mutex<Vec<(String, Value)>>,
    pub subscribed: Mutex<Vec<String>>,
    pub unsubscribed: Mutex<Vec<String>>,
    /// Set by a test that wants `call` to route back into a VM.
    pub route_to: OnceLock<AppVm>,
}

impl MockHost {
    pub fn published_on(&self, topic: &str) -> Vec<Value> {
        self.published
            .lock()
            .unwrap()
            .iter()
            .filter(|(t, _)| t == topic)
            .map(|(_, v)| v.clone())
            .collect()
    }
}

#[async_trait]
impl AppHost for MockHost {
    async fn call(&self, app: &str, method: &str, params: Value) -> Result<Value, String> {
        self.calls
            .lock()
            .unwrap()
            .push((app.into(), method.into(), params.clone()));
        if method == "test.refuse" {
            return Err(format!("`module:{app}` may not call `{method}`"));
        }
        if let Some(vm) = self.route_to.get() {
            return match vm.handle("count", &json!({}), &json!({})).await {
                Ok(Ok(v)) => Ok(v),
                Ok(Err(e)) => Err(e.message),
                Err(f) => Err(f.to_string()),
            };
        }
        Ok(json!({ "answered": method, "params": params }))
    }

    fn publish(&self, app: &str, topic: &str, value: Value) -> Result<(), String> {
        let _ = app;
        if topic == "undeclared" {
            return Err(format!(
                "`{topic}` is not a topic this application declared"
            ));
        }
        self.published.lock().unwrap().push((topic.into(), value));
        Ok(())
    }

    async fn subscribe(&self, _app: &str, topic: &str, _params: &Value) -> Result<(), String> {
        if topic == "forbidden.topic" {
            return Err("not in the `hub` capability".into());
        }
        self.subscribed.lock().unwrap().push(topic.into());
        Ok(())
    }

    async fn unsubscribe(&self, _app: &str, topic: &str, _params: &Value) {
        self.unsubscribed.lock().unwrap().push(topic.into());
    }
}

/// A network that answers everything with 200 and remembers what it was
/// asked, so a test can tell "refused before the network" from "fetched".
#[derive(Default)]
pub struct MockFetcher {
    pub requests: Mutex<Vec<HttpRequest>>,
}

#[async_trait]
impl HttpFetcher for MockFetcher {
    async fn fetch(&self, _app: &str, request: HttpRequest) -> Result<HttpResponse, String> {
        let body = format!("hello from {}", host_of(&request.url).unwrap_or_default());
        self.requests.lock().unwrap().push(request);
        Ok(HttpResponse {
            status: 200,
            headers: vec![],
            body: body.into_bytes(),
        })
    }
}

pub struct Rig {
    pub vm: AppVm,
    pub host: Arc<MockHost>,
    pub http: Arc<MockFetcher>,
    pub kv: Arc<MemoryKv>,
    pub dir: tempfile::TempDir,
}

pub fn spec(capabilities: Capabilities, limits: Limits, dir: &Path) -> AppSpec {
    AppSpec {
        id: "acme.probe".into(),
        capabilities,
        limits,
        dirs: AppDirs {
            data: dir.join("data"),
        },
        config: json!({ "greeting": "hi" }),
    }
}

pub fn limits(timeout_ms: u64) -> Limits {
    Limits {
        memory_bytes: 32 << 20,
        timeout: Duration::from_millis(timeout_ms),
    }
}

/// A VM for the fixture `name`, with recording services.
#[cfg(feature = "compile")]
pub async fn rig(name: &str, capabilities: Capabilities, limits: Limits) -> Rig {
    rig_with(name, |dir| spec(capabilities, limits, dir)).await
}

#[cfg(feature = "compile")]
pub async fn rig_with(name: &str, spec: impl FnOnce(&Path) -> AppSpec) -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let host = Arc::new(MockHost::default());
    let http = Arc::new(MockFetcher::default());
    let kv = Arc::new(MemoryKv::new());
    let services = Services {
        host: host.clone(),
        kv: kv.clone(),
        http: http.clone(),
    };
    let vm = AppVm::start(engine(), &component(name), spec(dir.path()), services)
        .await
        .unwrap();
    Rig {
        vm,
        host,
        http,
        kv,
        dir,
    }
}

/// Wait until `done` holds, or fail after `within`.
pub async fn eventually(within: Duration, what: &str, done: impl Fn() -> bool) {
    let deadline = tokio::time::Instant::now() + within;
    while !done() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out waiting for {what}"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}
