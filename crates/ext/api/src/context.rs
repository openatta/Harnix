//! What an extension is handed at start: its whole reach into the process.
//!
//! Its id, what it was granted, its section of the profile, its directory, its credentials,
//! calls through the admission point as itself, and the services below — each checking the
//! capability it needs when it is used, never when the context is made. There is no hub, no
//! engine link, no route table and no data root in it.

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use harnix_contract::s1::{caps, Capability, ExtId, InstanceTrust, Subject, Trust};
use harnix_contract::sync::{read, write};
use harnix_engine_api::readers::{Recordings, Transcripts};
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::bulk::Bulk;
use crate::credentials::Credentials;
use crate::instances::Instances;
use crate::internal::{Ports, View};
use crate::sessions::Sessions;
use crate::topics::Topics;
use crate::ui::UiPublisher;
use crate::{internal_error, lacks, Rpc};

/// A section of the profile that did not read as its extension's configuration.
#[derive(Debug, Clone)]
pub struct ConfigError {
    section: String,
    message: String,
}

impl ConfigError {
    pub(crate) fn new(id: &ExtId, message: impl Into<String>) -> Self {
        Self { section: format!("[ext.{id}]"), message: message.into() }
    }

    /// The section, as written in a profile: `[ext.<id>]`.
    pub fn section(&self) -> &str {
        &self.section
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.section, self.message)
    }
}

impl std::error::Error for ConfigError {}

/// A section read as `C`. An absent one reads as an empty table, so a type whose fields all have
/// defaults always reads; failing that, as nothing at all, so `()` does.
pub(crate) fn read_config<C: DeserializeOwned>(id: &ExtId, value: &Value) -> Result<C, ConfigError> {
    match value {
        Value::Null => serde_json::from_value(Value::Object(Default::default()))
            .or_else(|_| serde_json::from_value(Value::Null))
            .map_err(|e| ConfigError::new(id, e.to_string())),
        written => serde_json::from_value(written.clone()).map_err(|e| ConfigError::new(id, e.to_string())),
    }
}

/// What an extension was granted, and how far it is trusted.
#[derive(Debug, Clone)]
pub struct GrantsView {
    caps: Vec<Capability>,
    trust: Trust,
}

impl GrantsView {
    pub(crate) fn new(caps: Vec<Capability>, trust: Trust) -> Self {
        Self { caps, trust }
    }

    pub fn has(&self, cap: &Capability) -> bool {
        self.caps.contains(cap)
    }

    pub fn capabilities(&self) -> impl Iterator<Item = &Capability> {
        self.caps.iter()
    }

    pub fn trust(&self) -> &Trust {
        &self.trust
    }
}

/// Where this process accepts connections.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listen {
    addrs: Vec<SocketAddr>,
    url: String,
}

impl Listen {
    pub(crate) fn new(addrs: Vec<SocketAddr>, url: String) -> Self {
        Self { addrs, url }
    }

    /// Every address it accepts on — both loopback families when bound to loopback — with the
    /// port the system chose when asked for any.
    pub fn addrs(&self) -> &[SocketAddr] {
        &self.addrs
    }

    /// The one to give a person or a tool: `http(s)://<first addr>/`. It carries no token.
    pub fn url(&self) -> &str {
        &self.url
    }
}

/// This process: its versions, how long it has run, its memory and threads, who is connected.
#[derive(Debug, Clone)]
pub struct ProcessInfo {
    value: Value,
}

impl ProcessInfo {
    pub(crate) fn new(value: Value) -> Self {
        Self { value }
    }

    /// All of it, as `observability.process` shows it.
    pub fn to_json(&self) -> Value {
        self.value.clone()
    }

    pub fn uptime_secs(&self) -> Option<u64> {
        self.value.get("uptime_secs").and_then(Value::as_u64)
    }
}

/// A read-only view of one instance. Not a way to call it: that is a method, through
/// [`ExtensionContext::call`].
#[derive(Clone)]
pub struct EngineView {
    view: View,
    ext: ExtensionContext,
}

impl EngineView {
    pub fn instance(&self) -> &str {
        &self.view.0.id
    }

    pub fn active_scenes(&self) -> &[String] {
        &self.view.0.active_scenes
    }

    pub fn model(&self) -> &str {
        &self.view.0.model
    }

    pub fn trust(&self) -> InstanceTrust {
        self.view.0.trust.clone()
    }

    /// The model calls its sessions made and what they cost. Needs `recordings.read`.
    pub fn recordings(&self) -> Rpc<Arc<dyn Recordings>> {
        self.ext.require(&caps::RECORDINGS_READ)?;
        self.view
            .recordings()
            .ok_or_else(|| crate::RpcError::missing(format!("`{}` keeps no recordings here", self.instance())))
    }

    /// Its sessions' transcripts, to search. Needs `transcripts.read`.
    pub fn transcripts(&self) -> Rpc<Arc<dyn Transcripts>> {
        self.ext.require(&caps::TRANSCRIPTS_READ)?;
        self.view
            .transcripts()
            .ok_or_else(|| crate::RpcError::missing(format!("`{}`'s transcripts cannot be read here", self.instance())))
    }
}

/// Paired devices: listing, pairing, revoking. The admission of a paired device stays the
/// kernel's; this is managing who is paired.
pub struct Devices<'a> {
    ext: &'a ExtensionContext,
}

impl Devices<'_> {
    pub fn list(&self) -> Value {
        self.ext.ports().devices().list()
    }

    /// Start pairing a device called `label`: `{handle, code, expires_in_secs}`. The code is in
    /// the clear only here, to be shown once.
    pub fn pair_begin(&self, label: &str) -> Value {
        self.ext.ports().devices().pair_begin(label)
    }

    /// Finish pairing with the code the device was shown, and its public key.
    pub fn pair_complete(&self, code: &str, public_key: &str) -> Rpc<Value> {
        self.ext.ports().devices().pair_complete(code, public_key)
    }

    /// Forget a device, and end every channel it holds. Whether it was paired.
    pub fn revoke(&self, device: &str) -> bool {
        self.ext.ports().devices().revoke(device)
    }
}

/// A named service of the host, beyond what every context has: its name, and the type it is
/// handed out as. What capability it takes is the host's that provides it, checked there against
/// the asking extension's grants — never something a type declared on the asking side could say.
pub trait Service: 'static {
    const NAME: &'static str;
    type Handle: ?Sized + Send + Sync + 'static;
}

struct Shared {
    id: ExtId,
    config: Value,
    data_dir: PathBuf,
    grants: GrantsView,
    ports: Ports,
    /// Its topics, by full name: what it may publish on, with their parameters.
    topics: Vec<(String, Vec<String>)>,
    state: RwLock<HashMap<TypeId, Arc<dyn Any + Send + Sync>>>,
}

/// One extension's context. Cheap to clone; every clone is the same extension.
#[derive(Clone)]
pub struct ExtensionContext {
    shared: Arc<Shared>,
}

impl ExtensionContext {
    pub(crate) fn assemble(
        id: ExtId,
        config: Value,
        data_dir: PathBuf,
        grants: GrantsView,
        ports: Ports,
        topics: Vec<(String, Vec<String>)>,
    ) -> Self {
        Self {
            shared: Arc::new(Shared {
                id,
                config,
                data_dir,
                grants,
                ports,
                topics,
                state: RwLock::new(HashMap::new()),
            }),
        }
    }

    pub fn id(&self) -> &ExtId {
        &self.shared.id
    }

    pub fn grants(&self) -> &GrantsView {
        &self.shared.grants
    }

    /// Its section, `[ext.<id>]`, read as `C`. An absent section reads as an empty table.
    pub fn config<C: DeserializeOwned>(&self) -> Result<C, crate::ConfigError> {
        read_config(&self.shared.id, &self.shared.config)
    }

    /// Its own directory, made before `migrate`: apart from Harnix's state and from every other
    /// extension's.
    pub fn data_dir(&self) -> &Path {
        &self.shared.data_dir
    }

    /// Where this product's projects live — what a picker opens on, and where a new project is
    /// made — as the person running it said (`--data-dir`). A starting point, not a fence.
    pub fn projects_dir(&self) -> &Path {
        self.shared.ports.projects_dir()
    }

    /// The directory this product keeps in each project — `.harnix`, or a branded application's
    /// `.<product_id>` — and the name its engine goes by. What an extension writes for a project
    /// goes in `product().data_dir_of(project)`, the engine's project tier beside it; never a
    /// `.harnix` of its own.
    pub fn product(&self) -> &harnix_engine_api::ProductNames {
        self.shared.ports.product()
    }

    pub fn credentials(&self) -> &dyn Credentials {
        self.shared.ports.credentials()
    }

    /// Call any method it may reach, as itself: the admission point a device goes through, and
    /// the same audit. A call made while answering one is [`Call::call`](crate::Call::call),
    /// which says so.
    pub async fn call(&self, method: &str, params: Value) -> Rpc<Value> {
        self.shared.ports.call_root(self.subject(), method, params).await
    }

    /// Drive sessions as a client does: create, send, interrupt, close, move, entries, tap.
    pub fn sessions(&self) -> Sessions<'_> {
        Sessions::new(self)
    }

    /// Build, ensure and stop instances from templates.
    pub fn instances(&self) -> Instances<'_> {
        Instances::new(self)
    }

    /// Its own topics to publish on, and any topic to subscribe to as itself.
    pub fn topics(&self) -> Topics<'_> {
        Topics::new(self)
    }

    /// What it may know of one instance, when there is one by that id.
    pub fn engine(&self, instance: &str) -> Option<EngineView> {
        self.shared.ports.instance(instance).map(|view| EngineView { view, ext: self.clone() })
    }

    /// This process. Needs `process.observe`.
    pub fn process(&self) -> Rpc<ProcessInfo> {
        self.require(&caps::PROCESS_OBSERVE)?;
        Ok(ProcessInfo::new(self.shared.ports.process()))
    }

    /// One-shot upload grants: what a client sends a large payload with.
    pub fn bulk(&self) -> Bulk<'_> {
        Bulk::new(self)
    }

    /// Interface entries it publishes while it runs.
    pub fn ui(&self) -> UiPublisher<'_> {
        UiPublisher::new(self)
    }

    /// Paired devices. Needs `devices.manage`.
    pub fn devices(&self) -> Rpc<Devices<'_>> {
        self.require(&caps::DEVICES_MANAGE)?;
        Ok(Devices { ext: self })
    }

    /// A named service of the host, if this extension holds the capability the host says it
    /// takes: refused as any capability is (`data.capability`) when it does not.
    pub fn service<S: Service>(&self) -> Rpc<Arc<S::Handle>> {
        let handle = self.shared.ports.service(&self.shared.id, S::NAME)?;
        handle
            .downcast_ref::<Arc<S::Handle>>()
            .cloned()
            .ok_or_else(|| internal_error(format!("the service `{}` is not what it was declared as", S::NAME)))
    }

    /// Where this process listens, once it does. Extensions start **before** the listener is
    /// bound, so this is `None` throughout `start`: wait in a task of its own on
    /// [`ExtensionContext::listening`].
    pub fn listen(&self) -> Option<Listen> {
        self.shared.ports.listen_now()
    }

    /// [`ExtensionContext::listen`], waiting until the listener is bound. Never returns in a
    /// process that does not serve.
    pub async fn listening(&self) -> Listen {
        self.shared.ports.listening().await
    }

    /// Keep a value for its handlers: `cx.state::<T>()`, `call.state::<T>()`.
    pub fn provide<T: Send + Sync + 'static>(&self, value: T) {
        self.provide_arc(Arc::new(value));
    }

    pub fn provide_arc<T: Send + Sync + 'static>(&self, value: Arc<T>) {
        write(&self.shared.state).insert(TypeId::of::<T>(), value);
    }

    pub fn state<T: Send + Sync + 'static>(&self) -> Rpc<Arc<T>> {
        read(&self.shared.state)
            .get(&TypeId::of::<T>())
            .cloned()
            .and_then(|v| v.downcast::<T>().ok())
            .ok_or_else(|| {
                internal_error(format!("extension `{}` has not provided a {}", self.shared.id, std::any::type_name::<T>()))
            })
    }

    pub(crate) fn subject(&self) -> Subject {
        Subject::extension(self.shared.id.clone())
    }

    pub(crate) fn ports(&self) -> &Ports {
        &self.shared.ports
    }

    /// Its topic `name` (a full name) and the parameters it takes, when it declared it.
    pub(crate) fn own_topic(&self, name: &str) -> Option<&[String]> {
        self.shared.topics.iter().find(|(n, _)| n == name).map(|(_, p)| p.as_slice())
    }

    /// `REFUSED` unless it holds `cap` now.
    pub(crate) fn require(&self, cap: &Capability) -> Rpc<()> {
        match self.shared.ports.holds(&self.shared.id, cap) {
            true => Ok(()),
            false => Err(lacks(cap)),
        }
    }
}
