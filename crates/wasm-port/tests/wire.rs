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
    async fn register(&self, r: Registration) -> Result<(), RpcError> {
        self.note(format!("register {} {}", r.app, r.methods.len()));
        Ok(())
    }
    async fn withdraw(&self, app: String) -> Result<(), RpcError> {
        Err(RpcError::refused(format!("{app} was never here")))
    }
    async fn call(&self, c: CallAs) -> Result<Value, RpcError> {
        Ok(json!({"method": c.method, "params": c.params, "chain": c.chain}))
    }
    async fn publish(&self, p: Publish) -> Result<(), RpcError> {
        self.note(format!("publish {} {}", p.topic, p.value));
        Ok(())
    }
    async fn subscribe(&self, s: Subscription) -> Result<Value, RpcError> {
        Ok(json!({"topic": s.topic, "value": 7}))
    }
    async fn unsubscribe(&self, s: Subscription) -> Result<(), RpcError> {
        self.note(format!("unsubscribe {}", s.topic));
        Ok(())
    }
    async fn progress(&self, p: Progress) -> Result<(), RpcError> {
        self.note(format!("progress {} {}", p.call_id, p.text));
        Ok(())
    }
}

#[async_trait]
impl Control for Kept {
    async fn place(&self, p: Place) -> Result<Placement, RpcError> {
        Ok(Placement {
            parts: json!({"agent": p.app}),
            disclosure: json!(p.version),
        })
    }
    async fn unplace(&self, u: Unplace) -> Result<Value, RpcError> {
        Ok(u.parts)
    }
    async fn start(&self, s: Start) -> Result<Started, RpcError> {
        Ok(Started {
            app: s.app,
            trust: "store".into(),
            world: Some("backend".into()),
            tools: vec![],
        })
    }
    async fn stop(&self, s: Stop) -> Result<(), RpcError> {
        self.note(format!("stop {}", s.app));
        Ok(())
    }
    async fn forget(&self, app: String) -> Result<(), RpcError> {
        self.note(format!("forget {app}"));
        Ok(())
    }
    async fn status(&self, app: String) -> Result<Value, RpcError> {
        Ok(json!({"app": app, "state": "running"}))
    }
    async fn call_method(&self, m: MethodCall) -> Result<Value, RpcError> {
        Ok(json!({"method": m.method, "caller": m.caller, "params": m.params}))
    }
    async fn deliver(&self, v: TopicValue) -> Result<(), RpcError> {
        self.note(format!("deliver {} {}", v.topic, v.value));
        Ok(())
    }
    async fn call_tool(&self, c: ToolCall) -> Result<ToolReply, RpcError> {
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
    async fn hook(&self, h: HookCall) -> Result<HookReply, RpcError> {
        Ok(HookReply::Block(h.event))
    }
    async fn cancel(&self, call_id: String) -> Result<(), RpcError> {
        self.note(format!("cancel {call_id}"));
        self.cancelled.notify_one();
        Ok(())
    }
    async fn shutdown(&self, reason: String) -> Result<(), RpcError> {
        self.note(format!("shutdown {reason}"));
        Ok(())
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
        .register(Registration {
            app: EXT.into(),
            methods: vec![MethodDecl {
                name: "notes.add".into(),
                description: String::new(),
                subjects: vec!["device".into()],
            }],
            topics: vec!["notes".into()],
            spaces: vec![SpaceDecl::new(EXT)],
            hub: vec![],
        })
        .await
        .unwrap();
    let refused = kernel.withdraw(EXT.into()).await.unwrap_err();
    assert_eq!(refused.code, codes::REFUSED, "an error crosses as it was");
    assert!(refused.message.contains("never here"));
    let called = kernel
        .call(CallAs {
            app: EXT.into(),
            method: "harnix.sessions.list".into(),
            params: json!({"n": 1}),
            chain: vec!["victim".into()],
        })
        .await
        .unwrap();
    assert_eq!(called["params"]["n"], json!(1));
    assert_eq!(
        called["chain"],
        json!([]),
        "the chain the process wrote into the value is not the one the product acts on"
    );
    let subscription = Subscription {
        app: EXT.into(),
        topic: "harnix.spaces".into(),
        params: Value::Null,
    };
    assert_eq!(kernel.subscribe(subscription.clone()).await.unwrap()["value"], json!(7));
    kernel.unsubscribe(subscription).await.unwrap();
    kernel
        .publish(Publish {
            app: EXT.into(),
            topic: "notes".into(),
            value: json!(1),
        })
        .await
        .unwrap();
    kernel
        .progress(Progress {
            app: EXT.into(),
            tool: "search".into(),
            call_id: "t-1".into(),
            text: "half".into(),
        })
        .await
        .unwrap();

    // The product to the process: `Control`, as `ctl.*`.
    let placement = control
        .place(Place {
            app: EXT.into(),
            app_dir: "/x".into(),
            version: "1.0.0".into(),
        })
        .await
        .unwrap();
    assert_eq!(placement.disclosure, json!("1.0.0"));
    assert_eq!(
        control
            .unplace(Unplace {
                app: EXT.into(),
                parts: json!({"agent": true}),
            })
            .await
            .unwrap(),
        json!({"agent": true})
    );
    let started = control
        .start(Start {
            app: EXT.into(),
            version: "1.0.0".into(),
            app_dir: "/x".into(),
            parts: Value::Null,
        })
        .await
        .unwrap();
    assert_eq!(started.world.as_deref(), Some("backend"));
    control
        .stop(Stop {
            app: EXT.into(),
            parts: Value::Null,
        })
        .await
        .unwrap();
    control.forget(EXT.into()).await.unwrap();
    assert_eq!(control.status(EXT.into()).await.unwrap()["state"], json!("running"));
    let method = control
        .call_method(MethodCall {
            app: EXT.into(),
            method: "notes.add".into(),
            caller: Subject::Kernel,
            params: json!([1]),
            chain: vec![],
        })
        .await
        .unwrap();
    assert_eq!(method["params"], json!([1]));
    control
        .deliver(TopicValue {
            app: EXT.into(),
            topic: "harnix.spaces".into(),
            value: json!(2),
        })
        .await
        .unwrap();
    let tool = control
        .call_tool(ToolCall {
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
        .hook(HookCall {
            app: EXT.into(),
            event: "tool.before".into(),
            payload: Value::Null,
            call_id: "h-1".into(),
        })
        .await
        .unwrap();
    assert_eq!(hook, HookReply::Block("tool.before".into()));
    control.cancel("t-0".into()).await.unwrap();
    control.shutdown("bye".into()).await.unwrap();

    assert_eq!(
        kept.log(),
        [
            "register acme.notes 1",
            "unsubscribe harnix.spaces",
            "publish notes 1",
            "progress t-1 half",
            "stop acme.notes",
            "forget acme.notes",
            "deliver harnix.spaces 2",
            "cancel t-0",
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
    let call = control.call_tool(ToolCall {
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
    let meta = CallMeta {
        deadline_ms: Some(now_ms() + 150),
        ..CallMeta::default()
    };
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

    let past = CallMeta {
        deadline_ms: Some(now_ms() - 1),
        ..CallMeta::default()
    };
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

    // Its id is replaced by one of the product's.
    process.send(request(1, "proc.x", json!({"call_id": "local-9"}), Value::Null)).await;
    assert_eq!(process.recv().await["result"]["meta"]["call_id"], json!("local-1"));
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

    // A parent the product is not waiting on is dropped; a chain is never read; a deadline far
    // away is brought in to the cap.
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
    assert!(seen.get("chain").is_none(), "{seen}");
    let deadline = seen["deadline_ms"].as_u64().unwrap();
    assert!(deadline > before && deadline <= now_ms() + 2_000, "{deadline} is not within the cap");
    assert_eq!(product.counts().dropped_parents, 1);

    // A parent the product is waiting on is kept, as the product's own id for it, and the chain
    // is the parent's with the process holding it on that call.
    let cancel = CancellationToken::new();
    let parent_deadline = now_ms() + 1_000;
    let product_call = tokio::spawn({
        let (product, cancel) = (product.clone(), cancel.clone());
        async move {
            let meta = CallMeta {
                call_id: "dev-7".into(),
                chain: vec![ChainLink {
                    ext: "other".into(),
                    call: "c0".into(),
                }],
                deadline_ms: Some(parent_deadline),
                ..CallMeta::default()
            };
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
    assert_eq!(seen["chain"], json!([{"ext": "other", "call": "c0"}, {"ext": EXT, "call": "dev-7"}]));
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

    // Answered, it is no one's parent any more.
    process.send(request(7, "proc.x", json!({"call_id": "z", "parent": theirs}), Value::Null)).await;
    assert!(process.recv().await["result"]["meta"].get("parent").is_none());
    assert_eq!(product.counts().dropped_parents, 2);
}

#[tokio::test]
async fn the_process_takes_the_products_word_and_names_its_own_parent() {
    let (a, b) = UnixStream::pair().unwrap();
    let process = SocketWire::client(a, Arc::new(Echo::default()), options());
    let mut product = Raw::new(b);
    let meta = json!({"call_id": "p1", "chain": [{"ext": "other", "call": "c0"}], "deadline_ms": u64::MAX});
    product.send(request(1, "ctl.call_method", meta, json!({"wait_cancel": true}))).await;
    // The process's own call made while answering `p1` names it as the product does.
    tokio::time::sleep(Duration::from_millis(50)).await;
    let call = tokio::spawn({
        let process = process.clone();
        async move {
            // The handler's id for the running call is this end's first.
            let meta = CallMeta {
                parent: Some("local-1".into()),
                ..CallMeta::default()
            };
            process.call("proc.call", Value::Null, meta, &CancellationToken::new()).await
        }
    });
    let sent = product.recv().await;
    assert_eq!(sent["params"]["meta"]["parent"], json!("p1"));
    product.send(json!({"jsonrpc": "2.0", "id": sent["id"], "result": null})).await;
    call.await.unwrap().unwrap();
    product.send(json!({"jsonrpc": "2.0", "method": "$/cancel", "params": {"call_id": "p1"}})).await;
    assert_eq!(product.recv().await["result"], json!({"cancelled": true}));

    product.send(request(2, "ctl.x", json!({"call_id": "p2", "chain": [{"ext": "other", "call": "c0"}]}), Value::Null)).await;
    let seen = product.recv().await["result"]["meta"].clone();
    assert_eq!(seen["chain"], json!([{"ext": "other", "call": "c0"}]), "the product's chain stands");
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
