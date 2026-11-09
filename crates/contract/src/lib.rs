//! What the kernel's four parts hand each other, and nothing else.
//!
//! Assembly, hub, authorization and transport depend on this crate and never
//! on each other in the wrong direction. Putting the shared vocabulary here
//! rather than in whichever layer happened to define it first is what lets
//! `tests/layering.rs` state the rule as a fact about the dependency graph:
//! transport does not know what a session is, authorization does not know
//! what a frame looks like, the hub does not know what HTTP is.
//!
//! Nothing here executes anything. It is types, three traits and a version —
//! and [`sync`], the one way the kernel takes a lock.

pub mod apps;
pub mod frame;
pub mod handshake;
pub mod space;
pub mod subject;
pub mod sync;

pub use frame::{Channel, Frame, FrameSink};
pub use handshake::{
    Handshake, Limits, Topology, CONTRIB_API_VERSION, MAX_FRAME_BYTES, MAX_IN_FLIGHT,
    MAX_UPLOAD_BYTES, PROTOCOL_VERSION,
};
pub use space::{DefaultInstance, SpaceDecl, PROJECTS_SPACE, SESSIONS_SPACE, SYSTEM_SPACE};
pub use subject::Subject;

/// The instance every process has: the engine built from the profile's
/// `[engine]` (concept §3).
pub const SYSTEM_INSTANCE: &str = "system";

/// The device a client admitted by the per-process token alone is: on a loopback listener there
/// is no paired device to name, and every such client is the one person at this machine.
pub const LOCAL_DEVICE: &str = "local";

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A JSON-RPC error, in the shape Agix uses. Harnix adds no codes of its
/// own below the engine's range; see [`codes`].
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct RpcError {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional, type = "unknown"))]
    pub data: Option<Value>,
}

impl RpcError {
    pub fn new(code: i32, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            data: None,
        }
    }

    pub fn invalid_params(message: impl Into<String>) -> Self {
        Self::new(codes::INVALID_PARAMS, message)
    }

    /// `METHOD_NOT_FOUND`: there is no such *method* (or topic). For a thing a method names that
    /// does not exist, [`RpcError::missing`].
    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(codes::METHOD_NOT_FOUND, message)
    }

    /// `NOT_FOUND`: the method exists and what it was asked about does not.
    pub fn missing(message: impl Into<String>) -> Self {
        Self::new(codes::NOT_FOUND, message)
    }

    /// `CONFLICT`: what the call acts on is not in the state the call needs any more.
    pub fn conflict(message: impl Into<String>) -> Self {
        Self::new(codes::CONFLICT, message)
    }

    pub fn refused(message: impl Into<String>) -> Self {
        Self::new(codes::REFUSED, message)
    }
}

impl std::fmt::Display for RpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ({})", self.message, self.code)
    }
}

impl std::error::Error for RpcError {}

/// The codes Harnix itself returns.
///
/// # Two bands, and why they do not overlap
///
/// The five below are JSON-RPC's own predefined codes. They mean the same
/// thing whichever layer produced them — "your params are wrong" needs no
/// attribution — so both sides using them is correct rather than a clash.
///
/// Everything else is different. Agix's codes live in JSON-RPC's reserved
/// implementation band (`-32000` … `-32099`); **Harnix's live outside it**, at
/// `-31000`, which the specification leaves to applications. The boundary is
/// therefore a rule of the protocol rather than an agreement about who grows
/// which way, and that matters because the two collided once: `REFUSED` was
/// `-32000`, which is also Agix's `SESSION_NOT_FOUND`, so "you may not
/// call this" and "there is no such session" arrived as the same number — and
/// the API tests used that number as their only way to tell the host's
/// refusal from the engine's answer.
pub mod codes {
    pub const PARSE_ERROR: i32 = -32700;
    pub const INVALID_REQUEST: i32 = -32600;
    pub const METHOD_NOT_FOUND: i32 = -32601;
    pub const INVALID_PARAMS: i32 = -32602;
    pub const INTERNAL_ERROR: i32 = -32603;

    /// The method exists and the subject may not call it. Distinct from
    /// `METHOD_NOT_FOUND` on purpose: "you may not" and "there is no such
    /// thing" are different answers, and a client that cannot tell them apart
    /// reports the wrong bug.
    pub const REFUSED: i32 = -31000;
    /// The handshake did not agree — version, contribution API, or topology.
    pub const HANDSHAKE_REFUSED: i32 = -31001;
    /// The call named an instance this process does not have.
    pub const INSTANCE_NOT_FOUND: i32 = -31002;
    /// The session does not belong to the space (or instance) the caller
    /// claimed. A guard against a client's bug sending into the wrong
    /// product surface — not a security boundary: the claim is the caller's
    /// own (concept §5.4, §6.2).
    pub const SPACE_MISMATCH: i32 = -31003;
    /// The instance exists and cannot be reached right now: a daemon that is
    /// restarting, or one that refused this Harnix (protocol version). The
    /// instance's status in `harnix.instances` says which.
    pub const INSTANCE_UNAVAILABLE: i32 = -31004;
    /// This connection already has as many requests in flight as it may (`limits.max_in_flight`).
    /// Nothing was done; send it again once one of the others has answered. `data.max_in_flight`
    /// says the number.
    pub const BUSY: i32 = -31005;
    /// The method exists and the thing it was asked about does not: a session with no recording,
    /// a call past the end of one, a run, a workspace, a topic a module never declared. Distinct
    /// from `METHOD_NOT_FOUND`, which is about the method: a client that read "no recording yet"
    /// as "this server has no such method" drew the wrong page. `INSTANCE_NOT_FOUND` stays the
    /// answer for an instance.
    pub const NOT_FOUND: i32 = -31006;
    /// The thing the call acts on changed under it, and the call cannot apply now: a flow task's
    /// lease that is not the caller's any more, a run whose stored revision moved on. Read it
    /// again before trying again.
    pub const CONFLICT: i32 = -31007;
}

/// The process's single admission point.
///
/// Transport holds one of these and knows nothing about what is behind it.
/// Authorization implements it by deciding, then delegating to the hub.
#[async_trait::async_trait]
pub trait Gate: Send + Sync {
    async fn call(&self, subject: &Subject, method: &str, params: Value)
        -> Result<Value, RpcError>;
}

/// Who issued an upload grant and where it lands.
///
/// Transport serves the upload route and asks through this; a module that issues grants answers
/// it. Here rather than in either, because transport may not know what a module is and a module
/// may not reach transport: uploads belong to a contribution, not to the kernel.
#[async_trait::async_trait]
pub trait BulkStore: Send + Sync {
    /// Claim a grant: where the bytes go, and the ceiling that grant was
    /// issued under. `None` means never issued, or already spent.
    ///
    /// The ceiling travels with the grant rather than being one number for
    /// the route, because the two things that use this channel do not have
    /// the same limit — a file attachment and an extension package are
    /// different sizes of thing, and each is advertised at its own number by
    /// the method that issues it.
    async fn claim(&self, token: &str) -> Option<(std::path::PathBuf, usize)>;
}
