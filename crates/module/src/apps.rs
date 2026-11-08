//! Hosting runtime applications: names that enter the process after it started.
//!
//! A compiled-in module declares its methods, topics and spaces once, at
//! registration. A **runtime application** (`app.<id>.*`) brings them while
//! the process runs, and takes them away again. This is the kernel's side of
//! that, handed to a module through [`ModuleContext::apps`] — only when the
//! profile says `[apps] enabled = true`; otherwise there is no host and
//! nothing here can be reached.
//!
//! What it guarantees, so whatever sits behind a handler (later, a WASM
//! component) does not have to:
//!
//! - **One admission point.** A runtime application's methods are rows in the
//!   same authorization table as everything else, with the principals it
//!   declared; a call is decided and audited there, like a compiled-in
//!   module's.
//! - **Its own namespace only.** Names are `app.<id>.<name>`. An id that
//!   overlaps a compiled-in module or another registered application is
//!   refused, and so is any name already taken — what was compiled in keeps
//!   precedence. A refusal writes nothing.
//! - **Withdrawal is clean.** [`AppHost::withdraw`] takes the names out of the
//!   table first, so a call decided afterwards is unknown (`-32601`, as if the
//!   name had never existed); calls already running finish. Each subscriber of
//!   its topics is sent `harnix.topic_ended`; its spaces leave
//!   `harnix.spaces.list`.
//! - **On the record.** Every name declared, withdrawn or refused is an audit
//!   entry under the hosting module's subject, with the time.
//! - **It acts as itself.** An application calls and subscribes as the
//!   subject `module:<id>` ([`AppHandle::call`], [`AppHandle::subscribe`]),
//!   through the same admission point and audit as everyone else — and only
//!   to what its [`AppSpec::hub`] lists, plus its own `app.<id>.*`. Those
//!   entries are granted to it exactly as a compiled-in module's manifest
//!   grants are (refused when unknown or the hub's own), and taken back when
//!   it is withdrawn. Anything else it tries is refused, and on the record.
//!
//! [`ModuleContext::apps`]: crate::ModuleContext::apps

use std::future::Future;
use std::sync::Arc;

use harnix_authz::Principal;
use harnix_contract::{RpcError, SpaceDecl, Subject};
use serde::Serialize;
use serde_json::Value;

/// What answers one of a runtime application's methods: its params, and who
/// is calling — already admitted by the authorization table.
#[async_trait::async_trait]
pub trait AppHandler: Send + Sync {
    async fn call(&self, caller: &Subject, params: Value) -> Result<Value, RpcError>;
}

struct FnHandler<F>(F);

#[async_trait::async_trait]
impl<F, Fut> AppHandler for FnHandler<F>
where
    F: Fn(Subject, Value) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<Value, RpcError>> + Send + 'static,
{
    async fn call(&self, caller: &Subject, params: Value) -> Result<Value, RpcError> {
        (self.0)(caller.clone(), params).await
    }
}

/// One method of a runtime application.
#[derive(Clone)]
pub struct AppMethod {
    /// The name inside the application's namespace: `notes.list` is
    /// `app.<id>.notes.list`.
    pub name: String,
    /// Who may call it. Nothing is open by default.
    pub principals: Vec<Principal>,
    pub description: String,
    pub handler: Arc<dyn AppHandler>,
}

/// Where the values of the hub topics an application subscribed to arrive:
/// the frames the hub would send a client, already taken apart.
pub trait AppTopicSink: Send + Sync {
    /// A new whole value of `topic` for these subscription parameters.
    fn deliver(&self, topic: &str, params: &Value, value: &Value);

    /// The topic was withdrawn (`harnix.topic_ended`): the subscription is
    /// gone, and nothing more will come for it.
    fn ended(&self, topic: &str, params: &Value, reason: &str) {
        let _ = (topic, params, reason);
    }
}

/// One topic of a runtime application: `app.<id>.<name>`, whole-value
/// snapshots like a module's.
#[derive(Debug, Clone)]
pub struct AppTopic {
    pub name: String,
    /// The subscription parameters it takes, by name.
    pub params: Vec<String>,
    /// Who may subscribe. Devices and modules unless said otherwise.
    pub principals: Vec<Principal>,
}

/// Everything one runtime application brings, registered all or nothing.
#[derive(Clone)]
pub struct AppSpec {
    /// Dot-separated segments of lowercase letters, digits and `-`, each
    /// starting with a letter: `fake`, `acme.notes`.
    pub id: String,
    pub methods: Vec<AppMethod>,
    pub topics: Vec<AppTopic>,
    /// Space ids are the application's id or `<id>.<name>`; one lands on an
    /// instance that exists (not a template).
    pub spaces: Vec<SpaceDecl>,
    /// What it may call and subscribe to as `module:<id>` beyond its own
    /// `app.<id>.*`: method names, and topics as `topic:<name>` — the
    /// authorization table's own spelling, as in a module's manifest grants.
    /// Nothing unless listed.
    pub hub: Vec<String>,
    /// Where values of the hub topics it subscribes to go. Without one it
    /// cannot subscribe.
    pub topic_sink: Option<Arc<dyn AppTopicSink>>,
}

impl AppSpec {
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            methods: Vec::new(),
            topics: Vec::new(),
            spaces: Vec::new(),
            hub: Vec::new(),
            topic_sink: None,
        }
    }

    /// What it may call and subscribe to as itself ([`AppSpec::hub`]).
    pub fn hub<S: Into<String>>(mut self, names: impl IntoIterator<Item = S>) -> Self {
        self.hub.extend(names.into_iter().map(Into::into));
        self
    }

    /// Where the values of what it subscribes to go.
    pub fn topic_sink(mut self, sink: Arc<dyn AppTopicSink>) -> Self {
        self.topic_sink = Some(sink);
        self
    }

    /// A method answered by an async function of the caller and the params.
    pub fn method<F, Fut>(self, name: &str, principals: &[Principal], f: F) -> Self
    where
        F: Fn(Subject, Value) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Value, RpcError>> + Send + 'static,
    {
        self.method_with(AppMethod {
            name: name.to_string(),
            principals: principals.to_vec(),
            description: String::new(),
            handler: Arc::new(FnHandler(f)),
        })
    }

    pub fn method_with(mut self, method: AppMethod) -> Self {
        self.methods.push(method);
        self
    }

    /// A topic devices and modules may subscribe to, with these parameters.
    pub fn topic(self, name: &str, params: &[&str]) -> Self {
        self.topic_with(AppTopic {
            name: name.to_string(),
            params: params.iter().map(|p| p.to_string()).collect(),
            principals: vec![Principal::Device, Principal::AnyModule],
        })
    }

    pub fn topic_with(mut self, topic: AppTopic) -> Self {
        self.topics.push(topic);
        self
    }

    pub fn space(mut self, space: SpaceDecl) -> Self {
        self.spaces.push(space);
        self
    }
}

/// One registered application, as `harnix.apps.list` and the host report it.
#[derive(Debug, Clone, Serialize)]
pub struct AppSummary {
    pub id: String,
    /// The module that registered it: whose subject its audit entries carry.
    pub by: String,
    /// Full names: `app.<id>.<name>`.
    pub methods: Vec<String>,
    pub topics: Vec<String>,
    pub spaces: Vec<String>,
    /// What it may reach as `module:<id>` beyond its own names (its `hub`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hub: Vec<String>,
    pub since_ms: u64,
}

/// What withdrawing an application took away.
#[derive(Debug, Clone, Serialize)]
pub struct AppWithdrawn {
    pub id: String,
    pub methods: Vec<String>,
    pub topics: Vec<String>,
    pub spaces: Vec<String>,
    /// Subscriptions that were sent `harnix.topic_ended`.
    pub subscriptions_ended: usize,
}

/// The process side, implemented by the application layer.
#[doc(hidden)]
#[async_trait::async_trait]
pub trait AppRuntime: Send + Sync {
    async fn register(&self, by: &str, spec: AppSpec) -> Result<AppSummary, RpcError>;
    async fn withdraw(&self, by: &str, id: &str) -> Result<AppWithdrawn, RpcError>;
    async fn publish(&self, by: &str, id: &str, topic: &str, params: Value, value: Value) -> Result<(), RpcError>;
    fn registered(&self) -> Vec<AppSummary>;

    /// One call through the admission point as `module:<id>`, if its `hub`
    /// lists the method (or the method is its own).
    async fn call_as(&self, by: &str, id: &str, method: &str, params: Value) -> Result<Value, RpcError> {
        let _ = (by, id, params);
        Err(RpcError::refused(format!("this host does not let applications call `{method}`")))
    }

    /// Subscribe as `module:<id>`, if its `hub` lists `topic:<topic>` (or the
    /// topic is its own); values go to its [`AppSpec::topic_sink`]. Answers
    /// the topic's current value.
    async fn subscribe_as(&self, by: &str, id: &str, topic: &str, params: Value) -> Result<Value, RpcError> {
        let _ = (by, id, params);
        Err(RpcError::refused(format!("this host does not let applications subscribe to `{topic}`")))
    }

    async fn unsubscribe_as(&self, by: &str, id: &str, topic: &str, params: Value) -> Result<(), RpcError> {
        let _ = (by, id, topic, params);
        Ok(())
    }
}

/// One module's handle on the runtime-application host. Every registration
/// through it is on the record as this module's.
#[derive(Clone)]
pub struct AppHost {
    runtime: Arc<dyn AppRuntime>,
    by: String,
}

impl AppHost {
    #[doc(hidden)]
    pub fn new(runtime: Arc<dyn AppRuntime>, by: &str) -> Self {
        Self {
            runtime,
            by: by.to_string(),
        }
    }

    /// Register one application: its methods reachable through the
    /// admission point, its topics subscribable, its spaces listed. All or
    /// nothing; a refusal is `REFUSED` with the reason.
    pub async fn register(&self, spec: AppSpec) -> Result<AppHandle, RpcError> {
        let summary = self.runtime.register(&self.by, spec).await?;
        Ok(AppHandle {
            host: self.clone(),
            id: summary.id,
        })
    }

    /// Withdraw an application this module registered. Only the module that
    /// registered one may withdraw it.
    pub async fn withdraw(&self, id: &str) -> Result<AppWithdrawn, RpcError> {
        self.runtime.withdraw(&self.by, id).await
    }

    /// Every application registered now, by any module.
    pub fn registered(&self) -> Vec<AppSummary> {
        self.runtime.registered()
    }
}

/// One registered application: publish on its topics, withdraw it. Cheap to
/// clone, so a handler can hold one; after withdrawal every call fails.
#[derive(Clone)]
pub struct AppHandle {
    host: AppHost,
    id: String,
}

impl AppHandle {
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Publish the whole current value of `app.<id>.<topic>` for these
    /// subscription parameters (`Value::Null` or `{}` for none).
    pub async fn publish(&self, topic: &str, params: Value, value: impl Serialize) -> Result<(), RpcError> {
        let value = serde_json::to_value(value)
            .map_err(|e| RpcError::new(harnix_contract::codes::INTERNAL_ERROR, e.to_string()))?;
        self.host.runtime.publish(&self.host.by, &self.id, topic, params, value).await
    }

    pub async fn withdraw(&self) -> Result<AppWithdrawn, RpcError> {
        self.host.withdraw(&self.id).await
    }

    /// Call a method as the application itself (`module:<id>`): only what
    /// its `hub` lists, and its own `app.<id>.*`. Admitted and audited like
    /// any call; a refusal is `REFUSED` and on the record too.
    pub async fn call(&self, method: &str, params: Value) -> Result<Value, RpcError> {
        self.host.runtime.call_as(&self.host.by, &self.id, method, params).await
    }

    /// Subscribe to a hub topic as the application: only `topic:<name>`
    /// entries of its `hub`, and its own topics. Values arrive at its
    /// [`AppSpec::topic_sink`]; the answer is the current value.
    pub async fn subscribe(&self, topic: &str, params: Value) -> Result<Value, RpcError> {
        self.host.runtime.subscribe_as(&self.host.by, &self.id, topic, params).await
    }

    pub async fn unsubscribe(&self, topic: &str, params: Value) -> Result<(), RpcError> {
        self.host.runtime.unsubscribe_as(&self.host.by, &self.id, topic, params).await
    }
}
