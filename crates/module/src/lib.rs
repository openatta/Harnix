//! The compile-time module contract (design §7).
//!
//! A **module** is source the application author links in: a space's
//! backend, a product's domain service, one of Harnix's own built-ins. It is
//! not a plugin — that word belongs to Agix's packages — and it is trusted
//! like the kernel, because it *is* part of the kernel's binary. What the
//! contract buys is not a sandbox but a shape: everything a module adds is
//! declared at registration (methods, topics, routes, native contributions to
//! the engine), so the authorization table, the diagnostics page and the
//! generated catalogs can all be read rather than inferred.
//!
//! A module never gets the hub's internals or an engine link. Its
//! [`ModuleContext`] calls through the same admission point a device does,
//! as `Subject::Module(id)`, so a module cannot go around the hub by accident
//! (design §14: a structural guard against mistakes).
//!
//! The interface's contribution points (`catalog`) live here too: the
//! front-end half of the same contract.

pub mod catalog;
mod context;
mod credentials;
mod events;
pub mod manifest;
mod methods;
mod routes;
mod sessions;

pub use catalog::{Frequency, Point, PointKind};
pub use context::{
    CallContext, InstanceHandle, Kernel, Listen, MigrateContext, MigrationReport, ModuleContext,
};
pub use credentials::{CredentialRef, Credentials, EnvCredentials};
pub use events::{EventRegistry, Events, TopicDecl};
pub use manifest::Manifest;
pub use methods::{Handler, MethodDecl, MethodRegistry};
pub use routes::{RouteDecl, RouteRegistry};
pub use sessions::{
    Entry, Instances, OnBusy, SessionRef, SessionSpec, Sessions, Tap, TapEvent, TapParts,
    SETTLED_HELD, TAP_CAPACITY,
};

pub use base::interface::credentials::Secret;
pub use harnix_instance::profile::EngineProfile;
pub use harnix_instance::templates::{AttachTo, InstanceTemplate, InstanceTemplates, TemplateCarrier};
pub use harnix_instance::EngineContrib;
pub use harnix_authz::Principal;
pub use harnix_contract::{DefaultInstance, RpcError, SpaceDecl, Subject};

/// What a module declares in `spaces()`.
#[derive(Default)]
pub struct SpaceRegistry {
    list: Vec<SpaceDecl>,
}

impl SpaceRegistry {
    pub fn add(&mut self, space: SpaceDecl) -> &mut Self {
        self.list.push(space);
        self
    }

    #[doc(hidden)]
    pub fn into_list(self) -> Vec<SpaceDecl> {
        self.list
    }
}

/// Bumped when the module contract changes in a way a module can observe.
///
/// A module states the version it is written against
/// ([`Module::API_VERSION`]) and the application builder checks it **at
/// compile time**, so a module written for another contract does not build
/// rather than misbehaving at runtime:
///
/// ```compile_fail
/// use harnix_module::{assert_api, Manifest, Module};
///
/// struct Old;
///
/// #[async_trait::async_trait]
/// impl Module for Old {
///     const API_VERSION: u32 = 0;
///     fn manifest(&self) -> Manifest {
///         Manifest::new("old", "0.1.0")
///     }
/// }
///
/// assert_api::<Old>();
/// ```
pub const MODULE_API_VERSION: u32 = 1;

/// One compile-time module. Every function but `manifest` has a default, so
/// a module implements only the parts it has.
///
/// Order of calls (design §7.3): the registration functions, once each, in
/// the order modules were added; then `migrate`; then — once the instances
/// exist — `start`. A module whose registration conflicts, whose `migrate`
/// fails or whose `start` fails is disabled with its methods, topics and
/// routes; everything else starts.
#[async_trait::async_trait]
pub trait Module: Send + Sync + 'static {
    /// The [`MODULE_API_VERSION`] this module is written against.
    const API_VERSION: u32;

    fn manifest(&self) -> Manifest;

    /// Native contributions to the `system` instance: scenes, native tools
    /// through a session customizer, a credential source.
    fn engine(&self, _engine: &mut EngineContrib) {}

    /// Templates this module builds its own instances from (design §5.2).
    /// Ids get the module's id as a prefix (`<id>.<template>`); an
    /// in-process template may carry native contributions of its own.
    fn templates(&self, _templates: &mut InstanceTemplates) {}

    /// The spaces this module is the backend of (design §8): where their new sessions land and in
    /// what scene. What they look like is the interface's.
    fn spaces(&self, _spaces: &mut SpaceRegistry) {}

    /// Methods, named `app.<id>.<name>` (built-ins: `harnix.<name>`).
    fn methods(&self, _methods: &mut MethodRegistry) {}

    /// Host topics this module publishes whole-value snapshots on.
    fn events(&self, _events: &mut EventRegistry) {}

    /// HTTP and WebSocket under `/apps/<id>/`.
    fn routes(&self, _routes: &mut RouteRegistry) {}

    /// Bring this module's own data up to date. Harnix decides when and what
    /// happens on failure; how is the module's.
    async fn migrate(&self, _cx: &MigrateContext) -> anyhow::Result<MigrationReport> {
        Ok(MigrationReport::none())
    }

    async fn start(&self, _cx: ModuleContext) -> anyhow::Result<()> {
        Ok(())
    }

    async fn stop(&self) {}
}

/// The compile-time check behind [`MODULE_API_VERSION`]. The application
/// builder calls it for every module it is given.
pub const fn assert_api<M: Module>() {
    const {
        assert!(
            M::API_VERSION == MODULE_API_VERSION,
            "this module is written against a different MODULE_API_VERSION than the harnix it is built with"
        )
    }
}

/// [`Module`] without the associated constant, so the application can hold
/// a list of different modules. Implemented for every `Module`.
#[doc(hidden)]
#[async_trait::async_trait]
pub trait ErasedModule: Send + Sync + 'static {
    fn manifest(&self) -> Manifest;
    fn engine(&self, engine: &mut EngineContrib);
    fn templates(&self, templates: &mut InstanceTemplates);
    fn spaces(&self, spaces: &mut SpaceRegistry);
    fn methods(&self, methods: &mut MethodRegistry);
    fn events(&self, events: &mut EventRegistry);
    fn routes(&self, routes: &mut RouteRegistry);
    async fn migrate(&self, cx: &MigrateContext) -> anyhow::Result<MigrationReport>;
    async fn start(&self, cx: ModuleContext) -> anyhow::Result<()>;
    async fn stop(&self);
}

#[async_trait::async_trait]
impl<M: Module> ErasedModule for M {
    fn manifest(&self) -> Manifest {
        Module::manifest(self)
    }
    fn engine(&self, engine: &mut EngineContrib) {
        Module::engine(self, engine)
    }
    fn templates(&self, templates: &mut InstanceTemplates) {
        Module::templates(self, templates)
    }
    fn spaces(&self, spaces: &mut SpaceRegistry) {
        Module::spaces(self, spaces)
    }
    fn methods(&self, methods: &mut MethodRegistry) {
        Module::methods(self, methods)
    }
    fn events(&self, events: &mut EventRegistry) {
        Module::events(self, events)
    }
    fn routes(&self, routes: &mut RouteRegistry) {
        Module::routes(self, routes)
    }
    async fn migrate(&self, cx: &MigrateContext) -> anyhow::Result<MigrationReport> {
        Module::migrate(self, cx).await
    }
    async fn start(&self, cx: ModuleContext) -> anyhow::Result<()> {
        Module::start(self, cx).await
    }
    async fn stop(&self) {
        Module::stop(self).await
    }
}

/// Bumped when a point's contract changes. Negotiated in the handshake
/// separately from the protocol version: an interface bundle can be current
/// on one and stale on the other.
pub use harnix_contract::handshake::CONTRIB_API_VERSION;
