//! Runtime applications embedded in this process: installing them, and their tools in every
//! session — the glue between the application host and the rest of Harnix.
//!
//! An application is a package (`.hxapp`, [`package`]; everything about packages is
//! `harnix-app-package`, re-exported here). It runs in a `harnix_app_host::Host`, one VM per
//! application in this process, which this crate drives through the host's `Control` and lends
//! the process through its `Platform` (`platform.rs`: the kernel's application host, Agix's
//! instances). [`Runtime`] is what the `apps` built-in module (`harnix_builtin::apps`) drives to
//! install, start, stop and remove one; the rest of the process reaches what is running through
//! three things it hands out:
//!
//! - [`AppsPlugin`], an Agix plugin for every in-process instance's assembly: the running
//!   applications' tools in each session built, and their `events` as a middleware on its tool
//!   calls.
//! - [`serve::router`], for the listener's `/apps/<id>/…`: the interface — modules for the page,
//!   pages for the isolated frames ([`surface`]) — and the MCP endpoint a spawned daemon reaches
//!   the tools through (it runs no VM of its own: one application, one VM, in this process).
//! - [`Runtime::ui_contributions`] and [`Runtime::mcp_servers`], re-read after every change
//!   ([`Runtime::on_change`]): what the page is told at hello, and what spawned daemons are
//!   configured with.
//!
//! Only a build with the `apps` feature links this crate, and through it the host and
//! `harnix-wasm`.

mod agent;
mod platform;
mod plugin;
mod runtime;
pub mod serve;
mod update;

pub use harnix_app_package::{
    acquire, disclosure, inline, manifest, package, publish, signature, store, surface,
};
pub use plugin::{AppsPlugin, NAME as PLUGIN_NAME};
pub use runtime::{server_name, tool_name, Loaded, Options, Registry, Runtime, TOOL_PATTERN};
