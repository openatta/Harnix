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
pub use harnix_app::{App, AppBuilder, ModuleStatus, Namespace, HARNIX_VERSION};
pub use harnix_contract::{codes, RpcError, Subject, SYSTEM_INSTANCE};
pub use harnix_module::{
    CallContext, CredentialRef, Credentials, EngineContrib, EnvCredentials, EventRegistry, Events,
    Handler, InstanceHandle, Manifest, MethodRegistry, MigrateContext, MigrationReport, Module,
    ModuleContext, Principal, RouteRegistry, Secret, CONTRIB_API_VERSION, MODULE_API_VERSION,
};

/// Everything a module's source usually needs.
pub mod prelude {
    pub use crate::async_trait;
    pub use crate::{
        CallContext, CredentialRef, EngineContrib, EventRegistry, Manifest, MethodRegistry,
        MigrateContext, MigrationReport, Module, ModuleContext, Principal, RouteRegistry,
        RpcError,
    };
}
