//! Agix, behind Harnix's engine interface: the one crate that links the engine.
//!
//! What extensions give an engine is written in `harnix-engine-api`'s terms — tools, providers,
//! hooks, scene layers, prompt gates. This crate turns each into Agix's own
//! (`adapt`), and stands the engine up in this process
//! ([`build`]) or serves it as a daemon ([`serve_daemon`]).
//!
//! It also carries the link the hub reaches an engine through — [`DaemonLink`], on a stream to a
//! daemon in another process or on an in-memory one to an engine in this ([`InProcessEngine`]),
//! and the [`ReplayLink`] that records or replays what it creates — and the translation between Agix's protocol and the
//! engine interface's (`translate.rs`), the one place that names Agix's methods and events. The
//! instance layer builds the links; the hub only ever holds them as `EngineLink`s.

mod adapt;
pub mod assemble;
mod daemon_link;
pub mod engine;
mod gate;
mod hooks;
pub mod readers;
mod replay_link;
pub mod scenes;
mod serve;
mod translate;

pub use daemon_link::{Connection, DaemonLink, Endpoint, Refusal, Taken};
pub use gate::WakeGate;
pub use engine::{active_scenes, build, Chains, Engine, EngineConfig};
pub use assemble::InProcessEngine;
pub use replay_link::{Recorder, Recording, ReplayLink};
pub use serve::serve_daemon;

/// Where an engine keeps its transcripts, for readers that would otherwise page the protocol to
/// get at text.
pub use history::store::HistoryStore;

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
