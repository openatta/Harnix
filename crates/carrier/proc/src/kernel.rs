//! The product's end of one package's extension wire: what its process is answered, and what the
//! product registered for it before it started.
//!
//! - **Its names are the product's to register.** Before its process is started, its descriptor is
//!   made here from its manifest and its consent — each `[[method]]` forwarded to the process
//!   (`ctl.call_method`), its topics published by it, its spaces, its tools from `[[tool]]`, its
//!   interface's files and its tool endpoint as routes — and registered through the component's
//!   registrar, marked starting. The process's `proc.register` only says what it runs: it must be
//!   within what was registered, with the digest of the manifest consented to, or it is refused
//!   and the process is ended ([`Connection::violation`]). Its `proc.withdraw` is counted and
//!   ignored: only the product takes rows out.
//! - **What it does as itself** — `proc.call`, `publish`, `subscribe` — goes through its handle,
//!   as `<id>`: the router admits, grants and audits. The wire has already re-signed each call:
//!   its id is the product's, its chain worked out from the product's own call it was made for.
//!   The values of what it subscribed to go back over the wire (`ctl.deliver`), in order; its
//!   subscriptions are the process's, and end with it.
//! - **Its HTTP** (`proc.fetch`) is sent only to the hosts its `net` lists: decided here.
//! - **The handshake** (`proc.hello`): a process of another build — another wire, Harnix, Agix or
//!   VM engine than this product's — is refused before anything else is said.

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};

use async_trait::async_trait;
use base64::Engine as _;
use harnix_ext::engine::ToolSpec;
use harnix_ext::{codes, Access, Call, CallMeta, DescriptorBuilder, ExtId, ExtensionDescriptor, MethodHandler, PackageCarrier, RpcError, Trust};
use harnix_host_api::{AdoptedEngine, ExtensionHandle, Installed, TopicSink};
use harnix_package::acquire::{internal, refused};
use harnix_package::egress::{self, Egress, Request};
use harnix_package::manifest::Manifest;
use harnix_wasm_port::rpc::{ControlClient, Wire};
use harnix_wasm_port::socket::{Answer, Hello, Inbound, KernelAnswer, SocketWire, HELLO};
use harnix_ext::DefaultInstance;
use harnix_wasm_port::{
    CallAs, Control, Fetch, Fetched, Kernel, MethodCall, MethodDecl, Progress, Publish, Registration, SpaceDecl, Subscription,
    ToolDef, TopicValue, TurnChain, TurnRef,
};
use serde_json::{json, Value};
use tokio::sync::{mpsc, watch};

use crate::registry::Registry;
use harnix_carrier_shared::{McpEndpoint, UiFiles};
use harnix_carrier_shared::sync::lock;

/// What a process says, as the audit names a refusal of it.
const REGISTER: &str = "proc.register";
const FETCH: &str = "proc.fetch";

/// A `[[tool]]`, as the port carries it.
pub(crate) fn declared_tool(t: &harnix_package::manifest::Tool) -> ToolDef {
    ToolDef {
        name: t.name.clone(),
        description: t.description.clone(),
        doc: t.doc.clone(),
        input_schema: t.input_schema_json(),
        read_only: t.read_only,
        concurrency_safe: t.concurrency_safe,
    }
}

/// What a process running the package `manifest` describes must register: its names as its
/// manifest has them, and the digest of the manifest consented to.
pub(crate) fn declared(manifest: &Manifest, digest: &str) -> Registration {
    Registration {
        app: manifest.app.id.clone(),
        methods: manifest
            .methods
            .iter()
            .map(|m| MethodDecl {
                name: m.name.clone(),
                description: m.description.clone().unwrap_or_default(),
                subjects: m.subject.names().iter().map(|s| s.to_string()).collect(),
            })
            .collect(),
        topics: manifest.topics.iter().map(|t| t.name.clone()).collect(),
        spaces: manifest
            .spaces
            .iter()
            .map(|space| {
                let mut decl = SpaceDecl::new(space.id.clone());
                if let Some(instance) = &space.instance {
                    decl = decl.instance(instance);
                }
                if let Some(scene) = &space.scene {
                    decl = decl.scene(scene.clone());
                }
                decl
            })
            .collect(),
        calls: manifest.capabilities.calls.clone(),
        tools: manifest.tools.iter().map(declared_tool).collect(),
        digest: digest.to_string(),
    }
}

/// How what a process registered is not within what was registered for it, if it is not: which
/// check it failed, as the audit records it, and why.
pub(crate) fn exceeds(expected: &Registration, got: &Registration) -> Option<(&'static str, String)> {
    if got.app != expected.app {
        return Some(("another package's registration", format!("it registered `{}`, and it is `{}`", got.app, expected.app)));
    }
    if got.digest != expected.digest {
        return Some(("not the manifest consented to", "its manifest is not the one consented to (its digest differs)".into()));
    }
    past(expected, got).map(|why| ("past its consent", why))
}

/// A name a process registered that its manifest does not declare, or opened wider.
fn past(expected: &Registration, got: &Registration) -> Option<String> {
    for m in &got.methods {
        match expected.methods.iter().find(|e| e.name == m.name) {
            None => return Some(format!("it registered the method `{}`, which its manifest does not declare", m.name)),
            Some(e) if e.subjects != m.subjects => {
                return Some(format!("it opened the method `{}` to {:?}; its manifest says {:?}", m.name, m.subjects, e.subjects))
            }
            Some(_) => {}
        }
    }
    let over = |what: &str, got: &[String], expected: &[String]| {
        got.iter()
            .find(|g| !expected.contains(g))
            .map(|g| format!("it registered the {what} `{g}`, which its manifest does not declare"))
    };
    let names = |spaces: &[SpaceDecl]| spaces.iter().map(|s| s.id.clone()).collect::<Vec<_>>();
    let tools = |tools: &[ToolDef]| tools.iter().map(|t| t.name.clone()).collect::<Vec<_>>();
    over("topic", &got.topics, &expected.topics)
        .or_else(|| over("space", &names(&got.spaces), &names(&expected.spaces)))
        .or_else(|| over("call", &got.calls, &expected.calls))
        .or_else(|| over("tool", &tools(&got.tools), &tools(&expected.tools)))
}

/// Who may call one of a package's methods: the subjects it names, and always the package itself
/// and the kernel. A name that is none of them is refused rather than read as one.
fn access(app: &ExtId, method: &str, subjects: &[String]) -> Result<Access, RpcError> {
    let mut access = Access::kernel().with_extension(app.clone());
    for subject in subjects {
        access = match subject.as_str() {
            "device" => access.with_device(),
            "module" => access.with_extensions(),
            "kernel" => access,
            other => {
                return Err(refused(format!(
                    "`{app}.{method}` names the subject `{other}`, and a method's subject is `device`, `module` or `kernel`"
                )))
            }
        };
    }
    Ok(access)
}

/// A package's tools as the descriptor lists them, from its manifest's `[[tool]]`.
fn tool_specs(app: &str, tools: &[ToolDef]) -> Vec<ToolSpec> {
    tools
        .iter()
        .map(|t| {
            let schema = serde_json::from_str(&t.input_schema).unwrap_or_else(|_| json!({"type": "object"}));
            let mut spec = ToolSpec::new(&harnix_package::tool_name(app, &t.name), &t.description, schema).source(&format!("ext:{app}"));
            if t.read_only {
                spec = spec.read_only();
            }
            spec
        })
        .collect()
}

/// The package's descriptor, from what was installed and consented to: the carrier's handlers over
/// its declared names. With an engine of its own (`engine`, the instance it is adopted as), its
/// spaces' sessions are there, whatever instance its manifest names.
pub(crate) fn descriptor(
    installed: &Installed,
    manifest: &Manifest,
    r: &Registration,
    registry: &Arc<Registry>,
    engine: Option<&str>,
) -> Result<ExtensionDescriptor, RpcError> {
    let id = ExtId::package(&r.app).map_err(|e| refused(e.to_string()))?;
    let accesses = r.methods.iter().map(|m| access(&id, &m.name, &m.subjects)).collect::<Result<Vec<_>, _>>()?;
    let ext_manifest = harnix_ext::Manifest::new(&r.app, installed.version())
        .title(manifest.app.title.as_deref().unwrap_or(&r.app))
        .disclosure(manifest.app.description.as_deref().unwrap_or_default());
    let calls = installed.calls().iter().cloned();
    let mut d = DescriptorBuilder::new(id.clone(), ext_manifest, Trust::consent(calls, PackageCarrier::Process));
    for (m, access) in r.methods.iter().zip(accesses) {
        d.method(&format!("{}.{}", r.app, m.name))
            .access(access)
            .describe(&m.description)
            .with(Arc::new(Forward { app: r.app.clone(), name: m.name.clone(), registry: Arc::downgrade(registry) }));
    }
    for topic in &r.topics {
        d.topic(&format!("{}.{topic}", r.app)).access(Access::device().with_extension(id.clone()));
    }
    for space in &r.spaces {
        d.space(space_on(&r.app, space, engine));
    }
    for spec in tool_specs(&r.app, &r.tools) {
        d.tool(spec);
    }
    d.route("ui", Access::device(), UiFiles::new(registry.clone()));
    d.route("mcp", Access::engine(), McpEndpoint::new(registry.clone()));
    Ok(d.build())
}

/// One `[[space]]`, its sessions on the package's own engine when it has one.
fn space_on(app: &str, space: &SpaceDecl, engine: Option<&str>) -> SpaceDecl {
    let Some(engine) = engine else { return space.clone() };
    if let DefaultInstance::Fixed(named) = &space.instance {
        if named != engine && named != harnix_ext::MAIN_INSTANCE {
            tracing::info!(package = %app, space = %space.id, instance = %named, "its space's sessions are on its own engine, not the instance its manifest names");
        }
    }
    let mut on = space.clone();
    on.instance = DefaultInstance::Fixed(engine.to_string());
    on
}

/// One `[[method]]` as the router has it: answered by the package's process, while it is up.
struct Forward {
    app: String,
    name: String,
    registry: Weak<Registry>,
}

#[async_trait]
impl MethodHandler for Forward {
    /// A method wakes a process let go while idle.
    async fn call(&self, call: Call, params: Value) -> harnix_ext::Rpc<Value> {
        let registry = self.registry.upgrade();
        let _held = registry.as_ref().map(|r| r.hold(&self.app));
        let control = match &registry {
            Some(registry) => registry.awake(&self.app).await,
            None => None,
        };
        let Some(control) = control else {
            let mut e = RpcError::new(codes::EXTENSION_UNAVAILABLE, format!("`{}.{}`: its process is not up", self.app, self.name));
            e.data = Some(json!({"failure": "starting"}));
            return Err(e);
        };
        // Its id, the call it was made in, its deadline, and the extensions waiting on it: what the
        // process refuses re-entry by.
        let meta = call.meta();
        let method = MethodCall { app: self.app.clone(), method: self.name.clone(), caller: call.subject().clone(), params };
        call.run(control.call_method(meta, method)).await
    }
}

/// Values of a package's subscriptions waiting for its process. Past this the package is not
/// keeping up with what it subscribed to, and dropping beats growing without bound.
const TOPIC_QUEUE: usize = 256;

/// The router's values for a subscription of the process's, over its wire in the order they came.
struct TopicValues {
    app: String,
    tx: mpsc::Sender<(String, Value)>,
    dropped: AtomicU64,
}

impl TopicValues {
    fn new(app: &str, wire: Weak<SocketWire>) -> Self {
        let (tx, mut rx) = mpsc::channel::<(String, Value)>(TOPIC_QUEUE);
        let name = app.to_string();
        tokio::spawn(async move {
            while let Some((topic, value)) = rx.recv().await {
                let Some(wire) = wire.upgrade() else { break };
                let control = ControlClient(wire as Arc<dyn Wire>);
                let _ = control.deliver(CallMeta::default(), TopicValue { app: name.clone(), topic, value }).await;
            }
        });
        Self { app: app.to_string(), tx, dropped: AtomicU64::new(0) }
    }
}

impl TopicSink for TopicValues {
    fn deliver(&self, topic: &str, _params: &Value, value: &Value) {
        if let Err(mpsc::error::TrySendError::Full(_)) = self.tx.try_send((topic.to_string(), value.clone())) {
            let dropped = self.dropped.fetch_add(1, Ordering::Relaxed) + 1;
            tracing::warn!(package = %self.app, %topic, dropped, "a package's topic queue is full; a value dropped");
        }
    }

    fn ended(&self, topic: &str, _params: &Value, reason: &str) {
        tracing::info!(%topic, %reason, "a topic a package's process subscribed to ended");
    }
}

/// What one connection was told that its keeper acts on: the hello, and a registration refused.
#[derive(Default)]
pub(crate) struct Told {
    /// The process's hello, once it came and was this build's; why it was refused otherwise.
    pub hello: Mutex<Option<Result<Hello, String>>>,
    pub hello_seen: tokio::sync::Notify,
    /// Why its `proc.register` was refused: the process is to be ended.
    pub violation: watch::Sender<Option<String>>,
    /// `proc.withdraw`s ignored.
    pub withdraws: AtomicU64,
    pub registered: AtomicBool,
}

/// The product's end of one process of a package: what answers its `proc.*`.
pub(crate) struct Connection {
    app: String,
    handle: ExtensionHandle,
    expected: Registration,
    net: Vec<String>,
    egress: Arc<Egress>,
    registry: Arc<Registry>,
    wire: Weak<SocketWire>,
    pub(crate) told: Arc<Told>,
    /// What it subscribed to as itself: given back when it goes.
    subscriptions: Mutex<BTreeSet<(String, String)>>,
    /// Its own engine, adopted, when it has one: whose turns' chains it may ask for.
    engine: Option<Arc<dyn AdoptedEngine>>,
}

/// What a [`Connection`] is made with, besides its wire.
pub(crate) struct Parts {
    pub app: String,
    pub handle: ExtensionHandle,
    pub expected: Registration,
    pub net: Vec<String>,
    pub egress: Arc<Egress>,
    pub registry: Arc<Registry>,
    pub engine: Option<Arc<dyn AdoptedEngine>>,
}

impl Connection {
    /// The product's end of `stream`, the wire of one process of `parts.app`: answered here, with
    /// `ours` the build a hello must be.
    pub(crate) fn open(stream: tokio::net::UnixStream, parts: Parts, ours: Hello, options: harnix_wasm_port::socket::Options) -> (Arc<SocketWire>, Arc<Connection>) {
        let told = Arc::new(Told::default());
        let mut made = None;
        let app = parts.app.clone();
        let wire = SocketWire::serve_with(
            stream,
            app.clone(),
            |weak| {
                let connection = Arc::new(Connection {
                    app: parts.app,
                    handle: parts.handle,
                    expected: parts.expected,
                    net: parts.net,
                    egress: parts.egress,
                    registry: parts.registry,
                    wire: weak,
                    told: told.clone(),
                    subscriptions: Mutex::new(BTreeSet::new()),
                    engine: parts.engine,
                });
                made = Some(connection.clone());
                Arc::new(ProductEnd { connection: connection.clone(), ours, kernel: KernelAnswer(connection) })
            },
            options,
        );
        (wire, made.expect("the answerer is made with the wire"))
    }

    fn check(&self, app: &str) -> Result<(), RpcError> {
        if app != self.app {
            return Err(refused(format!("this is the extension process of `{}`, not of `{app}`", self.app)));
        }
        Ok(())
    }

    /// What its process subscribed to, given back: its subscriptions end with it.
    pub(crate) async fn end(&self) {
        let subscriptions = std::mem::take(&mut *lock(&self.subscriptions));
        for (topic, params) in subscriptions {
            let params = serde_json::from_str(&params).unwrap_or(Value::Null);
            let _ = self.handle.unsubscribe(&topic, params).await;
        }
    }

    /// Why its registration was refused, once it was.
    pub(crate) fn violation(&self) -> watch::Receiver<Option<String>> {
        self.told.violation.subscribe()
    }
}

/// `proc.hello`, then the kernel: nothing of the kernel before a hello of this build.
struct ProductEnd {
    connection: Arc<Connection>,
    ours: Hello,
    kernel: KernelAnswer,
}

#[async_trait]
impl Answer for ProductEnd {
    async fn call(&self, call: Inbound, args: Value) -> Result<Value, RpcError> {
        let told = &self.connection.told;
        if call.method != HELLO {
            if !matches!(&*lock(&told.hello), Some(Ok(_))) {
                return Err(RpcError::new(codes::HANDSHAKE_REFUSED, "`proc.hello` comes first"));
            }
            return self.kernel.call(call, args).await;
        }
        let answer = match serde_json::from_value::<Hello>(args) {
            Err(e) => Err(format!("its hello did not read: {e}")),
            Ok(hello) if hello.ext != self.connection.app => {
                Err(format!("it says it is `{}`, and it was started as `{}`", hello.ext, self.connection.app))
            }
            Ok(hello) => match hello.differs(&self.ours) {
                Some(why) => Err(why),
                None => Ok(hello),
            },
        };
        let mut slot = lock(&told.hello);
        if slot.is_some() {
            return Err(RpcError::new(codes::HANDSHAKE_REFUSED, "a second `proc.hello`"));
        }
        *slot = Some(answer.clone());
        drop(slot);
        told.hello_seen.notify_waiters();
        match answer {
            Ok(_) => Ok(json!({})),
            Err(why) => {
                tracing::warn!(package = %self.connection.app, %why, "an extension process's hello was refused");
                self.connection.handle.refused(HELLO, "not a process of this build for it");
                Err(Hello::refused(why))
            }
        }
    }

    fn notify(&self, method: &str, params: Value) {
        if matches!(&*lock(&self.connection.told.hello), Some(Ok(_))) {
            self.kernel.notify(method, params);
        }
    }
}

#[async_trait]
impl Kernel for Connection {
    /// Within what the product registered for it, with the digest consented to — or refused, and
    /// the process ended.
    async fn register(&self, _meta: CallMeta, r: Registration) -> Result<(), RpcError> {
        if let Some((check, why)) = exceeds(&self.expected, &r) {
            tracing::warn!(package = %self.app, %why, "an extension process registered what was not consented to; ending it");
            self.handle.refused(REGISTER, check);
            self.told.violation.send_replace(Some(why.clone()));
            return Err(refused(format!("`{}`: {why}", self.app)));
        }
        self.told.registered.store(true, Ordering::Release);
        Ok(())
    }

    /// Counted and ignored: its rows are the product's to take out.
    async fn withdraw(&self, _meta: CallMeta, app: String) -> Result<(), RpcError> {
        self.check(&app)?;
        self.told.withdraws.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    /// Re-signed by the wire already: its chain the product's own, from the call it was made for.
    async fn call(&self, meta: CallMeta, call: CallAs) -> Result<Value, RpcError> {
        self.check(&call.app)?;
        self.handle.call(meta, &call.method, call.params).await
    }

    async fn publish(&self, _meta: CallMeta, p: Publish) -> Result<(), RpcError> {
        self.check(&p.app)?;
        self.handle.publish(&p.topic, json!({}), p.value).await
    }

    async fn subscribe(&self, _meta: CallMeta, s: Subscription) -> Result<Value, RpcError> {
        self.check(&s.app)?;
        let sink = Arc::new(TopicValues::new(&s.app, self.wire.clone()));
        let value = self.handle.subscribe(&s.topic, s.params.clone(), sink).await?;
        lock(&self.subscriptions).insert((s.topic, s.params.to_string()));
        Ok(value)
    }

    async fn unsubscribe(&self, _meta: CallMeta, s: Subscription) -> Result<(), RpcError> {
        self.check(&s.app)?;
        lock(&self.subscriptions).remove(&(s.topic.clone(), s.params.to_string()));
        self.handle.unsubscribe(&s.topic, s.params).await
    }

    /// A package's request, sent only to the hosts its `net` lists: decided here.
    async fn fetch(&self, _meta: CallMeta, f: Fetch) -> Result<Fetched, RpcError> {
        self.check(&f.app)?;
        let body = match f.body_b64 {
            Some(body) => Some(base64::engine::general_purpose::STANDARD.decode(body).map_err(|e| RpcError::invalid_params(e.to_string()))?),
            None => None,
        };
        if let Err(why) = egress::allowed(&self.net, &f.url) {
            // Its own host checks first: a request past it is a process not to be taken at its word.
            tracing::warn!(package = %self.app, %why, "an extension process asked for a host outside its `net`");
            self.handle.refused(FETCH, "a host outside its net");
            return Err(refused(why));
        }
        let request = Request { method: f.method, url: f.url, headers: f.headers, body };
        let response = self.egress.send(request).await.map_err(refused)?;
        Ok(Fetched {
            status: response.status,
            headers: response.headers,
            body_b64: base64::engine::general_purpose::STANDARD.encode(response.body),
        })
    }

    /// A turn on its own engine, as the hub runs it; none for any other instance's — the chains
    /// of the product's turns reach the process with each tool call.
    async fn turn_chain(&self, _meta: CallMeta, t: TurnRef) -> Result<TurnChain, RpcError> {
        let chain = match &self.engine {
            Some(engine) if engine.instance() == t.instance => engine.turn_chain(&t.session_id),
            _ => Vec::new(),
        };
        Ok(TurnChain { instance: t.instance, session_id: t.session_id, chain })
    }

    fn progress(&self, p: Progress) {
        if p.app == self.app {
            self.registry.told(&p.call_id, &p.text);
        }
    }
}

/// What a start says over the wire, from what was installed: the package and run directories are
/// the process's own, from its command line, and never go over it.
pub(crate) fn start_of(installed: &Installed, parts: Value, limits: harnix_wasm_port::Limits) -> harnix_wasm_port::Start {
    harnix_wasm_port::Start {
        app: installed.id().to_string(),
        version: installed.version().to_string(),
        trust: installed.trust_level().to_string(),
        cwasm_sha256: installed.cwasm_digest().map(str::to_string),
        config: installed.config_value().clone(),
        secrets: installed.secret_values().clone(),
        parts,
        limits,
        pkg: None,
        run: None,
    }
}

/// The digest of the manifest consented to, as its record keeps it.
pub(crate) fn consented_digest(installed: &Installed) -> Result<String, RpcError> {
    installed
        .install_record()
        .get("digest")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| internal(format!("`{}`'s record names no manifest digest", installed.id())))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registration() -> Registration {
        Registration {
            app: "acme.notes".into(),
            methods: vec![MethodDecl { name: "add".into(), description: String::new(), subjects: vec!["device".into()] }],
            topics: vec!["notes".into()],
            spaces: vec![SpaceDecl::new("acme.notes".to_string())],
            calls: vec!["harnix.session.create".into()],
            tools: Vec::new(),
            digest: "d1".into(),
        }
    }

    #[test]
    fn a_registration_within_the_consented_one_passes_and_one_more_name_does_not() {
        let expected = registration();
        assert_eq!(exceeds(&expected, &expected), None);
        let mut fewer = expected.clone();
        fewer.topics.clear();
        fewer.methods.clear();
        assert_eq!(exceeds(&expected, &fewer), None, "less than consented is within it");

        let mut more = expected.clone();
        more.methods.push(MethodDecl { name: "wipe".into(), description: String::new(), subjects: vec!["device".into()] });
        assert!(exceeds(&expected, &more).unwrap().1.contains("`wipe`"));
        let mut wider = expected.clone();
        wider.methods[0].subjects.push("module".into());
        assert!(exceeds(&expected, &wider).unwrap().1.contains("opened the method `add`"));
        let mut calls = expected.clone();
        calls.calls.push("harnix.session.delete".into());
        assert!(exceeds(&expected, &calls).unwrap().1.contains("harnix.session.delete"));
        let mut other = expected.clone();
        other.digest = "d2".into();
        assert!(exceeds(&expected, &other).unwrap().1.contains("digest"));
        assert_eq!(exceeds(&expected, &other).unwrap().0, "not the manifest consented to");
        assert_eq!(exceeds(&expected, &more).unwrap().0, "past its consent");
        let mut someone = expected.clone();
        someone.app = "acme.other".into();
        assert!(exceeds(&expected, &someone).unwrap().1.contains("acme.other"));
    }
}
