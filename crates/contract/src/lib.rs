//! What the kernel's four parts hand each other, and nothing else.
//!
//! Assembly, hub, authorization and transport depend on this crate and never
//! on each other in the wrong direction. Putting the shared vocabulary here
//! rather than in whichever layer happened to define it first is what lets
//! `tests/layering.rs` state the rule as a fact about the dependency graph:
//! transport does not know what a session is, authorization does not know
//! what a frame looks like, the hub does not know what HTTP is.
//!
//! Nothing here executes anything. It is types, two traits and a version —
//! and [`sync`], the one way the kernel takes a lock.

pub mod dir_lock;
pub mod ext_id;
pub mod frame;
pub mod handshake;
pub mod s1;
pub mod skills;
pub mod sync;
pub mod wire;

pub use frame::{Channel, Frame, FrameSink};
pub use handshake::{
    Handshake, Limits, Topology, CONTRIB_API_VERSION, MAX_FRAME_BYTES, MAX_IN_FLIGHT,
    MAX_UPLOAD_BYTES, PROTOCOL_VERSION,
};
pub use s1::product::{self, ProductNames};
pub use s1::{codes, ChainLink, DefaultInstance, ExtId, RpcError, SessionRef, SpaceDecl, Subject};
pub use skills::SkillSource;

/// The instance every process has: the engine built from the profile's
/// `[engine]` (concept §3).
pub const MAIN_INSTANCE: &str = "main";

/// The device a client admitted by the per-process token alone is: on a loopback listener there
/// is no paired device to name, and every such client is the one person at this machine.
pub const LOCAL_DEVICE: &str = "local";

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
