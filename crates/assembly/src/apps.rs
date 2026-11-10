//! The runtime-application host: what lets names into the process after it started.
//!
//! Everything else in this crate puts rows in the route table once, at
//! assembly. This is the one path that does so later, and it is narrow on
//! purpose (`[apps] enabled`; `harnix_module::apps` for the contract):
//!
//! **Registering** one application is all or nothing, one at a time (a
//! registration or withdrawal holds `serial` throughout). Checked first —
//! nothing is written by a refusal, and every refusal is audited — the id
//! must be well formed and overlap no registered id: a compiled-in module's,
//! enabled or not, keeps its namespace and the application fails
//! `namespace_taken`; nor may it start with a reserved word. Then the table's
//! own check (namespace, names taken). Then, in this order:
//!
//! 1. its topics are declared in the hub and its spaces added — invisible to
//!    a client so far, since subscribing is decided by the table;
//! 2. its consent is written in the grants, and its rows go in the table,
//!    each with its handler, in one write. From here a call is admitted.
//!
//! A failure at either step undoes the earlier one. So a name the table admits
//! always has its handler and its topic behind it.
//!
//! **Withdrawing** goes the other way: the rows leave the table first (one
//! write; a call decided after it is unknown, `-32601`, like a name never
//! declared; a call already running holds its own handler and finishes),
//! then each subscriber of its topics is sent `harnix.topic_ended` and the
//! topics are gone, then its spaces leave `harnix.spaces.list` (pushed on
//! `harnix.spaces`).
//!
//! Every name declared, withdrawn or refused is an audit entry under the
//! subject of the module that hosts the application, with the time.
//!
//! **Acting as itself.** An application reaches the rest of the process as
//! `ext:<id>` (`call_as`, `subscribe_as`): through the router, like any
//! extension, but trusted by consent — only to what its `calls` lists (method
//! names, topics as `topic:<name>`) and its own `app.<id>.*`, each row as far
//! as it is open to extensions, and a row about a space or a session only in
//! its own space. Its `calls` entries are checked at registration (an unknown
//! name or one the hub owns refuses it); listing a row that requires a
//! capability opens nothing, since a package holds none of the kernel's.
//! Withdrawing takes its consent back.
//!
//! Its subscriptions are one client of the hub per application, made at
//! registration when it gives a topic sink and removed at withdrawal; what the
//! hub pushes that client goes to the sink.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use harnix_authz::{access_for, Change};
use harnix_contract::sync::lock;
use harnix_contract::{Channel, Frame, FrameSink, RpcError, SpaceDecl, Subject};
use harnix_hub::Hub;
use harnix_kernel::{CallContext, Chain, ClientId, Dispatch, Owner};
use harnix_module::apps::AppRuntime;
use harnix_module::{AppSpec, AppSummary, AppTopic, AppTopicSink, AppWithdrawn, Principal};
use harnix_contract::s1::{PackageCarrier, Trust};
use harnix_kernel::GrantTable;
use harnix_router::names::{app_name, names_an_application, topic, valid_app_id};
use harnix_router::{Row, Router, TableError};
use serde_json::{json, Value};

use crate::gate::RuntimeMethod;

/// What an application's topics and spaces are kept in: the hub, or a
/// stand-in in the tests.
#[async_trait::async_trait]
pub(crate) trait Surfaces: Send + Sync {
    /// `false` when the topic exists already.
    async fn declare_topic(&self, name: &str, params: Vec<String>) -> bool;
    /// Subscriptions ended.
    async fn withdraw_topic(&self, name: &str, reason: &str) -> usize;
    async fn publish(&self, name: &str, params: Value, value: Value);
    fn add_space(&self, decl: SpaceDecl) -> Result<(), String>;
    fn remove_space(&self, id: &str) -> bool;
    /// A client of the hub's topics that is not a connection: an
    /// application's subscriptions. `None` where there is no hub.
    async fn add_client(&self, sink: Arc<dyn FrameSink>) -> Option<u64> {
        let _ = sink;
        None
    }
    async fn remove_client(&self, client: u64) {
        let _ = client;
    }
}

pub(crate) struct HubSurfaces(pub Arc<Hub>);

#[async_trait::async_trait]
impl Surfaces for HubSurfaces {
    async fn declare_topic(&self, name: &str, params: Vec<String>) -> bool {
        self.0.declare_runtime_topic(name, params).await
    }
    async fn withdraw_topic(&self, name: &str, reason: &str) -> usize {
        self.0.withdraw_topic(name, reason).await
    }
    async fn publish(&self, name: &str, params: Value, value: Value) {
        self.0.publish(name, params, value).await
    }
    fn add_space(&self, decl: SpaceDecl) -> Result<(), String> {
        self.0.add_space(decl)
    }
    fn remove_space(&self, id: &str) -> bool {
        self.0.remove_space(id)
    }
    async fn add_client(&self, sink: Arc<dyn FrameSink>) -> Option<u64> {
        Some(self.0.add_client(sink).await.0)
    }
    async fn remove_client(&self, client: u64) {
        self.0.remove_client(ClientId(client)).await
    }
}

/// One package's rows (full names), topics (by their names inside its own), spaces and consent:
/// what installing it takes.
pub(crate) struct Package {
    pub id: String,
    pub rows: Vec<Row>,
    pub topics: Vec<AppTopic>,
    pub spaces: Vec<SpaceDecl>,
    /// What it may call and subscribe to besides its own names.
    pub calls: Vec<String>,
    pub caps: Vec<harnix_contract::s1::Capability>,
    pub carrier: PackageCarrier,
    pub sink: Option<Arc<dyn AppTopicSink>>,
}

struct Live {
    summary: AppSummary,
    topics: Vec<AppTopic>,
    /// Its subscriptions' client of the hub, when it gave a topic sink.
    client: Option<u64>,
}

/// What the hub pushes an application's subscriptions, handed to its sink.
struct TopicFrames(Arc<dyn AppTopicSink>);

#[async_trait::async_trait]
impl FrameSink for TopicFrames {
    async fn deliver(&self, frame: Frame) -> bool {
        let Ok(note) = serde_json::from_str::<Value>(&frame.payload) else {
            return true;
        };
        let p = &note["params"];
        let topic = p["topic"].as_str().unwrap_or_default();
        match note["method"].as_str() {
            Some("harnix.topic") => self.0.deliver(topic, &p["params"], &p["value"]),
            Some(harnix_hub::TOPIC_ENDED) => {
                self.0.ended(topic, &p["params"], p["reason"].as_str().unwrap_or_default())
            }
            _ => {}
        }
        true
    }

    /// Topics only: a session's events are not for an application's backend.
    fn wants(&self, channel: Channel) -> bool {
        channel == Channel::HostEvents
    }
}

pub(crate) struct Apps {
    router: Arc<Router>,
    surfaces: Arc<dyn Surfaces>,
    /// Where an application's consent is written, and its interface's.
    grants: Arc<GrantTable>,
    live: Mutex<BTreeMap<String, Live>>,
    serial: tokio::sync::Mutex<()>,
}

/// An id that was checked already: a host module's, or a registered application's.
fn ext_of(id: &str) -> harnix_contract::ExtId {
    harnix_contract::ExtId::native(id).or_else(|_| harnix_contract::ExtId::package(id)).expect("an id is checked before it acts")
}

/// The subject a checked id acts as.
fn subject_of(id: &str) -> Subject {
    Subject::extension(ext_of(id))
}

/// A refusal on the record under `by`, with why.
fn router_audit(router: &Router, by: &Subject, name: &str, why: &'static str) {
    router.audit().record_change(by, name, Change::Refuse, Some(why));
}

/// Refused, audited under `by`, as `REFUSED` with the reason.
fn refuse(router: &Router, by: &Subject, name: &str, why: &'static str, message: String) -> RpcError {
    router.audit().record_change(by, name, Change::Refuse, Some(why));
    RpcError::refused(message)
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Keep only the declared parameters, so `{job: 1, extra: 2}` and `{job: 1}`
/// are the same value.
fn normalize(params: Value, declared: &[String]) -> Result<Value, RpcError> {
    let mut out = serde_json::Map::new();
    let given = params.as_object().cloned().unwrap_or_default();
    for name in declared {
        match given.get(name) {
            Some(v) if !v.is_null() => {
                out.insert(name.clone(), v.clone());
            }
            _ => return Err(RpcError::invalid_params(format!("missing topic parameter `{name}`"))),
        }
    }
    Ok(Value::Object(out))
}

impl Apps {
    pub fn new(router: Arc<Router>, surfaces: Arc<dyn Surfaces>, grants: Arc<GrantTable>) -> Self {
        Self {
            router,
            surfaces,
            grants,
            live: Mutex::new(BTreeMap::new()),
            serial: tokio::sync::Mutex::new(()),
        }
    }

    /// Install one package's rows, topics and spaces, with its consent: all or nothing, one at a
    /// time. What `register` and the extension host's runtime registration both come down to.
    pub(crate) async fn install(&self, by: &str, spec: Package) -> Result<AppSummary, RpcError> {
        let _serial = self.serial.lock().await;
        let host = subject_of(by);
        let id = spec.id.clone();
        let all = app_name(&id, "*");
        let az = &*self.router;
        if !valid_app_id(&id) {
            return Err(refuse(
                az,
                &host,
                &all,
                "not an application id",
                format!("`{id}` is not an application id: `<publisher>.<name>`, lowercase segments each starting with a letter, the first not a reserved word"),
            ));
        }
        // Its id: free of every registered one — compiled-in modules, enabled or not, and the
        // applications registered — and not under a reserved word. A compiled-in module's names
        // keep precedence: the application is the one refused, saying who has its namespace.
        if let Err(TableError::NamespaceTaken { by: taken, .. }) = az.table().check_app(&id, std::iter::empty()) {
            router_audit(az, &host, &all, "namespace_taken");
            let mut e = RpcError::refused(format!(
                "the application id `{id}` overlaps `{taken}`, which is registered already and keeps its names"
            ));
            e.data = Some(json!({"failure": "namespace_taken", "by": taken}));
            return Err(e);
        }
        for space in &spec.spaces {
            if space.id != id && !space.id.starts_with(&format!("{id}.")) {
                return Err(refuse(
                    az,
                    &host,
                    &format!("space:{}", space.id),
                    "a space outside the application's namespace",
                    format!("the space `{}` is not `{id}` or `{id}.<name>`", space.id),
                ));
            }
        }

        // Its `calls` are its consent: names that exist and are not the hub's own. Another
        // application's names are left to that application's principals. What a name requires
        // beyond that it does not hold: a package holds none of the kernel's capabilities.
        for name in &spec.calls {
            if names_an_application(name) {
                continue;
            }
            if let Err(e) = az.table().check_listed(name) {
                return Err(refuse(
                    az,
                    &host,
                    name,
                    "the application's `calls` name what it may not reach",
                    format!("`{id}`'s `calls` capability: {e}"),
                ));
            }
        }

        let topics: Vec<String> = spec.topics.iter().map(|t| app_name(&id, &t.name)).collect();
        let methods: Vec<String> = spec
            .rows
            .iter()
            .map(Row::name)
            .filter(|name| !topics.iter().any(|t| topic(t) == *name))
            .map(str::to_string)
            .collect();
        let rows = spec.rows;
        if let Err(e) = az.table().check_app(&id, rows.iter().map(Row::name)) {
            let name = match &e {
                TableError::Taken(n) | TableError::Namespace(n) => n.clone(),
                _ => all.clone(),
            };
            return Err(refuse(az, &host, &name, "the authorization table refused it", e.to_string()));
        }

        // 1. Topics and spaces: nobody can subscribe or claim a session's space by them through
        //    the table yet.
        let mut declared_topics = Vec::new();
        for (t, name) in spec.topics.iter().zip(&topics) {
            if !self.surfaces.declare_topic(name, t.params.clone()).await {
                self.undo(&declared_topics, &[]).await;
                return Err(refuse(az, &host, &topic(name), "the topic exists", format!("there is already a topic `{name}`")));
            }
            declared_topics.push(name.clone());
        }
        let mut added_spaces = Vec::new();
        for space in &spec.spaces {
            if let Err(reason) = self.surfaces.add_space(space.clone()) {
                self.undo(&declared_topics, &added_spaces).await;
                return Err(refuse(az, &host, &format!("space:{}", space.id), "the space was refused", reason));
            }
            added_spaces.push(space.id.clone());
        }
        // 2. Its consent, then the table in one write: its rows with their handlers. Audited
        //    there. From here its subject reaches its `calls` and its own names, narrowed to its
        //    own space, and nothing more.
        self.grants.set(ext_of(&id), spec.caps, Trust::consent(spec.calls.iter().cloned(), spec.carrier));
        if let Err(e) = az.install_app(&host, &id, rows) {
            self.grants.remove(&ext_of(&id));
            self.undo(&declared_topics, &added_spaces).await;
            return Err(RpcError::refused(e.to_string()));
        }
        for space in &added_spaces {
            az.audit().record_change(&host, &format!("space:{space}"), Change::Declare, Some("declared by a runtime application"));
        }
        let client = match &spec.sink {
            Some(sink) => self.surfaces.add_client(Arc::new(TopicFrames(sink.clone()))).await,
            None => None,
        };

        tracing::info!(app = %id, by, methods = methods.len(), topics = topics.len(), spaces = added_spaces.len(), "runtime application registered");
        let summary = AppSummary {
            id: id.clone(),
            by: by.to_string(),
            methods,
            topics,
            spaces: added_spaces,
            calls: spec.calls.clone(),
            since_ms: now_ms(),
        };
        self.live().insert(
            id,
            Live {
                summary: summary.clone(),
                topics: spec.topics,
                client,
            },
        );
        Ok(summary)
    }

    fn live(&self) -> std::sync::MutexGuard<'_, BTreeMap<String, Live>> {
        lock(&self.live)
    }

    async fn undo(&self, topics: &[String], spaces: &[String]) {
        for id in spaces {
            self.surfaces.remove_space(id);
        }
        for name in topics {
            self.surfaces.withdraw_topic(name, "the application was not registered").await;
        }
    }
}

#[async_trait::async_trait]
impl AppRuntime for Apps {
    async fn register(&self, by: &str, spec: AppSpec) -> Result<AppSummary, RpcError> {
        let id = spec.id.clone();
        // The application itself always reaches its own names: its tools ask its backend.
        let with_self = |principals: &[Principal]| {
            let mut principals = principals.to_vec();
            principals.push(Principal::Module(id.clone()));
            access_for(&principals)
        };
        let rows: Vec<Row> = spec
            .methods
            .iter()
            .map(|m| {
                Row::method(app_name(&id, &m.name), with_self(&m.principals), Arc::new(RuntimeMethod(m.handler.clone())))
                    .describe(m.description.clone())
            })
            .chain(spec.topics.iter().map(|t| Row::topic(&app_name(&id, &t.name), with_self(&t.principals))))
            .collect();
        self.install(
            by,
            Package {
                id: id.clone(),
                rows,
                topics: spec.topics,
                spaces: spec.spaces,
                calls: spec.calls,
                caps: Vec::new(),
                carrier: PackageCarrier::Wasm,
                sink: spec.topic_sink,
            },
        )
        .await
    }

    async fn withdraw(&self, by: &str, id: &str) -> Result<AppWithdrawn, RpcError> {
        let _serial = self.serial.lock().await;
        let host = subject_of(by);
        let (summary, client) = match self.live().get(id) {
            None => return Err(RpcError::invalid_params(format!("no application `{id}` is registered"))),
            Some(live) if live.summary.by != by => {
                return Err(RpcError::refused(format!(
                    "the application `{id}` was registered by the module `{}`; only it may withdraw it",
                    live.summary.by
                )))
            }
            Some(live) => (live.summary.clone(), live.client),
        };
        // The table first: from here a call is decided as unknown; then its consent, so it reaches
        // nothing as itself.
        self.router
            .withdraw(&host, &Owner::extension(ext_of(id)), "withdrawn with its runtime application");
        self.grants.remove(&ext_of(id));
        // Its own subscriptions end with it; the hub forgets the client's.
        if let Some(client) = client {
            self.surfaces.remove_client(client).await;
        }
        let reason = format!("the application `{id}` was withdrawn");
        let mut ended = 0;
        for name in &summary.topics {
            ended += self.surfaces.withdraw_topic(name, &reason).await;
        }
        for space in &summary.spaces {
            self.surfaces.remove_space(space);
            self.router.audit().record_change(
                &host,
                &format!("space:{space}"),
                Change::Withdraw,
                Some("withdrawn with its runtime application"),
            );
        }
        self.live().remove(id);
        tracing::info!(app = %id, by, subscriptions_ended = ended, "runtime application withdrawn");
        Ok(AppWithdrawn {
            id: id.to_string(),
            methods: summary.methods,
            topics: summary.topics,
            spaces: summary.spaces,
            subscriptions_ended: ended,
        })
    }

    async fn publish(&self, by: &str, id: &str, topic: &str, params: Value, value: Value) -> Result<(), RpcError> {
        let (name, params) = {
            let live = self.live();
            let Some(app) = live.get(id).filter(|a| a.summary.by == by) else {
                return Err(RpcError::invalid_params(format!("no application `{id}` is registered by this module")));
            };
            let Some(decl) = app.topics.iter().find(|t| t.name == topic) else {
                return Err(RpcError::not_found(format!("`app.{id}.{topic}` is not a topic this application declared")));
            };
            (app_name(id, topic), normalize(params, &decl.params)?)
        };
        self.surfaces.publish(&name, params, value).await;
        Ok(())
    }

    fn registered(&self) -> Vec<AppSummary> {
        self.live().values().map(|l| l.summary.clone()).collect()
    }

    async fn call_as(&self, by: &str, id: &str, method: &str, params: Value) -> Result<Value, RpcError> {
        self.call_chained(by, id, method, params, Chain::default()).await
    }

    async fn call_chained(&self, by: &str, id: &str, method: &str, params: Value, chain: Chain) -> Result<Value, RpcError> {
        // Whether it may is the table's; whether `by` speaks for it is this.
        self.registered_by(by, id)?;
        // On no connection: the application cannot stand in for one.
        let cx = CallContext::root(subject_of(id), None).chained(chain);
        self.router.call(cx, method, params).await
    }

    async fn subscribe_as(&self, by: &str, id: &str, name: &str, params: Value) -> Result<Value, RpcError> {
        self.watch(by, id, name, params, "harnix.topics.subscribe").await
    }

    async fn unsubscribe_as(&self, by: &str, id: &str, name: &str, params: Value) -> Result<(), RpcError> {
        self.watch(by, id, name, params, "harnix.topics.unsubscribe").await.map(|_| ())
    }

    fn frame(&self, by: &str, id: &str, calls: Option<Vec<String>>) -> Result<(), RpcError> {
        let ext = harnix_contract::ExtId::package(id).map_err(|e| RpcError::invalid_params(e.to_string()))?;
        let change = if calls.is_some() { Change::Declare } else { Change::Withdraw };
        self.router.audit().record_change(&subject_of(by), &format!("frame:{id}"), change, Some("its interface in frames"));
        self.grants.set_frame(&ext, calls.map(|c| c.into_iter().collect()));
        Ok(())
    }
}

impl Apps {
    /// The live application `id`'s topic client, when `by` registered it.
    fn registered_by(&self, by: &str, id: &str) -> Result<Option<u64>, RpcError> {
        match self.live().get(id).filter(|l| l.summary.by == by) {
            Some(app) => Ok(app.client),
            None => Err(RpcError::invalid_params(format!("no application `{id}` is registered by this module"))),
        }
    }

    /// Subscribe or unsubscribe the application's client, as the application: the table decides
    /// `topic:<name>` for it like for anyone.
    async fn watch(&self, by: &str, id: &str, name: &str, params: Value, method: &str) -> Result<Value, RpcError> {
        let me = subject_of(id);
        let Some(client) = self.registered_by(by, id)? else {
            return Err(RpcError::refused(format!("`{id}` was registered without anywhere for topic values to go")));
        };
        let cx = CallContext::root(me, Some(ClientId(client)));
        self.router.call(cx, method, json!({"topic": name, "params": params})).await
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use harnix_authz::Audit;
    use harnix_contract::codes;
    use harnix_kernel::{Access, Handler, KernelPart, Rpc};
    use harnix_router::{RouteTable, OUTSIDE_HUB};
    use serde_json::json;

    use super::*;

    /// One call a kernel row answered: who called what, with what, as which client.
    type Answered = (String, String, Value, Option<ClientId>);

    /// What the kernel's rows answer with, here.
    #[derive(Default)]
    struct Below {
        calls: AtomicUsize,
        seen: Mutex<Vec<Answered>>,
    }

    struct BelowRow(Arc<Below>, &'static str);

    #[async_trait::async_trait]
    impl Handler for BelowRow {
        async fn call(&self, cx: CallContext, p: Value) -> Rpc<Value> {
            self.0.calls.fetch_add(1, Ordering::SeqCst);
            lock(&self.0.seen).push((cx.subject.label(), self.1.to_string(), p, cx.client));
            Ok(json!({"below": self.1}))
        }
    }

    /// One call, made with nothing else to its name.
    async fn call(router: &Router, subject: &Subject, method: &str, params: Value) -> Result<Value, RpcError> {
        router.call(CallContext::root(subject.clone(), None), method, params).await
    }

    #[derive(Default)]
    struct Stub {
        topics: Mutex<BTreeMap<String, Vec<String>>>,
        /// Subscribers per topic, as if clients had subscribed.
        subscribers: Mutex<BTreeMap<String, usize>>,
        ended: Mutex<Vec<(String, String)>>,
        published: Mutex<Vec<(String, Value, Value)>>,
        spaces: Mutex<BTreeSet<String>>,
        /// Clients made for applications' subscriptions, and the ones removed.
        clients: Mutex<Vec<Arc<dyn FrameSink>>>,
        removed: Mutex<Vec<u64>>,
    }

    #[async_trait::async_trait]
    impl Surfaces for Stub {
        async fn declare_topic(&self, name: &str, params: Vec<String>) -> bool {
            let mut topics = lock(&self.topics);
            if topics.contains_key(name) {
                return false;
            }
            topics.insert(name.into(), params);
            true
        }
        async fn withdraw_topic(&self, name: &str, reason: &str) -> usize {
            lock(&self.topics).remove(name);
            lock(&self.ended).push((name.into(), reason.into()));
            lock(&self.subscribers).remove(name).unwrap_or(0)
        }
        async fn publish(&self, name: &str, params: Value, value: Value) {
            if lock(&self.topics).contains_key(name) {
                lock(&self.published).push((name.into(), params, value));
            }
        }
        fn add_space(&self, decl: SpaceDecl) -> Result<(), String> {
            if decl.id == "acme.taken" || !lock(&self.spaces).insert(decl.id.clone()) {
                return Err(format!("there is already a space `{}`", decl.id));
            }
            Ok(())
        }
        fn remove_space(&self, id: &str) -> bool {
            lock(&self.spaces).remove(id)
        }
        async fn add_client(&self, sink: Arc<dyn FrameSink>) -> Option<u64> {
            let mut clients = lock(&self.clients);
            clients.push(sink);
            Some(100 + clients.len() as u64)
        }
        async fn remove_client(&self, client: u64) {
            lock(&self.removed).push(client);
        }
    }

    struct World {
        apps: Arc<Apps>,
        az: Arc<Router>,
        below: Arc<Below>,
        stub: Arc<Stub>,
    }

    fn device() -> Subject {
        Subject::Device { id: "d1".into() }
    }

    /// One process: the hub's methods, and a compiled-in module `probe` whose `app.probe.echo`
    /// is in the table.
    fn world() -> World {
        let below = Arc::new(Below::default());
        let row = |name: &'static str, access: Access| Row::method(name, access, Arc::new(BelowRow(below.clone(), name)));
        let mut table = RouteTable::new();
        table
            .install(
                &Owner::Kernel(KernelPart::Hub),
                vec![
                    row("harnix.hello", Access::device()),
                    row("harnix.topics.subscribe", Access::device()),
                    row("harnix.topics.unsubscribe", Access::device()),
                    row("harnix.session.create", Access::device()),
                    row("settings.write", Access::refused("not for devices")),
                    Row::closed("harnix.turn", Access::hub_owned("the hub's")),
                    Row::topic("harnix.sessions.status", Access::device()),
                    Row::topic("system.process", Access::kernel()),
                ],
            )
            .unwrap();
        let compiled = Arc::new(RuntimeMethod(Arc::new(Fixed(json!("the compiled-in probe")))));
        table
            .install(&Owner::extension(harnix_contract::ExtId::native("probe").unwrap()), vec![Row::method("app.probe.echo", Access::device(), compiled)])
            .unwrap();
        // A compiled-in module with no names in the table still has its id.
        table.register(&ext_of("quiet"), harnix_router::IdKind::Native).unwrap();
        let grants = Arc::new(GrantTable::new());
        for native in ["probe", "quiet", "host", "other"] {
            grants.set(ext_of(native), [], Trust::Origin);
        }
        let az = Arc::new(Router::new(table, grants.clone(), Arc::new(Audit::default())));
        let stub = Arc::new(Stub::default());
        let apps = Arc::new(Apps::new(az.clone(), stub.clone(), grants));
        World { apps, az, below, stub }
    }

    struct Fixed(Value);

    #[async_trait::async_trait]
    impl harnix_module::AppHandler for Fixed {
        async fn call(&self, _caller: &Subject, _params: Value) -> Result<Value, RpcError> {
            Ok(self.0.clone())
        }
    }

    fn fake(id: &str) -> AppSpec {
        AppSpec::new(id)
            .method("echo", &[Principal::Device], |caller, params| async move {
                Ok(json!({"params": params, "caller": caller}))
            })
            .method("internal", &[Principal::Kernel], |_c, _p| async move { Ok(json!("kernel only")) })
            .topic("ticks", &["job"])
            .space(SpaceDecl::new(id))
    }

    #[tokio::test]
    async fn a_registered_method_is_admitted_routed_and_audited_then_withdrawn() {
        let w = world();
        let summary = w.apps.register("host", fake("acme.fake")).await.unwrap();
        assert_eq!(summary.methods, vec!["app.acme.fake.echo", "app.acme.fake.internal"]);
        assert_eq!(summary.topics, vec!["app.acme.fake.ticks"]);
        assert_eq!(summary.spaces, vec!["acme.fake"]);

        let answer = call(&w.az, &device(), "app.acme.fake.echo", json!({"x": 1})).await.unwrap();
        assert_eq!(answer["params"], json!({"x": 1}));
        assert_eq!(answer["caller"], json!({"kind": "device", "id": "d1"}));
        // Only who it was declared for.
        let refused = call(&w.az, &device(), "app.acme.fake.internal", json!({})).await.unwrap_err();
        assert_eq!(refused.code, codes::REFUSED);
        assert_eq!(call(&w.az, &Subject::Kernel, "app.acme.fake.internal", json!({})).await.unwrap(), json!("kernel only"));
        assert!(w.az.reachable(&device()).contains(&"app.acme.fake.echo".to_string()));

        // On the record: who declared what, and when.
        let declared: Vec<_> = w.az.audit().recent(50).into_iter().filter(|e| e.decision == "declare").collect();
        let names: BTreeSet<_> = declared.iter().map(|e| e.method.as_str()).collect();
        assert_eq!(names, BTreeSet::from(["app.acme.fake.echo", "app.acme.fake.internal", "topic:app.acme.fake.ticks", "space:acme.fake"]));
        assert!(declared.iter().all(|e| e.subject == "ext:host" && e.at_ms.is_some()));
        let called = w.az.audit().recent(50).into_iter().find(|e| e.method == "app.acme.fake.echo" && e.decision == "allow");
        assert_eq!(called.unwrap().subject, "device:d1");

        lock(&w.stub.subscribers).insert("app.acme.fake.ticks".into(), 2);
        let withdrawn = w.apps.withdraw("host", "acme.fake").await.unwrap();
        assert_eq!(withdrawn.subscriptions_ended, 2);
        assert_eq!(
            lock(&w.stub.ended).as_slice(),
            [("app.acme.fake.ticks".to_string(), "the application `acme.fake` was withdrawn".to_string())]
        );
        assert!(lock(&w.stub.spaces).is_empty());

        // Refused afterwards like a name that never existed, and never handed to anything else.
        let gone = call(&w.az, &device(), "app.acme.fake.echo", json!({})).await.unwrap_err();
        assert_eq!(gone.code, codes::METHOD_NOT_FOUND);
        assert!(!w.az.reachable(&device()).contains(&"app.acme.fake.echo".to_string()));
        assert_eq!(w.below.calls.load(Ordering::SeqCst), 0);
        let withdrew: Vec<_> = w.az.audit().recent(80).into_iter().filter(|e| e.decision == "withdraw").collect();
        assert_eq!(withdrew.len(), 4);
        assert!(w.apps.registered().is_empty());
        // And the id is free again.
        w.apps.register("host", fake("acme.fake")).await.unwrap();
    }

    /// What a sink is handed.
    #[derive(Default)]
    struct Heard(Mutex<Vec<(String, Value, Value)>>, Mutex<Vec<String>>);

    impl AppTopicSink for Heard {
        fn deliver(&self, topic: &str, params: &Value, value: &Value) {
            lock(&self.0).push((topic.into(), params.clone(), value.clone()));
        }
        fn ended(&self, topic: &str, _params: &Value, reason: &str) {
            lock(&self.1).push(format!("{topic}: {reason}"));
        }
    }

    fn me(id: &str) -> Subject {
        subject_of(id)
    }

    #[tokio::test]
    async fn an_application_acts_as_itself_and_only_as_far_as_its_hub_says() {
        let w = world();
        let heard = Arc::new(Heard::default());
        let spec = fake("acme.notes")
            .calls(["harnix.session.create", "settings.write", "topic:harnix.sessions.status"])
            .topic_sink(heard.clone());
        w.apps.register("host", spec).await.unwrap();

        // Listed, and open to every module anyway: through the router, as `module:acme.notes`,
        // and never on a connection.
        assert_eq!(
            w.apps.call_as("host", "acme.notes", "harnix.session.create", json!({"x": 1})).await.unwrap(),
            json!({"below": "harnix.session.create"})
        );
        let (subject, _, params, client) = lock(&w.below.seen).last().cloned().unwrap();
        assert_eq!(subject, "ext:acme.notes");
        assert_eq!((params, client), (json!({"x": 1}), None));
        // Listed, and refused to devices: a package holds none of the kernel's capabilities, so
        // listing it opens nothing.
        let e = w.apps.call_as("host", "acme.notes", "settings.write", json!({})).await.unwrap_err();
        assert_eq!(e.code, codes::REFUSED);
        assert_eq!(call(&w.az, &me("other"), "settings.write", json!({})).await.unwrap_err().code, codes::REFUSED);
        // Its own methods, unlisted and declared for devices only.
        assert_eq!(
            w.apps.call_as("host", "acme.notes", "app.acme.notes.echo", json!({"y": 2})).await.unwrap()["caller"],
            json!({"kind": "extension", "id": "acme.notes"})
        );
        // Not listed, though open to every module: refused by the table, and on the record.
        let refused = w.apps.call_as("host", "acme.notes", "harnix.hello", json!({})).await.unwrap_err();
        assert_eq!(refused.code, codes::REFUSED);
        assert!(refused.message.contains("`calls` capability"), "{}", refused.message);
        let entry = w.az.audit().recent(100).into_iter().find(|e| e.method == "harnix.hello").unwrap();
        assert_eq!((entry.subject.as_str(), entry.decision), ("ext:acme.notes", "refuse"));
        assert_eq!(entry.reason, Some(OUTSIDE_HUB));
        // The same answer through the router directly: there is no other door to go around.
        assert_eq!(call(&w.az, &me("acme.notes"), "harnix.hello", json!({})).await.unwrap_err().code, codes::REFUSED);
        // And what it is told it reaches is exactly that.
        let reach = w.apps.call_as("host", "acme.notes", harnix_router::REACHABLE, json!({})).await.unwrap();
        assert_eq!(
            reach["methods"],
            json!(["app.acme.notes.echo", "app.acme.notes.internal", "harnix.session.create", "harnix.topics.subscribe", "harnix.topics.unsubscribe"])
        );
        assert_eq!(reach["topics"], json!(["app.acme.notes.ticks", "harnix.sessions.status"]));
        // Only the module that registered it speaks for it.
        assert!(w.apps.call_as("other", "acme.notes", "harnix.session.create", json!({})).await.is_err());

        // Subscribing: a listed topic, through its own client of the hub; an unlisted one refused.
        w.apps.subscribe_as("host", "acme.notes", "harnix.sessions.status", json!({})).await.unwrap();
        let (subject, method, params, client) = lock(&w.below.seen).last().cloned().unwrap();
        assert_eq!((subject.as_str(), method.as_str()), ("ext:acme.notes", "harnix.topics.subscribe"));
        assert_eq!(params, json!({"topic": "harnix.sessions.status", "params": {}}));
        assert_eq!(client, Some(ClientId(101)), "as its own client of the hub");
        assert_eq!(
            w.apps.subscribe_as("host", "acme.notes", "system.process", json!({})).await.unwrap_err().code,
            codes::REFUSED
        );
        assert!(w
            .az
            .audit()
            .recent(100)
            .iter()
            .any(|e| e.subject == "ext:acme.notes" && e.method == "harnix.topics.subscribe" && e.reason == Some(OUTSIDE_HUB)));
        // Its own topic needs no listing.
        w.apps.subscribe_as("host", "acme.notes", "app.acme.notes.ticks", json!({"job": 1})).await.unwrap();
        // What the hub pushes its client reaches the sink, topic values and endings both.
        let client = lock(&w.stub.clients)[0].clone();
        let push = |method: &str, params: Value| Frame::host(json!({"jsonrpc": "2.0", "method": method, "params": params}).to_string());
        client.deliver(push("harnix.topic", json!({"topic": "harnix.sessions.status", "params": {}, "value": {"n": 1}}))).await;
        client.deliver(push(harnix_hub::TOPIC_ENDED, json!({"topic": "app.x.t", "params": {}, "reason": "gone"}))).await;
        assert_eq!(lock(&heard.0).as_slice(), [("harnix.sessions.status".to_string(), json!({}), json!({"n": 1}))]);
        assert_eq!(lock(&heard.1).as_slice(), ["app.x.t: gone"]);
        assert!(!client.wants(Channel::SessionEvents));

        // Withdrawn: its consent is gone, the client removed, and nothing more is called as it.
        w.apps.withdraw("host", "acme.notes").await.unwrap();
        assert_eq!(lock(&w.stub.removed).as_slice(), [101]);
        let e = call(&w.az, &me("acme.notes"), "harnix.session.create", json!({})).await.unwrap_err();
        assert_eq!((e.code, e.message.as_str()), (codes::REFUSED, harnix_router::UNREGISTERED));
        assert_eq!(w.apps.call_as("host", "acme.notes", "harnix.session.create", json!({})).await.unwrap_err().code, codes::INVALID_PARAMS);
    }

    /// The extensions waiting on a call an application makes as itself reach the application
    /// method it calls, beside its params: what that method's host refuses re-entry by.
    #[tokio::test]
    async fn a_calls_chain_reaches_the_application_method_it_calls() {
        struct Chained;
        #[async_trait::async_trait]
        impl harnix_module::AppHandler for Chained {
            async fn call(&self, _caller: &Subject, _params: Value) -> Result<Value, RpcError> {
                Ok(Value::Null)
            }
            async fn call_in(&self, cx: &CallContext, _params: Value) -> Result<Value, RpcError> {
                Ok(json!(cx.chain.extensions().collect::<Vec<_>>()))
            }
        }
        let w = world();
        let todo = AppSpec::new("acme.todo").method_with(harnix_module::AppMethod {
            name: "list".into(),
            principals: vec![Principal::AnyModule],
            description: String::new(),
            handler: Arc::new(Chained),
        });
        w.apps.register("host", todo).await.unwrap();
        w.apps.register("host", fake("acme.notes").calls(["app.acme.todo.list"])).await.unwrap();
        let waiting = Chain::of_extensions(["acme.notes"]);
        let answer = w.apps.call_chained("host", "acme.notes", "app.acme.todo.list", json!({}), waiting).await.unwrap();
        assert_eq!(answer, json!(["acme.notes"]));
        // A call nobody waits on carries none.
        assert_eq!(w.apps.call_as("host", "acme.notes", "app.acme.todo.list", json!({})).await.unwrap(), json!([]));
    }

    #[tokio::test]
    async fn a_hub_capability_that_cannot_be_granted_refuses_the_registration() {
        let w = world();
        for name in ["harnix.turn", "nothing.here", "topic:nowhere"] {
            let e = w.apps.register("host", fake("acme.notes").calls([name])).await.unwrap_err();
            assert_eq!(e.code, codes::REFUSED, "{name}");
            assert!(e.message.contains(name), "{name}: {}", e.message);
        }
        assert!(w.apps.registered().is_empty());
        assert!(lock(&w.stub.topics).is_empty());
        // Another application's names are that application's to admit, so they are not checked here.
        w.apps.register("host", fake("acme.notes").calls(["app.acme.todo.list"])).await.unwrap();
    }

    #[tokio::test]
    async fn collisions_are_refused_and_leave_nothing_behind() {
        let w = world();
        // A compiled-in module's id, with names in the table or not, or nested under one.
        for (id, by) in [("probe.sub", "probe"), ("quiet.x", "quiet")] {
            let e = w.apps.register("host", fake(id)).await.unwrap_err();
            assert_eq!(e.code, codes::REFUSED, "{id}");
            assert_eq!(e.data, Some(json!({"failure": "namespace_taken", "by": by})), "{id}: {}", e.message);
        }
        // The same id as one is not a package's at all, nor is one under a reserved word.
        for id in ["probe", "quiet", "engine.notes"] {
            assert_eq!(w.apps.register("host", fake(id)).await.unwrap_err().code, codes::REFUSED, "{id}");
        }
        // The compiled-in method is still the compiled-in one.
        assert_eq!(call(&w.az, &device(), "app.probe.echo", json!({})).await.unwrap(), json!("the compiled-in probe"));

        // Twice, or nested in a registered one.
        w.apps.register("host", fake("acme.notes")).await.unwrap();
        for id in ["acme.notes", "acme", "acme.notes.pro"] {
            assert_eq!(w.apps.register("host", fake(id)).await.unwrap_err().code, codes::REFUSED, "{id}");
        }
        // A bad id, a bad method name, a duplicate name, a space outside the namespace, a space
        // that is taken: refused, and nothing of them was left declared, bound or listed.
        let bad = [
            AppSpec::new("Bad"),
            AppSpec::new("acme.one").method("a b", &[Principal::Device], |_c, p| async move { Ok(p) }),
            AppSpec::new("acme.two")
                .method("x", &[Principal::Device], |_c, p| async move { Ok(p) })
                .method("x", &[Principal::Device], |_c, p| async move { Ok(p) }),
            AppSpec::new("acme.three").topic("t", &[]).space(SpaceDecl::new("elsewhere")),
            AppSpec::new("acme.taken")
                .topic("t", &[])
                .method("m", &[Principal::Device], |_c, p| async move { Ok(p) })
                .space(SpaceDecl::new("acme.taken")),
        ];
        for spec in bad {
            let id = spec.id.clone();
            assert_eq!(w.apps.register("host", spec).await.unwrap_err().code, codes::REFUSED, "{id}");
        }
        assert_eq!(lock(&w.stub.topics).keys().collect::<Vec<_>>(), ["app.acme.notes.ticks"]);
        assert_eq!(lock(&w.stub.spaces).iter().collect::<Vec<_>>(), ["acme.notes"]);
        assert_eq!(
            call(&w.az, &device(), "app.acme.taken.m", json!({})).await.unwrap_err().code,
            codes::METHOD_NOT_FOUND
        );
        assert_eq!(w.apps.registered().len(), 1);
        let refusals = w.az.audit().recent(100).into_iter().filter(|e| e.decision == "refuse").count();
        assert_eq!(refusals, 13);
    }

    #[tokio::test]
    async fn only_the_registering_module_withdraws_or_publishes() {
        let w = world();
        w.apps.register("host", fake("acme.fake")).await.unwrap();
        assert_eq!(w.apps.withdraw("other", "acme.fake").await.unwrap_err().code, codes::REFUSED);
        assert!(w.apps.publish("other", "acme.fake", "ticks", json!({"job": 1}), json!(1)).await.is_err());
        assert_eq!(w.apps.withdraw("host", "nobody").await.unwrap_err().code, codes::INVALID_PARAMS);

        // Its own topics only, with their parameters, normalized.
        assert!(w.apps.publish("host", "acme.fake", "nope", json!({}), json!(1)).await.is_err());
        assert_eq!(
            w.apps.publish("host", "acme.fake", "ticks", json!({}), json!(1)).await.unwrap_err().code,
            codes::INVALID_PARAMS
        );
        w.apps.publish("host", "acme.fake", "ticks", json!({"job": "a", "extra": 1}), json!({"n": 1})).await.unwrap();
        assert_eq!(
            lock(&w.stub.published).as_slice(),
            [("app.acme.fake.ticks".to_string(), json!({"job": "a"}), json!({"n": 1}))]
        );
        w.apps.withdraw("host", "acme.fake").await.unwrap();
        assert!(w.apps.publish("host", "acme.fake", "ticks", json!({"job": "a"}), json!(2)).await.is_err());
    }

    #[tokio::test]
    async fn a_call_in_flight_finishes_and_the_next_one_is_not_found() {
        let w = world();
        let (started_tx, started) = tokio::sync::oneshot::channel::<()>();
        let started_tx = Arc::new(Mutex::new(Some(started_tx)));
        let release = Arc::new(tokio::sync::Notify::new());
        let spec = {
            let release = release.clone();
            AppSpec::new("acme.slow").method("wait", &[Principal::Device], move |_c, _p| {
                let release = release.clone();
                let started = lock(&started_tx).take();
                async move {
                    if let Some(tx) = started {
                        let _ = tx.send(());
                    }
                    release.notified().await;
                    Ok(json!("finished"))
                }
            })
        };
        w.apps.register("host", spec).await.unwrap();
        let az = w.az.clone();
        let in_flight = tokio::spawn(async move { call(&az, &device(), "app.acme.slow.wait", json!({})).await });
        started.await.unwrap();
        w.apps.withdraw("host", "acme.slow").await.unwrap();
        assert_eq!(
            call(&w.az, &device(), "app.acme.slow.wait", json!({})).await.unwrap_err().code,
            codes::METHOD_NOT_FOUND
        );
        release.notify_one();
        assert_eq!(in_flight.await.unwrap().unwrap(), json!("finished"));
    }

    /// Registering, withdrawing and calling at once: every call is answered by the application
    /// or is not found — never refused for the wrong reason, never handed to another row — and
    /// when it is over the table holds exactly what assembly put in.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn registering_withdrawing_and_calling_at_once() {
        let w = world();
        let rows_before = w.az.table().len();
        let mut tasks = Vec::new();
        for n in 0..4 {
            let apps = w.apps.clone();
            tasks.push(tokio::spawn(async move {
                // Two hosts contend for each id; whichever registers, only it withdraws.
                let id = format!("race{}", n % 2);
                let host = format!("host{n}");
                for _ in 0..200 {
                    if apps.register(&host, fake(&id)).await.is_ok() {
                        tokio::task::yield_now().await;
                        apps.withdraw(&host, &id).await.unwrap();
                    }
                }
            }));
        }
        for n in 0..4 {
            let az = w.az.clone();
            tasks.push(tokio::spawn(async move {
                for _ in 0..2000 {
                    let method = format!("app.race{}.echo", n % 2);
                    match call(&az, &device(), &method, json!({"n": n})).await {
                        Ok(v) => assert_eq!(v["params"]["n"], json!(n)),
                        Err(e) => assert_eq!(e.code, codes::METHOD_NOT_FOUND, "{}", e.message),
                    }
                    // The compiled-in method never moves.
                    assert_eq!(call(&az, &device(), "app.probe.echo", json!({})).await.unwrap(), json!("the compiled-in probe"));
                }
            }));
        }
        for task in tasks {
            task.await.unwrap();
        }
        assert_eq!(w.below.calls.load(Ordering::SeqCst), 0);
        assert_eq!(w.az.table().len(), rows_before);
        assert!(w.apps.registered().is_empty());
        assert!(lock(&w.stub.topics).is_empty());
        assert!(lock(&w.stub.spaces).is_empty());
    }
}
