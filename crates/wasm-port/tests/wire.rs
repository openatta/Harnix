//! The extension wire on a socket pair: the port both ways, cancellation, deadlines, what the
//! product makes of the process's metadata, backpressure and the frame limit.
//!
//! Where a test needs the other end to misbehave — forge an id, a parent, a chain, send more than
//! it may — that end is written by hand, a line at a time.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use harnix_wasm_port::rpc::{ControlClient, KernelClient};
use harnix_wasm_port::socket::{
    now_ms, Answer, CallMeta, CancellationToken, ChainLink, ControlAnswer, Inbound, KernelAnswer, Options, SocketWire,
    MAX_FRAME_BYTES, MAX_IN_FLIGHT,
};
use harnix_wasm_port::*;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::UnixStream;

const EXT: &str = "acme.notes";
const PATIENCE: Duration = Duration::from_secs(10);

/* ── the other end, by hand ─────────────────────────────────────────── */

struct Raw {
    read: Lines<BufReader<OwnedReadHalf>>,
    write: OwnedWriteHalf,
}

impl Raw {
    fn new(stream: UnixStream) -> Self {
        let (read, write) = stream.into_split();
        Self {
            read: BufReader::new(read).lines(),
            write,
        }
    }

    async fn send(&mut self, frame: Value) {
        self.write_raw(format!("{frame}\n").as_bytes()).await;
    }

    async fn write_raw(&mut self, bytes: &[u8]) {
        self.write.write_all(bytes).await.unwrap();
    }

    async fn recv(&mut self) -> Value {
        let line = tokio::time::timeout(PATIENCE, self.read.next_line())
            .await
            .expect("a frame in time")
            .unwrap()
            .expect("a frame, not the end of the stream");
        serde_json::from_str(&line).unwrap()
    }

    /// The next frame if one comes within `ms`.
    async fn quiet(&mut self, ms: u64) -> Option<Value> {
        match tokio::time::timeout(Duration::from_millis(ms), self.read.next_line()).await {
            Ok(Ok(Some(line))) => Some(serde_json::from_str(&line).unwrap()),
            _ => None,
        }
    }

    async fn at_end(&mut self) -> bool {
        matches!(tokio::time::timeout(PATIENCE, self.read.next_line()).await, Ok(Ok(None)) | Ok(Err(_)))
    }
}

fn request(id: u64, method: &str, meta: Value, args: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "method": method, "params": {"meta": meta, "args": args}})
}

fn code(answer: &Value) -> i64 {
    answer["error"]["code"].as_i64().unwrap_or_else(|| panic!("an error, not {answer}"))
}

/// Ids this end makes, recognisably its own.
fn options() -> Options {
    let next = Arc::new(AtomicUsize::new(1));
    Options {
        mint: Arc::new(move || format!("local-{}", next.fetch_add(1, Ordering::Relaxed))),
        ..Options::default()
    }
}

/* ── what answers ───────────────────────────────────────────────────── */

/// Answers with what it was told about the call; `{wait_cancel: true}` waits for the call's token.
#[derive(Default)]
struct Echo {
    runs: AtomicUsize,
    notes: Mutex<Vec<String>>,
}

#[async_trait]
impl Answer for Echo {
    async fn call(&self, call: Inbound, args: Value) -> Result<Value, RpcError> {
        self.runs.fetch_add(1, Ordering::SeqCst);
        if args.get("wait_cancel") == Some(&json!(true)) {
            let cancelled = tokio::time::timeout(PATIENCE, call.cancel.cancelled()).await.is_ok();
            return Ok(json!({"cancelled": cancelled}));
        }
        Ok(json!({"method": call.method, "meta": call.meta, "args": args}))
    }

    fn notify(&self, method: &str, _: Value) {
        self.notes.lock().unwrap().push(method.to_string());
    }
}

/// A kernel and a host that keep what they were told.
#[derive(Default)]
struct Kept {
    log: Mutex<Vec<String>>,
    cancelled: tokio::sync::Notify,
}

impl Kept {
    fn note(&self, what: impl Into<String>) {
        self.log.lock().unwrap().push(what.into());
    }
    fn log(&self) -> Vec<String> {
        self.log.lock().unwrap().clone()
    }
}

#[async_trait]
impl Kernel for Kept {
    async fn register(&self, _: CallMeta, r: Registration) -> Result<(), RpcError> {
        self.note(format!("register {} {}", r.app, r.methods.len()));
        Ok(())
    }
    async fn withdraw(&self, _: CallMeta, app: String) -> Result<(), RpcError> {
        Err(RpcError::refused(format!("{app} was never here")))
    }
    async fn call(&self, meta: CallMeta, c: CallAs) -> Result<Value, RpcError> {
        Ok(json!({"method": c.method, "params": c.params, "meta": meta}))
    }
    async fn publish(&self, _: CallMeta, p: Publish) -> Result<(), RpcError> {
        self.note(format!("publish {} {}", p.topic, p.value));
        Ok(())
    }
    async fn subscribe(&self, _: CallMeta, s: Subscription) -> Result<Value, RpcError> {
        Ok(json!({"topic": s.topic, "value": 7}))
    }
    async fn unsubscribe(&self, _: CallMeta, s: Subscription) -> Result<(), RpcError> {
        self.note(format!("unsubscribe {}", s.topic));
        Ok(())
    }
    async fn fetch(&self, _: CallMeta, f: Fetch) -> Result<Fetched, RpcError> {
        self.note(format!("fetch {} {}", f.method, f.url));
        Ok(Fetched { status: 204, headers: vec![], body_b64: String::new() })
    }
    async fn turn_chain(&self, _: CallMeta, t: TurnRef) -> Result<TurnChain, RpcError> {
        Ok(TurnChain { instance: t.instance, session_id: t.session_id, chain: vec![ChainLink::new("acme.todo", "4")] })
    }
    fn progress(&self, p: Progress) {
        self.note(format!("progress {} {}", p.call_id, p.text));
    }
}

#[async_trait]
impl Control for Kept {
    async fn place(&self, _: CallMeta, p: Place) -> Result<Placement, RpcError> {
        Ok(Placement {
            parts: json!({"agent": p.app}),
            disclosure: json!(p.version),
        })
    }
    async fn unplace(&self, _: CallMeta, u: Unplace) -> Result<Value, RpcError> {
        Ok(u.parts)
    }
    async fn start(&self, _: CallMeta, s: Start) -> Result<Started, RpcError> {
        Ok(Started {
            app: s.app,
            trust: "store".into(),
            world: Some("backend".into()),
            tools: vec![],
            events: false,
        })
    }
    async fn stop(&self, _: CallMeta, s: Stop) -> Result<(), RpcError> {
        self.note(format!("stop {}", s.app));
        Ok(())
    }
    async fn forget(&self, _: CallMeta, app: String) -> Result<(), RpcError> {
        self.note(format!("forget {app}"));
        Ok(())
    }
    async fn status(&self, _: CallMeta, app: String) -> Result<Value, RpcError> {
        Ok(json!({"app": app, "state": "running"}))
    }
    async fn call_method(&self, meta: CallMeta, m: MethodCall) -> Result<Value, RpcError> {
        Ok(json!({"method": m.method, "caller": m.caller, "params": m.params, "meta": meta}))
    }
    async fn deliver(&self, _: CallMeta, v: TopicValue) -> Result<(), RpcError> {
        self.note(format!("deliver {} {}", v.topic, v.value));
        Ok(())
    }
    async fn call_tool(&self, _: CallMeta, c: ToolCall) -> Result<ToolReply, RpcError> {
        if c.input == json!("wait") {
            // Until the host cancels it by its own id.
            tokio::time::timeout(PATIENCE, self.cancelled.notified()).await.ok();
            return Ok(ToolReply::Failed {
                failure: Failure::Cancelled,
                message: "cancelled".into(),
            });
        }
        Ok(ToolReply::Output {
            content: format!("{} {}", c.tool, c.input),
            structured: None,
            is_error: false,
        })
    }
    async fn hook(&self, _: CallMeta, h: HookCall) -> Result<HookReply, RpcError> {
        Ok(HookReply::Block(h.event))
    }
    async fn cancel(&self, _: CallMeta, call_id: String) -> Result<(), RpcError> {
        self.note(format!("cancel {call_id}"));
        self.cancelled.notify_one();
        Ok(())
    }
    async fn shutdown(&self, _: CallMeta, reason: String) -> Result<(), RpcError> {
        self.note(format!("shutdown {reason}"));
        Ok(())
    }
    fn turn_started(&self, t: TurnChain) {
        self.note(format!("turn_started {} {}", t.session_id, t.chain.len()));
    }
    fn turn_settled(&self, t: TurnRef) {
        self.note(format!("turn_settled {}", t.session_id));
    }
}

/// A product and an extension process, each end the real one.
fn both(kept: &Arc<Kept>) -> (Arc<SocketWire>, Arc<SocketWire>) {
    let (a, b) = UnixStream::pair().unwrap();
    let product = SocketWire::serve(a, EXT, Arc::new(KernelAnswer(kept.clone())), options());
    let process = SocketWire::client(b, Arc::new(ControlAnswer(kept.clone())), options());
    (product, process)
}

/* ── the port, both ways ────────────────────────────────────────────── */

#[tokio::test]
async fn every_port_method_crosses_the_wire_its_own_way() {
    let kept = Arc::new(Kept::default());
    let (product, process) = both(&kept);
    let kernel = KernelClient(process.clone());
    let control = ControlClient(product.clone());

    // The process to the product: `Kernel`, as `proc.*`.
    kernel
        .register(CallMeta::default(), Registration {
            app: EXT.into(),
            methods: vec![MethodDecl {
                name: "notes.add".into(),
                description: String::new(),
                subjects: vec!["device".into()],
            }],
            topics: vec!["notes".into()],
            spaces: vec![SpaceDecl::new(EXT)],
            calls: vec![],
            tools: vec![],
            digest: "sha".into(),
        })
        .await
        .unwrap();
    let refused = kernel.withdraw(CallMeta::default(), EXT.into()).await.unwrap_err();
    assert_eq!(refused.code, codes::REFUSED, "an error crosses as it was");
    assert!(refused.message.contains("never here"));
    let forged = CallMeta::default().chain([ChainLink::new("victim", "1")]);
    let called = kernel
        .call(
            forged,
            CallAs {
                app: EXT.into(),
                method: "harnix.sessions.list".into(),
                params: json!({"n": 1}),
            },
        )
        .await
        .unwrap();
    assert_eq!(called["params"]["n"], json!(1));
    assert!(
        called["meta"].get("chain").is_none(),
        "the chain the process wrote is not the one the product acts on: {called}"
    );
    let fetch = Fetch { app: EXT.into(), method: "GET".into(), url: "https://example.com".into(), headers: vec![], body_b64: None };
    assert_eq!(kernel.fetch(CallMeta::default(), fetch).await.unwrap().status, 204);
    let turn = TurnRef { instance: "main".into(), session_id: "s1".into() };
    assert_eq!(kernel.turn_chain(CallMeta::default(), turn.clone()).await.unwrap().chain, [ChainLink::new("acme.todo", "4")]);
    let subscription = Subscription {
        app: EXT.into(),
        topic: "harnix.spaces".into(),
        params: Value::Null,
    };
    assert_eq!(kernel.subscribe(CallMeta::default(), subscription.clone()).await.unwrap()["value"], json!(7));
    kernel.unsubscribe(CallMeta::default(), subscription).await.unwrap();
    kernel
        .publish(CallMeta::default(), Publish {
            app: EXT.into(),
            topic: "notes".into(),
            value: json!(1),
        })
        .await
        .unwrap();
    kernel.progress(Progress {
        app: EXT.into(),
        tool: "search".into(),
        call_id: "t-1".into(),
        text: "half".into(),
    });

    // The product to the process: `Control`, as `ctl.*`.
    let placement = control
        .place(CallMeta::default(), Place {
            app: EXT.into(),
            app_dir: "/x".into(),
            version: "1.0.0".into(),
            instances: Some(vec!["proc.acme.notes".into()]),
        })
        .await
        .unwrap();
    assert_eq!(placement.disclosure, json!("1.0.0"));
    assert_eq!(
        control
            .unplace(CallMeta::default(), Unplace {
                app: EXT.into(),
                parts: json!({"agent": true}),
            })
            .await
            .unwrap(),
        json!({"agent": true})
    );
    let started = control
        .start(CallMeta::default(), Start {
            app: EXT.into(),
            version: "1.0.0".into(),
            trust: "store".into(),
            cwasm_sha256: Some("ab".into()),
            config: json!({"k": 1}),
            secrets: [("TOKEN".to_string(), "t".to_string())].into(),
            parts: Value::Null,
            limits: Default::default(),
            pkg: None,
            run: None,
        })
        .await
        .unwrap();
    assert_eq!(started.world.as_deref(), Some("backend"));
    control
        .stop(CallMeta::default(), Stop {
            app: EXT.into(),
            parts: Value::Null,
        })
        .await
        .unwrap();
    control.forget(CallMeta::default(), EXT.into()).await.unwrap();
    assert_eq!(control.status(CallMeta::default(), EXT.into()).await.unwrap()["state"], json!("running"));
    let method = control
        .call_method(CallMeta::default(), MethodCall {
            app: EXT.into(),
            method: "notes.add".into(),
            caller: Subject::Kernel,
            params: json!([1]),
        })
        .await
        .unwrap();
    assert_eq!(method["params"], json!([1]));
    control
        .deliver(CallMeta::default(), TopicValue {
            app: EXT.into(),
            topic: "harnix.spaces".into(),
            value: json!(2),
        })
        .await
        .unwrap();
    let tool = control
        .call_tool(CallMeta::default(), ToolCall {
            app: EXT.into(),
            tool: "search".into(),
            input: json!("q"),
            call_id: "t-2".into(),
            project: None,
            progress: false,
        })
        .await
        .unwrap();
    assert!(matches!(tool, ToolReply::Output { ref content, .. } if content == "search \"q\""), "{tool:?}");
    let hook = control
        .hook(CallMeta::default(), HookCall {
            app: EXT.into(),
            event: "tool.before".into(),
            payload: Value::Null,
            call_id: "h-1".into(),
        })
        .await
        .unwrap();
    assert_eq!(hook, HookReply::Block("tool.before".into()));
    control.cancel(CallMeta::default(), "t-0".into()).await.unwrap();
    control.turn_started(TurnChain { instance: "main".into(), session_id: "s1".into(), chain: vec![ChainLink::new(EXT, "9")] });
    control.turn_settled(turn);
    control.shutdown(CallMeta::default(), "bye".into()).await.unwrap();

    assert_eq!(
        kept.log(),
        [
            "register acme.notes 1",
            "fetch GET https://example.com",
            "unsubscribe harnix.spaces",
            "publish notes 1",
            "progress t-1 half",
            "stop acme.notes",
            "forget acme.notes",
            "deliver harnix.spaces 2",
            "cancel t-0",
            "turn_started s1 1",
            "turn_settled s1",
            "shutdown bye",
        ]
    );
    // Each end answers the other's names only.
    let wrong_way = product
        .call("ctl.status", json!(EXT), CallMeta::default(), &CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(wrong_way["app"], json!(EXT));
    let not_its_name = process
        .call("ctl.status", json!(EXT), CallMeta::default(), &CancellationToken::new())
        .await
        .unwrap_err();
    assert_eq!(not_its_name.code, codes::METHOD_NOT_FOUND, "the product does not answer `ctl.*`");
    assert_eq!(product.counts(), Default::default());
    assert_eq!(process.counts(), Default::default());
}

#[tokio::test]
async fn notifications_and_the_handshake_carry_no_envelope() {
    let echo = Arc::new(Echo::default());
    let (a, b) = UnixStream::pair().unwrap();
    let product = SocketWire::serve(a, EXT, echo.clone(), options());
    let mut process = Raw::new(b);

    process
        .send(json!({"jsonrpc": "2.0", "id": 1, "method": "proc.hello", "params": {"wire": EXT_WIRE_VERSION}}))
        .await;
    let hello = process.recv().await;
    assert_eq!(hello["id"], json!(1));
    assert_eq!(hello["result"]["args"], json!({"wire": 1}), "the handshake's params are the value itself");

    process.send(json!({"jsonrpc": "2.0", "method": "proc.changed", "params": {"generation": 2}})).await;
    product.notify("ctl.turn_settled", json!({"session_id": "s"})).unwrap();
    let note = process.recv().await;
    assert_eq!(note["method"], json!("ctl.turn_settled"));
    assert!(note.get("id").is_none());
    // A call after the notification is read after it.
    process.send(request(2, "proc.x", json!({"call_id": "2"}), Value::Null)).await;
    process.recv().await;
    assert_eq!(*echo.notes.lock().unwrap(), ["proc.changed"]);
}

#[test]
fn a_hello_of_another_build_says_how_it_differs() {
    use harnix_wasm_port::socket::Hello;
    let ours = Hello {
        wire: EXT_WIRE_VERSION,
        ext: EXT.into(),
        version: "0.5.0".into(),
        agix_version: Some("0.3.9".into()),
        engine: "engine-47-aarch64".into(),
        harnix_api: 2,
        pid: 1,
    };
    let same = Hello { pid: 99, ..ours.clone() };
    assert_eq!(same.differs(&ours), None, "who and which pid are not the build");
    let other = Hello { engine: "engine-47-x86_64".into(), ..ours.clone() };
    assert!(other.differs(&ours).unwrap().contains("VM engine"));
    let older = Hello { version: "0.4.2".into(), ..ours.clone() };
    assert!(older.differs(&ours).unwrap().contains("Harnix 0.4.2"));
    assert_eq!(Hello::refused("no").code, codes::HANDSHAKE_REFUSED);
    let wire: Hello = serde_json::from_value(serde_json::to_value(&ours).unwrap()).unwrap();
    assert_eq!(wire, ours);
}

/* ── cancellation ───────────────────────────────────────────────────── */

#[tokio::test]
async fn a_call_cancelled_before_it_is_written_never_leaves() {
    let (a, b) = UnixStream::pair().unwrap();
    let process = SocketWire::client(a, Arc::new(Echo::default()), options());
    let mut product = Raw::new(b);

    // The other end reads nothing yet: the writer is held inside a frame far larger than the
    // socket's buffers, and what comes after it waits in the queue.
    let big = tokio::spawn({
        let process = process.clone();
        async move {
            let args = json!("x".repeat(8 << 20));
            process.call("proc.big", args, CallMeta::default(), &CancellationToken::new()).await
        }
    });
    tokio::time::sleep(Duration::from_millis(100)).await;
    let cancel = CancellationToken::new();
    let queued = tokio::spawn({
        let (process, cancel) = (process.clone(), cancel.clone());
        async move { process.call("proc.small", json!(1), CallMeta::default(), &cancel).await }
    });
    tokio::time::sleep(Duration::from_millis(50)).await;
    cancel.cancel();
    assert_eq!(queued.await.unwrap().unwrap_err().code, codes::CANCELLED, "answered here, at once");

    let first = product.recv().await;
    assert_eq!(first["method"], json!("proc.big"));
    assert_eq!(
        product.quiet(300).await,
        None,
        "the cancelled call is not sent, and nothing needs cancelling on the other end"
    );
    big.abort();
}

#[tokio::test]
async fn a_call_cancelled_on_the_wire_is_followed_by_a_cancel() {
    let (a, b) = UnixStream::pair().unwrap();
    let process = SocketWire::client(a, Arc::new(Echo::default()), options());
    let mut product = Raw::new(b);

    let cancel = CancellationToken::new();
    let call = tokio::spawn({
        let (process, cancel) = (process.clone(), cancel.clone());
        async move { process.call("proc.call", json!(1), CallMeta::default(), &cancel).await }
    });
    let sent = product.recv().await;
    assert_eq!(sent["params"]["meta"]["call_id"], json!(sent["id"].to_string()), "a call is named by its number");
    cancel.cancel();
    assert_eq!(call.await.unwrap().unwrap_err().code, codes::CANCELLED);
    let cancelled = product.recv().await;
    assert_eq!(cancelled["method"], json!("$/cancel"));
    assert_eq!(cancelled["params"]["call_id"], sent["params"]["meta"]["call_id"]);
    assert!(cancelled.get("id").is_none(), "a cancel is a notification");

    // Its answer, coming anyway, settles nothing.
    product.send(json!({"jsonrpc": "2.0", "id": sent["id"], "result": 1})).await;
    // Dropping a call's future is cancelling it.
    let dropped = tokio::spawn({
        let process = process.clone();
        async move { process.call("proc.call", json!(2), CallMeta::default(), &CancellationToken::new()).await }
    });
    let sent = product.recv().await;
    dropped.abort();
    let cancelled = product.recv().await;
    assert_eq!(cancelled["method"], json!("$/cancel"));
    assert_eq!(cancelled["params"]["call_id"], sent["params"]["meta"]["call_id"]);
    assert_eq!(process.counts().late_answers, 1);
}

#[tokio::test]
async fn a_tool_call_dropped_by_the_product_is_cancelled_by_its_own_id() {
    let kept = Arc::new(Kept::default());
    let (product, _process) = both(&kept);
    let control = ControlClient(product.clone());
    let call = control.call_tool(CallMeta::default(), ToolCall {
        app: EXT.into(),
        tool: "search".into(),
        input: json!("wait"),
        call_id: "t-9".into(),
        project: None,
        progress: true,
    });
    // The session gave up on it: the tool adapter's future is dropped.
    assert!(tokio::time::timeout(Duration::from_millis(200), call).await.is_err());
    let deadline = tokio::time::Instant::now() + PATIENCE;
    while !kept.log().contains(&"cancel t-9".to_string()) {
        assert!(tokio::time::Instant::now() < deadline, "the host never heard: {:?}", kept.log());
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[tokio::test]
async fn a_cancel_that_names_no_running_call_is_dropped_and_counted() {
    let (a, b) = UnixStream::pair().unwrap();
    let product = SocketWire::serve(a, EXT, Arc::new(Echo::default()), options());
    let mut process = Raw::new(b);
    process.send(json!({"jsonrpc": "2.0", "method": "$/cancel", "params": {"call_id": "nope"}})).await;
    process.send(request(1, "proc.x", json!({"call_id": "1"}), Value::Null)).await;
    assert_eq!(process.recv().await["id"], json!(1), "the wire goes on");
    assert_eq!(product.counts().unknown_cancels, 1);
}

#[tokio::test]
async fn the_read_loop_registers_a_call_before_it_reads_its_cancel() {
    let (a, b) = UnixStream::pair().unwrap();
    let product = SocketWire::serve(a, EXT, Arc::new(Echo::default()), options());
    let mut process = Raw::new(b);
    for n in 0..20u64 {
        // One write: the cancel is read before the call's task can have started.
        let call = request(n, "proc.x", json!({"call_id": format!("c{n}")}), json!({"wait_cancel": true}));
        let cancel = json!({"jsonrpc": "2.0", "method": "$/cancel", "params": {"call_id": format!("c{n}")}});
        process.write_raw(format!("{call}\n{cancel}\n").as_bytes()).await;
        assert_eq!(process.recv().await["result"], json!({"cancelled": true}));
    }
    assert_eq!(product.counts().unknown_cancels, 0);
}

/* ── deadlines ──────────────────────────────────────────────────────── */

#[tokio::test]
async fn a_call_that_arrives_past_its_deadline_is_not_started() {
    let echo = Arc::new(Echo::default());
    let (a, b) = UnixStream::pair().unwrap();
    let _process = SocketWire::client(a, echo.clone(), options());
    let mut product = Raw::new(b);
    let late = json!({"call_id": "1", "deadline_ms": now_ms() - 1_000});
    product.send(request(1, "ctl.status", late, json!(EXT))).await;
    assert_eq!(code(&product.recv().await), codes::DEADLINE_EXCEEDED as i64);
    assert_eq!(echo.runs.load(Ordering::SeqCst), 0, "never started");

    // Mid-call, the deadline fires the call's token.
    let soon = json!({"call_id": "2", "deadline_ms": now_ms() + 150});
    product.send(request(2, "ctl.x", soon, json!({"wait_cancel": true}))).await;
    assert_eq!(product.recv().await["result"], json!({"cancelled": true}));
}

#[tokio::test]
async fn the_sender_keeps_its_own_deadline() {
    let (a, b) = UnixStream::pair().unwrap();
    let product = SocketWire::serve(a, EXT, Arc::new(Echo::default()), options());
    let mut process = Raw::new(b);
    let meta = CallMeta::default().deadline_ms(now_ms() + 150);
    let started = std::time::Instant::now();
    let call = tokio::spawn({
        let product = product.clone();
        async move { product.call("ctl.call_method", json!({}), meta, &CancellationToken::new()).await }
    });
    let sent = process.recv().await;
    assert!(sent["params"]["meta"]["deadline_ms"].as_u64().is_some(), "the deadline travels");
    assert_eq!(call.await.unwrap().unwrap_err().code, codes::DEADLINE_EXCEEDED, "the process never answered");
    assert!(started.elapsed() < Duration::from_secs(5));
    let cancelled = process.recv().await;
    assert_eq!(cancelled["method"], json!("$/cancel"));
    assert_eq!(cancelled["params"]["call_id"], sent["params"]["meta"]["call_id"]);

    let past = CallMeta::default().deadline_ms(now_ms() - 1);
    let refused = product.call("ctl.x", Value::Null, past, &CancellationToken::new()).await.unwrap_err();
    assert_eq!(refused.code, codes::DEADLINE_EXCEEDED);
    assert_eq!(process.quiet(200).await, None, "a call already late is not sent");
}

/* ── the process's metadata is a hint ───────────────────────────────── */

#[tokio::test]
async fn the_product_re_signs_what_the_process_says_about_a_call() {
    let (a, b) = UnixStream::pair().unwrap();
    let product = SocketWire::serve(
        a,
        EXT,
        Arc::new(Echo::default()),
        Options {
            deadline_cap: Duration::from_secs(2),
            ..options()
        },
    );
    let mut process = Raw::new(b);

    // Its id is replaced by one of the product's; naming no parent, it holds nothing.
    process.send(request(1, "proc.x", json!({"call_id": "local-9"}), Value::Null)).await;
    let seen = process.recv().await["result"]["meta"].clone();
    assert_eq!(seen["call_id"], json!("local-1"));
    assert!(seen.get("chain").is_none(), "{seen}");
    // An id made up to look like one the product already gave out does not become it.
    process.send(request(2, "proc.x", json!({"call_id": "local-1"}), Value::Null)).await;
    assert_eq!(process.recv().await["result"]["meta"]["call_id"], json!("local-2"));

    // Two running calls cannot share an id.
    process.send(request(3, "proc.x", json!({"call_id": "dup"}), json!({"wait_cancel": true}))).await;
    process.send(request(4, "proc.x", json!({"call_id": "dup"}), Value::Null)).await;
    let second = process.recv().await;
    assert_eq!(second["id"], json!(4));
    assert_eq!(code(&second), codes::INVALID_REQUEST as i64);
    process.send(json!({"jsonrpc": "2.0", "method": "$/cancel", "params": {"call_id": "dup"}})).await;
    assert_eq!(process.recv().await["result"], json!({"cancelled": true}), "the first one was untouched");

    // A parent the product is not waiting on is dropped, and is the process's own: the chain
    // starts with the process held on it. The chain it sent is never read; a deadline far away
    // is brought in to the cap.
    let forged = json!({
        "call_id": "x",
        "parent": "999",
        "chain": [{"ext": "victim", "call": "c1"}],
        "deadline_ms": u64::MAX,
    });
    let before = now_ms();
    process.send(request(5, "proc.x", forged, Value::Null)).await;
    let seen = process.recv().await["result"]["meta"].clone();
    assert!(seen.get("parent").is_none(), "{seen}");
    assert_eq!(seen["chain"], json!([{"ext": EXT, "call": "999"}]));
    let deadline = seen["deadline_ms"].as_u64().unwrap();
    assert!(deadline > before && deadline <= now_ms() + 2_000, "{deadline} is not within the cap");
    assert_eq!(product.counts().dropped_parents, 1);

    // A parent the product is waiting on is kept, as the product's own id for it, and the chain
    // is the parent's with the process holding it on that call — by the number the process knows
    // that call by.
    let cancel = CancellationToken::new();
    let parent_deadline = now_ms() + 1_000;
    let product_call = tokio::spawn({
        let (product, cancel) = (product.clone(), cancel.clone());
        async move {
            let meta = CallMeta::new("dev-7").chain([ChainLink::new("other", "c0")]).deadline_ms(parent_deadline);
            product.call("ctl.call_method", json!({"m": 1}), meta, &cancel).await
        }
    });
    let outbound = process.recv().await;
    let theirs = outbound["params"]["meta"]["call_id"].clone();
    assert_eq!(theirs, json!(outbound["id"].to_string()), "the product's call goes out under its number, not `dev-7`");
    let made_for_it = json!({
        "call_id": "y",
        "parent": theirs,
        "chain": [{"ext": "victim", "call": "c1"}],
        "deadline_ms": u64::MAX,
    });
    process.send(request(6, "proc.x", made_for_it, Value::Null)).await;
    let seen = process.recv().await["result"]["meta"].clone();
    assert_eq!(seen["parent"], json!("dev-7"));
    assert_eq!(seen["chain"], json!([{"ext": "other", "call": "c0"}, {"ext": EXT, "call": theirs}]));
    assert_eq!(seen["deadline_ms"], json!(parent_deadline), "no later than its parent's");

    // The process's numbers and the product's are apart: a call of the process's under the same
    // number and id as the product's call is its own call, and answering it settles nothing.
    let same = request(outbound["id"].as_u64().unwrap(), "proc.x", json!({"call_id": theirs}), Value::Null);
    process.send(same).await;
    let answered = process.recv().await;
    assert_eq!(answered["id"], outbound["id"]);
    assert!(answered["result"].is_object());
    assert!(!product_call.is_finished(), "the product's call still waits for its own answer");
    process.send(json!({"jsonrpc": "2.0", "id": outbound["id"], "result": "done"})).await;
    assert_eq!(product_call.await.unwrap().unwrap(), json!("done"));

    // Answered, it is no one's parent any more: the chain holds the process on it alone, and the
    // process's host, done with it, waits on nothing.
    process.send(request(7, "proc.x", json!({"call_id": "z", "parent": theirs}), Value::Null)).await;
    let seen = process.recv().await["result"]["meta"].clone();
    assert!(seen.get("parent").is_none(), "{seen}");
    assert_eq!(seen["chain"], json!([{"ext": EXT, "call": theirs}]));
    assert_eq!(product.counts().dropped_parents, 2);
}

#[tokio::test]
async fn the_process_takes_the_products_word_and_names_its_own_parent() {
    let (a, b) = UnixStream::pair().unwrap();
    let process = SocketWire::client(a, Arc::new(Echo::default()), options());
    let mut product = Raw::new(b);
    let meta = json!({"call_id": "p1", "chain": [{"ext": "other", "call": "c0"}], "deadline_ms": u64::MAX});
    product.send(request(1, "ctl.call_method", meta, json!({"wait_cancel": true}))).await;
    // The process's own call made while answering `p1` names it as the product does: the
    // handler's id for the running call is the product's own.
    tokio::time::sleep(Duration::from_millis(50)).await;
    let call = tokio::spawn({
        let process = process.clone();
        async move {
            let meta = CallMeta::default().parent("p1");
            process.call("proc.call", Value::Null, meta, &CancellationToken::new()).await
        }
    });
    let sent = product.recv().await;
    assert_eq!(sent["params"]["meta"]["parent"], json!("p1"));
    product.send(json!({"jsonrpc": "2.0", "id": sent["id"], "result": null})).await;
    call.await.unwrap().unwrap();
    product.send(json!({"jsonrpc": "2.0", "method": "$/cancel", "params": {"call_id": "p1"}})).await;
    assert_eq!(product.recv().await["result"], json!({"cancelled": true}));

    // A call made for something of its own — a timer's work, which the product never sent —
    // names it by the process's own id.
    let own = tokio::spawn({
        let process = process.clone();
        async move {
            let meta = CallMeta::default().parent("delivery-1");
            process.call("proc.call", Value::Null, meta, &CancellationToken::new()).await
        }
    });
    let sent = product.recv().await;
    assert_eq!(sent["params"]["meta"]["parent"], json!("delivery-1"));
    product.send(json!({"jsonrpc": "2.0", "id": sent["id"], "result": null})).await;
    own.await.unwrap().unwrap();

    product.send(request(2, "ctl.x", json!({"call_id": "p2", "chain": [{"ext": "other", "call": "c0"}]}), Value::Null)).await;
    let seen = product.recv().await["result"]["meta"].clone();
    assert_eq!(seen["call_id"], json!("p2"), "the product's id stands");
    assert_eq!(seen["chain"], json!([{"ext": "other", "call": "c0"}]), "and its chain");
    assert!(seen.get("deadline_ms").is_none(), "and no cap of the process's own");
}

/* ── backpressure and the frame limit ───────────────────────────────── */

#[tokio::test]
async fn one_call_more_than_the_wire_carries_is_busy() {
    assert_eq!(Options::default().max_in_flight, MAX_IN_FLIGHT);
    assert_eq!(MAX_IN_FLIGHT, 256);

    // The receiver's count.
    let (a, b) = UnixStream::pair().unwrap();
    let product = SocketWire::serve(
        a,
        EXT,
        Arc::new(Echo::default()),
        Options {
            max_in_flight: 4,
            ..options()
        },
    );
    let mut process = Raw::new(b);
    for n in 0..5u64 {
        process.send(request(n, "proc.x", json!({"call_id": format!("c{n}")}), json!({"wait_cancel": true}))).await;
    }
    let busy = process.recv().await;
    assert_eq!(busy["id"], json!(4));
    assert_eq!(code(&busy), codes::BUSY as i64);
    assert_eq!(busy["error"]["data"]["limit"], json!("ext_wire"));
    assert_eq!(busy["error"]["data"]["max_in_flight"], json!(4));
    for n in 0..4u64 {
        process.send(json!({"jsonrpc": "2.0", "method": "$/cancel", "params": {"call_id": format!("c{n}")}})).await;
        process.recv().await;
    }
    assert_eq!(product.counts().busy, 1);

    // The sender's: one more is refused before it is queued.
    let (a, b) = UnixStream::pair().unwrap();
    let process = SocketWire::client(
        a,
        Arc::new(Echo::default()),
        Options {
            max_in_flight: 2,
            ..options()
        },
    );
    let mut product = Raw::new(b);
    let mut waiting = Vec::new();
    for _ in 0..2 {
        let process = process.clone();
        waiting.push(tokio::spawn(async move {
            process.call("proc.call", Value::Null, CallMeta::default(), &CancellationToken::new()).await
        }));
        product.recv().await;
    }
    let refused = process.call("proc.call", Value::Null, CallMeta::default(), &CancellationToken::new()).await.unwrap_err();
    assert_eq!(refused.code, codes::BUSY);
    assert_eq!(product.quiet(200).await, None, "not sent");
    for call in waiting {
        call.abort();
    }
}

#[tokio::test]
async fn a_frame_longer_than_the_limit_closes_the_wire() {
    assert_eq!(Options::default().max_frame_bytes, MAX_FRAME_BYTES);
    assert_eq!(MAX_FRAME_BYTES, 16 << 20);

    let (a, b) = UnixStream::pair().unwrap();
    let product = SocketWire::serve(
        a,
        EXT,
        Arc::new(Echo::default()),
        Options {
            max_frame_bytes: 1024,
            ..options()
        },
    );
    let mut process = Raw::new(b);

    // This end will not send one either: the call is refused, and the wire stays open.
    let refused = product
        .call("ctl.x", json!("x".repeat(2048)), CallMeta::default(), &CancellationToken::new())
        .await
        .unwrap_err();
    assert_eq!(refused.code, codes::INVALID_PARAMS);
    assert!(!product.is_closed());

    let waiting = tokio::spawn({
        let product = product.clone();
        async move { product.call("ctl.x", Value::Null, CallMeta::default(), &CancellationToken::new()).await }
    });
    process.recv().await;
    process.write_raw(&[b'x'; 2048]).await;
    let why = tokio::time::timeout(PATIENCE, product.closed()).await.unwrap();
    assert!(why.contains("longer than 1024"), "{why}");
    let failed = waiting.await.unwrap().unwrap_err();
    assert_eq!(failed.code, codes::EXTENSION_UNAVAILABLE, "{failed:?}");
    assert_eq!(failed.data, Some(json!({"failure": "stopped"})));
    assert!(process.at_end().await, "the product hung up");
    let after = product.call("ctl.x", Value::Null, CallMeta::default(), &CancellationToken::new()).await.unwrap_err();
    assert_eq!(after.code, codes::EXTENSION_UNAVAILABLE);
}

#[tokio::test]
async fn a_closed_wire_cancels_what_it_was_running() {
    let echo = Arc::new(Echo::default());
    let (a, b) = UnixStream::pair().unwrap();
    let process = SocketWire::client(a, echo.clone(), options());
    let mut product = Raw::new(b);
    product.send(request(1, "ctl.x", json!({"call_id": "1"}), json!({"wait_cancel": true}))).await;
    while echo.runs.load(Ordering::SeqCst) == 0 {
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    drop(product);
    let why = tokio::time::timeout(PATIENCE, process.closed()).await.unwrap();
    assert!(why.contains("closed"), "{why}");
    let failed = process.call("proc.call", Value::Null, CallMeta::default(), &CancellationToken::new()).await.unwrap_err();
    assert_eq!(failed.code, codes::INTERNAL_ERROR, "the process's view: the product is gone");
}

#[tokio::test]
async fn an_end_that_never_reads_its_answers_is_cut_off() {
    let (a, b) = UnixStream::pair().unwrap();
    let product = SocketWire::serve(
        a,
        EXT,
        Arc::new(Echo::default()),
        Options {
            max_in_flight: 4,
            ..options()
        },
    );
    let mut process = Raw::new(b);
    // Answers far larger than the socket's buffers, none of them read.
    let args = json!("x".repeat(256 << 10));
    for n in 0..64u64 {
        if product.is_closed() {
            break;
        }
        let call = request(n, "proc.x", json!({"call_id": format!("c{n}")}), args.clone());
        process.write_raw(format!("{call}\n").as_bytes()).await;
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let why = tokio::time::timeout(PATIENCE, product.closed()).await.unwrap();
    assert!(why.contains("unread"), "{why}");
}

/// An answerer that calls back over its own wire holds only a weak reference to it: the wire
/// still closes — the other end sees the stream end — once its last strong reference is dropped.
#[tokio::test]
async fn an_answerer_made_from_the_wire_does_not_keep_it_open() {
    struct Back(std::sync::Weak<SocketWire>);

    #[async_trait]
    impl Answer for Back {
        async fn call(&self, call: Inbound, args: Value) -> Result<Value, RpcError> {
            let Some(wire) = self.0.upgrade() else { return Err(RpcError::invalid_params("gone")) };
            // Back over the same wire, on behalf of the call being answered.
            let meta = CallMeta::default().with_parent(Some(call.meta.call_id().to_string()));
            wire.call("ctl.echo", args, meta, &CancellationToken::new()).await
        }
    }

    let (product, process) = UnixStream::pair().unwrap();
    let wire = SocketWire::serve_with(product, EXT, |weak| Arc::new(Back(weak)), options());
    let mut raw = Raw::new(process);
    raw.send(request(1, "proc.call", json!({"call_id": "p1"}), json!({"n": 1}))).await;
    let echoed = raw.recv().await;
    assert_eq!(echoed["method"], json!("ctl.echo"), "{echoed}");
    assert_eq!(echoed["params"]["args"], json!({"n": 1}));
    raw.send(json!({"jsonrpc": "2.0", "id": echoed["id"], "result": {"echo": true}})).await;
    assert_eq!(raw.recv().await["result"], json!({"echo": true}));

    drop(wire);
    assert!(raw.at_end().await, "the wire stayed open behind its own answerer");
}
