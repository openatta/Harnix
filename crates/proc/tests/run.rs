//! An extension process's life over a socket pair, in this process: the product's end is the
//! wire's serving end with a product that keeps what it was told. No package: what a package
//! does behind the wire is `harnix-testapp`'s `ext_proc` test, against the real binary.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use harnix_engine::{Connection, DaemonLink};
use harnix_engine_api::link::{EngineLink, InstanceCarrier};
use harnix_proc::{run, Options};
use harnix_wasm_port::rpc::ControlClient;
use harnix_wasm_port::socket::{Answer, Inbound, Options as WireOptions, SocketWire};
use harnix_wasm_port::{codes, CallMeta, Control, RpcError, Start, EXT_WIRE_VERSION};
use serde_json::{json, Value};
use tokio::net::UnixStream;

/// The product: answers `proc.hello` (or refuses it), and nothing else.
struct Product {
    refuse: bool,
    hellos: Mutex<Vec<Value>>,
}

#[async_trait]
impl Answer for Product {
    async fn call(&self, call: Inbound, args: Value) -> Result<Value, RpcError> {
        assert_eq!(call.method, "proc.hello", "nothing comes before the hello");
        self.hellos.lock().unwrap().push(args);
        match self.refuse {
            true => Err(RpcError::new(codes::HANDSHAKE_REFUSED, "not the same build")),
            false => Ok(json!({})),
        }
    }
}

fn pair(refuse: bool) -> (Arc<Product>, Arc<SocketWire>, UnixStream) {
    let (product, process) = UnixStream::pair().unwrap();
    let answer = Arc::new(Product {
        refuse,
        hellos: Mutex::new(Vec::new()),
    });
    let wire = SocketWire::serve(product, "example.showcase", answer.clone(), WireOptions::default());
    (answer, wire, process)
}

fn options() -> Options {
    Options {
        ext: "example.showcase".into(),
        pkg: None,
        run: None,
        engine: None,
    }
}

async fn hello(product: &Product) -> Value {
    for _ in 0..200 {
        if let Some(hello) = product.hellos.lock().unwrap().first().cloned() {
            return hello;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("no proc.hello");
}

#[tokio::test(flavor = "multi_thread")]
async fn it_says_hello_answers_the_product_and_goes_when_asked() {
    let (product, wire, process) = pair(false);
    let running = tokio::spawn(run(options(), process, std::future::pending()));
    let hello = hello(&product).await;
    assert_eq!(hello["wire"], json!(EXT_WIRE_VERSION));
    assert_eq!(hello["ext"], json!("example.showcase"));
    assert_eq!(hello["version"], json!(harnix_proc::HARNIX_VERSION));
    assert_eq!(hello["pid"], json!(std::process::id()));
    // The Agix it links and the VM engine it builds, which the product holds against its own.
    assert_eq!(hello["agix_version"], json!(harnix_engine::identity().version));
    assert!(hello["engine"].as_str().is_some_and(|e| !e.is_empty()), "{hello}");

    let control = ControlClient(wire.clone());
    assert_eq!(control.status(CallMeta::default(), "example.showcase".into()).await.unwrap(), json!({"state": "stopped"}));
    // The answer to `ctl.shutdown` is written before the process lets go of the wire.
    control.shutdown(CallMeta::default(), "the product is going".into()).await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), running).await.unwrap().unwrap().unwrap();
    assert!(tokio::time::timeout(Duration::from_secs(5), wire.closed()).await.is_ok());
}

#[tokio::test(flavor = "multi_thread")]
async fn it_goes_when_the_wire_closes_or_stdin_does() {
    let (product, wire, process) = pair(false);
    let running = tokio::spawn(run(options(), process, std::future::pending()));
    hello(&product).await;
    wire.close("the product went").await;
    tokio::time::timeout(Duration::from_secs(5), running).await.unwrap().unwrap().unwrap();

    let (product, _wire, process) = pair(false);
    let (gone, stdin) = tokio::sync::oneshot::channel::<()>();
    let running = tokio::spawn(run(options(), process, async move {
        let _ = stdin.await;
    }));
    hello(&product).await;
    drop(gone);
    tokio::time::timeout(Duration::from_secs(5), running).await.unwrap().unwrap().unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_process_the_product_refuses_goes_at_once() {
    let (_product, _wire, process) = pair(true);
    let ran = tokio::time::timeout(Duration::from_secs(5), run(options(), process, std::future::pending()))
        .await
        .unwrap();
    let e = ran.unwrap_err().to_string();
    assert!(e.contains("not the same build"), "{e}");
}

const ID: &str = "example.showcase";

fn start(app: &str) -> Start {
    Start {
        app: app.into(),
        version: "0.1.0".into(),
        trust: "store".into(),
        cwasm_sha256: Some("00".repeat(32)),
        config: json!(null),
        secrets: Default::default(),
        parts: json!({}),
        pkg: None,
        run: None,
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_second_start_is_refused() {
    let (product, wire, process) = pair(false);
    let running = tokio::spawn(run(options(), process, std::future::pending()));
    hello(&product).await;
    let control = ControlClient(wire.clone());
    // It has no package to start: the host says so, and that was its one start.
    let first = control.start(CallMeta::default(), start(ID)).await.unwrap_err();
    assert!(first.message.contains("no package"), "{}", first.message);
    let second = control.start(CallMeta::default(), start(ID)).await.unwrap_err();
    assert_eq!(second.code, codes::REFUSED, "{second:?}");
    assert!(second.message.contains("comes once"), "{}", second.message);
    control.shutdown(CallMeta::default(), "done".into()).await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), running).await.unwrap().unwrap().unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_start_that_brings_a_package_or_run_directory_is_refused() {
    for key in ["pkg", "run"] {
        let (product, wire, process) = pair(false);
        let running = tokio::spawn(run(options(), process, std::future::pending()));
        hello(&product).await;
        let mut start = start(ID);
        match key {
            "pkg" => start.pkg = Some("/elsewhere".into()),
            _ => start.run = Some("/elsewhere".into()),
        }
        let refused = ControlClient(wire.clone()).start(CallMeta::default(), start).await.unwrap_err();
        assert_eq!(refused.code, codes::REFUSED, "{refused:?}");
        assert!(refused.message.contains(&format!("carries `{key}`")), "{}", refused.message);
        wire.close("done").await;
        tokio::time::timeout(Duration::from_secs(5), running).await.unwrap().unwrap().unwrap();
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_start_for_another_extension_is_refused_and_ends_the_process() {
    let (product, wire, process) = pair(false);
    let running = tokio::spawn(run(options(), process, std::future::pending()));
    hello(&product).await;
    let refused = ControlClient(wire.clone()).start(CallMeta::default(), start("example.other")).await.unwrap_err();
    assert_eq!(refused.code, codes::REFUSED, "{refused:?}");
    let ended = tokio::time::timeout(Duration::from_secs(5), running).await.unwrap().unwrap().unwrap_err();
    assert!(format!("{ended:#}").contains("example.other"), "{ended:#}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_component_that_is_not_the_verified_one_is_refused_and_ends_the_process() {
    let dir = tempfile::tempdir().unwrap();
    let pkg = dir.path().join("pkg");
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/apps/showcase");
    copy_tree(&source, &pkg);
    let run_dir = dir.path().join("run");
    std::fs::create_dir_all(&run_dir).unwrap();

    let (product, wire, process) = pair(false);
    let options = Options { ext: ID.into(), pkg: Some(pkg.clone()), run: Some(run_dir), engine: None };
    let running = tokio::spawn(run(options, process, std::future::pending()));
    // The component for this engine is there, and its bytes are not what the product verified.
    let engine = hello(&product).await["engine"].as_str().unwrap().to_string();
    std::fs::create_dir_all(pkg.join("wasm")).unwrap();
    std::fs::write(pkg.join("wasm").join(format!("tools.{engine}.cwasm")), b"not the component").unwrap();
    let refused = ControlClient(wire.clone()).start(CallMeta::default(), start(ID)).await.unwrap_err();
    assert!(refused.message.contains("not the one that was verified"), "{}", refused.message);
    let ended = tokio::time::timeout(Duration::from_secs(5), running).await.unwrap().unwrap().unwrap_err();
    assert!(format!("{ended:#}").contains("sha256"), "{ended:#}");
}

fn copy_tree(from: &std::path::Path, to: &std::path::Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn it_serves_its_own_engine_on_the_stream_it_was_given() {
    let dir = tempfile::tempdir().unwrap();
    let run_dir = dir.path().join("run");
    std::fs::create_dir_all(&run_dir).unwrap();
    let (product, wire, process) = pair(false);
    let (theirs, engine) = UnixStream::pair().unwrap();
    let options = Options { ext: ID.into(), pkg: None, run: Some(run_dir.clone()), engine: Some(engine) };
    let running = tokio::spawn(run(options, process, std::future::pending()));
    hello(&product).await;

    // The product is the only one on it, and the engine answers as a daemon of this build does:
    // the link the product adopts it with takes the stream, and lists its sessions.
    let link = DaemonLink::new(InstanceCarrier::Adopted, "proc.example.showcase");
    let _taken = tokio::time::timeout(Duration::from_secs(30), link.take(Connection::unix(theirs), None))
        .await
        .unwrap()
        .unwrap_or_else(|e| panic!("the engine was not taken: {e:?}"));
    assert!(link.ops().list().await.unwrap().is_empty());
    // Its data root is under the run directory.
    assert!(run_dir.join("engine").is_dir());

    ControlClient(wire.clone()).shutdown(CallMeta::default(), "done".into()).await.unwrap();
    tokio::time::timeout(Duration::from_secs(10), running).await.unwrap().unwrap().unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn an_engine_stream_without_a_run_directory_is_a_setup_failure() {
    let (_product, _wire, process) = pair(false);
    let (_theirs, engine) = UnixStream::pair().unwrap();
    let options = Options { ext: ID.into(), pkg: None, run: None, engine: Some(engine) };
    let ended = run(options, process, std::future::pending()).await.unwrap_err();
    assert!(format!("{ended:#}").contains("--run"), "{ended:#}");
}
