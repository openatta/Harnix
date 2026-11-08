//! Runtime applications: the package, installing one, and its tools in every session.
//!
//! An application is a package (`.hxapp`, [`package`]): a manifest ([`manifest`]), an optional
//! WebAssembly component that gives the agent tools, an optional Agix package (`agent/`: skills,
//! agent types, QuickJS scripts, MCP servers, a scene) and optional interface modules (`ui/`).
//! [`Runtime`] is what the `apps` built-in module (`harnix_builtin::apps`) drives to install,
//! start, stop and remove one; the rest of the process reaches what is running through three
//! things it hands out:
//!
//! - [`AppsPlugin`], an Agix plugin for every in-process instance's assembly: the running
//!   applications' tools in each session built, and their `events` as a middleware on its tool
//!   calls.
//! - [`serve::router`], for the listener's `/apps/<id>/…`: the interface modules, and the MCP
//!   endpoint a spawned daemon reaches the tools through (it runs no VM of its own: one
//!   application, one VM, in this process).
//! - [`Runtime::ui_contributions`] and [`Runtime::mcp_servers`], re-read after every change
//!   ([`Runtime::on_change`]): what the page is told at hello, and what spawned daemons are
//!   configured with.
//!
//! Where a package comes from and how far it is trusted: [`signature`] (who signed it, over
//! which bytes), [`store`] (the catalog, and downloading from it), [`disclosure`] (what a person is
//! shown and consents to), [`publish`] (the publisher's and the store's tools).
//!
//! Only a build with the `apps` feature links this crate. The component runs in `harnix-wasm`.

mod acquire;
mod backend;
pub mod disclosure;
mod host;
pub mod manifest;
pub mod package;
mod plugin;
pub mod publish;
mod runtime;
pub mod serve;
pub mod signature;
pub mod store;

pub use plugin::{AppsPlugin, NAME as PLUGIN_NAME};
pub use runtime::{server_name, tool_name, Loaded, Options, Registry, Runtime, TOOL_PATTERN};
