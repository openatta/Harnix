//! Harnix — the crate an application depends on.
//!
//! ```no_run
//! use harnix::prelude::*;
//!
//! #[derive(Default)]
//! struct Hello;
//!
//! #[harnix::async_trait]
//! impl Extension for Hello {
//!     type Config = ();
//!
//!     fn manifest(&self) -> Manifest {
//!         Manifest::new("hello", "0.1.0").disclosure("says hello")
//!     }
//!
//!     fn register(&self, r: &mut Registrar<'_, ()>) {
//!         // hello.greet, for a paired device
//!         r.method("greet").access(Access::device()).raw(|_call, p| async move {
//!             Ok(serde_json::json!(format!("hello, {}", p["name"].as_str().unwrap_or("you"))))
//!         });
//!     }
//! }
//!
//! fn main() -> anyhow::Result<()> {
//!     harnix::App::builder()
//!         .name("hello-app", "0.1.0")
//!         .extensions(harnix::first_party::standard())
//!         .extension(Hello)
//!         .ungranted()
//!         .run()
//! }
//! ```
//!
//! What is here is a re-export: the application builder (`harnix-assembly`), the extension
//! interface ([`ext`]), Harnix's own extensions ([`first_party`]), and what an extension gives the
//! engine — tools, tool providers, hooks, scene layers, prompt gates ([`engine`]).
//! An application depends on this crate alone, and this crate on the assembly and the extension
//! interface alone: everything at its root is one of those two re-exported, so the extension
//! interface has one source (`crates/harnix/tests/layering.rs`, l24 and c19). See
//! `docs/harnix-ext.md`.

/// What an extension gives the engine, in Harnix's words: a tool (`engine::Tool`), a provider of
/// tools for every session (`engine::ToolProvider`), a hook before a call (`engine::ToolHook`), a
/// scene as a layer over another (`engine::SceneLayer`), a gate on messages (`engine::PromptGate`),
/// and the readers of what the engine writes down. The same list as [`ext::engine`]: the engine
/// interface crate whole, with nothing of the skeleton's half in it.
pub use harnix_ext::engine;

/// The extension interface: [`ext::Extension`], its registrar and descriptor, its context.
/// Stable from 0.4.4 on: it only grows.
pub use harnix_ext as ext;

/// Harnix's own extensions, each granted what Harnix grants it: the source is Harnix's own grant
/// list. A product adds the set (`App::builder().extensions(harnix::first_party::standard())`), or
/// the ones it wants, each by its type.
#[cfg(feature = "first-party")]
pub mod first_party {
    use harnix_ext::{caps, ExtensionSet};

    pub use harnix_ext_devices::Devices;
    pub use harnix_ext_harnix_app::HarnixApp;
    pub use harnix_ext_observability::Observability;
    pub use harnix_ext_plugins::Plugins;
    pub use harnix_ext_settings::Settings;
    pub use harnix_ext_system::System;

    #[cfg(feature = "flow")]
    pub use harnix_ext_flow::Flow;

    #[cfg(any(feature = "wasm", feature = "proc"))]
    pub use harnix_ext_packages::Packages;

    /// What the `packages` component is granted: registering packages as extensions, installing
    /// their agent packages.
    #[cfg(any(feature = "wasm", feature = "proc"))]
    pub const PACKAGES_GRANTS: [harnix_ext::Capability; 2] = [caps::EXTENSIONS_MANAGE, caps::ENGINE_PLUGINS];

    /// The `packages` component, on the carriers this build links: in this process (`wasm`, mode
    /// 2), in processes of their own (`proc`, mode 3), or both, the profile choosing.
    #[cfg(any(feature = "wasm", feature = "proc"))]
    pub fn packages() -> Packages {
        let packages = Packages::new();
        #[cfg(feature = "wasm")]
        let packages = packages.carrier(harnix_carrier_wasm::carrier());
        #[cfg(feature = "proc")]
        let packages = packages.carrier(harnix_carrier_proc::carrier(harnix_proc::this_build()));
        packages
    }

    /// [`standard`], and `packages` as the product configured it.
    #[cfg(any(feature = "wasm", feature = "proc"))]
    pub fn standard_with(packages: Packages) -> ExtensionSet {
        standard().with(packages, PACKAGES_GRANTS)
    }

    /// What the flow extension is granted: naming the sessions it opens, a main session's tasks.
    #[cfg(feature = "flow")]
    pub const FLOW_GRANTS: [harnix_ext::Capability; 2] = [caps::ENGINE_CONTROL, caps::ENGINE_INSPECT];

    /// [`standard`], and `flow` as the product configured it.
    #[cfg(feature = "flow")]
    pub fn standard_with_flow(flow: Flow) -> ExtensionSet {
        standard().with(flow, FLOW_GRANTS)
    }

    /// Every first-party extension, in the order they register: the flow extension is the
    /// product's to configure and add ([`standard_with_flow`]).
    pub fn standard() -> ExtensionSet {
        ExtensionSet::new()
            .with(HarnixApp, [caps::TRANSCRIPTS_READ, caps::ENGINE_EXPORT, caps::ENGINE_CONTROL])
            .with(
                System,
                [caps::INSTANCES_ADMIN, caps::ENGINE_INSPECT, caps::ENGINE_CONTROL, caps::PROCESS_OBSERVE],
            )
            .with(Observability, [caps::RECORDINGS_READ])
            .with(Settings, [caps::ENGINE_CONFIG_READ, caps::ENGINE_CONFIG_WRITE])
            .with(Plugins::default(), [caps::ENGINE_PLUGINS])
            .with(Devices, [caps::DEVICES_MANAGE])
    }
}

/// A product's route table without starting it ([`testing::describe`]), and a process started
/// without its listener ([`testing::Kernel`]): the real router and hub, an engine that replays.
/// **Not stable**.
#[cfg(feature = "testing")]
pub mod testing {
    pub use harnix_assembly::testing::{describe, Kernel};
}

pub use async_trait::async_trait;

/// The axum crate [`ext::http::axum::router`] takes its router from, at the version it is bound to.
/// With the `axum` feature; **not stable**, like that module.
#[cfg(feature = "axum")]
pub use axum;

/// The application: [`App::builder`].
pub struct App;

impl App {
    /// The builder of [`harnix_assembly::App`], with the `ext-proc` subcommand registered in a
    /// build with the `proc` feature: a product that runs packages in their own process is
    /// re-executed as `<product> ext-proc`, and needs no second binary or line in `main`.
    pub fn builder() -> AppBuilder {
        let builder = harnix_assembly::App::builder();
        #[cfg(feature = "proc")]
        let builder = builder.child_entry("ext-proc", harnix_proc::entry);
        builder
    }
}
pub use harnix_assembly::{AppBuilder, ExtensionSlot, ExtensionStatus, HARNIX_VERSION, SHUTDOWN_GRACE};
pub use harnix_ext::{
    caps, codes, Access, Attachment, Call, Capability, CapabilitySpec, CredentialRef, Credentials, Entry,
    EnvCredentials, ExtId, Extension, ExtensionContext, ExtensionSet, Manifest, MigrateContext, MigrationReport,
    OnBusy, Registrar, RowScope, RpcError, Secret, SendOptions, SessionRef, SessionSpec, SpaceDecl, Subject, Tap,
    TapEvent, CONTRIB_API_VERSION, EXT_API_VERSION, MAIN_INSTANCE, SETTLED_HELD, TAP_CAPACITY,
};

/// Everything an extension's source usually needs.
pub mod prelude {
    pub use crate::async_trait;
    pub use crate::engine::{EngineProfile, InstanceTemplate};
    pub use crate::{
        Access, Call, CredentialRef, Entry, Extension, ExtensionContext, Manifest, MigrateContext, MigrationReport,
        OnBusy, Registrar, RowScope, RpcError, SessionRef, SessionSpec, SpaceDecl, TapEvent,
    };
}
