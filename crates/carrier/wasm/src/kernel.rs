//! The host's [`Kernel`], over each package's extension handle: what a package in this process is
//! lent. (Its [`harnix_wasm_port::Agent`] is `agent.rs`.)
//!
//! - **Its names**: when the host starts a package it registers it here, and this is where its
//!   descriptor is made — each `[[method]]` a method answered by [`Control::call_method`], its
//!   topics published by its backend, its spaces, its tools from `[[tool]]`, its interface's files
//!   as a route, trusted by the consent its component recorded — and
//!   registered through that component's registrar: on the record as the component's, by the
//!   same host path a native extension takes.
//! - **What it does as itself** — `host.call`, `publish`, `subscribe` — goes through its handle,
//!   as `ext:<id>`: the router admits, grants and audits. The values of what it subscribed to go
//!   back to the host ([`Control::deliver`]), in order.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, RwLock, Weak};

use async_trait::async_trait;
use harnix_carrier_shared::sync::{lock, read, write};
use base64::Engine as _;
use harnix_ext::{codes, RpcError};
use harnix_ext::engine::ToolSpec;
use harnix_ext::{Access, Call, CallMeta, DescriptorBuilder, ExtId, ExtensionContext, MethodHandler, PackageCarrier, Trust};
use harnix_host_api::{Availability, ExtensionHandle, HostServices, Installed, TopicSink};
use harnix_package::acquire::{internal, refused};
use harnix_package::egress::{self, Egress, Request};
use harnix_wasm_port::{
    CallAs, Control, Fetch, Fetched, Kernel, MethodCall, Progress, Publish, Registration, Subscription, TopicValue, TurnChain,
    TurnRef,
};
use serde_json::{json, Value};
use tokio::sync::mpsc;

use crate::registry::Registry;
use harnix_carrier_shared::UiFiles;

pub(crate) struct CarrierKernel {
    /// The component's context, from the first package it started.
    cx: RwLock<Option<ExtensionContext>>,
    /// The host this lends to: what the methods' forwarders and topic values call. Weak, so the
    /// router holding a handler does not hold the host.
    control: OnceLock<Weak<dyn Control>>,
    /// What each package being started was installed with, until it is registered.
    pending: Mutex<BTreeMap<String, Installed>>,
    /// Each registered package's handle.
    handles: RwLock<BTreeMap<String, ExtensionHandle>>,
    pub(crate) registry: Arc<Registry>,
    /// Where a package's HTTP goes out, once decided.
    pub(crate) egress: Egress,
}

impl CarrierKernel {
    pub fn new(registry: Arc<Registry>) -> Self {
        Self {
            egress: Egress::new(),
            cx: RwLock::new(None),
            control: OnceLock::new(),
            pending: Mutex::new(BTreeMap::new()),
            handles: RwLock::new(BTreeMap::new()),
            registry,
        }
    }

    pub fn lend_to(&self, control: Weak<dyn Control>) {
        let _ = self.control.set(control);
    }

    /// The component's context: every start hands it in.
    pub fn bind(&self, cx: &ExtensionContext) {
        let mut slot = write(&self.cx);
        if slot.is_none() {
            *slot = Some(cx.clone());
        }
    }

    pub(crate) fn cx(&self) -> Result<ExtensionContext, RpcError> {
        read(&self.cx).clone().ok_or_else(|| internal("no package has been started on this carrier yet"))
    }

    /// What `installed` is started with: read when its host registers it.
    pub fn expect(&self, installed: Installed) {
        lock(&self.pending).insert(installed.id().to_string(), installed);
    }

    pub fn forget(&self, app: &str) {
        lock(&self.pending).remove(app);
    }

    /// Whether `app`, whose `net` is `net`, may send `url`: what is sent, when it may. Its host
    /// checks first, by the same rule: one past it is on the record as refused.
    pub(crate) fn allowed(&self, app: &str, net: &[String], url: &str) -> Result<egress::Allowed, String> {
        egress::allowed(net, url).inspect_err(|why| {
            tracing::warn!(package = %app, %why, "a package's request past its host's check of `net`");
            if let Ok(handle) = self.handle(app) {
                handle.refused("proc.fetch", "a host outside its net");
            }
        })
    }

    pub(crate) fn handle(&self, app: &str) -> Result<ExtensionHandle, RpcError> {
        read(&self.handles)
            .get(app)
            .cloned()
            .ok_or_else(|| RpcError::new(codes::INTERNAL_ERROR, format!("`{app}` is not registered by this carrier")))
    }
}

/// One `[[method]]` as the router has it: answered by the host.
struct Forward {
    app: String,
    name: String,
    control: Weak<dyn Control>,
}

#[async_trait]
impl MethodHandler for Forward {
    async fn call(&self, call: Call, params: Value) -> harnix_ext::Rpc<Value> {
        let Some(control) = self.control.upgrade() else {
            let mut e = RpcError::new(codes::EXTENSION_UNAVAILABLE, format!("`{}.{}`: the package is not running", self.app, self.name));
            e.data = Some(json!({"failure": "stopped"}));
            return Err(e);
        };
        // Its id, the call it was made in, its deadline, and the extensions waiting on it: what the
        // host refuses re-entry by.
        let call_meta = call.meta();
        let method = MethodCall { app: self.app.clone(), method: self.name.clone(), caller: call.subject().clone(), params };
        call.run(control.call_method(call_meta, method)).await
    }
}

/// Values of a package's subscriptions waiting for the host. Past this the package is not keeping
/// up with what it subscribed to, and dropping beats growing without bound; the host's own queue
/// counts what it drops after this one.
const TOPIC_QUEUE: usize = 256;

/// The router's values for a package's subscriptions, to the host in the order they came.
struct TopicValues {
    app: String,
    tx: mpsc::Sender<(String, Value)>,
    /// What a full queue dropped, all told.
    dropped: AtomicU64,
}

impl TopicValues {
    fn new(app: &str, control: Weak<dyn Control>) -> Self {
        let (tx, mut rx) = mpsc::channel::<(String, Value)>(TOPIC_QUEUE);
        let name = app.to_string();
        tokio::spawn(async move {
            while let Some((topic, value)) = rx.recv().await {
                let Some(control) = control.upgrade() else { break };
                let _ = control.deliver(CallMeta::default(), TopicValue { app: name.clone(), topic, value }).await;
            }
        });
        Self::onto(app, tx)
    }

    /// Onto `tx`, whoever takes from it.
    fn onto(app: &str, tx: mpsc::Sender<(String, Value)>) -> Self {
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
        tracing::info!(%topic, %reason, "a topic a package subscribed to ended");
    }
}

/// Who may call one of a package's methods: the subjects it names, and always the package itself
/// and the kernel. A name that is none of them is refused rather than read as one: the manifest
/// checks it, but a registration may come from over a link, and an unknown subject must not open
/// a method to anybody.
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
pub(crate) fn tool_specs(app: &str, tools: &[harnix_wasm_port::ToolDef]) -> Vec<ToolSpec> {
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

#[async_trait]
impl Kernel for CarrierKernel {
    async fn register(&self, _meta: CallMeta, r: Registration) -> Result<(), RpcError> {
        let id = ExtId::package(&r.app).map_err(|e| refused(e.to_string()))?;
        // Before anything is registered.
        let accesses = r
            .methods
            .iter()
            .map(|m| access(&id, &m.name, &m.subjects))
            .collect::<Result<Vec<_>, _>>()?;
        let installed = lock(&self.pending)
            .remove(&r.app)
            .ok_or_else(|| internal(format!("`{}` was started without being installed through this carrier", r.app)))?;
        let control = self.control.get().cloned().ok_or_else(|| internal("this carrier lends its kernel to no host"))?;
        let manifest = harnix_package::acquire::manifest_in(installed.pkg_dir())?;
        let calls = installed.calls().iter().cloned();
        let ext_manifest = harnix_ext::Manifest::new(&r.app, installed.version())
            .title(manifest.app.title.as_deref().unwrap_or(&r.app))
            .disclosure(manifest.app.description.as_deref().unwrap_or_default());
        let mut d = DescriptorBuilder::new(id.clone(), ext_manifest, Trust::consent(calls, PackageCarrier::Wasm));
        for (m, access) in r.methods.into_iter().zip(accesses) {
            d.method(&format!("{}.{}", r.app, m.name))
                .access(access)
                .describe(&m.description)
                .with(Arc::new(Forward { app: r.app.clone(), name: m.name, control: control.clone() }));
        }
        for topic in &r.topics {
            d.topic(&format!("{}.{topic}", r.app)).access(Access::device().with_extension(id.clone()));
        }
        for space in r.spaces {
            d.space(space);
        }
        for spec in tool_specs(&r.app, &r.tools) {
            d.tool(spec);
        }
        d.route("ui", Access::device(), UiFiles::new(self.registry.clone()));
        let host = self.cx()?.service::<HostServices>()?;
        let handle = host.registrar().register(d.build(), installed.capabilities().to_vec()).await?;
        handle.mark(Availability::Ready);
        write(&self.handles).insert(r.app.clone(), handle);
        Ok(())
    }

    async fn withdraw(&self, _meta: CallMeta, app: String) -> Result<(), RpcError> {
        let handle = write(&self.handles).remove(&app);
        match handle {
            Some(handle) => handle.withdraw().await.map(|_| ()),
            None => Err(RpcError::invalid_params(format!("`{app}` is not registered"))),
        }
    }

    /// The host is this process's own code: its word for who waits on the call is taken.
    async fn call(&self, meta: CallMeta, call: CallAs) -> Result<Value, RpcError> {
        self.handle(&call.app)?.call(meta, &call.method, call.params).await
    }

    async fn publish(&self, _meta: CallMeta, p: Publish) -> Result<(), RpcError> {
        self.handle(&p.app)?.publish(&p.topic, json!({}), p.value).await
    }

    async fn subscribe(&self, _meta: CallMeta, s: Subscription) -> Result<Value, RpcError> {
        let control = self.control.get().cloned().ok_or_else(|| internal("this carrier lends its kernel to no host"))?;
        let sink = Arc::new(TopicValues::new(&s.app, control));
        self.handle(&s.app)?.subscribe(&s.topic, s.params, sink).await
    }

    async fn unsubscribe(&self, _meta: CallMeta, s: Subscription) -> Result<(), RpcError> {
        self.handle(&s.app)?.unsubscribe(&s.topic, s.params).await
    }

    /// A package's request, sent only to the hosts its `net` lists: decided here.
    async fn fetch(&self, _meta: CallMeta, f: Fetch) -> Result<Fetched, RpcError> {
        let net = self.registry.net(&f.app).ok_or_else(|| refused(format!("`{}` is not running", f.app)))?;
        let url = self.allowed(&f.app, &net, &f.url).map_err(refused)?;
        let body = match f.body_b64 {
            Some(body) => Some(base64::engine::general_purpose::STANDARD.decode(body).map_err(|e| RpcError::invalid_params(e.to_string()))?),
            None => None,
        };
        let request = Request { method: f.method, url, headers: f.headers, body };
        let response = self.egress.send(request).await.map_err(refused)?;
        Ok(Fetched {
            status: response.status,
            headers: response.headers,
            body_b64: base64::engine::general_purpose::STANDARD.encode(response.body),
        })
    }

    /// Every engine of this process reads its turns' chains itself.
    async fn turn_chain(&self, _meta: CallMeta, t: TurnRef) -> Result<TurnChain, RpcError> {
        Ok(TurnChain { instance: t.instance, session_id: t.session_id, chain: Vec::new() })
    }

    fn progress(&self, p: Progress) {
        self.registry.told(&p.call_id, &p.text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A subscription's values the host does not take fast enough: past the queue they are
    /// dropped and counted, never waited for — the router publishes on, and what is queued keeps
    /// its order.
    #[test]
    fn values_past_the_topic_queue_are_dropped_and_counted() {
        let (tx, mut rx) = mpsc::channel(TOPIC_QUEUE);
        let sink = TopicValues::onto("acme.notes", tx);
        for n in 0..TOPIC_QUEUE + 3 {
            sink.deliver("harnix.sessions.status", &Value::Null, &json!(n));
        }
        assert_eq!(sink.dropped.load(Ordering::Relaxed), 3);
        for n in 0..TOPIC_QUEUE {
            assert_eq!(rx.try_recv().unwrap(), ("harnix.sessions.status".to_string(), json!(n)));
        }
        assert!(rx.try_recv().is_err(), "a value past the queue was kept");
        // Room again: the next value is taken.
        sink.deliver("harnix.sessions.status", &Value::Null, &json!("again"));
        assert_eq!(rx.try_recv().unwrap().1, json!("again"));
        assert_eq!(sink.dropped.load(Ordering::Relaxed), 3);
    }

    #[test]
    fn a_subject_is_one_of_three_or_the_registration_is_refused() {
        let id = ExtId::package("acme.notes").unwrap();
        let named = |subjects: &[&str]| access(&id, "save", &subjects.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        let device = named(&["device"]).unwrap();
        assert!(device.opens_devices() && device.opens_extension(&id) && !device.opens_every_extension());
        let modules = named(&["module", "kernel"]).unwrap();
        assert!(!modules.opens_devices() && modules.opens_every_extension());
        let own = named(&[]).unwrap();
        assert!(!own.opens_devices() && own.opens_extension(&id));
        let err = named(&["device", "everyone"]).unwrap_err();
        assert_eq!(err.code, codes::REFUSED);
        assert!(err.message.contains("`acme.notes.save` names the subject `everyone`"), "{}", err.message);
    }
}
