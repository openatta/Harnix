//! The instance layer: engines, and the links the hub reaches them through
//! (concept §3.1.1).
//!
//! An instance is one Agix engine — one session pool with its own data root,
//! settings, providers, MCP connections, plugins and scenes. This crate
//! builds one in this process ([`engine`]), or spawns or attaches to one in
//! another ([`Supervised`]), and hands the hub an `EngineLink` either way: the
//! engine's own links, which this layer stands up and keeps connected.
//! It also holds where instances come from: the profile, and templates
//! ([`templates`]) with the ledger that remembers what was built from them.
//!
//! This layer does not know the client-facing protocol exists, nor what the
//! engine's methods are called. It depends on the engine and on neither
//! transport nor authorization —
//! `crates/harnix/tests/layering.rs` states that as a fact about the
//! dependency graph rather than as a paragraph here.
//!
//! # Extensions are not assembled here
//!
//! Harnix has no extension subsystem. Scripts and plugins are Agix's:
//! their manifest, their disclosure, their capability gate, their carrier,
//! their lifecycle. Harnix neither reads a package nor runs one — it hands
//! Agix a file and calls Agix's own install (see
//! `harnix_builtin::plugins`). Anything here that read a manifest would be a
//! second truth about what an extension may do.

// A daemon's unix socket, the session it leads (`setsid`), the `flock` on its data root: the
// instance layer is built on unix, and says so once, here, rather than in fallbacks for a platform
// nothing builds it for.
#[cfg(not(unix))]
compile_error!("harnix-instance builds on unix only (Linux and macOS are the ones exercised)");

pub mod data_lock;
mod daemon;
mod defaults;
mod factory;
mod inherit;
pub mod link;
pub mod profile;
mod reap;
pub mod registry;
mod serve;
mod servers;
pub mod templates;

pub use daemon::{DaemonSpec, SpawnOptions, SpawnSpec, Supervised};
/// The engine itself, started in this process: `harnix-engine`'s.
pub use harnix_engine::{Endpoint, Engine, EngineConfig, Recording};
pub use factory::{Built, Facts, Factory};
pub use link::DownLink;
pub use profile::{EngineProfile, InstanceDecl, InstanceSections, TemplateDecl};
pub use registry::{InstanceFacts, InstanceRegistry, InstanceRow, RegistryCell, RegistryConfig, INSTANCES_TOPIC};
pub use serve::{refusal_path, serve_daemon, DaemonArgs, REFUSED_EXIT};
pub use templates::{
    AttachTo, InstanceTemplate, InstanceTemplates, Ledger, LedgerEntry, Owner, TemplateCarrier,
};
