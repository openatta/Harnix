//! Agix, behind Harnix's engine interface: the one crate that links the engine.
//!
//! What extensions give an engine is written in `harnix-engine-api`'s terms — tools, providers,
//! hooks, scene layers, prompt gates, a credential source. This crate turns each into Agix's own
//! (`adapt`), and stands the engine up in this process
//! ([`build`]). The instance layer builds engines with it and keeps the links to them.

mod adapt;
pub mod engine;
mod hooks;
pub mod readers;
pub mod scenes;
pub mod settings;
pub mod timeline;

#[cfg(feature = "agix")]
pub mod agix;

pub use engine::{active_scenes, build, Engine, EngineConfig};
pub use hooks::RawHook;
