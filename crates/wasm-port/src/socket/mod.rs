//! The port over a stream: the extension wire (`harnix-ext-wire@1`), between a product and an
//! extension process of its own.
//!
//! NDJSON JSON-RPC 2.0, both ways on one ordered stream: each end sends calls and answers the
//! other's. A request's params are `{meta, args}` — a [`CallMeta`] beside the port's value —
//! except the handshake's ([`BARE`]). Each end numbers its own calls: a frame with a `method` is a
//! call of the other end's, a frame without one answers a call of this end's, so the two ends'
//! numbers never meet. The process sends `proc.*` and answers `ctl.*`; the product the other way.
//!
//! The two ends do not trust alike. [`SocketWire::serve`] is the product's: what the process says
//! about a call is a hint. Its id is replaced by one of the product's; its `parent` is kept only
//! when it names a call the product sent on this wire and still waits on; its `chain` is computed
//! from that parent and never read — the parent's chain, with the process held on the parent, by
//! the number the process knows that call by; its deadline is the earliest of the process's, the
//! parent's and a cap. [`SocketWire::client`] is the extension process's, and takes the product's
//! word: a call keeps the id the product sent it under, so a chain link that holds the process on
//! a call names it as the process's host knows it.
//!
//! A call goes out under a number of this end's, unique among every wire of this process: a chain
//! kept past one process of an extension (a queued turn's) never names a call of the next one.
//!
//! What a call can count on, either way:
//!
//! - **Cancellation.** A call cancelled (or its future dropped) before it was written is taken off
//!   the send queue and never sent; one already written is followed by `$/cancel {call_id}`. The
//!   read loop registers a call's token before it starts the call's task, and the stream is
//!   ordered, so a `$/cancel` finds the call it follows unless that call has answered. One that
//!   finds nothing is dropped and counted.
//! - **Deadlines**, kept by both ends: the sender answers `DEADLINE_EXCEEDED` itself when one
//!   passes (and cancels the call); the receiver answers it at once for a call that arrives too
//!   late, and fires the call's token when it passes mid-call.
//! - **Backpressure**: at most `max_in_flight` calls each way. One more is answered `BUSY`
//!   (`data.limit = "ext_wire"`) — by the sender before it is queued, or by the receiver.
//! - **A frame limit**: a line longer than `max_frame_bytes` closes the wire; a frame this end
//!   would send over it is refused instead.
//!
//! Cancelling a running call is cooperative: its task is never dropped, since what a handler
//! stopped half-way left behind nobody could trust. [`Inbound::cancel`] fires and the handler
//! decides.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::{oneshot, watch, Notify};
use tokio::task::JoinHandle;
pub use tokio_util::sync::CancellationToken;

use crate::rpc::Wire;
use crate::{codes, RpcError};

mod answer;
pub use answer::{ControlAnswer, KernelAnswer};

/// A line longer than this closes the wire.
pub const MAX_FRAME_BYTES: usize = 16 * 1024 * 1024;
/// Calls running at once, each way.
pub const MAX_IN_FLIGHT: usize = 256;
/// The latest deadline the product gives a call from the process.
pub const DEADLINE_CAP: Duration = Duration::from_secs(30);
/// What the process sends.
pub const PROC: &str = "proc.";
/// What the product sends.
pub const CTL: &str = "ctl.";
/// Cancels a call of the sender's: `{call_id}`, as the sender numbered it.
pub const CANCEL: &str = "$/cancel";
/// Calls whose params are the value itself, with no envelope: the handshake comes before there
/// is anything to say about a call.
pub const BARE: &[&str] = &[HELLO];

/// How long a closed wire goes on writing the answers and notifications it already had.
const DRAIN: Duration = Duration::from_secs(5);

pub use crate::{CallMeta, ChainLink};

/// The process's first frame, `proc.hello`: which wire it speaks, who it is, and of which build.
/// It only says which build; who the process is, the product knows from the stream it gave it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hello {
    /// [`EXT_WIRE_VERSION`](crate::EXT_WIRE_VERSION).
    pub wire: u32,
    pub ext: String,
    /// Harnix's version.
    pub version: String,
    /// The Agix it links, when it serves an engine of its own.
    pub agix_version: Option<String>,
    /// Its VM engine's fingerprint: what a precompiled component must have been compiled for.
    pub engine: String,
    /// The package interface it runs (`harnix_api`).
    pub harnix_api: u32,
    pub pid: u32,
}

impl Hello {
    /// Why a product of build `ours` refuses this hello, if it does: another wire, another
    /// Harnix, another Agix, or another VM engine — a process of another build, whose components
    /// the product's would not load even when the version says the same.
    pub fn differs(&self, ours: &Hello) -> Option<String> {
        let what = if self.wire != ours.wire {
            format!("the extension wire {} (this product speaks {})", self.wire, ours.wire)
        } else if self.version != ours.version {
            format!("Harnix {} (this product is {})", self.version, ours.version)
        } else if self.agix_version != ours.agix_version {
            format!("Agix {:?} (this product links {:?})", self.agix_version, ours.agix_version)
        } else if self.engine != ours.engine {
            format!("the VM engine `{}` (this product's is `{}`)", self.engine, ours.engine)
        } else {
            return None;
        };
        Some(format!("the extension process is of another build: {what}"))
    }

    /// The refusal of a hello, said to the process before the wire closes.
    pub fn refused(why: impl Into<String>) -> RpcError {
        RpcError::new(codes::HANDSHAKE_REFUSED, why)
    }
}

/// The handshake's method.
pub const HELLO: &str = "proc.hello";

/// Every call any wire of this process sends goes out under a number from here.
static NUMBERS: AtomicU64 = AtomicU64::new(1);

/// Now, as [`CallMeta::deadline`] counts: wall-clock milliseconds since the Unix epoch.
pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// A call of the other end's, as this end answers it.
#[derive(Debug, Clone)]
pub struct Inbound {
    /// Its full name: `ctl.call_tool`, `proc.call`, `proc.hello`.
    pub method: String,
    /// What this end makes of its metadata (see the module's documentation).
    pub meta: CallMeta,
    /// Fires when the other end cancels it, when its deadline passes, and when the wire closes.
    pub cancel: CancellationToken,
}

/// What answers the other end's calls.
#[async_trait]
pub trait Answer: Send + Sync + 'static {
    async fn call(&self, call: Inbound, args: Value) -> Result<Value, RpcError>;
    /// A notification of the other end's, in the order they came. It runs in the read loop, so it
    /// must not wait: nothing else is read until it returns.
    fn notify(&self, _method: &str, _params: Value) {}
}

/// The wire's limits, and where a call's own ids come from.
#[derive(Clone)]
pub struct Options {
    pub max_frame_bytes: usize,
    pub max_in_flight: usize,
    /// The product's end only: no call from the process gets a deadline later than this from now.
    pub deadline_cap: Duration,
    /// This end's id for a call of the other end's, or for one of its own made with none.
    pub mint: Arc<dyn Fn() -> String + Send + Sync>,
}

impl Default for Options {
    fn default() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Self {
            max_frame_bytes: MAX_FRAME_BYTES,
            max_in_flight: MAX_IN_FLIGHT,
            deadline_cap: DEADLINE_CAP,
            mint: Arc::new(|| format!("wire-{}", NEXT.fetch_add(1, Ordering::Relaxed))),
        }
    }
}

/// What the wire dropped or refused so far.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counts {
    /// `$/cancel`s that named no running call.
    pub unknown_cancels: u64,
    /// Parents that named no call of this end's still waiting for its answer.
    pub dropped_parents: u64,
    /// Calls answered `BUSY`, by either side of this end.
    pub busy: u64,
    /// Answers to calls nobody waited for any more: cancelled, or past their deadline.
    pub late_answers: u64,
    /// Lines that were not JSON-RPC.
    pub malformed: u64,
}

#[derive(Default)]
struct Counters {
    unknown_cancels: AtomicU64,
    dropped_parents: AtomicU64,
    busy: AtomicU64,
    late_answers: AtomicU64,
    malformed: AtomicU64,
}

fn bump(counter: &AtomicU64) {
    counter.fetch_add(1, Ordering::Relaxed);
}

enum End {
    /// The extension process's.
    Client,
    /// The product's, to the extension `ext`.
    Serve { ext: String },
}

impl End {
    fn sends(&self) -> &'static str {
        match self {
            End::Client => PROC,
            End::Serve { .. } => CTL,
        }
    }
}

/// A frame waiting to be written; `call` names it when it is a call of this end's.
struct Frame {
    call: Option<u64>,
    /// An answer to a call of the other end's: it counts against the other end until written.
    answer: bool,
    bytes: Vec<u8>,
}

/// A call of this end's, waiting for its answer.
struct Sent {
    reply: oneshot::Sender<Result<Value, RpcError>>,
    written: bool,
    /// As the caller gave it: what a call of the other end's naming this one as its parent gets.
    meta: CallMeta,
}

/// A call of the other end's, running.
struct Running {
    local: String,
    cancel: CancellationToken,
}

#[derive(Default)]
struct State {
    queue: VecDeque<Frame>,
    /// By this end's number for them.
    sent: HashMap<u64, Sent>,
    /// By the other end's id for them.
    running: HashMap<String, Running>,
    /// This end's id for a running call → the other end's.
    remote_of: HashMap<String, String>,
    /// Answers in the queue, not yet written.
    answers: usize,
    closed: Option<String>,
}

struct Inner {
    end: End,
    options: Options,
    answer: Arc<dyn Answer>,
    state: Mutex<State>,
    wake: Notify,
    closed: watch::Sender<Option<String>>,
    counts: Counters,
}

/// One end of the extension wire. A [`Wire`]: each port call goes out as `<prefix><name>` with
/// metadata of its own; [`SocketWire::call`] sends one with the caller's.
pub struct SocketWire {
    inner: Arc<Inner>,
    writer: Mutex<Option<JoinHandle<()>>>,
}

impl SocketWire {
    /// The extension process's end: it sends `proc.*` and answers `ctl.*` with `answer`.
    pub fn client<S>(stream: S, answer: Arc<dyn Answer>, options: Options) -> Arc<Self>
    where
        S: AsyncRead + AsyncWrite + Send + 'static,
    {
        Self::start(stream, End::Client, answer, options)
    }

    /// The product's end, to the extension process of `ext`: it sends `ctl.*` and answers `proc.*`
    /// with `answer`, re-signing what the process says about each call.
    pub fn serve<S>(stream: S, ext: impl Into<String>, answer: Arc<dyn Answer>, options: Options) -> Arc<Self>
    where
        S: AsyncRead + AsyncWrite + Send + 'static,
    {
        Self::start(stream, End::Serve { ext: ext.into() }, answer, options)
    }

    fn start<S>(stream: S, end: End, answer: Arc<dyn Answer>, options: Options) -> Arc<Self>
    where
        S: AsyncRead + AsyncWrite + Send + 'static,
    {
        let (read, write) = tokio::io::split(stream);
        let inner = Arc::new(Inner {
            end,
            options,
            answer,
            state: Mutex::new(State::default()),
            wake: Notify::new(),
            closed: watch::channel(None).0,
            counts: Counters::default(),
        });
        tokio::spawn(read_loop(inner.clone(), read));
        let writer = tokio::spawn(write_loop(inner.clone(), write));
        Arc::new(Self {
            inner,
            writer: Mutex::new(Some(writer)),
        })
    }

    /// Send a call, `method` by its full name, as `meta` says: its id this end's own (one is made
    /// when it is empty), its parent a call of the other end's this one is made while answering.
    /// Cancelling `cancel`, dropping the future, or `meta.deadline_ms` passing withdraws it.
    pub async fn call(&self, method: &str, args: Value, meta: CallMeta, cancel: &CancellationToken) -> Result<Value, RpcError> {
        let deadline = meta.deadline();
        if deadline.is_some_and(|d| d <= now_ms()) {
            return Err(deadline_exceeded());
        }
        let (id, reply) = self.inner.enqueue(method, args, meta)?;
        let _withdraw = Withdraw { inner: &self.inner, id };
        tokio::select! {
            answer = reply => answer.unwrap_or_else(|_| Err(self.inner.closed_error("the wire closed"))),
            _ = cancel.cancelled() => Err(cancelled()),
            _ = until(deadline) => Err(deadline_exceeded()),
        }
    }

    /// Send a notification: no answer, no metadata.
    pub fn notify(&self, method: &str, params: Value) -> Result<(), RpcError> {
        let bytes = line(&json!({"jsonrpc": "2.0", "method": method, "params": params}));
        self.inner.fits(&bytes)?;
        let mut state = self.inner.lock();
        if let Some(why) = &state.closed {
            return Err(self.inner.closed_error(why));
        }
        state.queue.push_back(Frame { call: None, answer: false, bytes });
        drop(state);
        self.inner.wake.notify_one();
        Ok(())
    }

    /// Close the wire: every call waiting for an answer is answered that it closed, every running
    /// call's token fires. What was already queued to the other end besides calls — answers,
    /// notifications — is still written, for a few seconds at most; this returns once it was.
    pub async fn close(&self, why: &str) {
        self.inner.close(why.to_string());
        let writer = self.writer.lock().unwrap_or_else(|e| e.into_inner()).take();
        if let Some(writer) = writer {
            let _ = writer.await;
        }
    }

    /// Why the wire closed, once it has.
    pub async fn closed(&self) -> String {
        let mut closed = self.inner.closed.subscribe();
        let why = closed.wait_for(Option::is_some).await;
        why.map(|w| (*w).clone().unwrap_or_default()).unwrap_or_default()
    }

    pub fn is_closed(&self) -> bool {
        self.inner.lock().closed.is_some()
    }

    pub fn counts(&self) -> Counts {
        let c = &self.inner.counts;
        let get = |a: &AtomicU64| a.load(Ordering::Relaxed);
        Counts {
            unknown_cancels: get(&c.unknown_cancels),
            dropped_parents: get(&c.dropped_parents),
            busy: get(&c.busy),
            late_answers: get(&c.late_answers),
            malformed: get(&c.malformed),
        }
    }
}

impl Drop for SocketWire {
    fn drop(&mut self) {
        self.inner.close("the wire's end was dropped".into());
    }
}

#[async_trait]
impl Wire for SocketWire {
    async fn send(&self, method: &'static str, meta: CallMeta, params: Value) -> Result<Value, RpcError> {
        let method = format!("{}{method}", self.inner.end.sends());
        self.call(&method, params, meta, &CancellationToken::new()).await
    }

    fn notify(&self, method: &'static str, params: Value) {
        let method = format!("{}{method}", self.inner.end.sends());
        // A closed wire has nobody to tell.
        let _ = SocketWire::notify(self, &method, params);
    }
}

/// Takes a call back when its future ends without an answer: off the queue if it was not written,
/// with a `$/cancel` if it was. After an answer there is nothing to take back.
struct Withdraw<'a> {
    inner: &'a Inner,
    id: u64,
}

impl Drop for Withdraw<'_> {
    fn drop(&mut self) {
        self.inner.withdraw(self.id);
    }
}

impl Inner {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn fits(&self, line: &[u8]) -> Result<(), RpcError> {
        let size = line.len() - 1;
        if size > self.options.max_frame_bytes {
            return Err(RpcError::invalid_params(format!(
                "{size} bytes is more than the extension wire carries in a frame ({})",
                self.options.max_frame_bytes
            )));
        }
        Ok(())
    }

    fn closed_error(&self, why: &str) -> RpcError {
        match self.end {
            // The product's view: the extension is gone until it is started again.
            End::Serve { .. } => RpcError {
                code: codes::EXTENSION_UNAVAILABLE,
                message: format!("the extension wire closed: {why}"),
                data: Some(json!({"failure": "stopped"})),
            },
            End::Client => RpcError::new(codes::INTERNAL_ERROR, format!("the product's wire closed: {why}")),
        }
    }

    fn enqueue(&self, method: &str, args: Value, mut meta: CallMeta) -> Result<(u64, oneshot::Receiver<Result<Value, RpcError>>), RpcError> {
        if meta.call_id().is_empty() {
            meta = meta.with_call_id((self.options.mint)());
        }
        let id = NUMBERS.fetch_add(1, Ordering::Relaxed);
        // The parent goes out as the other end knows it, or not at all.
        let parent = meta.parent_id().and_then(|p| self.lock().remote_of.get(p).cloned());
        let params = if BARE.contains(&method) {
            args
        } else {
            let on_wire = CallMeta::new(id.to_string())
                .with_parent(parent)
                .chain(meta.links().iter().cloned())
                .with_deadline(meta.deadline());
            json!({"meta": on_wire, "args": args})
        };
        let bytes = line(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        self.fits(&bytes)?;
        let (reply, answer) = oneshot::channel();
        let mut state = self.lock();
        if let Some(why) = &state.closed {
            return Err(self.closed_error(why));
        }
        if state.sent.len() >= self.options.max_in_flight {
            bump(&self.counts.busy);
            return Err(busy(self.options.max_in_flight));
        }
        state.sent.insert(id, Sent { reply, written: false, meta });
        state.queue.push_back(Frame { call: Some(id), answer: false, bytes });
        drop(state);
        self.wake.notify_one();
        Ok((id, answer))
    }

    fn withdraw(&self, id: u64) {
        let mut state = self.lock();
        let Some(sent) = state.sent.remove(&id) else { return };
        if sent.written {
            let bytes = line(&json!({"jsonrpc": "2.0", "method": CANCEL, "params": {"call_id": id.to_string()}}));
            state.queue.push_back(Frame { call: None, answer: false, bytes });
            drop(state);
            self.wake.notify_one();
        } else {
            state.queue.retain(|f| f.call != Some(id));
        }
    }

    fn close(&self, why: String) {
        let (sent, running) = {
            let mut state = self.lock();
            if state.closed.is_some() {
                return;
            }
            state.closed = Some(why.clone());
            state.queue.retain(|f| f.call.is_none());
            state.remote_of.clear();
            (std::mem::take(&mut state.sent), std::mem::take(&mut state.running))
        };
        let error = self.closed_error(&why);
        for (_, sent) in sent {
            let _ = sent.reply.send(Err(error.clone()));
        }
        for (_, running) in running {
            running.cancel.cancel();
        }
        self.closed.send_replace(Some(why));
        self.wake.notify_one();
    }

    /// One line from the other end. Everything but starting a call's task happens here, before the
    /// next line is read: that is what lets a `$/cancel` find the call it follows.
    fn receive(self: &Arc<Self>, line: &[u8]) {
        if line.iter().all(u8::is_ascii_whitespace) {
            return;
        }
        let Ok(Value::Object(mut frame)) = serde_json::from_slice::<Value>(line) else {
            bump(&self.counts.malformed);
            return;
        };
        let params = frame.remove("params").unwrap_or(Value::Null);
        let id = frame.remove("id").filter(|id| !id.is_null());
        match (frame.get("method").and_then(Value::as_str), id) {
            (Some(CANCEL), _) => self.cancel(params.get("call_id").and_then(Value::as_str)),
            (Some(method), None) => self.answer.notify(method, params),
            (Some(method), Some(id)) => self.admit(method.to_string(), id, params),
            (None, Some(id)) => self.settle(id, frame),
            (None, None) => bump(&self.counts.malformed),
        }
    }

    fn cancel(&self, call_id: Option<&str>) {
        match call_id.and_then(|id| self.lock().running.get(id).map(|r| r.cancel.clone())) {
            Some(token) => token.cancel(),
            None => bump(&self.counts.unknown_cancels),
        }
    }

    fn settle(&self, id: Value, mut frame: serde_json::Map<String, Value>) {
        let Some(sent) = id.as_u64().and_then(|id| self.lock().sent.remove(&id)) else {
            bump(&self.counts.late_answers);
            return;
        };
        let answer = match frame.remove("error") {
            Some(error) => Err(serde_json::from_value(error)
                .unwrap_or_else(|e| RpcError::new(codes::INTERNAL_ERROR, format!("an error the wire could not read: {e}")))),
            None => Ok(frame.remove("result").unwrap_or(Value::Null)),
        };
        let _ = sent.reply.send(answer);
    }

    fn admit(self: &Arc<Self>, method: String, id: Value, params: Value) {
        // A call is the other end's until its answer is written, not only while it runs: one that
        // sends calls and never reads what they answer would otherwise fill this end's memory.
        let unread = self.lock().answers;
        if unread >= self.options.max_in_flight {
            return self.close(format!("the other end left {unread} answers unread and sent another call"));
        }
        let (remote, args) = if BARE.contains(&method.as_str()) {
            let call_id = match &id {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            (CallMeta::new(call_id), params)
        } else {
            match envelope(params) {
                Ok(call) => call,
                Err(e) => return self.respond(id, Err(e)),
            }
        };
        // The product names a call of the process's its own way; the process keeps the product's.
        let local = match self.end {
            End::Serve { .. } => (self.options.mint)(),
            End::Client => remote.call_id().to_string(),
        };
        let now = now_ms();
        let mut state = self.lock();
        if state.closed.is_some() {
            return;
        }
        if state.running.len() + state.answers >= self.options.max_in_flight {
            drop(state);
            bump(&self.counts.busy);
            return self.respond(id, Err(busy(self.options.max_in_flight)));
        }
        let remote_id = remote.call_id().to_string();
        if state.running.contains_key(&remote_id) {
            drop(state);
            return self.respond(
                id,
                Err(RpcError::new(codes::INVALID_REQUEST, format!("call `{remote_id}` is already running"))),
            );
        }
        // The parent is a call this end sent and still waits on, or nothing.
        let parent = remote
            .parent_id()
            .and_then(|p| p.parse::<u64>().ok())
            .and_then(|n| state.sent.get(&n).map(|sent| (n, sent.meta.clone())));
        if remote.parent_id().is_some() && parent.is_none() {
            bump(&self.counts.dropped_parents);
        }
        let (chain, cap) = match &self.end {
            // The process is held on the parent, by the number it was sent under: the id its host
            // knows that call by, and checks a chain against.
            End::Serve { ext } => {
                let chain = parent.as_ref().map(|(n, p)| {
                    let mut chain = p.links().to_vec();
                    chain.push(ChainLink::new(ext.as_str(), n.to_string()));
                    chain
                });
                let cap = now.saturating_add(self.options.deadline_cap.as_millis() as u64);
                (chain.unwrap_or_default(), Some(cap))
            }
            End::Client => (remote.links().to_vec(), None),
        };
        let deadline = [remote.deadline(), parent.as_ref().and_then(|(_, p)| p.deadline()), cap]
            .into_iter()
            .flatten()
            .min();
        if deadline.is_some_and(|d| d <= now) {
            drop(state);
            return self.respond(id, Err(deadline_exceeded()));
        }
        let cancel = CancellationToken::new();
        state.running.insert(remote_id.clone(), Running { local: local.clone(), cancel: cancel.clone() });
        state.remote_of.insert(local.clone(), remote_id.clone());
        drop(state);

        let call = Inbound {
            method,
            meta: CallMeta::new(local)
                .with_parent(parent.map(|(_, p)| p.call_id().to_string()))
                .chain(chain)
                .with_deadline(deadline),
            cancel: cancel.clone(),
        };
        let inner = self.clone();
        tokio::spawn(async move {
            let answer = inner.answer.clone();
            let running = answer.call(call, args);
            tokio::pin!(running);
            let result = tokio::select! {
                result = &mut running => result,
                _ = until(deadline) => {
                    cancel.cancel();
                    running.await
                }
            };
            inner.finish(&remote_id, id, result);
        });
    }

    fn finish(&self, remote: &str, id: Value, result: Result<Value, RpcError>) {
        {
            let mut state = self.lock();
            if let Some(running) = state.running.remove(remote) {
                state.remote_of.remove(&running.local);
            }
        }
        self.respond(id, result);
    }

    fn respond(&self, id: Value, result: Result<Value, RpcError>) {
        let frame = |result: Result<Value, RpcError>| match result {
            Ok(value) => json!({"jsonrpc": "2.0", "id": id, "result": value}),
            Err(error) => json!({"jsonrpc": "2.0", "id": id, "error": error}),
        };
        let mut bytes = line(&frame(result));
        if let Err(error) = self.fits(&bytes) {
            bytes = line(&frame(Err(RpcError::new(codes::INTERNAL_ERROR, format!("the answer: {}", error.message)))));
        }
        let mut state = self.lock();
        state.answers += 1;
        state.queue.push_back(Frame { call: None, answer: true, bytes });
        drop(state);
        self.wake.notify_one();
    }

    /// The next frame to write, marking a call written as it leaves the queue.
    fn next_frame(&self) -> (Option<Vec<u8>>, bool) {
        let mut state = self.lock();
        let frame = state.queue.pop_front();
        match &frame {
            Some(Frame { call: Some(id), .. }) => {
                if let Some(sent) = state.sent.get_mut(id) {
                    sent.written = true;
                }
            }
            Some(Frame { answer: true, .. }) => state.answers -= 1,
            _ => {}
        }
        (frame.map(|f| f.bytes), state.closed.is_some())
    }
}

/// `{meta, args}`.
fn envelope(params: Value) -> Result<(CallMeta, Value), RpcError> {
    let Value::Object(mut params) = params else {
        return Err(RpcError::new(codes::INVALID_REQUEST, "a call's params are {meta, args}"));
    };
    let meta: CallMeta = params
        .remove("meta")
        .ok_or_else(|| RpcError::new(codes::INVALID_REQUEST, "a call's params are {meta, args}: no meta"))
        .and_then(|m| serde_json::from_value(m).map_err(|e| RpcError::new(codes::INVALID_REQUEST, format!("a call's meta: {e}"))))?;
    if meta.call_id().is_empty() {
        return Err(RpcError::new(codes::INVALID_REQUEST, "a call's meta names no call_id"));
    }
    Ok((meta, params.remove("args").unwrap_or(Value::Null)))
}

fn line(frame: &Value) -> Vec<u8> {
    let mut bytes = serde_json::to_vec(frame).unwrap_or_default();
    bytes.push(b'\n');
    bytes
}

fn cancelled() -> RpcError {
    RpcError::new(codes::CANCELLED, "the call was cancelled")
}

fn deadline_exceeded() -> RpcError {
    RpcError::new(codes::DEADLINE_EXCEEDED, "the call's deadline passed")
}

fn busy(max: usize) -> RpcError {
    RpcError {
        code: codes::BUSY,
        message: format!("{max} calls are already running over the extension wire"),
        data: Some(json!({"limit": "ext_wire", "max_in_flight": max})),
    }
}

/// Ready once the wire is closed. (The guard `wait_for` answers is not held past it.)
async fn shut(closed: &mut watch::Receiver<Option<String>>) {
    let _ = closed.wait_for(Option::is_some).await;
}

/// Ready at `deadline`, never without one.
async fn until(deadline: Option<u64>) {
    match deadline {
        Some(at) => tokio::time::sleep(Duration::from_millis(at.saturating_sub(now_ms()))).await,
        None => std::future::pending().await,
    }
}

/// One line into `buf`, without its newline: `false` at the end of the stream. Longer than `max`
/// is an error, read no further.
async fn frame<R: AsyncBufRead + Unpin>(read: &mut R, buf: &mut Vec<u8>, max: usize) -> std::io::Result<bool> {
    buf.clear();
    loop {
        let available = read.fill_buf().await?;
        if available.is_empty() {
            return match buf.is_empty() {
                true => Ok(false),
                false => Err(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "the stream ended inside a frame")),
            };
        }
        let (take, done) = match available.iter().position(|b| *b == b'\n') {
            Some(at) => (at, true),
            None => (available.len(), false),
        };
        if buf.len() + take > max {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("a frame longer than {max} bytes"),
            ));
        }
        buf.extend_from_slice(&available[..take]);
        read.consume(if done { take + 1 } else { take });
        if done {
            return Ok(true);
        }
    }
}

async fn read_loop<R: AsyncRead + Unpin>(inner: Arc<Inner>, read: R) {
    let mut read = BufReader::new(read);
    let mut closed = inner.closed.subscribe();
    let mut buf = Vec::new();
    let why = loop {
        tokio::select! {
            got = frame(&mut read, &mut buf, inner.options.max_frame_bytes) => match got {
                Ok(true) => inner.receive(&buf),
                Ok(false) => break "the other end closed it".to_string(),
                Err(e) => break e.to_string(),
            },
            _ = shut(&mut closed) => return,
        }
    };
    inner.close(why);
}

async fn write_loop<W: AsyncWrite + Unpin>(inner: Arc<Inner>, mut write: W) {
    let mut closed = inner.closed.subscribe();
    let mut drain = None;
    loop {
        let (next, is_closed) = inner.next_frame();
        if is_closed && drain.is_none() {
            drain = Some(tokio::time::Instant::now() + DRAIN);
        }
        let Some(bytes) = next else {
            if is_closed {
                break;
            }
            inner.wake.notified().await;
            continue;
        };
        let writing = write.write_all(&bytes);
        tokio::pin!(writing);
        let wrote = match drain {
            Some(at) => tokio::time::timeout_at(at, &mut writing).await,
            // A write the other end does not read can block; closing must not wait on it forever.
            None => tokio::select! {
                wrote = &mut writing => Ok(wrote),
                _ = shut(&mut closed) => {
                    let at = tokio::time::Instant::now() + DRAIN;
                    drain = Some(at);
                    tokio::time::timeout_at(at, &mut writing).await
                }
            },
        };
        match wrote {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                inner.close(format!("writing to it: {e}"));
                break;
            }
            Err(_) => break,
        }
    }
    let _ = tokio::time::timeout(Duration::from_secs(1), write.shutdown()).await;
}
