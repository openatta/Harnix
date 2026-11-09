//! Runtime applications embedded in this process: installing them, and their tools in every
//! session — the glue between the application host and the rest of Harnix.
//!
//! An application is a package (`.hxapp`, [`package`]; everything about packages is
//! `harnix-app-package`, re-exported here). It runs in a `harnix_app_host::Host`, one VM per
//! application in this process, which this crate drives through the host's `Control` and lends
//! the process through its `Kernel` (`platform.rs`: the kernel's application host) and its
//! `Agent` (`agent.rs`: Agix's instances). [`Runtime`] is what the `apps` built-in module
//! (`harnix_builtin::apps`) drives to install, start, stop and remove one; the rest of the process
//! reaches what is running through three things it hands out:
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
//!
//! # Which side each file is on
//!
//! Embedded, the product and the application's VM are one process, and this crate is the glue
//! between them. In an edition with a process per application they are two, and its files fall on
//! either side of the link (`docs/app-development.md` §9):
//!
//! | file | side | what it is |
//! |---|---|---|
//! | `runtime.rs`, `update.rs` | product | installing, starting, updating, removing: unpack, `Control::place`, `Control::start`, the rollback; what is running, for the rest |
//! | `serve.rs` | product | `/apps/<id>/ui` (the frames' pages, under their CSP) and `/apps/<id>/mcp` (the tools, for a spawned daemon) |
//! | `platform.rs` | product | the port's `Kernel` over this process's kernel: registering, calling, publishing and subscribing as `module:<id>` |
//! | `agent.rs` | VM | the port's `Agent`: the agent package, skills and tools into this process's Agix — the application process's own Agix there |
//! | `plugin.rs` | VM | `AppsPlugin`, the tools and events in the sessions of the Agix beside the VM |
//!
//! The `Control` this crate drives, `harnix_app_host::Host`, is the VM side's too: called in place
//! here, over the link there.

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
