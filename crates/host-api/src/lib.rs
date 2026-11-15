//! What a host component reaches beyond the extension interface.
//!
//! A **host component** is written in an extension's shape and comes in through the same
//! registration and grants — the host component that installs packages is one, and so is each
//! carrier a package runs on — but needs a few pipes an ordinary extension must not see:
//! registering extensions while the process runs, starting an extension process, configuring a
//! spawned daemon's tool endpoints, adopting an extension process's engine. Those are here, and
//! an extension reaches them as one service, [`HostServices`], which takes `extensions.manage`.
//!
//! This is not the extension interface: it is the seam between the skeleton and its host
//! components, changed when the process model changes, and an ordinary extension does not depend
//! on it.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::Arc;

use harnix_contract::s1::{caps, CallMeta, Capability, ExtId, PackageCarrier};
use harnix_contract::ChainLink;
use harnix_engine_api::ToolProvider;
use harnix_ext::http::RouteHandler;
use harnix_ext::{ExtensionContext, ExtensionDescriptor, Rpc, Service};
use serde::Serialize;
use serde_json::Value;
use tokio::sync::{mpsc, watch};

/// The host's pipes, as the service a context hands out to an extension granted
/// `extensions.manage`: `cx.service::<HostServices>()`.
pub struct HostServices;

impl Service for HostServices {
    const NAME: &'static str = "harnix.host";
    const CAPABILITY: Capability = caps::EXTENSIONS_MANAGE;
    type Handle = dyn HostApi;
}

/// What the host does for one host component. Every registration and every process it starts
/// through this is on the record as that component's.
#[async_trait::async_trait]
pub trait HostApi: Send + Sync {
    /// Registering extensions while the process runs.
    fn registrar(&self) -> DynamicRegistrar;

    /// Start an extension process: the host makes the socket pair, binds the extension's subject
    /// to the product's end, applies the sandbox and hands the child to its supervisor. The
    /// sandbox is the host's to apply; a carrier cannot leave it out.
    fn spawn_child(&self, id: &ExtId, req: ChildRequest) -> Rpc<ChildHandle>;

    /// The sandbox the extension processes this component starts are in, as the host applies it:
    /// what its list says of them.
    fn child_sandbox(&self) -> ChildSandbox;

    /// The tool endpoints every daemon this process spawns is configured with: replaces what this
    /// component shared before.
    fn share_tools(&self, entries: Vec<ToolEndpoint>) -> Rpc<()>;

    /// Adopt an extension process's own engine as the instance `proc.<id>`, before its process
    /// first starts: listed, as an extension's engine, starting until a run of the process hands
    /// over its engine's stream ([`AdoptedEngine::attach`]). Only for an extension this component
    /// registered.
    async fn adopt(&self, id: &ExtId, spec: AdoptSpec) -> Rpc<Arc<dyn AdoptedEngine>>;

    /// Let go of what was adopted for `id`.
    async fn release(&self, id: &ExtId);

    /// The interface entries of the extensions this component runs, as the page is told them in
    /// `harnix.ui`: a trusted package's modules, every other one's frames. Replaces what this
    /// component published before, and pushes `harnix.ui`.
    fn publish_ui(&self, entries: Vec<Value>);

    /// The profile's `[ext."<id>"]` sections no linked extension owns and whose id is a package's:
    /// each package's configuration over its manifest's `[config]`. One for a package that is not
    /// installed is this component's to report.
    fn package_sections(&self) -> BTreeMap<String, Value>;
}

/// One MCP endpoint a spawned daemon is configured with: its server name, and its URL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolEndpoint {
    name: String,
    url: String,
}

impl ToolEndpoint {
    pub fn new(name: &str, url: &str) -> Self {
        Self { name: name.to_string(), url: url.to_string() }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn url(&self) -> &str {
        &self.url
    }
}

/// The operating-system sandbox an extension process is started in. The component's own section
/// says whether there is one (`sandbox = "off"` turns it off, for development); the host says how,
/// or why there cannot be one here.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChildSandbox {
    /// Applied, by this mechanism (`sandbox-exec`, `landlock`).
    On(String),
    /// Turned off: processes start unconfined.
    Off,
    /// Required, and not to be had here: no process starts, for this reason.
    Unavailable(String),
}

/// When an extension process is started again after it exits.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RestartPolicy {
    /// Never: it is gone until asked for again.
    Never,
    /// After a failure, backing off, until it fails too often.
    #[default]
    OnFailure,
}

/// When a started extension process counts as up.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Readiness {
    /// Once it has said hello on its wire.
    #[default]
    Hello,
    /// At once.
    Started,
}

/// What to start an extension process with.
#[derive(Debug, Clone, Default)]
pub struct ChildRequest {
    program: Option<PathBuf>,
    pkg_dir: Option<PathBuf>,
    run_dir: Option<PathBuf>,
    with_engine: bool,
    restart: RestartPolicy,
    readiness: Readiness,
}

impl ChildRequest {
    pub fn new() -> Self {
        Self::default()
    }

    /// The program, when not this binary.
    pub fn program(mut self, program: PathBuf) -> Self {
        self.program = Some(program);
        self
    }

    pub fn pkg_dir(mut self, dir: PathBuf) -> Self {
        self.pkg_dir = Some(dir);
        self
    }

    pub fn run_dir(mut self, dir: PathBuf) -> Self {
        self.run_dir = Some(dir);
        self
    }

    /// With an engine of its own, on a second stream.
    pub fn with_engine(mut self) -> Self {
        self.with_engine = true;
        self
    }

    pub fn restart(mut self, policy: RestartPolicy) -> Self {
        self.restart = policy;
        self
    }

    pub fn readiness(mut self, readiness: Readiness) -> Self {
        self.readiness = readiness;
        self
    }

    pub fn program_path(&self) -> Option<&PathBuf> {
        self.program.as_ref()
    }

    pub fn has_engine(&self) -> bool {
        self.with_engine
    }

    pub fn pkg(&self) -> Option<&PathBuf> {
        self.pkg_dir.as_ref()
    }

    pub fn run(&self) -> Option<&PathBuf> {
        self.run_dir.as_ref()
    }

    pub fn restart_policy(&self) -> RestartPolicy {
        self.restart
    }
}

/// An extension process, on the product's side, kept for one run of it: the host's supervisor
/// starts it again after an exit nobody asked for, and says so here, each start with the
/// product's end of its wire. Its subject is bound to those ends; dropped, the process is ended and
/// the subject revoked. A run ends with [`ChildHandle::stop`] (or once the host gives up on it); a
/// process that is to come back after that is started with a new handle.
pub struct ChildHandle {
    id: ExtId,
    run: Box<dyn ChildRun>,
}

impl ChildHandle {
    pub fn new(id: ExtId, run: Box<dyn ChildRun>) -> Self {
        Self { id, run }
    }

    pub fn id(&self) -> &ExtId {
        &self.id
    }

    /// What happened to it next; `None` once the run is over.
    pub async fn next(&mut self) -> Option<ChildEvent> {
        self.run.next().await
    }

    /// Kill the process that runs now, outright: it counts as an exit like any other, and it is
    /// started again. For one that is up and does not answer.
    pub fn kill(&self) {
        self.run.kill()
    }

    /// End the process — stdin closed, a grace, TERM, a shorter grace, KILL — and start it no more.
    pub async fn stop(self) {
        self.run.stop().await
    }

    /// The last lines its process wrote to its stderr, across the run's starts, oldest first: a few
    /// dozen, a few KB at most. Each one went to the product's log as well.
    pub fn stderr(&self) -> Vec<String> {
        self.run.stderr()
    }
}

/// The host's side of a [`ChildHandle`].
#[async_trait::async_trait]
pub trait ChildRun: Send + Sync {
    async fn next(&mut self) -> Option<ChildEvent>;
    fn kill(&self);
    async fn stop(self: Box<Self>);
    /// The last lines of its stderr; none kept, none told.
    fn stderr(&self) -> Vec<String> {
        Vec::new()
    }
}

/// What happened to an extension process, in order.
#[non_exhaustive]
#[derive(Debug)]
pub enum ChildEvent {
    /// It started: the product's end of its wire (fd 3 in the process), and of its engine's
    /// (fd 4) when it was asked for one.
    Started {
        pid: u32,
        wire: std::os::unix::net::UnixStream,
        engine: Option<std::os::unix::net::UnixStream>,
    },
    /// It exited: with `code`, or killed by a signal (`None`).
    Exited { code: Option<i32>, ran_for: std::time::Duration },
    /// It is started again after this wait.
    Restarting { after: std::time::Duration },
    /// It could not be started.
    SpawnFailed { error: String },
    /// It is not started again: the run is over.
    GaveUp { reason: String },
    /// [`ChildHandle::stop`] ended it.
    Stopped,
}

/// What wakes an idle extension process when a session of its engine is asked for: started
/// again, and answered once it is ready — its engine attached again included.
#[async_trait::async_trait]
pub trait Waker: Send + Sync {
    async fn wake(&self) -> Rpc<()>;
}

/// How an extension process's engine is adopted.
#[derive(Clone)]
pub struct AdoptSpec {
    data_root: PathBuf,
    inherit_providers: bool,
    waker: Arc<dyn Waker>,
    spaces: Vec<String>,
}

impl AdoptSpec {
    /// Its engine's data root (`run/engine/`), and what wakes its process.
    pub fn new(data_root: PathBuf, waker: Arc<dyn Waker>) -> Self {
        Self { data_root, inherit_providers: false, waker, spaces: Vec::new() }
    }

    pub fn inherit_providers(mut self) -> Self {
        self.inherit_providers = true;
        self
    }

    /// The extension's spaces, whose sessions are on its engine: calls from them may use it, as
    /// may the extension itself and a call reaching across spaces. With none, it is no space's.
    pub fn spaces(mut self, spaces: impl IntoIterator<Item = String>) -> Self {
        self.spaces = spaces.into_iter().collect();
        self
    }

    pub fn declared_spaces(&self) -> &[String] {
        &self.spaces
    }

    pub fn data_root(&self) -> &PathBuf {
        &self.data_root
    }

    pub fn inherits_providers(&self) -> bool {
        self.inherit_providers
    }

    pub fn waker(&self) -> &Arc<dyn Waker> {
        &self.waker
    }
}

/// An extension process's engine, adopted as the instance `proc.<id>`: one instance across the
/// process's runs, each handing over its engine's stream.
#[async_trait::async_trait]
pub trait AdoptedEngine: Send + Sync {
    /// Its instance's name.
    fn instance(&self) -> &str;

    /// A run of its process started: the product's end of its engine's stream. Up once the
    /// engine answers as one of this build's.
    async fn attach(&self, stream: std::os::unix::net::UnixStream) -> Rpc<()>;

    /// Its process is gone, and started again: degraded until the next run attaches.
    async fn detach(&self, why: &str);

    /// Its process is let go while idle: listed hibernating, its session list answered from what
    /// it listed last, until a call that needs it wakes it (the spec's [`Waker`]).
    async fn hibernate(&self);

    /// Whether it hibernates now.
    fn hibernating(&self) -> bool;

    /// Whether anything uses its engine: a session in its memory, or one with a watcher, a turn,
    /// a task or a question.
    async fn engaged(&self) -> bool;

    /// The chain of the turn running in one of its sessions; none when none runs.
    fn turn_chain(&self, session: &str) -> Vec<ChainLink>;

    /// What it tells its carrier, from now on; the first caller gets it.
    fn notes(&self) -> Option<mpsc::UnboundedReceiver<EngineNote>>;
}

/// What an adopted engine tells its carrier.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq)]
pub enum EngineNote {
    /// A turn started in one of its sessions, or the running one is now on behalf of more: its
    /// whole chain.
    TurnStarted { session_id: String, chain: Vec<ChainLink> },
    TurnSettled { session_id: String },
    /// It kept sending more than an extension's engine may: its process is to be ended.
    Flooded,
}

/// Registering extensions while the process runs, as one host component.
///
/// The host checks a descriptor's structure, not its consent — the consent record is the
/// component's, and the host never sees it — and refuses, writing nothing and on the record, a
/// descriptor that is not a package's: trust other than consent, names outside its own, an engine
/// contribution, a template, a route that is not a directory or its carrier's tool endpoint, a
/// capability only a native extension may hold, a listed call to a row the hub owns, one that
/// takes a capability or a first-party row not open to packages, an interface module without
/// `ui.page`, a contribution point, two tools by one name.
#[derive(Clone)]
pub struct DynamicRegistrar {
    inner: Arc<dyn DynamicRegistry>,
}

impl DynamicRegistrar {
    pub fn new(inner: Arc<dyn DynamicRegistry>) -> Self {
        Self { inner }
    }

    /// Register one extension, with what its consent grants: all or nothing, by the same host
    /// path a native extension takes.
    pub async fn register(&self, d: ExtensionDescriptor, caps: Vec<Capability>) -> Rpc<ExtensionHandle> {
        self.inner.register(d, caps).await.map(|inner| ExtensionHandle { inner })
    }

    /// Every extension registered so far, by any component.
    pub fn registered(&self) -> Vec<ExtensionSummary> {
        self.inner.registered()
    }
}

/// The host's side of [`DynamicRegistrar`]: implemented by the extension host.
#[async_trait::async_trait]
pub trait DynamicRegistry: Send + Sync {
    async fn register(&self, d: ExtensionDescriptor, caps: Vec<Capability>) -> Rpc<Arc<dyn RegisteredExtension>>;
    fn registered(&self) -> Vec<ExtensionSummary>;
}

/// The host's side of an [`ExtensionHandle`].
#[async_trait::async_trait]
pub trait RegisteredExtension: Send + Sync {
    fn id(&self) -> &ExtId;
    async fn call(&self, meta: CallMeta, method: &str, params: Value) -> Rpc<Value>;
    async fn publish(&self, topic: &str, params: Value, value: Value) -> Rpc<()>;
    async fn subscribe(&self, topic: &str, params: Value, sink: Arc<dyn TopicSink>) -> Rpc<Value>;
    async fn unsubscribe(&self, topic: &str, params: Value) -> Rpc<()>;
    fn mark(&self, availability: Availability);
    fn frame(&self, calls: Option<BTreeSet<String>>) -> Rpc<()>;
    async fn withdraw(&self) -> Rpc<Withdrawn>;
    fn refused(&self, what: &str, why: &'static str);
}

/// One extension registered while the process runs. Cheap to clone; once withdrawn, every call
/// through it fails.
#[derive(Clone)]
pub struct ExtensionHandle {
    inner: Arc<dyn RegisteredExtension>,
}

impl ExtensionHandle {
    pub fn id(&self) -> &ExtId {
        self.inner.id()
    }

    /// A call as the registered extension. `meta` is a hint: the host issues its own call id,
    /// keeps the chain of extensions waiting, and nothing else of it.
    pub async fn call(&self, meta: CallMeta, method: &str, params: Value) -> Rpc<Value> {
        self.inner.call(meta, method, params).await
    }

    /// The whole current value of one of its topics, by its name inside the extension's own.
    pub async fn publish(&self, topic: &str, params: Value, value: Value) -> Rpc<()> {
        self.inner.publish(topic, params, value).await
    }

    /// Watch a topic as the registered extension: values go to `sink`. Answers its value now.
    pub async fn subscribe(&self, topic: &str, params: Value, sink: Arc<dyn TopicSink>) -> Rpc<Value> {
        self.inner.subscribe(topic, params, sink).await
    }

    pub async fn unsubscribe(&self, topic: &str, params: Value) -> Rpc<()> {
        self.inner.unsubscribe(topic, params).await
    }

    /// Whether it can answer now. Its rows stay in the table; while it is not ready they answer
    /// `EXTENSION_UNAVAILABLE` with why.
    pub fn mark(&self, availability: Availability) {
        self.inner.mark(availability)
    }

    /// What its pages in frames may call besides its own names (`[interface] calls`), or `None`
    /// for no interface in frames: the router holds a frame's calls to this, whatever the page
    /// asks the shell for. Refused, setting nothing, when it lists what a package may not reach.
    pub fn frame(&self, calls: Option<BTreeSet<String>>) -> Rpc<()> {
        self.inner.frame(calls)
    }

    pub async fn withdraw(&self) -> Rpc<Withdrawn> {
        self.inner.withdraw().await
    }

    /// Put on the record that something the extension told its carrier failed the carrier's check:
    /// `what` it said (`proc.register`, `proc.hello`, `proc.fetch`), refused for `why`. In the audit
    /// as the extension's own refusal, with when; what is done about it is the carrier's.
    pub fn refused(&self, what: &str, why: &'static str) {
        self.inner.refused(what, why)
    }
}

/// Where the values of a topic a registered extension watches arrive.
pub trait TopicSink: Send + Sync {
    fn deliver(&self, topic: &str, params: &Value, value: &Value);

    /// The topic was withdrawn: nothing more will come for it.
    fn ended(&self, _topic: &str, _params: &Value, _reason: &str) {}
}

/// Whether a registered extension can answer now.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Availability {
    /// Starting, or waking: `EXTENSION_UNAVAILABLE {failure: "starting"}`.
    Starting,
    Ready,
    /// Stopped: `EXTENSION_UNAVAILABLE {failure: "stopped"}`.
    Stopped,
    /// Set aside after failing: `EXTENSION_UNAVAILABLE {failure: "faulted"}`, with why.
    Faulted(String),
}

impl Availability {
    /// How `EXTENSION_UNAVAILABLE` names it; `None` when it answers.
    pub fn failure(&self) -> Option<&'static str> {
        match self {
            Availability::Ready => None,
            Availability::Starting => Some("starting"),
            Availability::Stopped => Some("stopped"),
            Availability::Faulted(_) => Some("faulted"),
        }
    }
}

/// One registered extension, as the host lists it.
#[derive(Debug, Clone, Serialize)]
pub struct ExtensionSummary {
    id: String,
    by: String,
    carrier: &'static str,
    methods: Vec<String>,
    topics: Vec<String>,
    spaces: Vec<String>,
    calls: Vec<String>,
    since_ms: u64,
}

impl ExtensionSummary {
    pub fn new(id: &ExtId, by: &ExtId, carrier: PackageCarrier, since_ms: u64) -> Self {
        Self {
            id: id.to_string(),
            by: by.to_string(),
            carrier: carrier_name(carrier),
            methods: Vec::new(),
            topics: Vec::new(),
            spaces: Vec::new(),
            calls: Vec::new(),
            since_ms,
        }
    }

    /// What its consent lets it call and subscribe to besides its own names (`topic:<name>` for a
    /// topic).
    pub fn consented(mut self, calls: Vec<String>) -> Self {
        self.calls = calls;
        self
    }

    pub fn names(mut self, methods: Vec<String>, topics: Vec<String>, spaces: Vec<String>) -> Self {
        self.methods = methods;
        self.topics = topics;
        self.spaces = spaces;
        self
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    /// The host component that registered it.
    pub fn by(&self) -> &str {
        &self.by
    }

    pub fn methods(&self) -> &[String] {
        &self.methods
    }

    pub fn topics(&self) -> &[String] {
        &self.topics
    }

    pub fn spaces(&self) -> &[String] {
        &self.spaces
    }

    pub fn calls(&self) -> &[String] {
        &self.calls
    }
}

/// What withdrawing one took away.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Withdrawn {
    methods: Vec<String>,
    topics: Vec<String>,
    spaces: Vec<String>,
    subscriptions_ended: usize,
}

impl Withdrawn {
    pub fn new(methods: Vec<String>, topics: Vec<String>, spaces: Vec<String>, subscriptions_ended: usize) -> Self {
        Self { methods, topics, spaces, subscriptions_ended }
    }

    pub fn methods(&self) -> &[String] {
        &self.methods
    }

    pub fn topics(&self) -> &[String] {
        &self.topics
    }

    pub fn spaces(&self) -> &[String] {
        &self.spaces
    }

    /// Subscriptions that were told the topic ended.
    pub fn subscriptions_ended(&self) -> usize {
        self.subscriptions_ended
    }
}

/// How the profile names a carrier, and how a summary does: `wasm`, `process`.
pub fn carrier_name(carrier: PackageCarrier) -> &'static str {
    match carrier {
        PackageCarrier::Process => "process",
        _ => "wasm",
    }
}

/// Why a carrier is asked to stop one of its packages.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StopReason {
    Disabled,
    Uninstalled,
    Updating,
    ShuttingDown,
    Faulted(String),
}

/// What the component's configuration says its carriers run packages under: told each carrier
/// as it registers, before anything starts.
#[derive(Debug, Clone, Default)]
pub struct CarrierSettings {
    dev: bool,
    store_keys: Option<Vec<String>>,
    trusted_publishers: BTreeMap<String, Vec<String>>,
    idle: Option<std::time::Duration>,
    proc_program: Option<PathBuf>,
}

impl CarrierSettings {
    pub fn new() -> Self {
        Self::default()
    }

    /// Unsigned code runs (`[ext.packages] dev`).
    pub fn dev(mut self, dev: bool) -> Self {
        self.dev = dev;
        self
    }

    /// The store's keys (hex ed25519) in place of the one built in: a test application's.
    pub fn store_keys(mut self, keys: Vec<String>) -> Self {
        self.store_keys = Some(keys);
        self
    }

    /// `[ext.packages.trusted_publishers]`: a publisher's id and its keys (hex ed25519).
    pub fn trusted_publishers(mut self, publishers: BTreeMap<String, Vec<String>>) -> Self {
        self.trusted_publishers = publishers;
        self
    }

    /// How long every on-demand backend idles before it is let go, in place of its manifest's:
    /// a test application's.
    pub fn idle(mut self, idle: Option<std::time::Duration>) -> Self {
        self.idle = idle;
        self
    }

    /// The program an extension process is started as (`[ext.packages] proc_program`), when not
    /// this binary.
    pub fn proc_program(mut self, program: Option<PathBuf>) -> Self {
        self.proc_program = program;
        self
    }

    pub fn is_dev(&self) -> bool {
        self.dev
    }

    pub fn proc_program_path(&self) -> Option<&PathBuf> {
        self.proc_program.as_ref()
    }

    pub fn store_key_list(&self) -> Option<&[String]> {
        self.store_keys.as_deref()
    }

    pub fn trusted(&self) -> &BTreeMap<String, Vec<String>> {
        &self.trusted_publishers
    }

    pub fn idle_for(&self) -> Option<std::time::Duration> {
        self.idle
    }
}

/// What a carrier gives every engine built in this process, and the routes it serves under its
/// component's id: gathered by the host component, which declares them as its own.
#[derive(Default)]
pub struct CarrierRegistrar<'a> {
    settings: CarrierSettings,
    providers: Vec<Arc<dyn ToolProvider>>,
    admitted: Vec<String>,
    routes: Vec<(String, Arc<dyn RouteHandler>)>,
    _carrier: std::marker::PhantomData<&'a ()>,
}

impl CarrierRegistrar<'_> {
    pub fn new(settings: CarrierSettings) -> Self {
        Self { settings, ..Self::default() }
    }

    /// What it runs packages under.
    pub fn settings(&self) -> &CarrierSettings {
        &self.settings
    }

    /// Tools for every session of every engine built in this process.
    pub fn provider(&mut self, provider: Arc<dyn ToolProvider>) -> &mut Self {
        self.providers.push(provider);
        self
    }

    /// Tool names every scene of those engines lets in besides its own list (`prefix*`).
    pub fn admit(&mut self, pattern: &str) -> &mut Self {
        self.admitted.push(pattern.to_string());
        self
    }

    /// A route under the component's id: the packages' tool endpoints, for one.
    pub fn route(&mut self, prefix: &str, handler: Arc<dyn RouteHandler>) -> &mut Self {
        self.routes.push((prefix.to_string(), handler));
        self
    }

    pub fn providers(&self) -> &[Arc<dyn ToolProvider>] {
        &self.providers
    }

    pub fn admitted(&self) -> &[String] {
        &self.admitted
    }

    pub fn routes(&self) -> &[(String, Arc<dyn RouteHandler>)] {
        &self.routes
    }
}

/// One of the two carriers a package runs on: in this process (`wasm`) or in one of its own
/// (`process`). Chosen by name, `[ext.packages] carrier`.
#[async_trait::async_trait]
pub trait Carrier: Send + Sync + 'static {
    fn kind(&self) -> PackageCarrier;

    /// How the profile names it.
    fn name(&self) -> &'static str;

    /// What a package's precompiled code must have been built for.
    fn fingerprint(&self) -> String;

    fn register(&self, r: &mut CarrierRegistrar<'_>);

    /// Start one package, putting its agent package and skills in place first when its record
    /// says nothing is. Its descriptor — the carrier's handlers over its manifest and its consent
    /// — is registered through `host`'s registrar while it starts, before its own start code runs:
    /// that code already acts as it.
    async fn start(&self, host: &dyn HostApi, cx: &ExtensionContext, app: Installed) -> Rpc<Running>;

    async fn stop(&self, id: &ExtId, why: StopReason);

    /// Take out what it placed for a package outside its own directory (an agent package, skills).
    async fn unplace(&self, cx: &ExtensionContext, app: &Installed) -> Rpc<Value>;

    async fn status(&self, id: &ExtId) -> Value;

    /// A generation number: when it changes, the component reads the carriers' status again.
    fn changed(&self) -> watch::Receiver<u64>;
}

/// One installed package, as its component hands it to a carrier: everything checked already.
#[derive(Debug, Clone)]
pub struct Installed {
    id: ExtId,
    version: String,
    /// The install record, consent included.
    record: Value,
    pkg_dir: PathBuf,
    run_dir: PathBuf,
    /// The package's manifest, as read.
    manifest: Value,
    caps: Vec<Capability>,
    calls: BTreeSet<String>,
    config: Value,
    /// Only the secrets its consent named.
    secrets: BTreeMap<String, String>,
    /// How far its signatures vouch for it: `store`, `trusted` or `unsigned`.
    trust: String,
    /// The sha256 of its precompiled code, from the signatures just verified.
    cwasm_sha256: Option<String>,
}

impl Installed {
    pub fn new(id: ExtId, version: &str, pkg_dir: PathBuf, run_dir: PathBuf) -> Self {
        Self {
            id,
            version: version.to_string(),
            record: Value::Null,
            pkg_dir,
            run_dir,
            manifest: Value::Null,
            caps: Vec::new(),
            calls: BTreeSet::new(),
            config: Value::Null,
            secrets: BTreeMap::new(),
            trust: String::new(),
            cwasm_sha256: None,
        }
    }

    pub fn record(mut self, record: Value) -> Self {
        self.record = record;
        self
    }

    pub fn manifest(mut self, manifest: Value) -> Self {
        self.manifest = manifest;
        self
    }

    pub fn grants(mut self, caps: Vec<Capability>, calls: BTreeSet<String>) -> Self {
        self.caps = caps;
        self.calls = calls;
        self
    }

    pub fn config(mut self, config: Value) -> Self {
        self.config = config;
        self
    }

    pub fn secrets(mut self, secrets: BTreeMap<String, String>) -> Self {
        self.secrets = secrets;
        self
    }

    pub fn trust(mut self, trust: &str) -> Self {
        self.trust = trust.to_string();
        self
    }

    pub fn cwasm_sha256(mut self, sha256: &str) -> Self {
        self.cwasm_sha256 = Some(sha256.to_string());
        self
    }

    pub fn id(&self) -> &ExtId {
        &self.id
    }

    pub fn version(&self) -> &str {
        &self.version
    }

    pub fn install_record(&self) -> &Value {
        &self.record
    }

    pub fn pkg_dir(&self) -> &PathBuf {
        &self.pkg_dir
    }

    pub fn run_dir(&self) -> &PathBuf {
        &self.run_dir
    }

    pub fn package_manifest(&self) -> &Value {
        &self.manifest
    }

    pub fn capabilities(&self) -> &[Capability] {
        &self.caps
    }

    pub fn calls(&self) -> &BTreeSet<String> {
        &self.calls
    }

    pub fn config_value(&self) -> &Value {
        &self.config
    }

    pub fn secret_values(&self) -> &BTreeMap<String, String> {
        &self.secrets
    }

    pub fn trust_level(&self) -> &str {
        &self.trust
    }

    pub fn cwasm_digest(&self) -> Option<&str> {
        self.cwasm_sha256.as_deref()
    }
}

/// What a carrier answers for a package it started: what it put in place outside its directory,
/// for its record, and what to tell whoever asked.
pub struct Running {
    placed: Value,
    summary: Value,
}

impl Running {
    pub fn new(placed: Value, summary: Value) -> Self {
        Self { placed, summary }
    }

    /// What its record's `parts` say from now on: its agent package and in which instances, its
    /// skills.
    pub fn placed(&self) -> &Value {
        &self.placed
    }

    pub fn summary(&self) -> &Value {
        &self.summary
    }

    pub fn into_parts(self) -> (Value, Value) {
        (self.placed, self.summary)
    }
}
