//! The hub's only view of an instance (design §5.3).
//!
//! Two carriers implement [`EngineLink`] — an engine in this process
//! ([`crate::InProcessLink`]) and an Agix daemon in another
//! ([`crate::DaemonLink`]) — and the hub has no branch on which one it holds.
//! Everything the hub needs is here: calls, the one stream of asynchronous
//! frames, an ordering mark, and the link's health.
//!
//! This is the kernel's interface. Modules are never handed one: a module
//! that could call `session.run_turn` on a link would be a way around the
//! hub's "subscribe before anyone" invariant.

use harnix_contract::{codes, RpcError};
use serde::Serialize;
use serde_json::{json, Value};
use tokio::sync::{mpsc, watch};

/// Every asynchronous frame a link carries, in the order the engine sent
/// them: `session.event`, `daemon.event`, and the link's own two —
/// [`BARRIER`] and [`RECONNECTED`].
pub type EventReceiver = mpsc::Receiver<Value>;

/// How many frames a link holds for the hub before it stops reading. A link
/// that stops reading makes the engine's own outbound queue fill, and the
/// engine then drops that subscription and says so (`subscription_dropped`) —
/// the hub resyncs. So a slow hub costs a resync, never memory.
pub const EVENT_CAPACITY: usize = 4096;

/// A mark written behind every frame the link had received when
/// [`EngineLink::mark`] was called.
pub const BARRIER: &str = "harnix.barrier";

/// Written by a link that lost its engine and found it again. Every
/// subscription it had is gone: the engine's subscriptions belong to the
/// connection (Agix protocol §5.6).
pub const RECONNECTED: &str = "harnix.reconnected";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum LinkStatus {
    Starting,
    Ready,
    /// Lost, and being found again. Turns on the far side keep running.
    Degraded { since_ms: u64, reason: String },
    /// Not coming back without someone changing something.
    Down { reason: String },
}

impl LinkStatus {
    pub fn is_ready(&self) -> bool {
        matches!(self, LinkStatus::Ready)
    }
}

/// Which carrier, for the instance list. The hub never branches on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Carrier {
    InProcess,
    Spawn,
    Attach,
}

#[async_trait::async_trait]
pub trait EngineLink: Send + Sync + 'static {
    /// One JSON-RPC call. A streaming method's frames arrive on the event
    /// stream; this returns its final answer.
    async fn call(&self, method: &str, params: Value) -> Result<Value, RpcError>;

    /// The event stream. There is one: the first caller gets it, and it
    /// stays the same receiver across reconnects.
    fn take_events(&self) -> Option<EventReceiver>;

    /// Put a [`BARRIER`] frame carrying `id` behind every frame this link
    /// had received when the call was made, and return once it is queued.
    ///
    /// Settling a turn waits for its mark to come out of the event stream:
    /// a call's reply can overtake that call's last events, and a watcher
    /// that saw `turn_settled` before `turn_complete` drew the turn twice.
    async fn mark(&self, id: u64);

    fn status(&self) -> watch::Receiver<LinkStatus>;

    /// What the engine says it is: `{version, protocol_version, capabilities}`.
    /// `Null` until the link has reached it once.
    fn identity(&self) -> Value;

    fn carrier(&self) -> Carrier;

    /// The process carrying the engine, when it is not this one and this
    /// one started it.
    fn pid(&self) -> Option<u32> {
        None
    }

    async fn shutdown(&self);
}

/// The frame [`EngineLink::mark`] writes.
pub fn barrier_frame(id: u64) -> Value {
    json!({"jsonrpc": "2.0", "method": BARRIER, "params": {"id": id}})
}

pub fn reconnected_frame() -> Value {
    json!({"jsonrpc": "2.0", "method": RECONNECTED, "params": {}})
}

/// The answer to a call on a link that cannot reach its engine.
pub fn unavailable(status: &LinkStatus) -> RpcError {
    let why = match status {
        LinkStatus::Starting => "is starting".to_string(),
        LinkStatus::Ready => "lost its engine mid-call".to_string(),
        LinkStatus::Degraded { reason, .. } => format!("is reconnecting ({reason})"),
        LinkStatus::Down { reason } => format!("is down ({reason})"),
    };
    RpcError::new(codes::INSTANCE_UNAVAILABLE, format!("the instance {why}"))
}

/// The protocol version this build of Harnix speaks to an Agix daemon: the
/// one of the Agix it links.
pub fn expected_protocol() -> u64 {
    daemon::discovery::identity()
        .get("protocol_version")
        .and_then(Value::as_u64)
        .unwrap_or(0)
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// An instance that could not be built: a ledger entry whose template is
/// gone or whose owner is disabled, or a carrier that failed to start. It is
/// listed `Down` with the reason, so the system space can say what happened
/// to it; its data root is not touched.
pub struct DownLink {
    status: watch::Sender<LinkStatus>,
    carrier: Carrier,
}

impl DownLink {
    pub fn new(carrier: Carrier, reason: String) -> std::sync::Arc<Self> {
        let (status, _) = watch::channel(LinkStatus::Down { reason });
        std::sync::Arc::new(Self { status, carrier })
    }
}

#[async_trait::async_trait]
impl EngineLink for DownLink {
    async fn call(&self, _method: &str, _params: Value) -> Result<Value, RpcError> {
        Err(unavailable(&self.status.borrow()))
    }
    fn take_events(&self) -> Option<EventReceiver> {
        None
    }
    async fn mark(&self, _id: u64) {}
    fn status(&self) -> watch::Receiver<LinkStatus> {
        self.status.subscribe()
    }
    fn identity(&self) -> Value {
        Value::Null
    }
    fn carrier(&self) -> Carrier {
        self.carrier
    }
    async fn shutdown(&self) {}
}
