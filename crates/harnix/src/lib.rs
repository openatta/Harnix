//! Harnix — the crate an application depends on.
//!
//! ```no_run
//! use harnix::prelude::*;
//!
//! #[derive(Default)]
//! struct Hello;
//!
//! #[harnix::async_trait]
//! impl Module for Hello {
//!     const API_VERSION: u32 = harnix::MODULE_API_VERSION;
//!
//!     fn manifest(&self) -> Manifest {
//!         Manifest::new("hello", "0.1.0").disclosure("says hello")
//!     }
//!
//!     fn methods(&self, m: &mut MethodRegistry) {
//!         // app.hello.greet, for a paired device
//!         m.method("greet", Principal::Device)
//!             .handler(|_cx: CallContext, p: serde_json::Value| async move {
//!                 Ok(format!("hello, {}", p["name"].as_str().unwrap_or("you")))
//!             });
//!     }
//! }
//!
//! fn main() -> anyhow::Result<()> {
//!     harnix::App::builder()
//!         .name("hello-app", "0.1.0")
//!         .with_builtins()
//!         .module(Hello)
//!         .run()
//! }
//! ```
//!
//! What is here is a re-export: the application builder (`harnix-assembly`), the extension
//! interface ([`ext`]), the module contract (`harnix-module`, read as extensions while Harnix's own
//! move onto the interface), and what an extension gives the engine — tools, tool providers,
//! hooks, scene layers, prompt gates ([`engine`]).
//! An application depends on this crate alone. See `docs/harnix-sdk.md`.

/// What a module gives the engine, in Harnix's words: a tool (`engine::Tool`), a provider of tools
/// for every session (`engine::ToolProvider`), a hook before a call (`engine::ToolHook`), a scene
/// as a layer over another (`engine::SceneLayer`), a gate on messages (`engine::PromptGate`).
pub use harnix_engine_api as engine;

/// The extension interface: [`ext::Extension`], its registrar and descriptor, its context.
/// Stable within 0.5.z: it only grows.
pub use harnix_ext as ext;

/// Harnix's own extensions, each granted what Harnix grants it: the source is Harnix's own grant
/// list. A product adds the set (`App::builder().extensions(harnix::first_party::standard())`), or
/// the ones it wants, each by its type.
#[cfg(feature = "first-party")]
pub mod first_party {
    use harnix_contract::s1::caps;
    use harnix_ext::ExtensionSet;

    pub use harnix_ext_devices::Devices;
    pub use harnix_ext_harnix_app::HarnixApp;
    pub use harnix_ext_observability::Observability;
    pub use harnix_ext_plugins::Plugins;
    pub use harnix_ext_settings::Settings;
    pub use harnix_ext_system::System;

    #[cfg(feature = "flow")]
    pub use harnix_ext_flow::Flow;

    /// What the flow extension is granted: naming the sessions it opens, a main session's tasks.
    #[cfg(feature = "flow")]
    pub const FLOW_GRANTS: [harnix_contract::s1::Capability; 2] = [caps::ENGINE_CONTROL, caps::ENGINE_INSPECT];

    /// [`standard`], and `flow` as the product configured it.
    #[cfg(feature = "flow")]
    pub fn standard_with_flow(flow: Flow) -> ExtensionSet {
        standard().with(flow, FLOW_GRANTS)
    }

    /// Every first-party extension, in the order they register: the flow extension is the
    /// product's to configure and add ([`standard_with_flow`]).
    pub fn standard() -> ExtensionSet {
        ExtensionSet::new()
            .with(HarnixApp, [caps::TRANSCRIPTS_READ, caps::ENGINE_EXPORT])
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

/// Agix itself, for what [`engine`] does not say: its crates, a raw session customizer
/// (`agix::RawHooks::raw_hook`), an Agix scene as a layer's base (`agix::RawScenes::raw`).
/// **Not stable**: it changes with Agix, outside every promise this crate makes.
#[cfg(feature = "agix")]
pub mod agix {
    pub use harnix_engine::agix::*;
}

pub use async_trait::async_trait;
pub use axum;
pub use harnix_assembly::{App, AppBuilder, ExtensionSlot, ExtensionStatus, Namespace, HARNIX_VERSION, SHUTDOWN_GRACE};
pub use harnix_contract::s1::{caps, Capability, CapabilitySpec, ExtId};
pub use harnix_ext::{Extension, ExtensionSet, EXT_API_VERSION};
pub use harnix_contract::{codes, RpcError, Subject, SYSTEM_INSTANCE};
pub use harnix_module::{
    AppHandle, AppHandler, AppHost, AppMethod, AppSpec, AppSummary, AppTopic, AppWithdrawn, AttachTo, Attachment, SendOptions, CallContext, CredentialRef, Credentials, EngineContrib, EngineProfile, Entry,
    EnvCredentials, EventRegistry, Events, Handler, InstanceHandle, InstanceTemplate,
    InstanceTemplates, Instances, Listen, Manifest, MethodRegistry, MigrateContext,
    MigrationReport, Module, ModuleContext, OnBusy, Principal, RouteRegistry, Secret, SessionRef, SessionSpec,
    Sessions, SpaceDecl, SpaceRegistry, Tap, TapEvent, TemplateCarrier, CONTRIB_API_VERSION,
    MODULE_API_VERSION, SETTLED_HELD, TAP_CAPACITY,
};

/// Everything a module's source usually needs.
pub mod prelude {
    pub use crate::async_trait;
    pub use crate::{
        CallContext, CredentialRef, EngineContrib, EngineProfile, Entry, EventRegistry,
        InstanceTemplate,
        InstanceTemplates, Manifest, MethodRegistry, MigrateContext, MigrationReport, Module,
        ModuleContext, OnBusy, Principal, RouteRegistry, RpcError, SessionRef, SessionSpec,
        SpaceDecl, SpaceRegistry, TapEvent, TemplateCarrier,
    };
}
