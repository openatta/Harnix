//! An extension process's life over a socket pair, in this process: the product's end is the
//! wire's serving end with a product that keeps what it was told. No package: what a package
//! does behind the wire is `harnix-testapp`'s `ext_proc` test, against the real binary.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use harnix_proc::{run, Options};
use harnix_wasm_port::rpc::ControlClient;
use harnix_wasm_port::socket::{Answer, Inbound, Options as WireOptions, SocketWire};
use harnix_wasm_port::{codes, CallMeta, Control, RpcError, EXT_WIRE_VERSION};
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
