//! The boundary between a runtime application's host and the product around it.
//!
//! The host (`harnix-app-host`) runs applications: their VMs, their lifecycle, their methods,
//! tools and topics. What it needs from outside is two things, [`Kernel`] and [`Agent`]; what the
//! product asks of it is [`Control`]. Nothing else crosses:
//!
//! - every method is async, and takes one serde value and answers one (or an [`RpcError`]);
//! - no reference, closure or handle crosses. What would be a callback is a call the other way:
//!   a value on a topic the application subscribed to is [`Control::deliver`], a tool call's
//!   progress is [`Kernel::progress`], cancelling one is [`Control::cancel`] with the call's id.
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
//! **One machine, in this first version.** Paths cross as paths: what a path names (an unpacked
//! package, an application's directory, an agent package's archive) is the host's to read, and
//! the host and the product are on the same machine. A link between machines would carry those
//! files rather than their names, and is not part of this version of the port.

use std::path::PathBuf;

use async_trait::async_trait;
pub use harnix_contract::{codes, RpcError, SpaceDecl, Subject};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub mod rpc;

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
    /// What the application may reach as itself (`[capabilities] hub`).
    pub hub: Vec<String>,
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
    /// The applications whose resident backends are waiting on this call, outermost first — `app`
    /// last, when it is its backend that calls. Routed into one of their methods, the call comes
    /// back as [`MethodCall::chain`], and is refused rather than left waiting for itself.
    #[serde(default)]
    pub chain: Vec<String>,
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
/// before it asks.
#[async_trait]
pub trait Kernel: Send + Sync {
    /// Declare the application's methods, topics and spaces, and its `hub` capability. A product
    /// that hosts no applications' names accepts this and declares nothing; what the application
    /// then does as itself is refused.
    async fn register(&self, registration: Registration) -> Result<(), RpcError>;
    /// Take them back. Calls queued for them are answered that the application stopped.
    async fn withdraw(&self, app: String) -> Result<(), RpcError>;

    /// `host.call`, as `module:<app>`. Routed into an application's method, it is answered by
    /// [`Control::call_method`] with the call's [`CallAs::chain`] as its [`MethodCall::chain`]:
    /// a backend already waiting on the call — however many hops back, over whatever carried it —
    /// refuses it rather than deadlock.
    async fn call(&self, call: CallAs) -> Result<Value, RpcError>;
    /// `host.publish`, in the order the backend published.
    async fn publish(&self, publish: Publish) -> Result<(), RpcError>;
    /// `host.subscribe`: answers the subscription, with the topic's `value` now if it has one;
    /// every later value comes as [`Control::deliver`].
    async fn subscribe(&self, subscription: Subscription) -> Result<Value, RpcError>;
    async fn unsubscribe(&self, subscription: Subscription) -> Result<(), RpcError>;

    /// A tool call's progress, for whoever started it ([`ToolCall::call_id`]).
    async fn progress(&self, progress: Progress) -> Result<(), RpcError>;
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Start {
    pub app: String,
    pub version: String,
    pub app_dir: PathBuf,
    /// The record's `parts`: which agent package and skills to switch on with it.
    pub parts: Value,
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
    /// The applications whose resident backends are waiting on this call, outermost first: the
    /// [`CallAs::chain`] of the `host.call` it was routed from, or none. A call whose chain holds
    /// its own application is one its backend is already waiting on, and is refused.
    #[serde(default)]
    pub chain: Vec<String>,
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

/// What the product asks of the host.
#[async_trait]
pub trait Control: Send + Sync {
    /// Put an unpacked package's agent package and skills in place ([`Place`]).
    async fn place(&self, place: Place) -> Result<Placement, RpcError>;
    /// Take back what [`Control::place`] put in place; answers what was taken.
    async fn unplace(&self, unplace: Unplace) -> Result<Value, RpcError>;

    /// Start an installed application: its signatures verified over its files as they are now,
    /// its agent package and skills switched on, its names registered ([`Kernel::register`]),
    /// its VM started, its tools listed and offered. A step that fails undoes the ones before it,
    /// and the reason is kept for [`Control::status`].
    async fn start(&self, start: Start) -> Result<Started, RpcError>;
    /// Stop it: its tools withdrawn, its names withdrawn, its VM stopped, its agent package and
    /// skills switched off. A call already running holds its own instance.
    async fn stop(&self, stop: Stop) -> Result<(), RpcError>;
    /// Forget why an application that is not running did not start.
    async fn forget(&self, app: String) -> Result<(), RpcError>;
    /// What is running of it, or why it is not: `{state, reason, world, methods, …, health}`.
    async fn status(&self, app: String) -> Result<Value, RpcError>;

    /// Answer one of its methods with its backend: params checked against the declared schema,
    /// then queued for `handle`. Refused at once when [`MethodCall::chain`] holds the
    /// application: its backend is waiting on this very call.
    async fn call_method(&self, call: MethodCall) -> Result<Value, RpcError>;
    /// Hand its backend a value on a topic it subscribed to, in the order they come.
    async fn deliver(&self, value: TopicValue) -> Result<(), RpcError>;
    /// Run one of its tools in an instance of its own.
    async fn call_tool(&self, call: ToolCall) -> Result<ToolReply, RpcError>;
    /// Offer its `events` hook a lifecycle event.
    async fn hook(&self, call: HookCall) -> Result<HookReply, RpcError>;
    /// Cancel a tool call or hook still running, by its id. Nothing happens to one that ended.
    async fn cancel(&self, call_id: String) -> Result<(), RpcError>;

    /// The process is going: stop every application.
    async fn shutdown(&self, reason: String) -> Result<(), RpcError>;
}
