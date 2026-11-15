//! An extension's declarations, as data: what [`Registrar`](crate::Registrar) gathers for a
//! native extension, and what the host component that installs packages builds for each one from
//! its manifest. One shape for both, and one host path.
//!
//! Every name here is a full name. The registrar puts an extension's own id in front of what it
//! is given; the [`DescriptorBuilder`] takes names as they are, and the host checks whose they
//! are when it installs them.

use std::future::Future;
use std::marker::PhantomData;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use harnix_contract::s1::{Access, ExtId, Trust};
use harnix_contract::SpaceDecl;
use harnix_engine_api::{EngineContribution, InstanceTemplate, ToolSpec};
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::Value;

use crate::call::Call;
use crate::context::ExtensionContext;
use crate::http::{Body, Request, Response, RouteContext, RouteHandler, StaticDir};
use crate::manifest::Manifest;
use crate::ui::UiDecl;
use crate::{internal_error, Rpc};

/// What answers one method. Usually made by [`MethodBuilder::handler`] from a typed function.
#[async_trait::async_trait]
pub trait MethodHandler: Send + Sync + 'static {
    async fn call(&self, call: Call, params: Value) -> Rpc<Value>;
}

/// What a computed topic's value is. Computed only while someone watches it: when they
/// subscribe, when [`Topics::changed`](crate::Topics::changed) says it may have changed, and
/// every so often if it was declared so.
#[async_trait::async_trait]
pub trait TopicCompute: Send + Sync + 'static {
    async fn compute(&self, ext: &ExtensionContext, params: &Value) -> Rpc<Value>;

    /// Someone subscribed to this parameter set; called before its value is computed for them.
    async fn on_subscribe(&self, _ext: &ExtensionContext, _params: &Value) {}
}

/// Where one extension's context is, once the host has seated it: shared by everything its
/// descriptor holds, so a handler finds its context however it was reached.
pub(crate) type Slot = Arc<OnceLock<ExtensionContext>>;

/// One method.
#[derive(Clone)]
pub struct MethodDecl {
    pub(crate) name: String,
    pub(crate) access: Access,
    pub(crate) schema: Option<Value>,
    pub(crate) description: String,
    pub(crate) handler: Arc<dyn MethodHandler>,
    pub(crate) slot: Slot,
}

impl MethodDecl {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn access(&self) -> &Access {
        &self.access
    }

    /// A JSON Schema for its params, when it gave one: listed, not enforced — its handler's own
    /// reading of the params is what holds them to a shape.
    pub fn schema(&self) -> Option<&Value> {
        self.schema.as_ref()
    }

    pub fn description(&self) -> &str {
        &self.description
    }
}

#[derive(Clone)]
pub(crate) enum TopicKind {
    Published,
    Computed { every: Option<Duration>, compute: Arc<dyn TopicCompute> },
}

/// One topic: whole values, never deltas, one per set of its parameters.
#[derive(Clone)]
pub struct TopicDecl {
    pub(crate) name: String,
    pub(crate) params: Vec<String>,
    pub(crate) access: Access,
    pub(crate) kind: TopicKind,
    pub(crate) slot: Slot,
}

impl TopicDecl {
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The subscription parameters it takes, by name.
    pub fn params(&self) -> &[String] {
        &self.params
    }

    pub fn access(&self) -> &Access {
        &self.access
    }

    /// Whether its value is computed when watched, rather than pushed by the extension.
    pub fn is_computed(&self) -> bool {
        matches!(self.kind, TopicKind::Computed { .. })
    }

    /// How often a computed one is computed again while watched, when it changes without saying.
    pub fn every(&self) -> Option<Duration> {
        match &self.kind {
            TopicKind::Computed { every, .. } => *every,
            TopicKind::Published => None,
        }
    }
}

/// One route, under `/<its id>/<prefix>`.
#[derive(Clone)]
pub struct RouteDecl {
    pub(crate) prefix: String,
    pub(crate) access: Access,
    pub(crate) handler: Arc<dyn RouteHandler>,
    /// A read-only directory rather than code of the extension's.
    pub(crate) static_dir: Option<PathBuf>,
    pub(crate) slot: Slot,
}

impl RouteDecl {
    pub fn prefix(&self) -> &str {
        &self.prefix
    }

    pub fn access(&self) -> &Access {
        &self.access
    }

    /// The directory it serves, when it is one.
    pub fn static_dir(&self) -> Option<&std::path::Path> {
        self.static_dir.as_deref()
    }

    /// Answer one request, after the transport admitted it and the router allowed it. Before the
    /// extension is seated, `503`.
    pub async fn handle(&self, req: Request<Body>) -> Response<Body> {
        match self.slot.get().cloned() {
            Some(ext) => self.handler.handle(RouteContext::new(ext, &self.prefix), req).await,
            None => crate::http::status(crate::http::StatusCode::SERVICE_UNAVAILABLE, "not served yet"),
        }
    }
}

/// Everything one extension provides, all of it data the host can read before anything runs.
#[derive(Clone)]
pub struct ExtensionDescriptor {
    pub(crate) id: ExtId,
    pub(crate) manifest: Manifest,
    pub(crate) trust: Trust,
    pub(crate) methods: Vec<MethodDecl>,
    pub(crate) topics: Vec<TopicDecl>,
    pub(crate) spaces: Vec<SpaceDecl>,
    pub(crate) tools: Vec<ToolSpec>,
    pub(crate) routes: Vec<RouteDecl>,
    pub(crate) ui: UiDecl,
    pub(crate) engine: Option<EngineContribution>,
    pub(crate) engine_everywhere: Option<EngineContribution>,
    pub(crate) templates: Vec<InstanceTemplate>,
    pub(crate) slot: Slot,
}

impl ExtensionDescriptor {
    pub fn id(&self) -> &ExtId {
        &self.id
    }

    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    pub fn trust(&self) -> &Trust {
        &self.trust
    }

    pub fn methods(&self) -> &[MethodDecl] {
        &self.methods
    }

    pub fn topics(&self) -> &[TopicDecl] {
        &self.topics
    }

    pub fn spaces(&self) -> &[SpaceDecl] {
        &self.spaces
    }

    /// The tools it gives sessions, as specs: what the diagnostics list. What runs is an engine
    /// contribution's (native) or its carrier's (a package).
    pub fn tools(&self) -> &[ToolSpec] {
        &self.tools
    }

    pub fn routes(&self) -> &[RouteDecl] {
        &self.routes
    }

    pub fn ui(&self) -> &UiDecl {
        &self.ui
    }

    /// What it gives the `main` instance's engine. Native extensions only.
    pub fn engine(&self) -> Option<&EngineContribution> {
        self.engine.as_ref()
    }

    /// What it gives every engine this process builds or spawns. Native extensions only.
    pub fn engine_everywhere(&self) -> Option<&EngineContribution> {
        self.engine_everywhere.as_ref()
    }

    /// Instance templates, by their full ids (`<its id>.<template>`). Native extensions only.
    pub fn templates(&self) -> &[InstanceTemplate] {
        &self.templates
    }

    /// Its context, once the host has seated it.
    pub fn context(&self) -> Option<ExtensionContext> {
        self.slot.get().cloned()
    }
}

/// Builds a descriptor from full names: what the registrar is a front for, and what a host
/// component uses for a package, from its manifest.
pub struct DescriptorBuilder {
    d: ExtensionDescriptor,
}

impl DescriptorBuilder {
    /// A descriptor for `id`, trusted as `trust`.
    pub fn new(id: ExtId, manifest: Manifest, trust: Trust) -> Self {
        Self {
            d: ExtensionDescriptor {
                id,
                manifest,
                trust,
                methods: Vec::new(),
                topics: Vec::new(),
                spaces: Vec::new(),
                tools: Vec::new(),
                routes: Vec::new(),
                ui: UiDecl::default(),
                engine: None,
                engine_everywhere: None,
                templates: Vec::new(),
                slot: Arc::new(OnceLock::new()),
            },
        }
    }

    /// A native extension's: trusted by origin.
    pub fn native(id: ExtId, manifest: Manifest) -> Self {
        Self::new(id, manifest, Trust::Origin)
    }

    pub fn id(&self) -> &ExtId {
        &self.d.id
    }

    /// The method `name`. It exists once a handler is given.
    pub fn method(&mut self, name: &str) -> MethodBuilder<'_> {
        MethodBuilder {
            name: name.to_string(),
            access: Access::kernel(),
            schema: None,
            description: String::new(),
            builder: self,
        }
    }

    /// The topic `name`: pushed by the extension, devices and every extension may subscribe,
    /// unless the builder says otherwise.
    pub fn topic(&mut self, name: &str) -> TopicBuilder<'_> {
        self.d.topics.push(TopicDecl {
            name: name.to_string(),
            params: Vec::new(),
            access: Access::device().with_extensions(),
            kind: TopicKind::Published,
            slot: self.d.slot.clone(),
        });
        TopicBuilder { decl: self.d.topics.last_mut().expect("just pushed") }
    }

    pub fn space(&mut self, decl: SpaceDecl) -> &mut Self {
        self.d.spaces.push(decl);
        self
    }

    pub fn tool(&mut self, spec: ToolSpec) -> &mut Self {
        self.d.tools.push(spec);
        self
    }

    pub fn route(&mut self, prefix: &str, access: Access, handler: impl RouteHandler) -> &mut Self {
        self.d.routes.push(RouteDecl {
            prefix: prefix.to_string(),
            access,
            handler: Arc::new(handler),
            static_dir: None,
            slot: self.d.slot.clone(),
        });
        self
    }

    /// Files under `dir`, read-only: no listing, nothing outside it.
    pub fn static_dir(&mut self, prefix: &str, dir: PathBuf, access: Access) -> &mut Self {
        self.d.routes.push(RouteDecl {
            prefix: prefix.to_string(),
            access,
            handler: Arc::new(StaticDir::new(dir.clone())),
            static_dir: Some(dir),
            slot: self.d.slot.clone(),
        });
        self
    }

    pub fn ui(&mut self) -> &mut UiDecl {
        &mut self.d.ui
    }

    pub fn engine(&mut self) -> &mut EngineContribution {
        self.d.engine.get_or_insert_with(EngineContribution::new)
    }

    pub fn engine_everywhere(&mut self) -> &mut EngineContribution {
        self.d.engine_everywhere.get_or_insert_with(EngineContribution::new)
    }

    /// A template, by its full id.
    pub fn template(&mut self, template: InstanceTemplate) -> &mut Self {
        self.d.templates.push(template);
        self
    }

    pub fn build(self) -> ExtensionDescriptor {
        self.d
    }
}

/// One method being declared: who may call it, what it takes, what answers it.
#[must_use = "a method exists once it is given a handler"]
pub struct MethodBuilder<'a> {
    builder: &'a mut DescriptorBuilder,
    name: String,
    access: Access,
    schema: Option<Value>,
    description: String,
}

impl MethodBuilder<'_> {
    /// Who may call it. Nobody but the kernel unless said.
    pub fn access(mut self, access: Access) -> Self {
        self.access = access;
        self
    }

    pub fn schema(mut self, schema: Value) -> Self {
        self.schema = Some(schema);
        self
    }

    pub fn describe(mut self, text: &str) -> Self {
        self.description = text.to_string();
        self
    }

    /// A typed handler: params read as `P` (one that does not read is `INVALID_PARAMS`), the
    /// result written from `R`.
    pub fn handler<P, R, F, Fut>(self, f: F)
    where
        P: DeserializeOwned + Send + 'static,
        R: Serialize + Send + 'static,
        F: Fn(Call, P) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Rpc<R>> + Send + 'static,
    {
        self.with(Arc::new(Typed { f, _types: PhantomData::<fn(P) -> R> }))
    }

    /// A handler on raw JSON.
    pub fn raw<F, Fut>(self, f: F)
    where
        F: Fn(Call, Value) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Rpc<Value>> + Send + 'static,
    {
        self.with(Arc::new(Raw(f)))
    }

    pub fn with(self, handler: Arc<dyn MethodHandler>) {
        let slot = self.builder.d.slot.clone();
        self.builder.d.methods.push(MethodDecl {
            name: self.name,
            access: self.access,
            schema: self.schema,
            description: self.description,
            handler,
            slot,
        });
    }
}

/// One topic being declared.
pub struct TopicBuilder<'a> {
    decl: &'a mut TopicDecl,
}

impl TopicBuilder<'_> {
    /// A subscription parameter: each set of values is a value of its own.
    pub fn param(self, name: &str) -> Self {
        self.decl.params.push(name.to_string());
        self
    }

    /// Who may subscribe. Devices and every extension unless said.
    pub fn access(self, access: Access) -> Self {
        self.decl.access = access;
        self
    }

    /// The extension pushes whole values ([`Topics::publish`](crate::Topics::publish)). The default.
    pub fn published(self) -> Self {
        self.decl.kind = TopicKind::Published;
        self
    }

    /// Computed when someone watches it, and pushed when it changed: again every `every`, and
    /// whenever the extension says it may have changed.
    pub fn computed(self, every: Option<Duration>, compute: impl TopicCompute) -> Self {
        self.decl.kind = TopicKind::Computed { every, compute: Arc::new(compute) };
        self
    }
}

struct Typed<F, P, R> {
    f: F,
    _types: PhantomData<fn(P) -> R>,
}

#[async_trait::async_trait]
impl<P, R, F, Fut> MethodHandler for Typed<F, P, R>
where
    P: DeserializeOwned + Send + 'static,
    R: Serialize + Send + 'static,
    F: Fn(Call, P) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Rpc<R>> + Send + 'static,
{
    async fn call(&self, call: Call, params: Value) -> Rpc<Value> {
        let params: P = serde_json::from_value(params).map_err(|e| crate::RpcError::invalid_params(e.to_string()))?;
        let result = (self.f)(call, params).await?;
        serde_json::to_value(result).map_err(|e| internal_error(e.to_string()))
    }
}

struct Raw<F>(F);

#[async_trait::async_trait]
impl<F, Fut> MethodHandler for Raw<F>
where
    F: Fn(Call, Value) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Rpc<Value>> + Send + 'static,
{
    async fn call(&self, call: Call, params: Value) -> Rpc<Value> {
        (self.0)(call, params).await
    }
}
