//! Agix, behind Harnix's engine interface: the one crate that links the engine.
//!
//! The engine is Agix's, started through its embedding entry ([`Engine`]): in this process behind
//! an in-memory stream ([`InProcessEngine`]), or as `harnix daemon` on a socket
//! ([`serve_daemon`]). Nothing of Harnix's is built into it: what extensions give an engine —
//! tools, hooks around a tool call, gates on a message, written in `harnix-engine-api`'s terms —
//! is registered on the link to it and answered there ([`HostSide`]); a scene layer is one of the
//! engine's own scenes with what the hub creates its sessions with, its own tools answered on the
//! link that created them.
//!
//! It also carries the link the hub reaches an engine through — [`DaemonLink`], on a stream to a
//! daemon in another process or to an engine in this — and the [`ReplayLink`] that records or
//! replays what it creates, and the translation between Agix's protocol and the engine
//! interface's (`translate.rs`), the one place that names Agix's methods and events. The
//! instance layer builds the links; the hub only ever holds them as `EngineLink`s. What an engine
//! writes down — a session's recording and transcript — is read back through the same link
//! ([`DaemonLink::readers`]), by the engine's own protocol.

mod activity;
mod daemon_link;
mod embed;
mod gate;
mod host;
mod host_hooks;
mod host_tests;
mod host_tools;
mod permits;
mod legacy;
pub mod readers;
mod replay_link;
mod translate;

pub use daemon_link::{Connection, DaemonLink, Endpoint, Refusal, Taken};
pub use embed::{active_scenes, serve_daemon, Engine, EngineConfig, InProcessEngine};
pub use gate::WakeGate;
pub use host::{Chains, HostSide};
pub use legacy::strip_package_skills;
pub use replay_link::{Recorder, Recording, ReplayLink};
pub use translate::REQUIRED_CAPABILITIES;

/// The Agix this build links, as its manifest says (`build.rs`): for what asks before an engine
/// is up. A running engine says it itself, in `daemon.ping`.
pub const AGIX_VERSION: &str = env!("HARNIX_AGIX_VERSION");

/// The protocol that Agix's daemon speaks: what a daemon this links to must speak too, and what
/// `hello.engine_protocol` reports.
pub const AGIX_PROTOCOL: u64 = parse_u64(env!("HARNIX_AGIX_PROTOCOL"));

const fn parse_u64(s: &str) -> u64 {
    let bytes = s.as_bytes();
    let mut n = 0u64;
    let mut i = 0;
    while i < bytes.len() {
        n = n * 10 + (bytes[i] - b'0') as u64;
        i += 1;
    }
    n
}
