//! Runs runtime applications: the supervisor between a product and its applications' VMs.
//!
//! [`Host`] keeps what is running — one VM per application (`harnix-wasm`), all on one engine —
//! and answers [`harnix_app_port::Control`]: putting an application's agent side in place,
//! starting and stopping it, its health and calls, its methods (checked against their schemas,
//! then queued for the resident backend), its tools (an instance per call, cancellable, with
//! progress), its event hook, and the values of what its backend subscribed to. Everything it
//! needs from outside is [`harnix_app_port::Platform`]: registering the application's names, what
//! it does as itself (`host.call`, `publish`, `subscribe`), the agent engine its tools and agent
//! package go to.
//!
//! It reads packages with `harnix-app-package` and nothing else of the product: no kernel, no
//! hub, no agent engine, no HTTP framework. So the same host runs embedded in a product's process
//! (`harnix-apps` lends it a platform over the kernel) or in a process of its own, with the port
//! carried over a link; its tests run it through `harnix_app_port::rpc::Loopback`.
//!
//! An application's own HTTP (`host.http-request`, already checked against its `net`) goes out
//! from wherever the host runs ([`http`]).

mod bridge;
mod host;
pub mod http;
mod placement;

pub use bridge::Stats;
pub use host::{Host, HostOptions};
/// The engine every VM of a host runs on (`Host::engine`).
pub use harnix_wasm::WasmEngine;
