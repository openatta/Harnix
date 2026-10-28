//! The instance layer: engines, and the links the hub reaches them through
//! (design §5).
//!
//! An instance is one Agix engine — one session pool with its own data root,
//! settings, providers, MCP connections, plugins and scenes. This crate
//! builds one in this process ([`engine`]), or spawns or attaches to one in
//! another ([`DaemonLink`]), and hands the hub an [`EngineLink`] either way.
//! It also holds where instances come from: the profile, and templates
//! ([`templates`]) with the ledger that remembers what was built from them.
//!
//! This layer does not know the client-facing protocol exists. It depends on
//! the engine and on neither transport nor authorization —
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

pub mod contrib;
pub mod data_lock;
mod daemon_link;
pub mod engine;
mod factory;
mod inproc;
pub mod link;
pub mod profile;
mod serve;
pub mod templates;

pub use contrib::EngineContrib;
pub use daemon_link::{DaemonLink, DaemonSpec, Endpoint, SpawnSpec};
pub use engine::{build as build_engine, Engine, EngineConfig};
pub use factory::{engine_config, Built, Facts, Factory};
pub use inproc::InProcessLink;
pub use link::{Carrier, DownLink, EngineLink, LinkStatus};
pub use profile::{EngineProfile, InstanceDecl, Profile, TemplateDecl, TransportProfile, UiProfile};
pub use serve::{serve_daemon, DaemonArgs};
pub use templates::{
    AttachTo, InstanceTemplate, InstanceTemplates, Ledger, LedgerEntry, Owner, TemplateCarrier,
};
