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
//! What is here is a re-export: the application builder (`harnix-app`) and
//! the module contract (`harnix-module`). See `docs/harnix-sdk.md`.

pub use async_trait::async_trait;
pub use axum;
pub use harnix_app::{App, AppBuilder, ModuleStatus, Namespace, HARNIX_VERSION, SHUTDOWN_GRACE};
pub use harnix_contract::{codes, RpcError, Subject, SYSTEM_INSTANCE};
pub use harnix_module::{
    AttachTo, CallContext, CredentialRef, Credentials, EngineContrib, EngineProfile, Entry,
    EnvCredentials, EventRegistry, Events, Handler, InstanceHandle, InstanceTemplate,
    InstanceTemplates, Instances, Manifest, MethodRegistry, MigrateContext, MigrationReport,
    Module, ModuleContext, OnBusy, Principal, RouteRegistry, Secret, SessionRef, SessionSpec,
    Sessions, SpaceDecl, SpaceRegistry, Tap, TapEvent, TemplateCarrier, CONTRIB_API_VERSION,
    MODULE_API_VERSION, TAP_CAPACITY,
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
