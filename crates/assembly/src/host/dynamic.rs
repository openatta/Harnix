//! What the host does for a host component: registering extensions while the process runs,
//! configuring spawned daemons' tool endpoints, and — with the process carrier — starting and
//! adopting extension processes.
//!
//! A runtime registration takes the same host path a native extension's does: a descriptor,
//! seated, its rows in the one table. What the host checks of it is its structure (a package's
//! consent is the component's record, which the host never sees); everything after that is what
//! the runtime-application host already does for one — its names, its topics, its spaces, its
//! consent, in one write, undone on any refusal.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, RwLock};

use harnix_contract::s1::{caps, CallMeta, Capability, ExtId, PackageCarrier, Trust};
use harnix_contract::sync::{lock, read, write};
use harnix_contract::{codes, RpcError};
use harnix_ext::ExtensionDescriptor;
use harnix_host_api::{
    AdoptSpec, Availability, ChildHandle, ChildRequest, DynamicRegistrar, DynamicRegistry, ExtensionSummary, HostApi,
    RegisteredExtension, TopicSink, ToolEndpoint, Withdrawn,
};
use harnix_instance::SharedServers;
use harnix_kernel::seat::Seated;
use harnix_kernel::{CallContext, Chain, Handler, Rpc};
use harnix_module::apps::AppRuntime;
use harnix_module::{AppTopic, AppTopicSink};
use harnix_router::names::{app_name, valid_app_id};
use harnix_router::{Row, Router};
use serde_json::{json, Value};

use super::seat::Seats;
use crate::apps::{Apps, Package};

/// The host's side of every host component's pipes.
pub(crate) struct DynamicHost {
    pub apps: Arc<Apps>,
    pub router: Arc<Router>,
    pub seats: Arc<Seats>,
    pub servers: Arc<SharedServers>,
    /// Where a registered extension's directory is made.
    pub state_root: std::path::PathBuf,
    /// What each component shares with every spawned daemon, merged.
    shared: Mutex<BTreeMap<ExtId, Vec<ToolEndpoint>>>,
}

impl DynamicHost {
    pub fn new(
        apps: Arc<Apps>,
        router: Arc<Router>,
        seats: Arc<Seats>,
        servers: Arc<SharedServers>,
        state_root: std::path::PathBuf,
    ) -> Arc<Self> {
        Arc::new(Self { apps, router, seats, servers, state_root, shared: Mutex::new(BTreeMap::new()) })
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
}

impl HostApi for HostFor {
    fn registrar(&self) -> DynamicRegistrar {
        DynamicRegistrar::new(Arc::new(RegistryFor { host: self.host.clone(), by: self.by.clone() }))
    }

    fn spawn_child(&self, _id: &ExtId, _req: ChildRequest) -> Rpc<ChildHandle> {
        Err(RpcError::refused("this build starts no extension processes: that is the process carrier's"))
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

    fn adopt(&self, _id: &ExtId, _child: &ChildHandle, _spec: AdoptSpec) -> Rpc<()> {
        Err(RpcError::refused("this build adopts no extension process's engine: that is the process carrier's"))
    }

    fn release(&self, _id: &ExtId) {}
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
fn check(d: &ExtensionDescriptor, caps: &[Capability], router: &Router) -> Rpc<()> {
    let id = d.id().as_str();
    if !matches!(d.trust(), Trust::Consent { .. }) {
        return Err(structure("its trust is not consent"));
    }
    if !valid_app_id(id) {
        return Err(structure("its id is not a package's"));
    }
    if d.spaces().iter().any(|s| s.cross_space) {
        return Err(structure("a package's space cannot reach across spaces"));
    }
    if d.engine().is_some() || d.engine_everywhere().is_some() || !d.templates().is_empty() {
        return Err(structure("a package gives no engine contribution and no instance template"));
    }
    if d.routes().iter().any(|r| r.static_dir().is_none()) {
        return Err(structure("a package's routes are directories of its files"));
    }
    let table = router.table();
    for cap in caps {
        if !table.defined(cap).is_some_and(|spec| spec.is_package()) {
            return Err(structure(&format!("`{cap}` is not a capability a package may hold")));
        }
    }
    for name in d.trust().calls().into_iter().flatten() {
        if harnix_router::names::names_an_application(name) {
            continue;
        }
        if let Some(row) = table.row(name) {
            if row.access().requirement().is_some() {
                return Err(structure(&format!("`{name}` takes a capability, which a consent cannot list")));
            }
        }
    }
    if !d.ui().modules().is_empty() && !caps.contains(&caps::UI_PAGE) {
        return Err(structure("modules in the page take `ui.page`"));
    }
    if !d.ui().points().is_empty() {
        return Err(structure("a package defines no contribution point"));
    }
    let mut tools = std::collections::BTreeSet::new();
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
        let audit = |name: &str, why: &'static str| {
            self.host.router.audit().record_change(
                &harnix_contract::Subject::extension(self.by.clone()),
                name,
                harnix_authz::Change::Refuse,
                Some(why),
            );
        };
        if let Err(e) = check(&d, &caps, &self.host.router) {
            audit(&app_name(d.id().as_str(), "*"), "a runtime descriptor that is not a package's");
            return Err(e);
        }
        let id = d.id().clone();
        let carrier = match d.trust() {
            Trust::Consent { carrier, .. } => *carrier,
            _ => PackageCarrier::Wasm,
        };
        let data_dir = self.host.state_root.join("ext").join(id.as_str()).join("data");
        d.seat(self.host.seats.seat(&id, &Value::Null, &data_dir));
        let availability = Arc::new(RwLock::new(Availability::Starting));
        let prefix = format!("app.{id}.");
        let rows: Vec<Row> = d
            .methods()
            .iter()
            .map(|m| {
                let gate = Gate { decl: m.clone(), availability: availability.clone() };
                Row::method(m.name(), m.access().clone(), Arc::new(gate)).describe(m.description())
            })
            .chain(d.topics().iter().map(|t| Row::topic(t.name(), t.access().clone())))
            .collect();
        let topics: Vec<AppTopic> = d
            .topics()
            .iter()
            .map(|t| AppTopic {
                name: t.name().strip_prefix(&prefix).unwrap_or(t.name()).to_string(),
                params: t.params().to_vec(),
                principals: Vec::new(),
            })
            .collect();
        let sinks = Arc::new(Sinks::default());
        let summary = self
            .host
            .apps
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
        Ok(Arc::new(Registered {
            host: self.host.clone(),
            by: self.by.clone(),
            id,
            carrier,
            availability,
            sinks,
            since_ms: summary.since_ms,
        }))
    }

    fn registered(&self) -> Vec<ExtensionSummary> {
        self.host
            .apps
            .registered()
            .into_iter()
            .filter_map(|a| {
                let id = ExtId::package(&a.id).ok()?;
                let by = ExtId::native(&a.by).or_else(|_| ExtId::package(&a.by)).ok()?;
                Some(ExtensionSummary::new(&id, &by, PackageCarrier::Wasm, a.since_ms).names(a.methods, a.topics, a.spaces))
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

impl AppTopicSink for Sinks {
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
    #[allow(dead_code)]
    carrier: PackageCarrier,
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
        // call it is held on: what a host refuses re-entry by while that call is running.
        let chain = Chain::from(meta.into_links());
        self.host.apps.call_chained(self.by.as_str(), self.id.as_str(), method, params, chain).await
    }

    async fn publish(&self, topic: &str, params: Value, value: Value) -> Rpc<()> {
        self.host.apps.publish(self.by.as_str(), self.id.as_str(), topic, params, value).await
    }

    async fn subscribe(&self, topic: &str, params: Value, sink: Arc<dyn TopicSink>) -> Rpc<Value> {
        lock(&self.sinks.0).insert(topic.to_string(), sink);
        self.host.apps.subscribe_as(self.by.as_str(), self.id.as_str(), topic, params).await
    }

    async fn unsubscribe(&self, topic: &str, params: Value) -> Rpc<()> {
        lock(&self.sinks.0).remove(topic);
        self.host.apps.unsubscribe_as(self.by.as_str(), self.id.as_str(), topic, params).await
    }

    fn mark(&self, availability: Availability) {
        *write(&self.availability) = availability;
    }

    async fn withdraw(&self) -> Rpc<Withdrawn> {
        let gone = self.host.apps.withdraw(self.by.as_str(), self.id.as_str()).await?;
        Ok(Withdrawn::new(gone.methods, gone.topics, gone.spaces, gone.subscriptions_ended))
    }
}
