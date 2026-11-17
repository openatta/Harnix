//! Flows in Harnix: the extension (`Flow`) that runs the flow library's runs in sessions, with
//! its programs, native tools, methods and topics. The library itself — definitions, run records,
//! the kernel, stores, packs, dry runs — is `harnix-flow`, re-exported here whole, so
//! `harnix_ext_flow::Engine` and `harnix_ext_flow::kernel::…` are the library's.
//!
//! An optional crate: an application that does not add it gets nothing different.
//!
//! ```text
//! wire.rs             the module's own params (`flow.start`)
//! module/             the Harnix extension: Flow, FlowHandle
//! ```

pub use harnix_flow::*;

pub mod module;
pub mod wire;

pub use module::{builtin_scenes, Flow, FlowBuilder, FlowHandle};
