//! What an instance is to the kernel: the link its engine is reached through.
//!
//! The kernel's interface, not an extension's: a link is handed to the hub, the one holder of
//! sessions, and to nobody else — an extension that could start a turn on a link would be a way
//! around the hub's "subscribe before anyone" invariant.
//!
//! A link is the [`EngineLink`]: what the engine is asked ([`EngineOps`]), the one stream of what
//! it says unasked ([`EngineEvent`]), an ordering mark, and the link's health. Whoever built it
//! keeps only its [`EngineControl`] half — how it is, and how to stop it — and hands the whole
//! link over once.

use harnix_contract::{codes, RpcError};
use serde::Serialize;
use serde_json::Value;
use tokio::sync::{mpsc, watch};

use crate::events::EngineEvent;
use crate::ops::EngineOps;

/// How many events a link holds for the hub before it stops reading. A link that stops reading
/// makes the engine's own outbound queue fill, and the engine then drops that subscription and
/// says so ([`EngineEvent::SubscriptionDropped`]) — the hub resyncs. So a slow hub costs a
/// resync, never memory.
pub const EVENT_CAPACITY: usize = 4096;

/// Every event a link carries, in the order the engine sent them.
pub type EventReceiver = mpsc::Receiver<EngineEvent>;

#[non_exhaustive]
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

    /// The answer to a call on a link in this state, which cannot reach its engine.
    pub fn unavailable(&self) -> RpcError {
        let why = match self {
            LinkStatus::Starting => "is starting".to_string(),
            LinkStatus::Ready => "lost its engine mid-call".to_string(),
            LinkStatus::Degraded { reason, .. } => format!("is reconnecting ({reason})"),
            LinkStatus::Down { reason } => format!("is down ({reason})"),
        };
        RpcError::new(codes::INSTANCE_UNAVAILABLE, format!("the instance {why}"))
    }
}

/// How an instance's engine is carried, for the instance list. Reported, never branched on: what
/// differs between carriers is the carrier's own doing, in the layer that built the link.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InstanceCarrier {
    /// Built in this process.
    InProcess,
    /// A daemon this process started.
    Spawn,
    /// A daemon somebody else runs.
    Attach,
    /// An engine in an extension's own process, reached over the stream that process was given.
    Adopted,
}

/// What the engine says it is. Empty until the link has reached it once.
#[non_exhaustive]
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct EngineIdentity {
    /// The engine's version.
    pub version: Option<String>,
    /// The protocol it speaks: what `hello.engine_protocol` reports.
    pub protocol_version: Option<u64>,
    /// What it says it can do, as it says it.
    pub capabilities: Value,
}

impl EngineIdentity {
    pub fn new(version: Option<String>, protocol_version: Option<u64>, capabilities: Value) -> Self {
        Self { version, protocol_version, capabilities }
    }
}

/// The half of a link its builder keeps: how it is, and how to stop it. It cannot reach a
/// session: that is the hub's, through the whole [`EngineLink`].
#[async_trait::async_trait]
pub trait EngineControl: Send + Sync + 'static {
    fn status(&self) -> watch::Receiver<LinkStatus>;

    fn identity(&self) -> EngineIdentity;

    fn carrier(&self) -> InstanceCarrier;

    /// The process carrying the engine, when it is not this one and this one started it.
    fn pid(&self) -> Option<u32> {
        None
    }

    /// Whether stopping or restarting the process carrying the engine also ends what is left in
    /// its session (ipdrun-07).
    fn reaps_on_stop(&self) -> bool {
        false
    }

    /// Have the engine read its settings again: what its builder does after writing them.
    async fn reload_settings(&self) -> Result<(), RpcError>;

    async fn shutdown(&self);
}

/// An instance's engine, as the hub reaches it. Two carriers implement it — an engine in this
/// process and one in another — and the hub has no branch on which one it holds.
#[async_trait::async_trait]
pub trait EngineLink: EngineControl {
    /// What the engine is asked: one method each, so the hub spells none of the engine's names.
    fn ops(&self) -> &dyn EngineOps;

    /// The event stream. There is one: the first caller gets it, and it stays the same receiver
    /// across reconnects. The hub's alone to take.
    fn take_events(&self) -> Option<EventReceiver>;

    /// Put an [`EngineEvent::Barrier`] carrying `id` behind every event this link had received
    /// when the call was made, and return once it is queued.
    ///
    /// Settling a turn waits for its mark to come out of the event stream: a call's reply can
    /// overtake that call's last events, and a watcher that saw the turn settled before it
    /// completed drew the turn twice.
    async fn mark(&self, id: u64);
}
