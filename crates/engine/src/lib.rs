//! Agix, behind Harnix's engine interface: the one crate that links the engine.
//!
//! What extensions give an engine is written in `harnix-engine-api`'s terms — tools, providers,
//! hooks, scene layers, prompt gates, a credential source. This crate turns each into Agix's own
//! (`adapt`), and stands the engine up in this process
//! ([`build`]) or serves it as a daemon ([`serve_daemon`]).
//!
//! It also carries the links the hub reaches an engine through — [`InProcessLink`] for one in
//! this process, [`DaemonLink`] for one at the other end of a stream, and the [`ReplayLink`] that
//! records or replays what either creates — and the translation between Agix's protocol and the
//! engine interface's (`translate.rs`), the one place that names Agix's methods and events. The
//! instance layer builds the links; the hub only ever holds them as `EngineLink`s.

mod adapt;
mod daemon_link;
pub mod engine;
mod hooks;
mod inproc;
pub mod readers;
mod replay_link;
pub mod scenes;
mod serve;
pub mod settings;
pub mod timeline;
mod translate;

#[cfg(feature = "agix")]
pub mod agix;

pub use daemon_link::{Connection, DaemonLink, Endpoint, Refusal, Taken};
pub use engine::{active_scenes, build, Engine, EngineConfig};
pub use hooks::RawHook;
pub use inproc::InProcessLink;
pub use replay_link::{Recorder, Recording, ReplayLink};
pub use serve::serve_daemon;

/// Where an engine keeps its transcripts, for readers that would otherwise page the protocol to
/// get at text.
pub use history::store::HistoryStore;

use harnix_engine_api::EngineIdentity;

/// What the engine this build links says it is: the version, and the protocol a daemon of it
/// speaks — the one `hello.engine_protocol` reports.
pub fn identity() -> EngineIdentity {
    let about = about();
    EngineIdentity::new(
        about.get("version").and_then(serde_json::Value::as_str).map(str::to_string),
        Some(protocol_version()),
        about.get("capabilities").cloned().unwrap_or(serde_json::Value::Null),
    )
}

/// The protocol a daemon of the engine this build links speaks: what a daemon this links to must
/// speak too.
pub fn protocol_version() -> u64 {
    about().get("protocol_version").and_then(serde_json::Value::as_u64).unwrap_or(0)
}

/// Everything the engine says about itself, as it says it: for a diagnostics page.
pub fn about() -> serde_json::Value {
    daemon::discovery::identity()
}
