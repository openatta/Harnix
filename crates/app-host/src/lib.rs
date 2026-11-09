//! Runs runtime applications: the supervisor between a product and its applications' VMs.
//!
//! [`Host`] keeps what is running — one VM per application (`harnix-wasm`), all on one engine —
//! and answers [`harnix_app_port::Control`]: putting an application's agent side in place,
//! starting and stopping it, its health and calls, its methods (checked against their schemas,
//! then queued for the resident backend), its tools (an instance per call, cancellable, with
//! progress), its event hook, and the values of what its backend subscribed to. An on-demand
//! backend hibernates while idle, and the next call starts it again (`idle.rs`). Everything it
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
mod idle;
mod placement;

pub use bridge::Stats;
pub use host::{Host, HostOptions};
/// The engine every VM of a host runs on (`Host::engine`).
pub use harnix_wasm::WasmEngine;

/// What an application keeps under `app_dir` — its files under `data/` and its key-value store
/// under `kv/` — measured as its quota measures its files: symlinks not followed, and `None` past
/// as many entries as no quota allows. For an application that is not running; one that is
/// reports it in its health (`storage_bytes`), counted as it writes.
pub fn data_bytes(app_dir: &std::path::Path) -> Option<u64> {
    Some(harnix_wasm::dir_bytes(&app_dir.join("data"))? + harnix_wasm::dir_bytes(&app_dir.join("kv"))?)
}

#[cfg(test)]
mod tests {
    #[test]
    fn what_an_application_keeps_is_its_data_and_its_store_and_nothing_else() {
        let dir = tempfile::tempdir().unwrap();
        let app = dir.path();
        std::fs::create_dir_all(app.join("data/sub")).unwrap();
        std::fs::create_dir_all(app.join("kv")).unwrap();
        std::fs::create_dir_all(app.join("1.0.0")).unwrap();
        std::fs::write(app.join("data/sub/a"), [0u8; 10]).unwrap();
        std::fs::write(app.join("kv/b"), [0u8; 5]).unwrap();
        std::fs::write(app.join("1.0.0/package-file"), [0u8; 100]).unwrap();
        std::fs::write(app.join("install.json"), [0u8; 50]).unwrap();
        assert_eq!(super::data_bytes(app), Some(15));
        assert_eq!(super::data_bytes(&app.join("absent")), Some(0));
    }
}
