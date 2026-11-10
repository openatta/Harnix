//! Agix, standing up in this process: the engine an in-process instance is.
//!
//! This is the one place that assembles Agix — resolving scenes, wrapping them (attended, admitting
//! the host's tools), gathering what modules contribute, and handing it all to Agix's own
//! assembly. The instance layer builds an engine with [`build`] and keeps the links to it.

#[macro_use]
mod delegate;
mod admit;
mod attended;
pub mod contrib;
pub mod engine;

pub use contrib::EngineContrib;
pub use engine::{active_scenes, build, Engine, EngineConfig};
