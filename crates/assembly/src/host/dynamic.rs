//! What the host does for a host component: registering extensions while the process runs,
//! configuring spawned daemons' tool endpoints, telling the page what it runs, and — with the
//! process carrier — starting and adopting extension processes.
//!
//! A runtime registration takes the same host path a native extension's does: a descriptor,
//! seated, its rows in the one table. What the host checks of it is its structure (a package's
//! consent is the component's record, which the host never sees); everything after that is the
//! packages' table's ([`crate::package_table`]) — its names, its topics, its spaces, its consent,
//! in one write, undone on any refusal — and its routes go where the transport finds the
//! packages' paths ([`PackageRoutes`]).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, RwLock};

use harnix_contract::s1::{caps, CallMeta, Capability, ExtId, PackageCarrier, Trust};
use harnix_contract::sync::{lock, read, write};
use harnix_contract::{codes, RpcError};
use harnix_ext::{ExtensionDescriptor, RouteDecl};
use harnix_host_api::{
    AdoptSpec, AdoptedEngine, Availability, ChildHandle, ChildRequest, ChildSandbox, DynamicRegistrar, DynamicRegistry,
    ExtensionSummary, HostApi, RegisteredExtension, TopicSink, ToolEndpoint, Withdrawn,
};
use harnix_instance::SharedServers;
use harnix_kernel::seat::Seated;
use harnix_kernel::{CallContext, Chain, Handler, Rpc};
use harnix_router::names::{full_name, valid_package_id};
use harnix_router::{Row, Router};
use serde_json::{json, Value};

use super::seat::Seats;
use crate::kernel_rows::UiModules;
use crate::package_table::{Package, PackageTable, PackageTopic};

/// The prefixes a carrier serves a package under: its interface's files, and its tool endpoint.
pub(crate) const CARRIER_ROUTES: &[&str] = &["ui", "mcp"];

/// The host's side of every host component's pipes.
pub(crate) struct DynamicHost {
    pub packages: Arc<PackageTable>,
    pub router: Arc<Router>,
    pub seats: Arc<Seats>,
    pub servers: Arc<SharedServers>,
    /// Where a registered extension's directory is made.
    pub state_root: std::path::PathBuf,
    /// What `harnix.ui` tells; `false` when `[ui] runtime_modules` keeps packages' modules out.
    pub ui: Arc<UiModules>,
    pub runtime_modules: bool,
    /// The profile's `[ext."<package id>"]` sections.
    pub sections: BTreeMap<String, Value>,
    pub routes: Arc<PackageRoutes>,
    /// The extension processes started, each bound to the stream it was handed.
    pub children: Arc<harnix_authz::ChildBindings>,
    /// The components whose section turns their processes' sandbox off.
    pub unsandboxed: BTreeSet<String>,
    /// Where an extension process's own engine is adopted.
    pub instances: Arc<harnix_instance::InstanceRegistry>,
    /// What each component shares with every spawned daemon, merged.
    shared: Mutex<BTreeMap<ExtId, Vec<ToolEndpoint>>>,
}

/// What [`DynamicHost`] is made of besides the packages' table.
pub(crate) struct DynamicParts {
    pub router: Arc<Router>,
    pub instances: Arc<harnix_instance::InstanceRegistry>,
    pub seats: Arc<Seats>,
    pub servers: Arc<SharedServers>,
    pub state_root: std::path::PathBuf,
    pub ui: Arc<UiModules>,
    pub runtime_modules: bool,
    pub sections: BTreeMap<String, Value>,
    pub unsandboxed: BTreeSet<String>,
}

impl DynamicHost {
    pub fn new(packages: Arc<PackageTable>, parts: DynamicParts) -> Arc<Self> {
        let DynamicParts { router, instances, seats, servers, state_root, ui, runtime_modules, sections, unsandboxed } = parts;
        Arc::new(Self {
            packages,
            router,
            seats,
            servers,
            state_root,
            ui,
            runtime_modules,
            sections,
            routes: Arc::default(),
            children: Arc::default(),
            unsandboxed,
            instances,
            shared: Mutex::new(BTreeMap::new()),
        })
    }
}

/// The packages' routes, by id: what the transport hands `/ext/<id>/…` for an id no native
/// extension took. A package's are its carrier's — its interface's files, served as the page's
/// own are, to frames that hold no credential; its tool endpoint, which admits only an engine this
/// process spawned — and a directory of its files, which a device reaches.
#[derive(Default)]
pub(crate) struct PackageRoutes(RwLock<BTreeMap<String, Vec<RouteDecl>>>);

impl PackageRoutes {
    fn set(&self, id: &str, routes: Vec<RouteDecl>) {
        let mut all = write(&self.0);
        match routes.is_empty() {
            true => all.remove(id),
            false => all.insert(id.to_string(), routes),
        };
    }

    /// The router the transport mounts under `/ext/`: each request to the route of the package
    /// its path names.
    pub fn router(self: &Arc<Self>) -> axum::Router {
        let routes = self.clone();
        axum::Router::new().fallback(move |req: axum::extract::Request| {
            let routes = routes.clone();
            async move { routes.serve(req).await }
        })
    }

    async fn serve(&self, mut req: axum::extract::Request) -> axum::response::Response {
        let not_found = || {
            let mut response = axum::response::Response::new(axum::body::Body::from("no extension serves this path"));
            *response.status_mut() = axum::http::StatusCode::NOT_FOUND;
            response
        };
        let path = req.uri().path().strip_prefix("/ext/").unwrap_or_default().to_string();
        let Some((id, rest)) = path.split_once('/') else { return not_found() };
        let Some(routes) = read(&self.0).get(id).cloned() else { return not_found() };
        // A directory of its files is a device's to read, like a native extension's.
        let device = matches!(req.extensions().get::<harnix_contract::Subject>(), Some(harnix_contract::Subject::Device { .. }));
        let routes: Vec<RouteDecl> = routes.into_iter().filter(|r| r.static_dir().is_none() || device).collect();
        let rest = format!("/{rest}");
        match rest.parse::<axum::http::Uri>() {
            Ok(uri) => *req.uri_mut() = with_query(uri, req.uri().query()),
            Err(_) => return not_found(),
        }
        super::routes::serve(&routes, req).await
    }
}

fn with_query(uri: axum::http::Uri, query: Option<&str>) -> axum::http::Uri {
    match query {
        Some(q) => format!("{}?{q}", uri.path()).parse().unwrap_or(uri),
        None => uri,
    }
}

/// The host's pipes, for one component: everything it does through them is its own.
pub(crate) struct HostFor {
    host: Arc<DynamicHost>,
    by: ExtId,
}

impl HostFor {
    pub fn new(host: Arc<DynamicHost>, by: ExtId) -> Self {
        Self { host, by }
    }

    fn unsandboxed(&self) -> bool {
        self.host.unsandboxed.contains(self.by.as_str())
    }
}

impl HostFor {
    /// Whether `id` is an extension this component registered.
    fn registered(&self, id: &ExtId) -> Rpc<()> {
        let ours = self.host.packages.registered().iter().any(|a| a.id == id.as_str() && a.by == self.by.as_str());
        match ours {
            true => Ok(()),
            false => Err(RpcError::refused(format!("`{id}` is not an extension `{}` registered", self.by))),
        }
    }
}

#[async_trait::async_trait]
impl HostApi for HostFor {
    fn registrar(&self) -> DynamicRegistrar {
        DynamicRegistrar::new(Arc::new(RegistryFor { host: self.host.clone(), by: self.by.clone() }))
    }

    /// Only for an extension this component registered: its process is that extension, and no
    /// other component's.
    fn spawn_child(&self, id: &ExtId, req: ChildRequest) -> Rpc<ChildHandle> {
        self.registered(id)?;
        super::spawn::spawn(&self.host.children, id, req, self.unsandboxed())
    }

    fn child_sandbox(&self) -> ChildSandbox {
        super::spawn::sandboxing(self.unsandboxed())
    }

    fn share_tools(&self, entries: Vec<ToolEndpoint>) -> Rpc<()> {
        let servers = {
            let mut shared = lock(&self.host.shared);
            shared.insert(self.by.clone(), entries);
            shared
                .values()
                .flatten()
                .map(|e| (e.name().to_string(), json!({"type": "streamable_http", "url": e.url()})))
                .collect()
        };
        self.host.servers.set(servers);
        Ok(())
    }

    /// Only for an extension this component registered: the engine is that extension's.
    async fn adopt(&self, id: &ExtId, spec: AdoptSpec) -> Rpc<Arc<dyn AdoptedEngine>> {
        self.registered(id)?;
        super::adopt::adopt(&self.host.instances, id, spec).await
    }

    async fn release(&self, id: &ExtId) {
        if self.registered(id).is_ok() {
            self.host.instances.release(id.as_str()).await;
        }
    }

    fn publish_ui(&self, entries: Vec<Value>) {
        let entries = if self.host.runtime_modules { entries } else { Vec::new() };
        self.host.ui.set_extension(self.by.as_str(), entries);
    }

    fn package_sections(&self) -> BTreeMap<String, Value> {
        self.host.sections.clone()
    }
}

struct RegistryFor {
    host: Arc<DynamicHost>,
    by: ExtId,
}

/// A refusal of a runtime registration: `REFUSED`, with which rule.
fn structure(rule: &str) -> RpcError {
    let mut e = RpcError::refused(format!("not a package's descriptor: {rule}"));
    e.data = Some(json!({"reason": "structure"}));
    e
}

/// What the host checks of a descriptor registered while the process runs: that it is a
/// package's, and asks nothing a package may not.
pub(crate) fn check(d: &ExtensionDescriptor, caps: &[Capability], router: &Router) -> Rpc<()> {
    let id = d.id().as_str();
    if !matches!(d.trust(), Trust::Consent { .. }) {
        return Err(structure("its trust is not consent"));
    }
    if !valid_package_id(id) {
        return Err(structure("its id is not a package's"));
    }
    if d.spaces().iter().any(|s| s.cross_space) {
        return Err(structure("a package's space cannot reach across spaces"));
    }
    if d.engine().is_some() || d.engine_everywhere().is_some() || !d.templates().is_empty() {
        return Err(structure("a package gives no engine contribution and no instance template"));
    }
    if d.routes().iter().any(|r| r.static_dir().is_none() && !CARRIER_ROUTES.contains(&r.prefix().trim_matches('/'))) {
        return Err(structure("a package's routes are a directory of its files and its carrier's (`ui`, `mcp`)"));
    }
    let table = router.table();
    for cap in caps {
        if !table.defined(cap).is_some_and(|spec| spec.is_package()) {
            return Err(structure(&format!("`{cap}` is not a capability a package may hold")));
        }
    }
    for name in d.trust().calls().into_iter().flatten() {
        if let Some(row) = table.row(name) {
            if row.access().requirement().is_some() {
                return Err(structure(&format!("`{name}` takes a capability, which a consent cannot list")));
            }
        }
        if let Err(why) = table.open_to_package(d.id(), name) {
            return Err(structure(&format!("`{name}` is {why}, which a consent cannot list")));
        }
    }
    if !d.ui().modules().is_empty() && !caps.contains(&caps::UI_PAGE) {
        return Err(structure("modules in the page take `ui.page`"));
    }
    if !d.ui().points().is_empty() {
        return Err(structure("a package defines no contribution point"));
    }
    let mut tools = BTreeSet::new();
    for tool in d.tools() {
        if !tools.insert(session_tool_name(id, tool.name())) {
            return Err(structure(&format!("two tools are called `{}`", tool.name())));
        }
    }
    Ok(())
}

/// A package's tool, as a session names it: its id's dots made underscores.
fn session_tool_name(id: &str, tool: &str) -> String {
    format!("ext__{}__{tool}", id.replace('.', "_"))
}

#[async_trait::async_trait]
impl DynamicRegistry for RegistryFor {
    async fn register(&self, d: ExtensionDescriptor, caps: Vec<Capability>) -> Rpc<Arc<dyn RegisteredExtension>> {
        if let Err(e) = check(&d, &caps, &self.host.router) {
            self.host.router.audit().record_change(
                &harnix_contract::Subject::extension(self.by.clone()),
                &full_name(d.id().as_str(), "*"),
                harnix_authz::Change::Refuse,
                Some("a runtime descriptor that is not a package's"),
            );
            return Err(e);
        }
        let id = d.id().clone();
        let carrier = match d.trust() {
            Trust::Consent { carrier, .. } => *carrier,
            _ => PackageCarrier::Wasm,
        };
        // A package writes nothing of its directory but `run/`: its record and its files are its
        // component's.
        let data_dir = self.host.state_root.join("ext").join(id.as_str()).join("run").join("data");
        d.seat(self.host.seats.seat(&id, &Value::Null, &data_dir));
        let availability = Arc::new(RwLock::new(Availability::Starting));
        let prefix = format!("{id}.");
        let rows: Vec<Row> = d
            .methods()
            .iter()
            .map(|m| {
                let gate = Gate { decl: m.clone(), availability: availability.clone() };
                Row::method(m.name(), m.access().clone(), Arc::new(gate)).describe(m.description())
            })
            .chain(d.topics().iter().map(|t| Row::topic(t.name(), t.access().clone())))
            .collect();
        let topics: Vec<PackageTopic> = d
            .topics()
            .iter()
            .map(|t| PackageTopic {
                name: t.name().strip_prefix(&prefix).unwrap_or(t.name()).to_string(),
                params: t.params().to_vec(),
            })
            .collect();
        let sinks = Arc::new(Sinks::default());
        let summary = self
            .host
            .packages
            .install(
                self.by.as_str(),
                Package {
                    id: id.to_string(),
                    rows,
                    topics,
                    spaces: d.spaces().to_vec(),
                    calls: d.trust().calls().into_iter().flatten().cloned().collect(),
                    caps,
                    carrier,
                    sink: Some(sinks.clone()),
                },
            )
            .await?;
        self.host.routes.set(id.as_str(), d.routes().to_vec());
        Ok(Arc::new(Registered {
            host: self.host.clone(),
            by: self.by.clone(),
            id,
            availability,
            sinks,
            since_ms: summary.since_ms,
        }))
    }

    fn registered(&self) -> Vec<ExtensionSummary> {
        self.host
            .packages
            .registered()
            .into_iter()
            .filter_map(|a| {
                let id = ExtId::package(&a.id).ok()?;
                let by = ExtId::native(&a.by).or_else(|_| ExtId::package(&a.by)).ok()?;
                Some(ExtensionSummary::new(&id, &by, a.carrier, a.since_ms).names(a.methods, a.topics, a.spaces).consented(a.calls))
            })
            .collect()
    }
}

/// A registered extension's method: its descriptor's, while the extension is ready.
struct Gate {
    decl: harnix_ext::MethodDecl,
    availability: Arc<RwLock<Availability>>,
}

#[async_trait::async_trait]
impl Handler for Gate {
    async fn call(&self, cx: CallContext, params: Value) -> Rpc<Value> {
        let now = read(&self.availability).clone();
        if let Some(failure) = now.failure() {
            let mut e = RpcError::new(codes::EXTENSION_UNAVAILABLE, format!("the extension is {failure}"));
            e.data = Some(match &now {
                Availability::Faulted(why) => json!({"failure": failure, "reason": why}),
                _ => json!({"failure": failure}),
            });
            return Err(e);
        }
        Handler::call(&self.decl, cx, params).await
    }
}

/// Where each topic a registered extension watches sends its values.
#[derive(Default)]
struct Sinks(Mutex<BTreeMap<String, Arc<dyn TopicSink>>>);

impl TopicSink for Sinks {
    fn deliver(&self, topic: &str, params: &Value, value: &Value) {
        if let Some(sink) = lock(&self.0).get(topic).cloned() {
            sink.deliver(topic, params, value);
        }
    }

    fn ended(&self, topic: &str, params: &Value, reason: &str) {
        if let Some(sink) = lock(&self.0).remove(topic) {
            sink.ended(topic, params, reason);
        }
    }
}

struct Registered {
    host: Arc<DynamicHost>,
    by: ExtId,
    id: ExtId,
    availability: Arc<RwLock<Availability>>,
    sinks: Arc<Sinks>,
    #[allow(dead_code)]
    since_ms: u64,
}

#[async_trait::async_trait]
impl RegisteredExtension for Registered {
    fn id(&self) -> &ExtId {
        &self.id
    }

    async fn call(&self, meta: CallMeta, method: &str, params: Value) -> Rpc<Value> {
        // Of what the call says about itself, the extensions waiting on it are kept, each with the
        // call it is held on — what a host refuses re-entry by while that call is running — and
        // its deadline, as a bound this call is held to at the latest. Its id and its parent are
        // this process's to issue.
        let deadline = meta.deadline().map(harnix_kernel::Deadline);
        let chain = Chain::from(meta.into_links());
        self.host.packages.call_chained(self.by.as_str(), self.id.as_str(), method, params, chain, deadline).await
    }

    async fn publish(&self, topic: &str, params: Value, value: Value) -> Rpc<()> {
        self.host.packages.publish(self.by.as_str(), self.id.as_str(), topic, params, value).await
    }

    async fn subscribe(&self, topic: &str, params: Value, sink: Arc<dyn TopicSink>) -> Rpc<Value> {
        lock(&self.sinks.0).insert(topic.to_string(), sink);
        self.host.packages.subscribe_as(self.by.as_str(), self.id.as_str(), topic, params).await
    }

    async fn unsubscribe(&self, topic: &str, params: Value) -> Rpc<()> {
        lock(&self.sinks.0).remove(topic);
        self.host.packages.unsubscribe_as(self.by.as_str(), self.id.as_str(), topic, params).await
    }

    fn mark(&self, availability: Availability) {
        *write(&self.availability) = availability;
    }

    fn frame(&self, calls: Option<BTreeSet<String>>) -> Rpc<()> {
        self.host.packages.frame(self.by.as_str(), self.id.as_str(), calls)
    }

    fn refused(&self, what: &str, why: &'static str) {
        let subject = harnix_contract::Subject::extension(self.id.clone());
        self.host.router.audit().record_change(&subject, what, harnix_authz::Change::Refuse, Some(why));
    }

    async fn withdraw(&self) -> Rpc<Withdrawn> {
        self.host.routes.set(self.id.as_str(), Vec::new());
        let gone = self.host.packages.withdraw(self.by.as_str(), self.id.as_str()).await?;
        Ok(Withdrawn::new(gone.methods, gone.topics, gone.spaces, gone.subscriptions_ended))
    }
}
