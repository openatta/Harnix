//! The extension interface.
//!
//! An **extension** is one concept with three carriers: Rust linked into the product (native),
//! a package's WebAssembly in this process, or the same package in a process of its own. Each
//! says everything it provides once, in one [`ExtensionDescriptor`]: a native one by
//! implementing [`Extension`] and declaring through its [`Registrar`]; a package through the host
//! component that installs it, from its manifest. The extension host installs a descriptor the
//! same way whichever carrier made it: its methods and topics are rows of the one route table,
//! its spaces the hub's, its routes the transport's, its interface the shell's.
//!
//! Privilege is a [`Capability`], granted by the product where it adds a native extension, or
//! consented to by a person for a package. An extension reaches the process only through its
//! [`ExtensionContext`], as itself: the same admission point a device goes through, the same
//! audit, and every service of the context checks the capability it needs when it is used.
//!
//! What it never reaches: the hub, an engine link, the route table, the grants, a data root's
//! path, a carrier's pipes. A host component that needs the last of those asks for it as a
//! [`Service`] (`harnix-host-api`), which takes a capability of its own.
//!
//! **Stability.** Everything public here is the extension interface, and from 0.4.4 on it only
//! grows: every public struct and enum is `#[non_exhaustive]` or has private fields only, a type
//! in a signature is this crate's, the contract's or the engine interface's, and a feature adds
//! functions and modules, never a field or a variant. [`http::axum`] (feature `axum`) is outside
//! that promise: it is bound to axum's major version.

mod bulk;
mod call;
pub mod catalog;
mod context;
mod credentials;
mod descriptor;
mod extension;
pub mod http;
mod instances;
mod internal;
mod manifest;
mod registrar;
mod sessions;
mod topics;
pub mod ui;

/// The number the documentation and `--version` print for this interface. Not a compile-time
/// check — an assertion an author writes against it is always true — compatibility is the
/// crate's version: from 0.4.4 on this interface only grows.
pub const EXT_API_VERSION: u32 = 1;

pub use bulk::{Bulk, UploadGrant};
pub use call::{Call, Deadline};
pub use context::{ConfigError, Devices, EngineView, ExtensionContext, GrantsView, Listen, ProcessInfo, Service};
pub use credentials::{CredentialRef, Credentials, EnvCredentials, Secret};
pub use descriptor::{
    DescriptorBuilder, ExtensionDescriptor, MethodBuilder, MethodDecl, MethodHandler, RouteDecl, TopicBuilder,
    TopicCompute, TopicDecl,
};
pub use extension::{ErasedExtension, Extension, ExtensionSet, MigrateContext, MigrationReport};
pub use instances::Instances;
pub use manifest::Manifest;
pub use registrar::Registrar;
pub use sessions::{
    Attachment, Created, Entry, Exposure, OnBusy, SendOptions, SessionSpec, Sessions, Tap, TapEvent, SETTLED_HELD,
    TAP_CAPACITY,
};
pub use topics::{TopicStream, TopicValue, Topics};

/// Who calls, under which id, with which privileges, trusted how far, how far a row is open:
/// the contract's words, which are this interface's.
pub use harnix_contract::s1::{
    caps, codes, Access, CallMeta, Capability, CapabilitySpec, ChainLink, DefaultInstance, ExtId, IdError,
    InstanceTrust, PackageCarrier, RowScope, RpcError, SessionRef, SpaceDecl, Subject, Trust, CONTRIB_API_VERSION,
    MAIN_INSTANCE,
};
/// The largest frame a connection carries: what an answer has to fit in.
pub use harnix_contract::s1::MAX_FRAME_BYTES;

/// What every handler answers.
pub type Rpc<T> = Result<T, RpcError>;

/// What an extension gives an engine, in Harnix's words — tools, tool providers, hooks, scene
/// layers, prompt gates, instance templates — and the readers of what an engine writes down: the
/// engine interface (`harnix-engine-api`) whole, one list.
pub mod engine {
    pub use harnix_engine_api::*;
}

/// An internal error, with what went wrong.
fn internal_error(message: impl Into<String>) -> RpcError {
    RpcError::new(codes::INTERNAL_ERROR, message)
}

/// `REFUSED`, naming the capability the caller lacks.
fn lacks(cap: &Capability) -> RpcError {
    let mut e = RpcError::refused(format!("this needs the capability `{cap}`, which this extension was not granted"));
    e.data = Some(serde_json::json!({"reason": "capability", "capability": cap.as_str()}));
    e
}
