//! What the kernel's four parts hand each other, and nothing else.
//!
//! Assembly, hub, authorization and transport depend on this crate and never
//! on each other in the wrong direction. Putting the shared vocabulary here
//! rather than in whichever layer happened to define it first is what lets
//! `tests/layering.rs` state the rule as a fact about the dependency graph:
//! transport does not know what a session is, authorization does not know
//! what a frame looks like, the hub does not know what HTTP is.
//!
//! Nothing here executes anything. It is types, two traits and a version.

pub mod frame;
pub mod handshake;
pub mod space;
pub mod subject;

pub use frame::{Channel, Frame, FrameSink};
pub use handshake::{
    Handshake, Limits, Topology, CONTRIB_API_VERSION, MAX_FRAME_BYTES, MAX_IN_FLIGHT,
    MAX_UPLOAD_BYTES, PROTOCOL_VERSION,
};
pub use space::{DefaultInstance, SpaceDecl, PROJECTS_SPACE, SESSIONS_SPACE, SYSTEM_SPACE};
pub use subject::Subject;

/// The instance every process has: the engine built from the profile's
/// `[engine]` (design §2).
pub const SYSTEM_INSTANCE: &str = "system";

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

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(codes::METHOD_NOT_FOUND, message)
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
    /// own (design §8.5, §14).
    pub const SPACE_MISMATCH: i32 = -31003;
    /// The instance exists and cannot be reached right now: a daemon that is
    /// restarting, or one that refused this Harnix (protocol version). The
    /// instance's status in `harnix.instances` says which.
    pub const INSTANCE_UNAVAILABLE: i32 = -31004;
    /// This connection already has as many requests in flight as it may (`limits.max_in_flight`).
    /// Nothing was done; send it again once one of the others has answered. `data.max_in_flight`
    /// says the number.
    pub const BUSY: i32 = -31005;
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
