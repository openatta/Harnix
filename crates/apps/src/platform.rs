//! The host's [`Kernel`], over this process's kernel: what an application embedded here is lent.
//! (Its [`harnix_app_port::Agent`], this process's Agix, is `agent.rs`.)
//!
//! - **Its names** are registered with the kernel's application host
//!   (`harnix_module::AppHost`), under the `apps` module: each `[[method]]` a row in the
//!   authorization table with the principals its manifest names (and always the kernel, and the
//!   application itself), answered by [`Control::call_method`]; its topics and spaces beside them.
//! - **What it does as itself** — `host.call`, `publish`, `subscribe` — goes through that
//!   registration, as `module:<id>`: the kernel admits, grants and audits. The values of what it
//!   subscribed to go back to the host ([`Control::deliver`]), in order.
//!
//! The kernel is reached through the `apps` module's context, which every call of the
//! `AppsRuntime` hands in; nothing here reaches it before the module has.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock, RwLock, Weak};

use async_trait::async_trait;
use harnix_app_package::acquire::{internal, refused};
use harnix_app_port::{
    CallAs, Control, Kernel, MethodCall, Progress, Publish, Registration, Subscription, TopicValue,
};
use harnix_contract::sync::{read, write};
use harnix_contract::{codes, RpcError, Subject};
use harnix_module::{
    AppHandle, AppHandler, AppMethod, AppTopic, AppTopicSink, ModuleContext, Principal,
};
use serde_json::{json, Value};
use tokio::sync::mpsc;

use crate::runtime::Registry;

pub struct KernelPlatform {
    /// The `apps` module's context, from the first call it made.
    cx: RwLock<Option<ModuleContext>>,
    /// The host this lends to: what the kernel's handlers and topic forwarders call. Weak, so the
    /// kernel holding a handler does not hold the host.
    control: OnceLock<Weak<dyn Control>>,
    /// Each registered application's registration.
    handles: RwLock<BTreeMap<String, AppHandle>>,
    pub(crate) registry: Arc<Registry>,
}

impl KernelPlatform {
    pub fn new(registry: Arc<Registry>) -> Self {
        Self {
            cx: RwLock::new(None),
            control: OnceLock::new(),
            handles: RwLock::new(BTreeMap::new()),
            registry,
        }
    }

    pub fn lend_to(&self, control: Weak<dyn Control>) {
        let _ = self.control.set(control);
    }

    /// The `apps` module's context: every call it makes into the runtime hands it in.
    pub fn bind(&self, cx: &ModuleContext) {
        let mut slot = write(&self.cx);
        if slot.is_none() {
            *slot = Some(cx.clone());
        }
    }

    pub(crate) fn cx(&self) -> Result<ModuleContext, RpcError> {
        read(&self.cx)
            .clone()
            .ok_or_else(|| internal("the `apps` module has not reached the application runtime yet"))
    }

    fn handle(&self, app: &str) -> Result<AppHandle, RpcError> {
        read(&self.handles).get(app).cloned().ok_or_else(|| {
            RpcError::new(
                codes::INTERNAL_ERROR,
                format!("`{app}` is not registered with this process's application host"),
            )
        })
    }
}

/// One `[[method]]` as the kernel has it: answered by the host.
struct Forward {
    app: String,
    name: String,
    control: Weak<dyn Control>,
}

/// Where a call's chain (`CallAs::chain`) rides from [`KernelPlatform::call`] to [`Forward`]: in
/// its params, as the transport's `__client` does, since all the kernel carries between the two is
/// a subject and params. Put there only for a call into a registered application's method, and
/// taken out before its backend sees them. Whoever else sets it can only have its own call
/// refused.
const CHAIN: &str = "__chain";

/// `params` with `chain` in them for a call of `method`, when that is one of `apps`' methods.
fn with_chain<'a>(
    mut params: Value,
    method: &str,
    chain: Vec<String>,
    mut apps: impl Iterator<Item = &'a String>,
) -> Value {
    let into_app = method
        .strip_prefix("app.")
        .is_some_and(|rest| apps.any(|id| rest.strip_prefix(id.as_str()).is_some_and(|r| r.starts_with('.'))));
    if chain.is_empty() || !into_app {
        return params;
    }
    if params.is_null() {
        params = json!({});
    }
    // Params that are not an object have nowhere to carry it: such a call is left to the VM's
    // own check, and to the backend's timeout.
    if let Some(obj) = params.as_object_mut() {
        obj.insert(CHAIN.into(), json!(chain));
    }
    params
}

/// The chain a call came with, taken out of its params.
fn take_chain(params: &mut Value) -> Vec<String> {
    params
        .as_object_mut()
        .and_then(|obj| obj.remove(CHAIN))
        .and_then(|chain| serde_json::from_value(chain).ok())
        .unwrap_or_default()
}

#[async_trait]
impl AppHandler for Forward {
    async fn call(&self, caller: &Subject, mut params: Value) -> Result<Value, RpcError> {
        let chain = take_chain(&mut params);
        let Some(control) = self.control.upgrade() else {
            let mut e = RpcError::new(
                codes::INTERNAL_ERROR,
                format!("`app.{}.{}`: the application is not running", self.app, self.name),
            );
            e.data = Some(json!({"app": self.app, "failure": "stopped"}));
            return Err(e);
        };
        control
            .call_method(MethodCall {
                app: self.app.clone(),
                method: self.name.clone(),
                caller: caller.clone(),
                params,
                chain,
            })
            .await
    }
}

/// Values of an application's subscriptions waiting for the host. Past this the application is not
/// keeping up with what it subscribed to, and dropping beats growing without bound; the host's own
/// queue counts what it drops after this one.
const TOPIC_QUEUE: usize = 256;

/// The kernel's frames for an application's subscriptions, to the host in the order they came.
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
                let value = TopicValue {
                    app: name.clone(),
                    topic,
                    value,
                };
                let _ = control.deliver(value).await;
            }
        });
        Self::onto(app, tx)
    }

    /// Onto `tx`, whoever takes from it.
    fn onto(app: &str, tx: mpsc::Sender<(String, Value)>) -> Self {
        Self {
            app: app.to_string(),
            tx,
            dropped: AtomicU64::new(0),
        }
    }
}

impl AppTopicSink for TopicValues {
    fn deliver(&self, topic: &str, _params: &Value, value: &Value) {
        if let Err(mpsc::error::TrySendError::Full(_)) =
            self.tx.try_send((topic.to_string(), value.clone()))
        {
            let dropped = self.dropped.fetch_add(1, Ordering::Relaxed) + 1;
            tracing::warn!(app = %self.app, %topic, dropped, "an application's topic queue is full; a value dropped");
        }
    }

    fn ended(&self, topic: &str, _params: &Value, reason: &str) {
        tracing::info!(%topic, %reason, "a topic an application subscribed to ended");
    }
}

/// Who may call one of an application's methods: the subjects it names, and always the kernel. A
/// name that is none of them is refused rather than read as one: the manifest checks it, but a
/// registration may come from over a link, and an unknown subject must not open a method to
/// anybody.
fn principals(app: &str, method: &str, subjects: &[String]) -> Result<Vec<Principal>, RpcError> {
    let mut principals = Vec::new();
    for subject in subjects {
        principals.push(match subject.as_str() {
            "device" => Principal::Device,
            "module" => Principal::AnyModule,
            "kernel" => Principal::Kernel,
            other => {
                return Err(refused(format!(
                    "`app.{app}.{method}` names the subject `{other}`, and a method's subject is \
                     `device`, `module` or `kernel`"
                )))
            }
        });
    }
    principals.push(Principal::Kernel);
    principals.dedup();
    Ok(principals)
}

#[async_trait]
impl Kernel for KernelPlatform {
    async fn register(&self, r: Registration) -> Result<(), RpcError> {
        // Before anything is registered, and whether or not there is a host to register with.
        for m in &r.methods {
            principals(&r.app, &m.name, &m.subjects)?;
        }
        let cx = self.cx()?;
        // A process without an application host: nothing is declared, and what the application
        // does as itself is refused as not registered.
        let Some(host) = cx.apps() else {
            return Ok(());
        };
        let control = self
            .control
            .get()
            .cloned()
            .ok_or_else(|| internal("this platform is lent to no application host"))?;
        let mut spec = harnix_module::AppSpec::new(r.app.clone())
            .hub(r.hub.clone())
            .topic_sink(Arc::new(TopicValues::new(&r.app, control.clone())));
        for m in r.methods {
            let principals = principals(&r.app, &m.name, &m.subjects)?;
            spec = spec.method_with(AppMethod {
                name: m.name.clone(),
                principals,
                description: m.description,
                handler: Arc::new(Forward {
                    app: r.app.clone(),
                    name: m.name,
                    control: control.clone(),
                }),
            });
        }
        for t in r.topics {
            spec = spec.topic_with(AppTopic {
                name: t,
                params: Vec::new(),
                principals: vec![Principal::Device, Principal::AnyModule],
            });
        }
        for space in r.spaces {
            spec = spec.space(space);
        }
        let handle = host.register(spec).await?;
        write(&self.handles).insert(r.app, handle);
        Ok(())
    }

    async fn withdraw(&self, app: String) -> Result<(), RpcError> {
        let handle = write(&self.handles).remove(&app);
        match handle {
            Some(handle) => handle.withdraw().await.map(|_| ()),
            None => Err(RpcError::invalid_params(format!("`{app}` is not registered"))),
        }
    }

    async fn call(&self, call: CallAs) -> Result<Value, RpcError> {
        let handle = self.handle(&call.app)?;
        let params = with_chain(call.params, &call.method, call.chain, read(&self.handles).keys());
        handle.call(&call.method, params).await
    }

    async fn publish(&self, p: Publish) -> Result<(), RpcError> {
        self.handle(&p.app)?
            .publish(&p.topic, json!({}), p.value)
            .await
    }

    async fn subscribe(&self, s: Subscription) -> Result<Value, RpcError> {
        self.handle(&s.app)?.subscribe(&s.topic, s.params).await
    }

    async fn unsubscribe(&self, s: Subscription) -> Result<(), RpcError> {
        self.handle(&s.app)?.unsubscribe(&s.topic, s.params).await
    }

    async fn progress(&self, p: Progress) -> Result<(), RpcError> {
        self.registry.told(&p.call_id, &p.text);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_calls_chain_rides_into_an_applications_method_and_nowhere_else() {
        let apps = ["acme.notes".to_string(), "acme".to_string()];
        let chain = || vec!["acme.notes".to_string()];
        // Into a registered application's method: carried, and taken out again whole.
        let mut params = with_chain(json!({"x": 1}), "app.acme.notes.list", chain(), apps.iter());
        assert_eq!(params, json!({"x": 1, "__chain": ["acme.notes"]}));
        assert_eq!(take_chain(&mut params), chain());
        assert_eq!(params, json!({"x": 1}), "the backend sees its own params only");
        assert_eq!(with_chain(Value::Null, "app.acme.x", chain(), apps.iter()), json!({"__chain": ["acme.notes"]}));
        // A kernel method, a compiled-in module's (`app.probe.*`), a name that only shares a
        // prefix, or no chain at all: untouched.
        for method in ["harnix.send", "app.probe.echo", "app.acmeish.x"] {
            assert_eq!(with_chain(json!({"x": 1}), method, chain(), apps.iter()), json!({"x": 1}), "{method}");
        }
        assert_eq!(with_chain(json!({"x": 1}), "app.acme.x", Vec::new(), apps.iter()), json!({"x": 1}));
        // What a caller made up is no chain.
        assert!(take_chain(&mut json!({"__chain": "acme"})).is_empty());
    }

    /// A subscription's values the host does not take fast enough: past the queue they are
    /// dropped and counted, never waited for — the kernel publishes on, and what is queued keeps
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
        let named = |subjects: &[&str]| {
            principals("acme.notes", "save", &subjects.iter().map(|s| s.to_string()).collect::<Vec<_>>())
        };
        assert_eq!(named(&["device"]).unwrap(), [Principal::Device, Principal::Kernel]);
        assert_eq!(
            named(&["module", "kernel"]).unwrap(),
            [Principal::AnyModule, Principal::Kernel]
        );
        assert_eq!(named(&[]).unwrap(), [Principal::Kernel]);
        let err = named(&["device", "everyone"]).unwrap_err();
        assert_eq!(err.code, codes::REFUSED);
        assert!(err.message.contains("`app.acme.notes.save` names the subject `everyone`"), "{}", err.message);
    }
}
