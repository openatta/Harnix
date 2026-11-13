//! The boundary between a runtime application's host and the product around it.
//!
//! The host (`harnix-wasm-host`) runs applications: their VMs, their lifecycle, their methods,
//! tools and topics. What it needs from outside is two things, [`Kernel`] and [`Agent`]; what the
//! product asks of it is [`Control`]. Nothing else crosses:
//!
//! - every call is async, and takes the call's [`CallMeta`] and one serde value, and answers one
//!   (or an [`RpcError`]); a notification ([`Kernel::progress`], [`Control::turn_started`]) takes
//!   the value alone, answers nothing and does not wait;
//! - no reference, closure or handle crosses. What would be a callback is a call the other way:
//!   a value on a topic the application subscribed to is [`Control::deliver`], a tool call's
//!   progress is [`Kernel::progress`], cancelling one is [`Control::cancel`] with the call's id.
//!
//! # A call's metadata
//!
//! [`CallMeta`] says which call this is, the call it was made while handling, the extensions
//! waiting on it, each with the call it is held on, and its deadline. A host gives every call it
//! makes while serving one of the product's the served call as its `parent`, and — when the served
//! call holds the application's backend — the served call's chain with the application held on it.
//! So a backend whose call comes back into itself, by however many hops over whatever carried it,
//! finds itself in the chain on a call it is still serving, and refuses at once
//! (`EXTENSION_UNAVAILABLE`, `failure: "reentrant"`); one that answered long ago is not refused.
//!
//! The two traits the host needs are two because they are two sides in an edition where each
//! application runs in a process of its own:
//!
//! - [`Kernel`] is the product's: registering the application's names, what it does as itself
//!   (`host.call`, `publish`, `subscribe`), a tool call's progress. Carried over the link to the
//!   product, wherever the host runs.
//! - [`Agent`] is the agent engine the application's agent package, skills and tools go to:
//!   installing them, switching them, offering its tools to sessions. Embedded, that is the
//!   product's Agix; in a process of its own, the application's own, beside the host — never over
//!   the link.
//!
//! So the same host runs embedded in the product's process (`harnix-apps` implements both over
//! the kernel and this process's Agix, and calls `Control` in place) or in a process of its own,
//! with `Kernel` and `Control` carried over a link. [`rpc`] is the shape of that: each method a
//! name and a JSON value; under the `loopback` feature, [`rpc::Loopback`] puts every call through
//! it, which is what the host's tests run behind.
//!
//! Under the `socket` feature, [`socket::SocketWire`] is that link over a stream: the extension
//! wire between a product and an extension process of its own, at [`EXT_WIRE_VERSION`].
//!
//! **One machine, in this first version.** Paths cross as paths: what a path names (an unpacked
//! package, an application's directory, an agent package's archive) is the host's to read, and
//! the host and the product are on the same machine. A link between machines would carry those
//! files rather than their names, and is not part of this version of the port.

use std::collections::BTreeMap;
use std::path::PathBuf;

use async_trait::async_trait;
pub use harnix_contract::s1::CallMeta;
pub use harnix_contract::{codes, ChainLink, RpcError, SpaceDecl, Subject};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub mod rpc;
#[cfg(feature = "socket")]
pub mod socket;

/// The extension wire's version. Both ends of it are one build, so the process's `proc.hello`
/// says the same number or is refused; it changes when the wire's frames or methods change shape.
pub const EXT_WIRE_VERSION: u32 = 1;

/* ── what the host asks of the kernel and the agent engine ───────────── */

/// An application's names in the product: what [`Kernel::register`] declares.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Registration {
    pub app: String,
    /// `[[method]]`: `app.<id>.<name>`, each answered by [`Control::call_method`].
    pub methods: Vec<MethodDecl>,
    /// `[[topic]]`: `app.<id>.<name>`, published by [`Kernel::publish`].
    pub topics: Vec<String>,
    /// `[[space]]`.
    pub spaces: Vec<SpaceDecl>,
    /// What the application may reach as itself (`[capabilities] calls`).
    pub calls: Vec<String>,
    /// `[[tool]]`: every tool it offers sessions, as its manifest declares them.
    #[serde(default)]
    pub tools: Vec<ToolDef>,
    /// The sha256 of its manifest as it is on disk: what the product compares with the manifest
    /// its consent was given to.
    #[serde(default)]
    pub digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MethodDecl {
    pub name: String,
    pub description: String,
    /// Who may call it: `device`, `module`, `kernel`. The application itself always may.
    pub subjects: Vec<String>,
}

/// A call the application makes as itself: `host.call`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CallAs {
    pub app: String,
    pub method: String,
    pub params: Value,
}

/// `host.publish`, on one of the application's `[[topic]]`s.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Publish {
    pub app: String,
    pub topic: String,
    pub value: Value,
}

/// `host.subscribe` and `host.unsubscribe`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Subscription {
    pub app: String,
    pub topic: String,
    pub params: Value,
}

/// An application's agent package, into one instance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentInstall {
    pub app: String,
    pub name: String,
    pub version: String,
    /// The package's `agent/`, zipped: what the agent engine installs from.
    pub archive: PathBuf,
    pub instance: String,
}

/// An application's agent package in one instance, by name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentRef {
    pub app: String,
    pub name: String,
    pub version: String,
    pub instance: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentSwitch {
    pub app: String,
    pub name: String,
    pub instance: String,
    pub on: bool,
}

/// An application's skills in one instance: placed from `source` (its `agent/skills/`), or taken
/// out (`source` unused).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Skills {
    pub app: String,
    pub instance: String,
    pub source: PathBuf,
    pub names: Vec<String>,
}

/// One tool an application's component lists.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolDef {
    pub name: String,
    pub description: String,
    pub doc: Option<String>,
    /// JSON Schema, as the component wrote it.
    pub input_schema: String,
    pub read_only: bool,
    pub concurrency_safe: bool,
}

/// What a running application offers sessions: its tools, by their own names, and whether it has
/// an `events` hook to offer every tool call. Offered again with nothing when it stops.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Offer {
    pub app: String,
    pub title: Option<String>,
    pub version: String,
    /// It has a component: its tools are offered, and its events run around every tool call.
    pub component: bool,
    pub tools: Vec<ToolDef>,
}

/// `host.http-request`, already checked against the application's `net` by the host: the
/// product checks it again against what it granted, and makes it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fetch {
    pub app: String,
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    /// The body, base64.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body_b64: Option<String>,
}

/// What the product's fetch answered. Redirects are never followed: a 3xx goes back as it came.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fetched {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    /// The body, base64.
    pub body_b64: String,
}

/// One session's turn, by its instance and session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnRef {
    pub instance: String,
    pub session_id: String,
}

/// The calls a session's running turn is on behalf of, outermost first: what its tool calls carry
/// as their chain. Empty when no turn runs, or one runs on behalf of no extension.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnChain {
    pub instance: String,
    pub session_id: String,
    pub chain: Vec<ChainLink>,
}

/// A tool call's `host.progress`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Progress {
    pub app: String,
    pub tool: String,
    pub call_id: String,
    pub text: String,
}

/// What the host needs from the product it runs in: the kernel.
///
/// The product admits, grants and audits what an application does as itself (`module:<id>`); the
/// host has already checked the application's own declarations (its `[[topic]]`s, its `net`)
/// before it asks. Every call carries its [`CallMeta`]: a host's own calls name the call they were
/// made serving as their parent.
#[async_trait]
pub trait Kernel: Send + Sync {
    /// Declare the application's methods, topics and spaces, and its `calls` capability. A product
    /// that hosts no applications' names accepts this and declares nothing; what the application
    /// then does as itself is refused.
    async fn register(&self, meta: CallMeta, registration: Registration) -> Result<(), RpcError>;
    /// Take them back. Calls queued for them are answered that the application stopped.
    async fn withdraw(&self, meta: CallMeta, app: String) -> Result<(), RpcError>;

    /// `host.call`, as `module:<app>`. Routed into an application's method, it reaches
    /// [`Control::call_method`] with the chain `meta` says and the product holds to: a backend
    /// still serving a call the chain holds it on — however many hops back, over whatever carried
    /// it — refuses it rather than deadlock.
    async fn call(&self, meta: CallMeta, call: CallAs) -> Result<Value, RpcError>;
    /// `host.publish`, in the order the backend published.
    async fn publish(&self, meta: CallMeta, publish: Publish) -> Result<(), RpcError>;
    /// `host.subscribe`: answers the subscription, with the topic's `value` now if it has one;
    /// every later value comes as [`Control::deliver`].
    async fn subscribe(&self, meta: CallMeta, subscription: Subscription) -> Result<Value, RpcError>;
    async fn unsubscribe(&self, meta: CallMeta, subscription: Subscription) -> Result<(), RpcError>;

    /// `host.http-request`, made by the product: what it granted (`net`) is checked there.
    async fn fetch(&self, meta: CallMeta, fetch: Fetch) -> Result<Fetched, RpcError>;
    /// The chain of a session's running turn, for a host whose own engine runs it and has not
    /// been told of it yet ([`Control::turn_started`]).
    async fn turn_chain(&self, meta: CallMeta, turn: TurnRef) -> Result<TurnChain, RpcError>;

    /// A tool call's progress, for whoever started it ([`ToolCall::call_id`]). A notification: it
    /// does not wait, and is told before the tool call's answer is.
    fn progress(&self, progress: Progress);
}

/// What the host needs from the agent engine its applications' agent side goes to.
#[async_trait]
pub trait Agent: Send + Sync {
    /// Whether the agent engine has this instance, for `[agent] install_into`.
    async fn has_instance(&self, instance: String) -> Result<bool, RpcError>;
    /// Install the application's agent package: answers what the engine discloses of it.
    async fn install_agent(&self, install: AgentInstall) -> Result<Value, RpcError>;
    async fn uninstall_agent(&self, agent: AgentRef) -> Result<(), RpcError>;
    /// Switch an installed agent package on or off with the application.
    async fn switch_agent(&self, switch: AgentSwitch) -> Result<(), RpcError>;
    /// Put the skills in the instance's skills directory, marked as the application's. A skill of
    /// that name that is not the application's is refused, by name.
    async fn place_skills(&self, skills: Skills) -> Result<(), RpcError>;
    /// Take out the skills marked as the application's, and nothing else.
    async fn remove_skills(&self, skills: Skills) -> Result<(), RpcError>;
    /// The instance re-reads its configuration, its skills included.
    async fn reload_instance(&self, instance: String) -> Result<(), RpcError>;
    /// Offer the application's tools to sessions built from now on (or withdraw them: no tools).
    async fn expose_tools(&self, offer: Offer) -> Result<(), RpcError>;
}

/* ── what the product asks of the host ──────────────────────────────── */

/// An unpacked package to put in place: its agent package and skills into the instances its
/// manifest names.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Place {
    pub app: String,
    /// `<apps>/<id>/`: the application's directory.
    pub app_dir: PathBuf,
    /// The version unpacked under it, at `<app_dir>/<version>/`.
    pub version: String,
    /// The instances its agent package goes into, as the carrier says: none, and the host puts it
    /// into each instance its manifest's `install_into` names that its agent engine has.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instances: Option<Vec<String>>,
}

/// What putting it in place did: the record's `parts` (`agent`, `skills`), and what the agent
/// engine disclosed of its package. A placement that fails takes back what it did before saying
/// so.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Placement {
    pub parts: Value,
    pub disclosure: Value,
}

/// Take back what a placement put elsewhere, from the record's `parts`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Unplace {
    pub app: String,
    pub parts: Value,
}

/// Start an installed application from its record.
///
/// What the product verified — the package's signatures over its files as they are now, the
/// manifest against the record — it says here, and the host takes its word: the host checks only
/// that the component it loads is the one the product verified ([`Start::cwasm_sha256`]).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Start {
    pub app: String,
    pub version: String,
    /// How far its signatures vouch for it, as the product just verified them: `store`,
    /// `trusted`, `unsigned`.
    pub trust: String,
    /// The sha256 of its precompiled component for this engine, from the signatures the product
    /// just verified: what the host checks the bytes it loads against. None only for a package
    /// with no component, or one the product runs unsigned.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwasm_sha256: Option<String>,
    /// Set over the manifest's `[config]`: the profile's settings for it (an object, or null).
    #[serde(default)]
    pub config: Value,
    /// What `host.secret` gives it, by name: only names its consent covered.
    #[serde(default)]
    pub secrets: BTreeMap<String, String>,
    /// The record's `parts`: which agent package and skills to switch on with it.
    pub parts: Value,
    /// What it is held to: the consented manifest's, with what the product shortens.
    pub limits: Limits,
    /// The unpacked package (`<ext>/<id>/<version>/`). Given by the carrier that runs several
    /// applications in one host; an extension process has its own from its command line, and is
    /// never sent one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pkg: Option<PathBuf>,
    /// Where the application keeps what it writes: its `data/` and its store `kv/`. As `pkg`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run: Option<PathBuf>,
}

/// What an application is held to as it runs. The product's to say, from the manifest consented
/// to — and its own settings, which may only shorten how long a backend idles — so that the host
/// runs it as the product keeps it: an extension process hibernates its backend when the product
/// expects it to.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Limits {
    /// Linear memory for the whole application (`[component] memory_mb`).
    pub memory_mb: u32,
    /// How long one call may take (`[component] timeout_ms`).
    pub timeout_ms: u64,
    /// Its store's and its `${data}` files' ceiling (`[capabilities] storage_mb`).
    pub storage_mb: u32,
    /// `[backend] resident`: `always`, or `on-demand`.
    pub resident: String,
    /// How long an on-demand backend idles before it hibernates: its `idle_secs`, or less where the
    /// product says. In milliseconds: a test application's is under a second.
    pub idle_ms: u64,
}

impl Default for Limits {
    /// A manifest's defaults.
    fn default() -> Self {
        Self { memory_mb: 64, timeout_ms: 30_000, storage_mb: 0, resident: "always".into(), idle_ms: 600_000 }
    }
}

impl Limits {
    /// Whether its backend is let go while idle.
    pub fn on_demand(&self) -> bool {
        self.resident == "on-demand"
    }
}

/// What starting it made.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Started {
    pub app: String,
    /// How far its package was trusted, verified on disk as it started: `store`, `trusted`,
    /// `unsigned`.
    pub trust: String,
    /// `tools` or `backend`; none without a component.
    pub world: Option<String>,
    pub tools: Vec<ToolDef>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Stop {
    pub app: String,
    /// The record's `parts`: what to switch off with it.
    pub parts: Value,
}

/// A call of one of the application's `[[method]]`s, already admitted by the product.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MethodCall {
    pub app: String,
    /// Its own name, without `app.<id>.`.
    pub method: String,
    pub caller: Subject,
    pub params: Value,
}

/// A value on a topic the application subscribed to.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TopicValue {
    pub app: String,
    pub topic: String,
    pub value: Value,
}

/// One of the application's tools, called by a session (or a spawned daemon's, over MCP).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCall {
    pub app: String,
    pub tool: String,
    pub input: Value,
    /// Names the call for [`Control::cancel`] and its [`Kernel::progress`].
    pub call_id: String,
    /// The calling session's project, for a manifest that asks for `${project}`.
    pub project: Option<PathBuf>,
    /// Whether its `host.progress` is wanted.
    pub progress: bool,
}

/// Why a call into the component produced no answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Failure {
    TimedOut,
    Cancelled,
    Faulted,
    Unavailable,
}

impl Failure {
    pub fn as_str(self) -> &'static str {
        match self {
            Failure::TimedOut => "timed_out",
            Failure::Cancelled => "cancelled",
            Failure::Faulted => "faulted",
            Failure::Unavailable => "unavailable",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum ToolReply {
    /// The tool answered (`is_error`: it answered that it failed).
    Output {
        content: String,
        structured: Option<Value>,
        is_error: bool,
    },
    /// It did not answer; `message` says why, as a person reads it.
    Failed { failure: Failure, message: String },
}

/// A lifecycle event offered to the application's `events` hook: `tool.before`, around a
/// session's tool call.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HookCall {
    pub app: String,
    pub event: String,
    pub payload: Value,
    /// Names it for [`Control::cancel`].
    pub call_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "decision", content = "text", rename_all = "snake_case")]
pub enum HookReply {
    Proceed,
    Block(String),
    AddContext(String),
    /// The hook did not answer: the call goes on.
    Failed(String),
}

/// What the product asks of the host. Every call carries its [`CallMeta`]; the host serves it as
/// the parent of every call it makes on its behalf.
#[async_trait]
pub trait Control: Send + Sync {
    /// Put an unpacked package's agent package and skills in place ([`Place`]).
    async fn place(&self, meta: CallMeta, place: Place) -> Result<Placement, RpcError>;
    /// Take back what [`Control::place`] put in place; answers what was taken.
    async fn unplace(&self, meta: CallMeta, unplace: Unplace) -> Result<Value, RpcError>;

    /// Start an installed application: its component checked to be the one the product verified,
    /// its agent package and skills switched on, its names registered ([`Kernel::register`]), its
    /// VM started, its tools listed and offered. A step that fails undoes the ones before it, and
    /// the reason is kept for [`Control::status`].
    async fn start(&self, meta: CallMeta, start: Start) -> Result<Started, RpcError>;
    /// Stop it: its tools withdrawn, its names withdrawn, its VM stopped, its agent package and
    /// skills switched off. A call already running holds its own instance.
    async fn stop(&self, meta: CallMeta, stop: Stop) -> Result<(), RpcError>;
    /// Forget why an application that is not running did not start.
    async fn forget(&self, meta: CallMeta, app: String) -> Result<(), RpcError>;
    /// What is running of it, or why it is not: `{state, reason, world, methods, …, health}`.
    async fn status(&self, meta: CallMeta, app: String) -> Result<Value, RpcError>;

    /// Answer one of its methods with its backend: params checked against the declared schema,
    /// then queued for `handle`. Refused at once when `meta`'s chain holds the application on a
    /// call its backend is still serving: that backend is waiting on this very call. Refused
    /// before the backend is asked when `meta`'s deadline has passed.
    async fn call_method(&self, meta: CallMeta, call: MethodCall) -> Result<Value, RpcError>;
    /// Hand its backend a value on a topic it subscribed to, in the order they come.
    async fn deliver(&self, meta: CallMeta, value: TopicValue) -> Result<(), RpcError>;
    /// Run one of its tools in an instance of its own.
    async fn call_tool(&self, meta: CallMeta, call: ToolCall) -> Result<ToolReply, RpcError>;
    /// Offer its `events` hook a lifecycle event.
    async fn hook(&self, meta: CallMeta, call: HookCall) -> Result<HookReply, RpcError>;
    /// Cancel a tool call or hook still running, by its id. Nothing happens to one that ended.
    async fn cancel(&self, meta: CallMeta, call_id: String) -> Result<(), RpcError>;

    /// The process is going: stop every application.
    async fn shutdown(&self, meta: CallMeta, reason: String) -> Result<(), RpcError>;

    /// A turn started on an engine of the host's own, on behalf of these calls: its tools carry
    /// the chain. A notification.
    fn turn_started(&self, turn: TurnChain);
    /// That turn settled. A notification.
    fn turn_settled(&self, turn: TurnRef);
}
