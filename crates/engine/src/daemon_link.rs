//! An Agix daemon in another process, behind [`EngineLink`] (concept §3.1.1).
//!
//! Newline-delimited JSON-RPC over a stream: a unix socket, loopback TCP with the daemon's token,
//! or any stream handed in — an inherited descriptor, say. The link takes a connection, checks
//! that the daemon speaks this build's protocol, subscribes to its events, and reads; whoever
//! gave it the connection decides what happens when it is lost. A spawned daemon's link is kept
//! connected, and its daemon running, by the instance layer; this one only speaks to it.
//!
//! A lost connection is not a lost instance. The link goes `Degraded` while it waits for the next
//! one, and once it has one writes [`EngineEvent::Reconnected`] into the event stream, so the hub
//! knows every subscription it had is gone and resyncs. Turns on the far side keep running
//! meanwhile (Agix §5.6).
//!
//! The daemon trusts an authenticated connection completely and has no per-method authorization
//! (Agix §5.1), so a daemon serves one Harnix: whoever hands this link a stream has made sure
//! nobody else holds one.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use harnix_contract::sync::lock;
use harnix_contract::{codes, RpcError};
use harnix_engine_api::link::{EngineControl, EngineIdentity, EngineLink, EventReceiver, InstanceCarrier, LinkStatus, EVENT_CAPACITY};
use harnix_engine_api::ops::EngineOps;
use harnix_engine_api::EngineEvent;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::{mpsc, oneshot, watch, Mutex};
use tokio_util::sync::CancellationToken;

use crate::gate::{GatedOps, WakeGate};
use crate::translate::{self, AgixOps, Rpc};

/// Where a daemon listens.
#[derive(Debug, Clone)]
pub enum Endpoint {
    Unix(PathBuf),
    /// Loopback TCP. The token is the daemon's `AGIX_DAEMON_TOKEN`, held by Harnix and never given
    /// to a client.
    Tcp { addr: String, token: String },
}

impl Endpoint {
    /// Whether something listens there yet.
    pub async fn listening(&self) -> bool {
        match self {
            Endpoint::Unix(path) => tokio::net::UnixStream::connect(path).await.is_ok(),
            Endpoint::Tcp { addr, .. } => tokio::net::TcpStream::connect(addr).await.is_ok(),
        }
    }

    /// What a connection to it presents first, when anything.
    pub fn token(&self) -> Option<&str> {
        match self {
            Endpoint::Unix(_) => None,
            Endpoint::Tcp { token, .. } => Some(token),
        }
    }
}

type Reader = Box<dyn AsyncRead + Send + Unpin>;
type Writer = Box<dyn AsyncWrite + Send + Unpin>;

/// One connection to a daemon: the two halves of a stream.
pub struct Connection {
    reader: Reader,
    writer: Writer,
}

impl Connection {
    pub fn new(reader: impl AsyncRead + Send + Unpin + 'static, writer: impl AsyncWrite + Send + Unpin + 'static) -> Self {
        Self { reader: Box::new(reader), writer: Box::new(writer) }
    }

    /// A stream somebody already opened: a unix socket, say, or one end of an inherited pair.
    pub fn unix(stream: tokio::net::UnixStream) -> Self {
        let (r, w) = stream.into_split();
        Self::new(r, w)
    }

    /// Open one to `endpoint`.
    pub async fn open(endpoint: &Endpoint) -> std::io::Result<Self> {
        Ok(match endpoint {
            Endpoint::Unix(path) => Self::unix(tokio::net::UnixStream::connect(path).await?),
            Endpoint::Tcp { addr, .. } => {
                let (r, w) = tokio::net::TcpStream::connect(addr).await?.into_split();
                Self::new(r, w)
            }
        })
    }
}

/// Why a connection was not taken.
#[derive(Debug)]
pub enum Refusal {
    /// Trying again cannot help: wrong protocol, wrong token.
    Fatal(String),
    Retry(String),
}

/// A connection the link has taken: done once [`Taken::lost`] returns.
pub struct Taken {
    lost: CancellationToken,
    reading: tokio::task::JoinHandle<()>,
}

impl Taken {
    /// Return once the connection has closed.
    pub async fn lost(&self) {
        self.lost.cancelled().await
    }
}

impl Drop for Taken {
    fn drop(&mut self) {
        self.reading.abort();
    }
}

/// How long a call waits for its reply before the link stops waiting. Generous: an ordinary call
/// answers in milliseconds and the slowest (an install that fetches a package) in seconds, so a
/// call past this is a daemon that stopped answering or a reader stuck behind a hub that stopped
/// reading — and from here the two look the same. Either way the caller is told the instance is
/// unavailable rather than left waiting for good. A turn is the exception ([`LASTS_A_TURN`]).
const CALL_TIMEOUT: Duration = Duration::from_secs(300);
/// Calls that last as long as the turn they run: no link-level timeout. A turn that ends is
/// reported on the event stream too, and a link that is lost drains them with the rest.
const LASTS_A_TURN: &[&str] = &[translate::SESSION_RUN_TURN];

type Pending = HashMap<u64, oneshot::Sender<Result<Value, RpcError>>>;

pub struct DaemonLink {
    wire: Arc<Wire>,
    ops: GatedOps,
    events_rx: StdMutex<Option<EventReceiver>>,
}

/// The connection's state, shared with the calls made on it.
struct Wire {
    carrier: InstanceCarrier,
    /// For the log: where the daemon is.
    label: String,
    writer: Mutex<Option<Writer>>,
    pending: StdMutex<Pending>,
    next_id: AtomicU64,
    events_tx: mpsc::Sender<EngineEvent>,
    status: watch::Sender<LinkStatus>,
    identity: StdMutex<EngineIdentity>,
    cancel: CancellationToken,
    call_timeout: Duration,
    /// A connection was taken before: the next one is a reconnection.
    connected_before: AtomicBool,
}

impl DaemonLink {
    /// A link that is `Starting` until it is given its first connection ([`DaemonLink::take`]).
    /// `label` says where the daemon is, for the log.
    pub fn new(carrier: InstanceCarrier, label: impl Into<String>) -> Arc<Self> {
        Self::with_call_timeout(carrier, label, CALL_TIMEOUT)
    }

    /// [`DaemonLink::new`], waiting `call_timeout` for a reply before calling the instance
    /// unavailable.
    pub fn with_call_timeout(carrier: InstanceCarrier, label: impl Into<String>, call_timeout: Duration) -> Arc<Self> {
        Self::build(carrier, label.into(), call_timeout, None)
    }

    /// [`DaemonLink::new`] for a daemon that may be let go while idle: `gate` wakes it before a
    /// call that needs it, and keeps its session list for while it sleeps.
    pub fn gated(carrier: InstanceCarrier, label: impl Into<String>, gate: Arc<dyn WakeGate>) -> Arc<Self> {
        Self::build(carrier, label.into(), CALL_TIMEOUT, Some(gate))
    }

    fn build(carrier: InstanceCarrier, label: String, call_timeout: Duration, gate: Option<Arc<dyn WakeGate>>) -> Arc<Self> {
        let (events_tx, events_rx) = mpsc::channel(EVENT_CAPACITY);
        let (status, _) = watch::channel(LinkStatus::Starting);
        let wire = Arc::new(Wire {
            carrier,
            label,
            writer: Mutex::new(None),
            pending: StdMutex::new(HashMap::new()),
            next_id: AtomicU64::new(1),
            events_tx,
            status,
            identity: StdMutex::new(EngineIdentity::default()),
            cancel: CancellationToken::new(),
            call_timeout,
            connected_before: AtomicBool::new(false),
        });
        let ops = GatedOps { agix: AgixOps(wire.clone()), gate };
        Arc::new(Self { ops, wire, events_rx: StdMutex::new(Some(events_rx)) })
    }


    /// What wakes it before a call, when it may be let go while idle ([`DaemonLink::gated`]).
    pub(crate) fn gate(&self) -> Option<Arc<dyn WakeGate>> {
        self.ops.gate.clone()
    }

    /// Take a connection to the daemon: present `token` when there is one, check that it speaks
    /// this build's protocol, subscribe to its events, and start reading. The link is `Ready` once
    /// this returns `Ok`; a connection it refuses is closed again.
    pub async fn take(&self, connection: Connection, token: Option<&str>) -> Result<Taken, Refusal> {
        let Connection { reader, writer } = connection;
        *self.wire.writer.lock().await = Some(writer);
        let lost = CancellationToken::new();
        let reading = tokio::spawn(self.wire.clone().read(reader, lost.clone()));
        let taken = Taken { lost, reading };
        if let Err(refusal) = self.wire.handshake(token).await {
            self.disconnect().await;
            return Err(refusal);
        }
        if self.wire.connected_before.swap(true, Ordering::Relaxed) {
            let _ = self.wire.events_tx.send(EngineEvent::Reconnected).await;
        }
        self.wire.status.send_replace(LinkStatus::Ready);
        tracing::info!(daemon = %self.wire.label, "daemon link ready");
        Ok(taken)
    }

    /// Let go of the connection: what is still waiting on a reply is told the instance is
    /// unavailable.
    pub async fn disconnect(&self) {
        self.wire.disconnect().await;
    }

    /// Lost, and being found again — unless the link is being shut down, whose `Down` stands.
    /// Checked under the status's own lock: `shutdown` cancels before it says `Down`, so a degrade
    /// that raced it either lands first and is overwritten, or sees the cancel.
    pub fn degrade(&self, reason: String) {
        let wire = &self.wire;
        wire.status.send_if_modified(|status| {
            if wire.cancel.is_cancelled() {
                return false;
            }
            let since_ms = match status {
                LinkStatus::Degraded { since_ms, .. } => *since_ms,
                _ => now_ms(),
            };
            *status = LinkStatus::Degraded { since_ms, reason };
            true
        });
    }

    /// Not coming back without someone changing something.
    pub fn down(&self, reason: String) {
        if self.wire.cancel.is_cancelled() {
            return;
        }
        tracing::error!(daemon = %self.wire.label, %reason, "daemon link down");
        self.wire.status.send_replace(LinkStatus::Down { reason });
    }

    /// Cancelled once the link is shut down: whoever keeps it connected stops then.
    pub fn stopping(&self) -> CancellationToken {
        self.wire.cancel.clone()
    }
}

impl translate::Raw for DaemonLink {
    fn rpc(&self) -> Arc<dyn Rpc> {
        self.wire.clone()
    }

    fn events(&self) -> Option<EventReceiver> {
        lock(&self.events_rx).take()
    }
}

impl Wire {
    async fn handshake(&self, token: Option<&str>) -> Result<(), Refusal> {
        if let Some(token) = token {
            self.call(translate::DAEMON_AUTH, json!({"token": token}))
                .await
                .map_err(|e| Refusal::Fatal(format!("the daemon refused the token: {}", e.message)))?;
        }
        let identity = self
            .call(translate::DAEMON_PING, json!({}))
            .await
            .map_err(|e| Refusal::Retry(format!("daemon.ping failed: {}", e.message)))?;
        let theirs = identity.get("protocol_version").and_then(Value::as_u64).unwrap_or(0);
        let ours = crate::protocol_version();
        if theirs != ours {
            return Err(Refusal::Fatal(format!(
                "the daemon speaks protocol version {theirs} and this Harnix speaks {ours}; \
                 run a daemon built from the same Agix"
            )));
        }
        *lock(&self.identity) = EngineIdentity::new(
            identity.get("version").and_then(Value::as_str).map(str::to_string),
            Some(theirs),
            identity.get("capabilities").cloned().unwrap_or(Value::Null),
        );
        self.call(translate::DAEMON_SUBSCRIBE_EVENTS, json!({}))
            .await
            .map_err(|e| Refusal::Retry(format!("daemon.subscribeEvents failed: {}", e.message)))?;
        Ok(())
    }

    /// Route every line: replies to their callers, everything else to the event stream, in the
    /// order read. A reply is therefore never handed over before the events the daemon wrote
    /// ahead of it.
    async fn read(self: Arc<Self>, reader: Reader, lost: CancellationToken) {
        let mut lines = BufReader::new(reader).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let Ok(frame) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            match frame.get("id").and_then(Value::as_u64) {
                Some(id) if frame.get("method").is_none() => {
                    let waiter = lock(&self.pending).remove(&id);
                    if let Some(waiter) = waiter {
                        let _ = waiter.send(reply(frame));
                    }
                }
                _ => {
                    let Some(event) = translate::event(frame) else { continue };
                    if self.events_tx.send(event).await.is_err() {
                        break;
                    }
                }
            }
        }
        lost.cancel();
    }

    async fn disconnect(&self) {
        *self.writer.lock().await = None;
        let waiting: Vec<_> = lock(&self.pending).drain().collect();
        let status = self.status.borrow().clone();
        for (_, waiter) in waiting {
            let _ = waiter.send(Err(status.unavailable()));
        }
    }

    async fn call(&self, method: &str, params: Value) -> Result<Value, RpcError> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        lock(&self.pending).insert(id, tx);
        let mut line = serde_json::to_string(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}))
            .map_err(|e| RpcError::invalid_params(e.to_string()))?;
        line.push('\n');
        let written = {
            let mut writer = self.writer.lock().await;
            match writer.as_mut() {
                Some(w) => w.write_all(line.as_bytes()).await.is_ok() && w.flush().await.is_ok(),
                None => false,
            }
        };
        if !written {
            lock(&self.pending).remove(&id);
            return Err(self.status.borrow().unavailable());
        }
        if LASTS_A_TURN.contains(&method) {
            return rx.await.unwrap_or_else(|_| Err(self.status.borrow().unavailable()));
        }
        match tokio::time::timeout(self.call_timeout, rx).await {
            Ok(reply) => reply.unwrap_or_else(|_| Err(self.status.borrow().unavailable())),
            Err(_) => {
                lock(&self.pending).remove(&id);
                tracing::warn!(daemon = %self.label, method, "the daemon did not answer in time");
                Err(RpcError::new(
                    codes::INSTANCE_UNAVAILABLE,
                    format!("the instance did not answer `{method}` within {}s", self.call_timeout.as_secs()),
                ))
            }
        }
    }
}

/// A call through the link, once it is ready.
#[async_trait::async_trait]
impl Rpc for Wire {
    async fn call(&self, method: &str, params: Value) -> Result<Value, RpcError> {
        let status = self.status.borrow().clone();
        if !status.is_ready() {
            return Err(status.unavailable());
        }
        Wire::call(self, method, params).await
    }
}

fn reply(frame: Value) -> Result<Value, RpcError> {
    if let Some(error) = frame.get("error") {
        return Err(RpcError {
            code: error.get("code").and_then(Value::as_i64).unwrap_or(-32603) as i32,
            message: error.get("message").and_then(Value::as_str).unwrap_or("").to_string(),
            data: error.get("data").cloned(),
        });
    }
    Ok(frame.get("result").cloned().unwrap_or(Value::Null))
}

#[async_trait::async_trait]
impl EngineControl for DaemonLink {
    fn status(&self) -> watch::Receiver<LinkStatus> {
        self.wire.status.subscribe()
    }

    fn identity(&self) -> EngineIdentity {
        lock(&self.wire.identity).clone()
    }

    fn carrier(&self) -> InstanceCarrier {
        self.wire.carrier
    }

    async fn reload_settings(&self) -> Result<(), RpcError> {
        Rpc::call(&*self.wire, translate::CONFIG_RELOAD, json!({})).await.map(|_| ())
    }

    async fn shutdown(&self) {
        self.wire.cancel.cancel();
        self.wire.disconnect().await;
        self.wire.status.send_replace(LinkStatus::Down { reason: "stopped".into() });
    }
}

#[async_trait::async_trait]
impl EngineLink for DaemonLink {
    fn ops(&self) -> &dyn EngineOps {
        &self.ops
    }

    fn take_events(&self) -> Option<EventReceiver> {
        translate::Raw::events(self)
    }

    /// Replies and events come off one socket in the order the daemon wrote them, and the reader
    /// hands events over before it resolves a later reply. A mark queued after a reply arrived is
    /// therefore behind that call's events already.
    async fn mark(&self, id: u64) {
        let _ = self.wire.events_tx.send(EngineEvent::Barrier { id }).await;
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::UnixListener;

    /// A daemon that answers `daemon.ping` with this build's protocol and anything else with
    /// `{}` — or holds a call, unanswered, when told to.
    struct Fake {
        lines: tokio::io::Lines<BufReader<tokio::net::unix::OwnedReadHalf>>,
        writer: tokio::net::unix::OwnedWriteHalf,
    }

    impl Fake {
        async fn accept(listener: &UnixListener) -> Self {
            let (stream, _) = listener.accept().await.unwrap();
            let (r, w) = stream.into_split();
            Self { lines: BufReader::new(r).lines(), writer: w }
        }

        async fn request(&mut self) -> Option<(u64, String)> {
            let line = self.lines.next_line().await.ok()??;
            let v: Value = serde_json::from_str(&line).unwrap();
            Some((v["id"].as_u64().unwrap(), v["method"].as_str().unwrap().to_string()))
        }

        async fn answer(&mut self, id: u64, result: Value) {
            let line = format!("{}\n", json!({"jsonrpc": "2.0", "id": id, "result": result}));
            self.writer.write_all(line.as_bytes()).await.unwrap();
        }

        /// Answer the handshake.
        async fn greet(&mut self) {
            for _ in 0..2 {
                let (id, method) = self.request().await.unwrap();
                let result = if method == translate::DAEMON_PING {
                    json!({"version": "x", "protocol_version": crate::protocol_version()})
                } else {
                    json!({})
                };
                self.answer(id, result).await;
            }
        }
    }

    async fn pair(dir: &std::path::Path, call_timeout: Duration) -> (Arc<DaemonLink>, UnixListener, PathBuf) {
        let socket = dir.join("d.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        (DaemonLink::with_call_timeout(InstanceCarrier::Attach, "test", call_timeout), listener, socket)
    }

    fn unavailable(r: &Result<Value, RpcError>) -> bool {
        matches!(r, Err(e) if e.code == codes::INSTANCE_UNAVAILABLE)
    }

    /// The second connection a link takes is a reconnection: the hub is told every subscription
    /// is gone.
    #[tokio::test]
    async fn a_second_connection_is_a_reconnection_the_hub_is_told_of() {
        let dir = tempfile::tempdir().unwrap();
        let (link, listener, socket) = pair(dir.path(), CALL_TIMEOUT).await;
        let mut events = link.take_events().unwrap();
        for round in 0..2 {
            let endpoint = Endpoint::Unix(socket.clone());
            let (taken, fake) = tokio::join!(
                async { link.take(Connection::open(&endpoint).await.unwrap(), None).await },
                async {
                    let mut fake = Fake::accept(&listener).await;
                    fake.greet().await;
                    fake
                }
            );
            let taken = taken.map_err(|r| format!("{r:?}")).unwrap();
            assert!(link.status().borrow().is_ready());
            assert_eq!(link.identity().version.as_deref(), Some("x"));
            if round == 1 {
                assert!(matches!(events.recv().await, Some(EngineEvent::Reconnected)));
            }
            drop(fake);
            taken.lost().await;
            link.disconnect().await;
            link.degrade("the connection closed".into());
        }
        assert!(events.try_recv().is_err(), "the first connection is not a reconnection");
    }

    #[tokio::test]
    async fn calls_in_flight_when_the_connection_drops_are_drained_as_unavailable() {
        let dir = tempfile::tempdir().unwrap();
        let (link, listener, socket) = pair(dir.path(), CALL_TIMEOUT).await;
        let endpoint = Endpoint::Unix(socket);
        let (taken, mut fake) = tokio::join!(
            async { link.take(Connection::open(&endpoint).await.unwrap(), None).await },
            async {
                let mut fake = Fake::accept(&listener).await;
                fake.greet().await;
                fake
            }
        );
        let taken = taken.map_err(|r| format!("{r:?}")).unwrap();
        let call = {
            let link = link.clone();
            tokio::spawn(async move { link.ops().passthrough("daemon.status", json!({})).await })
        };
        // Taken, and never answered.
        assert!(fake.request().await.is_some());
        drop(fake);
        taken.lost().await;
        link.disconnect().await;
        assert!(unavailable(&call.await.unwrap()));
    }

    #[tokio::test]
    async fn a_call_the_daemon_never_answers_is_answered_unavailable_after_the_timeout() {
        let dir = tempfile::tempdir().unwrap();
        let (link, listener, socket) = pair(dir.path(), Duration::from_millis(200)).await;
        let endpoint = Endpoint::Unix(socket);
        let (taken, mut fake) = tokio::join!(
            async { link.take(Connection::open(&endpoint).await.unwrap(), None).await },
            async {
                let mut fake = Fake::accept(&listener).await;
                fake.greet().await;
                fake
            }
        );
        let _taken = taken.map_err(|r| format!("{r:?}")).unwrap();
        let answer = link.ops().passthrough("daemon.status", json!({})).await;
        assert!(unavailable(&answer), "{answer:?}");
        assert!(fake.request().await.is_some(), "the call went out");
        assert!(link.status().borrow().is_ready(), "a slow answer is not a lost link");
    }

    #[tokio::test]
    async fn a_daemon_on_another_protocol_is_refused_for_good() {
        let dir = tempfile::tempdir().unwrap();
        let (link, listener, socket) = pair(dir.path(), CALL_TIMEOUT).await;
        let endpoint = Endpoint::Unix(socket);
        let (taken, ()) = tokio::join!(
            async { link.take(Connection::open(&endpoint).await.unwrap(), None).await },
            async {
                let mut fake = Fake::accept(&listener).await;
                let (id, _) = fake.request().await.unwrap();
                fake.answer(id, json!({"protocol_version": crate::protocol_version() + 1})).await;
            }
        );
        assert!(matches!(taken, Err(Refusal::Fatal(_))));
        assert!(!link.status().borrow().is_ready());
    }

    /// Shut down, a link stays down whatever its connection does after.
    #[tokio::test]
    async fn a_link_shut_down_stays_down() {
        let link = DaemonLink::new(InstanceCarrier::Attach, "test");
        link.shutdown().await;
        link.degrade("the connection closed".into());
        link.down("anything".into());
        assert_eq!(*link.status().borrow(), LinkStatus::Down { reason: "stopped".into() });
        assert!(link.stopping().is_cancelled());
    }
}
