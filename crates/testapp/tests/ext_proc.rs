//! This binary re-executed as an extension process, `harnix-testapp ext-proc`, kept by
//! `harnix-supervise` with the extension wire on an inherited socket pair, running the showcase
//! (`examples/apps/showcase`) behind it: its methods, topics, tools and cancellation, as a product
//! would drive them, with a fake product's kernel at the wire's serving end.
//!
//! `HARNIX_EXT_PROC_BIN` runs another build of the binary instead (a release one, to measure); it
//! must have the `proc` feature, and the showcase is packed for its engine either way. What it
//! measures — how long the process takes to say hello and to start the showcase, and what it holds
//! once idle — it prints: `cargo test -p harnix-testapp --features proc --test ext_proc -- --nocapture`.
//!
//! A backend whose call the product routes back into itself is refused in the process at once, by
//! the chain the product worked out from the call's parent: not left to wait for itself.

use std::os::fd::OwnedFd;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use harnix_supervise::{ChildEvent, ChildSpec, RestartPolicy, Supervisor};
use harnix_wasm_port::rpc::ControlClient;
use harnix_wasm_port::socket::{
    Answer, CallMeta, CancellationToken, Inbound, KernelAnswer, Options as WireOptions, SocketWire,
};
use harnix_wasm_port::*;
use serde_json::{json, Value};
use tokio::sync::mpsc;

const ID: &str = "example.showcase";

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn program() -> PathBuf {
    std::env::var_os("HARNIX_EXT_PROC_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_harnix-testapp")))
}

/// Run `cargo` with `args` in `dir`, or fail saying what did not build.
fn cargo(dir: &Path, args: &[&str], env: &[(&str, &Path)]) -> String {
    let mut command = std::process::Command::new(env!("CARGO"));
    command.args(args).current_dir(dir);
    for (k, v) in env {
        command.env(k, v);
    }
    let out = command.output().expect("cargo runs");
    assert!(out.status.success(), "cargo {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8(out.stdout).unwrap()
}

/// The showcase, packed with its component compiled for this engine by the author's tool: once
/// per test binary, and cached by cargo across runs.
fn showcase_package() -> &'static Path {
    static PACKED: OnceLock<PathBuf> = OnceLock::new();
    PACKED.get_or_init(|| {
        let tmp = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
        let target = tmp.join("showcase");
        cargo(
            &root().join("examples/apps/showcase/component"),
            &["build", "--release", "--target", "wasm32-wasip2"],
            &[("CARGO_TARGET_DIR", &target)],
        );
        let wasm = target.join("wasm32-wasip2/release/showcase_tools.wasm");
        let out = tmp.join("ext-proc-showcase.hxext");
        let (wasm, out_s) = (wasm.to_str().unwrap(), out.to_str().unwrap());
        cargo(
            &root(),
            &["run", "-q", "-p", "harnix-package", "--features", "compile", "--bin", "harnix-pack", "--",
              "pack", "examples/apps/showcase", "--wasm", wasm, "-o", out_s],
            &[],
        );
        out
    })
}

/// The showcase unpacked where an install puts it: `<ext>/<id>/<version>/`. Answers its directory
/// and version.
fn unpack(dir: &Path) -> (PathBuf, String) {
    let app_dir = dir.join("ext").join(ID);
    let version = "0.1.0".to_string();
    harnix_package::package::extract(showcase_package(), &app_dir.join(&version)).unwrap();
    std::fs::create_dir_all(app_dir.join("run")).unwrap();
    (app_dir, version)
}

/* ── the product's end ───────────────────────────────────────────────── */

/// A product's kernel, as far as the showcase reaches it: what it was told, and calls into the
/// showcase's own methods routed back over the wire.
#[derive(Default)]
struct Product {
    hellos: Mutex<Vec<(Instant, Value)>>,
    registered: Mutex<Vec<Registration>>,
    published: Mutex<Vec<(String, Value)>>,
    progress: Mutex<Vec<(String, String)>>,
    said: Mutex<Vec<String>>,
    /// The process, as this end reaches it.
    control: OnceLock<Arc<dyn Control>>,
    /// When set, a call the showcase makes that its `calls` lists is routed into this method of
    /// its own instead.
    reroute: OnceLock<String>,
    /// What a call routed back into the showcase answered when it failed, and how long it took.
    routed_errors: Mutex<Vec<(RpcError, Duration)>>,
    /// The metadata of every call the showcase made as itself, as this end worked it out.
    metas: Mutex<Vec<CallMeta>>,
}

impl Product {
    fn calls(&self) -> Vec<String> {
        self.registered.lock().unwrap().iter().find(|r| r.app == ID).map(|r| r.calls.clone()).unwrap_or_default()
    }

    fn said(&self, prefix: &str) -> Vec<String> {
        self.said.lock().unwrap().iter().filter(|s| s.starts_with(prefix)).cloned().collect()
    }

    fn progress_of(&self, call_id: &str) -> usize {
        self.progress.lock().unwrap().iter().filter(|(id, _)| id == call_id).count()
    }
}

#[async_trait]
impl Kernel for Product {
    async fn register(&self, _: CallMeta, r: Registration) -> Result<(), RpcError> {
        self.said.lock().unwrap().push(format!("register {}", r.app));
        self.registered.lock().unwrap().push(r);
        Ok(())
    }
    async fn withdraw(&self, _: CallMeta, app: String) -> Result<(), RpcError> {
        self.said.lock().unwrap().push(format!("withdraw {app}"));
        Ok(())
    }
    async fn call(&self, meta: CallMeta, c: CallAs) -> Result<Value, RpcError> {
        self.metas.lock().unwrap().push(meta.clone());
        let own = match c.method.strip_prefix(&format!("{ID}.")) {
            Some(own) => Some((own.to_string(), c.params.clone())),
            None => self.reroute.get().map(|own| (own.clone(), json!({}))),
        };
        if let Some((own, params)) = own {
            // Routed on as a product does: a call of its own, with the chain it came with.
            let began = Instant::now();
            let control = self.control.get().unwrap().clone();
            let call = MethodCall {
                app: ID.into(),
                method: own,
                caller: Subject::extension(harnix::ExtId::package(ID).unwrap()),
                params,
            };
            let routed = CallMeta::default().chain(meta.links().iter().cloned());
            let outcome = control.call_method(routed, call).await;
            if let Err(e) = &outcome {
                self.routed_errors.lock().unwrap().push((e.clone(), began.elapsed()));
            }
            return outcome;
        }
        if !self.calls().contains(&c.method) {
            return Err(RpcError::refused(format!("`module:{ID}` may not call `{}`", c.method)));
        }
        self.said.lock().unwrap().push(format!("call {}", c.method));
        Ok(json!({"session_id": "s-from-the-backend"}))
    }
    async fn publish(&self, _: CallMeta, p: Publish) -> Result<(), RpcError> {
        self.published.lock().unwrap().push((p.topic, p.value));
        Ok(())
    }
    async fn subscribe(&self, _: CallMeta, s: Subscription) -> Result<Value, RpcError> {
        self.said.lock().unwrap().push(format!("subscribe {}", s.topic));
        Ok(json!({"topic": s.topic, "value": {"sessions": [{"session_id": "a"}]}}))
    }
    async fn unsubscribe(&self, _: CallMeta, s: Subscription) -> Result<(), RpcError> {
        self.said.lock().unwrap().push(format!("unsubscribe {}", s.topic));
        Ok(())
    }
    async fn fetch(&self, _: CallMeta, f: Fetch) -> Result<Fetched, RpcError> {
        Err(RpcError::refused(format!("this product makes no request for `{}`", f.url)))
    }
    async fn turn_chain(&self, _: CallMeta, t: TurnRef) -> Result<TurnChain, RpcError> {
        Ok(TurnChain { instance: t.instance, session_id: t.session_id, chain: Vec::new() })
    }
    fn progress(&self, p: Progress) {
        self.progress.lock().unwrap().push((p.call_id, p.text));
    }
}

/// `proc.hello`, then the kernel. The handshake is the product's to answer: same wire, same build.
struct ProductEnd {
    product: Arc<Product>,
    kernel: KernelAnswer,
}

#[async_trait]
impl Answer for ProductEnd {
    async fn call(&self, call: Inbound, args: Value) -> Result<Value, RpcError> {
        if call.method != "proc.hello" {
            return self.kernel.call(call, args).await;
        }
        self.product.hellos.lock().unwrap().push((Instant::now(), args.clone()));
        if args["wire"] != json!(EXT_WIRE_VERSION) || args["version"] != json!(harnix::HARNIX_VERSION) {
            return Err(RpcError::new(codes::HANDSHAKE_REFUSED, "not the same build"));
        }
        Ok(json!({}))
    }

    fn notify(&self, method: &str, params: Value) {
        self.kernel.notify(method, params);
    }
}

/// One start of the process: the product's end of its wire.
struct Connection {
    product: Arc<Product>,
    wire: Arc<SocketWire>,
    control: Arc<dyn Control>,
    spawned: Instant,
}

impl Connection {
    fn new(stream: std::os::unix::net::UnixStream, spawned: Instant) -> Self {
        stream.set_nonblocking(true).unwrap();
        let stream = tokio::net::UnixStream::from_std(stream).unwrap();
        let product = Arc::new(Product::default());
        let answer = Arc::new(ProductEnd {
            product: product.clone(),
            kernel: KernelAnswer(product.clone()),
        });
        let wire = SocketWire::serve(stream, ID, answer, WireOptions::default());
        let control: Arc<dyn Control> = Arc::new(ControlClient(wire.clone()));
        let _ = product.control.set(control.clone());
        Self {
            product,
            wire,
            control,
            spawned,
        }
    }

    /// The process's hello, and how long after it was spawned it came.
    async fn hello(&self) -> (Value, Duration) {
        eventually("proc.hello", async || !self.product.hellos.lock().unwrap().is_empty()).await;
        let (at, hello) = self.product.hellos.lock().unwrap()[0].clone();
        (hello, at - self.spawned)
    }

    /// Started as the product would after verifying it: the sha256 of the component the process
    /// loads, compiled for the engine its hello named. Where it runs from is the process's own.
    async fn start(&self, app_dir: &Path, version: &str) -> Started {
        let engine = self.hello().await.0["engine"].as_str().unwrap().to_string();
        let cwasm = app_dir.join(version).join("wasm").join(format!("tools.{engine}.cwasm"));
        let start = Start {
            app: ID.into(),
            version: version.into(),
            trust: "store".into(),
            cwasm_sha256: Some(harnix_package::signature::sha256(&std::fs::read(cwasm).unwrap())),
            config: Value::Null,
            secrets: Default::default(),
            parts: json!({}),
            limits: Default::default(),
            pkg: None,
            run: None,
        };
        self.control.start(CallMeta::default(), start).await.unwrap()
    }

    async fn device_call(&self, method: &str, params: Value) -> Result<Value, RpcError> {
        let call = MethodCall {
            app: ID.into(),
            method: method.into(),
            caller: Subject::Device { id: "d1".into() },
            params,
        };
        self.control.call_method(CallMeta::default(), call).await
    }
}

/// The spec of one start: the process's end of a fresh socket pair at fd 3, and the product's end
/// sent to `ends`.
fn spec(pkg: &Path, run: &Path, ends: &mpsc::UnboundedSender<(std::os::unix::net::UnixStream, Instant)>) -> std::io::Result<ChildSpec> {
    let (product, process) = std::os::unix::net::UnixStream::pair()?;
    let _ = ends.send((product, Instant::now()));
    Ok(ChildSpec::new(program())
        .args(["ext-proc", "--ext", ID, "--pkg", pkg.to_str().unwrap(), "--run", run.to_str().unwrap()])
        .inherit_fd(3, OwnedFd::from(process))
        .own_session())
}

/// Resident memory of a process, in KiB.
fn rss_kib(pid: u32) -> u64 {
    let out = std::process::Command::new("ps").args(["-o", "rss=", "-p", &pid.to_string()]).output().unwrap();
    String::from_utf8_lossy(&out.stdout).trim().parse().unwrap_or(0)
}

async fn eventually(what: &str, mut f: impl AsyncFnMut() -> bool) {
    for _ in 0..400 {
        if f().await {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("never: {what}");
}

async fn next_start(events: &mut Supervisor) -> u32 {
    loop {
        match tokio::time::timeout(Duration::from_secs(30), events.next_event()).await.unwrap() {
            Some(ChildEvent::Started { pid }) => return pid,
            Some(ChildEvent::SpawnFailed { error }) => panic!("not spawned: {error}"),
            Some(ChildEvent::GaveUp { reason }) => panic!("given up: {reason}"),
            Some(_) => continue,
            None => panic!("the supervisor finished"),
        }
    }
}

fn tool(name: &str, input: Value, call_id: &str) -> ToolCall {
    ToolCall {
        app: ID.into(),
        tool: name.into(),
        input,
        call_id: call_id.into(),
        project: None,
        progress: true,
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn the_showcase_runs_in_an_extension_process_of_this_binary() {
    showcase_package();
    let dir = tempfile::tempdir().unwrap();
    let (app_dir, version) = unpack(dir.path());
    let run = app_dir.join("run");
    let pkg = app_dir.join(&version);
    let (ends, mut product_ends) = mpsc::unbounded_channel();
    let (spawn_pkg, spawn_run) = (pkg.clone(), run.clone());
    let mut supervisor = Supervisor::start(move || spec(&spawn_pkg, &spawn_run, &ends), RestartPolicy::extension());
    let pid = next_start(&mut supervisor).await;
    let (stream, spawned) = product_ends.recv().await.unwrap();
    let process = Connection::new(stream, spawned);

    // The hello: who it is, of which build, and the pid it was spawned as.
    let (hello, to_hello) = process.hello().await;
    assert_eq!(hello["ext"], json!(ID));
    assert_eq!(hello["pid"], json!(pid));
    assert_eq!(hello["harnix_api"], json!(harnix_package::manifest::HARNIX_API));
    let idle_bare = rss_kib(pid);

    // Started from the record: its names come back as `proc.register`, then `init` subscribes.
    let before = Instant::now();
    let started = process.start(&app_dir, &version).await;
    let to_started = before.elapsed();
    assert_eq!(started.world.as_deref(), Some("backend"));
    assert_eq!(started.tools.len(), 8);
    let registration = process.product.registered.lock().unwrap()[0].clone();
    assert_eq!(registration.digest.len(), 64, "its manifest's sha256");
    assert!(registration.methods.iter().any(|m| m.name == "state" && m.subjects == ["device", "module"]));
    assert_eq!(registration.topics, ["notes", "heartbeat", "fetched"]);
    assert_eq!(registration.spaces[0].id, ID);
    assert_eq!(process.product.said("subscribe"), ["subscribe harnix.sessions.status"]);
    tokio::time::sleep(Duration::from_secs(1)).await;
    let idle_started = rss_kib(pid);

    // Methods: checked against their schemas in the process, then the backend; who called is the
    // product's word.
    assert_eq!(process.device_call("notes.add", json!({"text": "one"})).await.unwrap(), json!({"count": 1}));
    process.device_call("notes.add", json!({"text": "two"})).await.unwrap();
    let bad = process.device_call("notes.add", json!({"text": 3})).await.unwrap_err();
    assert_eq!(bad.code, codes::INVALID_PARAMS);
    let listed = process.device_call("notes.list", json!({})).await.unwrap();
    assert_eq!(listed["notes"][0]["by"], json!("device:d1"));

    // Topics: what it publishes, in order; what its subscription answered, then a value delivered.
    eventually("both notes published", async || process.product.published.lock().unwrap().len() >= 2).await;
    let counts: Vec<_> = process.product.published.lock().unwrap().iter().map(|(_, v)| v["count"].clone()).collect();
    assert_eq!(counts, [json!(1), json!(2)]);
    process.device_call("timer.start", json!({"ms": 20, "times": 3})).await.unwrap();
    eventually("three heartbeats", async || {
        process.product.published.lock().unwrap().iter().filter(|(t, _)| t == "heartbeat").count() == 3
    })
    .await;
    eventually("the subscription's value", async || {
        process.device_call("state", json!({})).await.unwrap()["status_seen"] == json!(1)
    })
    .await;
    process
        .control
        .deliver(CallMeta::default(), TopicValue {
            app: ID.into(),
            topic: "harnix.sessions.status".into(),
            value: json!({"sessions": [{}, {}, {}]}),
        })
        .await
        .unwrap();
    eventually("the delivered value", async || {
        process.device_call("state", json!({})).await.unwrap()["running_sessions"] == json!(3)
    })
    .await;
    // As itself, through the product: what its hub lists goes through, the rest is refused.
    process.device_call("session.start", json!({"message": "hi"})).await.unwrap();
    assert_eq!(process.product.said("call "), ["call harnix.session.create", "call harnix.session.send"]);
    assert_eq!(process.device_call("overreach", json!({})).await.unwrap()["refused"], json!(true));

    // Tools: an answer; progress, all of it before the answer; one that asks its own backend
    // through the product and back over the wire.
    match process.control.call_tool(CallMeta::default(), tool("echo", json!({"text": "hi"}), "c1")).await.unwrap() {
        ToolReply::Output { structured, is_error, .. } => {
            assert_eq!(structured, Some(json!({"echoed": "hi"})));
            assert!(!is_error);
        }
        other => panic!("{other:?}"),
    }
    process.control.call_tool(CallMeta::default(), tool("slow", json!({"steps": 3, "ms": 10}), "c2")).await.unwrap();
    assert_eq!(process.product.progress_of("c2"), 3);
    match process.control.call_tool(CallMeta::default(), tool("board", json!({}), "c3")).await.unwrap() {
        ToolReply::Output { content, .. } => assert!(content.contains("the board has 2 notes"), "{content}"),
        other => panic!("{other:?}"),
    }

    // Cancelled by the port's own `cancel`: the tool answers that it was.
    let control = process.control.clone();
    let slow = tokio::spawn(async move { control.call_tool(CallMeta::default(), tool("slow", json!({"steps": 100, "ms": 100}), "c4")).await });
    eventually("c4 under way", async || process.product.progress_of("c4") > 0).await;
    process.control.cancel(CallMeta::default(), "c4".into()).await.unwrap();
    assert!(matches!(slow.await.unwrap().unwrap(), ToolReply::Failed { failure: Failure::Cancelled, .. }));
    // Cancelled by the wire: the caller lets go, `$/cancel` follows, and the process stops it.
    let token = CancellationToken::new();
    let wire = process.wire.clone();
    let cancel = token.clone();
    let args = serde_json::to_value(tool("slow", json!({"steps": 100, "ms": 100}), "c5")).unwrap();
    let dropped = tokio::spawn(async move { wire.call("ctl.call_tool", args, CallMeta::default(), &cancel).await });
    eventually("c5 under way", async || process.product.progress_of("c5") > 0).await;
    token.cancel();
    assert_eq!(dropped.await.unwrap().unwrap_err().code, codes::CANCELLED);
    let at_cancel = process.product.progress_of("c5");
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert!(process.product.progress_of("c5") <= at_cancel + 1, "it went on after `$/cancel`");
    let busy = rss_kib(pid);

    eprintln!(
        "ext-proc ({}): hello {} ms after spawn, showcase started {} ms after that; \
         resident {} KiB idle before start, {} KiB idle after, {} KiB after the calls",
        program().display(),
        to_hello.as_millis(),
        to_started.as_millis(),
        idle_bare,
        idle_started,
        busy,
    );

    // Killed outright: the supervisor starts it again on a fresh pair, and it says hello anew.
    // SAFETY: a signal to the child this test started.
    unsafe { libc::kill(pid as i32, libc::SIGKILL) };
    assert!(tokio::time::timeout(Duration::from_secs(5), process.wire.closed()).await.is_ok());
    let again = next_start(&mut supervisor).await;
    assert_ne!(again, pid);
    let (stream, spawned) = product_ends.recv().await.unwrap();
    let process = Connection::new(stream, spawned);
    assert_eq!(process.hello().await.0["pid"], json!(again));
    process.start(&app_dir, &version).await;
    // Its notes are in its storage, under its own directory: they outlived the process.
    assert_eq!(process.device_call("notes.list", json!({})).await.unwrap()["notes"].as_array().unwrap().len(), 2);

    // Stopped: its names withdrawn and its subscription given back, over the wire.
    process.control.stop(CallMeta::default(), Stop { app: ID.into(), parts: json!({}) }).await.unwrap();
    assert_eq!(process.product.said("withdraw"), [format!("withdraw {ID}")]);
    assert_eq!(process.product.said("unsubscribe"), ["unsubscribe harnix.sessions.status"]);
    // Asked to go, it answers before it lets go of the wire; the product then stops keeping it,
    // which closes its stdin.
    process.control.shutdown(CallMeta::default(), "the product is going".into()).await.unwrap();
    supervisor.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn it_goes_with_the_product_and_says_why_it_cannot_start() {
    // Its stdin closed — the product gone, killed outright included — and it exits on its own.
    let dir = tempfile::tempdir().unwrap();
    let (ends, mut product_ends) = mpsc::unbounded_channel();
    let child = spec(dir.path(), dir.path(), &ends).unwrap().spawn().unwrap();
    let (stream, spawned) = product_ends.recv().await.unwrap();
    let process = Connection::new(stream, spawned);
    process.hello().await;
    let before = Instant::now();
    let status = child.stop(Duration::from_secs(10)).await.unwrap();
    assert!(status.success(), "{status:?}");
    assert!(before.elapsed() < Duration::from_secs(5), "it was killed rather than going");

    // No wire where it was told to find one: it says so and exits 78.
    let mut child = ChildSpec::new(program()).args(["ext-proc", "--ext", ID]).spawn().unwrap();
    assert_eq!(child.wait().await.unwrap().code(), Some(78));
    // Asked for an engine's stream that is not there either.
    let (ends, _product_ends) = mpsc::unbounded_channel();
    let mut child = ChildSpec::new(program())
        .args(["ext-proc", "--ext", ID, "--engine-fd", "4"])
        .inherit_fd(3, OwnedFd::from(spec_pair(&ends)))
        .spawn()
        .unwrap();
    assert_eq!(child.wait().await.unwrap().code(), Some(78));
}

/// A backend whose call the product routes back into its own method — the call that made it
/// still waiting on the answer — is refused by the process at once, by the chain the product
/// worked out from the call's parent: the process held on the very call it is serving.
#[tokio::test(flavor = "multi_thread")]
async fn a_backend_that_calls_back_into_itself_over_the_wire_is_refused_at_once() {
    showcase_package();
    let dir = tempfile::tempdir().unwrap();
    let (app_dir, version) = unpack(dir.path());
    let (ends, mut product_ends) = mpsc::unbounded_channel();
    let child = spec(&app_dir.join(&version), &app_dir.join("run"), &ends).unwrap().spawn().unwrap();
    let (stream, spawned) = product_ends.recv().await.unwrap();
    let process = Connection::new(stream, spawned);
    process.start(&app_dir, &version).await;
    process.product.reroute.set("state".into()).unwrap();

    let answer = process.device_call("overreach", json!({})).await.unwrap();
    assert_eq!(answer["refused"], json!(true), "{answer}");
    let (refused, took) = process.product.routed_errors.lock().unwrap()[0].clone();
    assert_eq!(refused.code, codes::EXTENSION_UNAVAILABLE, "{refused:?}");
    assert_eq!(refused.data.as_ref().unwrap()["failure"], json!("reentrant"));
    assert!(took < Duration::from_millis(100), "refused after {took:?}");
    // The product held the process on its own call, by the number it sent it under; the parent is
    // the product's own id for that call.
    let made = process.product.metas.lock().unwrap()[0].clone();
    assert!(made.parent_id().is_some(), "{made:?}");
    assert!(made.links().iter().any(|l| l.ext() == ID), "{made:?}");
    // Not recycled, and the same chain once that call was answered is not waited on.
    let status = process.control.status(CallMeta::default(), ID.into()).await.unwrap();
    assert_eq!(status["health"]["recycles"], json!(0), "{status}");
    let later = MethodCall {
        app: ID.into(),
        method: "state".into(),
        caller: Subject::Kernel,
        params: json!({}),
    };
    let again = CallMeta::default().chain(made.links().iter().cloned());
    process.control.call_method(again, later).await.unwrap();

    process.control.shutdown(CallMeta::default(), "done".into()).await.unwrap();
    drop(process);
    child.stop(Duration::from_secs(10)).await.unwrap();
}

fn spec_pair(ends: &mpsc::UnboundedSender<(std::os::unix::net::UnixStream, Instant)>) -> std::os::unix::net::UnixStream {
    let (product, process) = std::os::unix::net::UnixStream::pair().unwrap();
    let _ = ends.send((product, Instant::now()));
    process
}
