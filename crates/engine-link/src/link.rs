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

use harnix_contract::RpcError;
use serde::Serialize;
use serde_json::Value;
use tokio::sync::{mpsc, watch};

use crate::events::EngineEvent;
use crate::ops::EngineOps;
use crate::scene::SceneLayers;

/// How many events a link holds for the hub before it stops reading. A link that stops reading
/// makes the engine's own outbound queue fill, and the engine then drops that subscription and
/// says so ([`EngineEvent::SubscriptionDropped`]) — the hub resyncs. So a slow hub costs a
/// resync, never memory.
pub const EVENT_CAPACITY: usize = 4096;

/// Every event a link carries, in the order the engine sent them.
pub type EventReceiver = mpsc::Receiver<EngineEvent>;

/// How a link is and how an instance's engine is carried: the wire's words for them, declared with
/// the rest of the kernel's wire shapes (`harnix_contract::wire`) and said here under their old
/// names.
pub use harnix_contract::wire::{InstanceCarrier, LinkStatus};

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
    /// The modules its build has, as it says them: what a method that answers "not found" would
    /// need.
    pub modules: Vec<String>,
}

impl EngineIdentity {
    pub fn new(version: Option<String>, protocol_version: Option<u64>, capabilities: Value) -> Self {
        Self { version, protocol_version, capabilities, modules: Vec::new() }
    }

    pub fn modules(mut self, modules: Vec<String>) -> Self {
        self.modules = modules;
        self
    }
}

/// One hook a link registered on its connection (`daemon.setHooks`).
#[non_exhaustive]
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HostHook {
    /// The engine's event.
    pub event: String,
    /// The tool patterns it applies to; empty for an event about no tool.
    pub tools: Vec<String>,
    /// `allow` or `block`: what a failure to answer does.
    pub on_error: String,
    /// The providers whose hooks it asks, by name: before a tool call.
    pub providers: Vec<String>,
}

impl HostHook {
    pub fn new(event: impl Into<String>, tools: Vec<String>, on_error: impl Into<String>) -> Self {
        Self { event: event.into(), tools, on_error: on_error.into(), providers: Vec::new() }
    }

    pub fn providers(mut self, providers: Vec<String>) -> Self {
        self.providers = providers;
        self
    }
}

/// The half of a link its builder keeps: how it is, and how to stop it. It cannot reach a
/// session: that is the hub's, through the whole [`EngineLink`].
#[async_trait::async_trait]
pub trait EngineControl: Send + Sync + 'static {
    fn status(&self) -> watch::Receiver<LinkStatus>;

    fn identity(&self) -> EngineIdentity;

    fn carrier(&self) -> InstanceCarrier;

    /// Whether the engine runs on this machine, so that a path of this one's is a path of its.
    /// An engine somebody else runs may be anywhere.
    fn on_this_machine(&self) -> bool {
        self.carrier() != InstanceCarrier::Attach
    }

    /// The process carrying the engine, when it is not this one and this one started it.
    fn pid(&self) -> Option<u32> {
        None
    }

    /// Whether stopping or restarting the process carrying the engine also ends what is left in
    /// its session (ipdrun-07).
    fn reaps_on_stop(&self) -> bool {
        false
    }

    /// The scene layers its sessions may be created in, expanded: the hub sets a session up from
    /// one, and the link answers that session's own tools. None by default.
    fn scene_layers(&self) -> SceneLayers {
        SceneLayers::default()
    }

    /// The hooks the link registered on its connection, as of its last registration: what this
    /// process answers in every session of the engine. None by default.
    fn host_hooks(&self) -> Vec<HostHook> {
        Vec::new()
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
