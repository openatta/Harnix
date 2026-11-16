//! What the kernel says on the client's wire, as structures: `harnix.hello`, the frames the hub
//! pushes, the rows of its lists and topics.
//!
//! The interface's declarations are generated from these (`ui/packages/core/src/protocol/generated/`,
//! by `crates/harnix/tests/ts_export.rs`), so a field added here is a field the client's types
//! know, and one the generated file is checked against. A field added to a `json!` is known to
//! nobody — which is how a client once read a timeout under a name the server never sent.
//!
//! Two kinds of field are the engine's shape, not this protocol's (`harnix-ext.md` §16): an
//! event's payload and anything an `engine.*` method answers. They are carried as [`Value`] and
//! declared in the interface by hand, against the engine's protocol (`hello.engine_protocol`).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::s1::{codes, RpcError};
use crate::Limits;

/* ── harnix.hello ──────────────────────────────────────────────────────── */

/// `harnix.hello`: what a client is told once, after the handshake.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Hello {
    pub protocol_version: u32,
    pub contrib_api_version: u32,
    /// The engine's protocol. A session event's payload and an `engine.*` answer are its shape,
    /// not this protocol's: a client reads them by this number.
    pub engine_protocol: u64,
    /// The `main` instance's engine.
    pub engine: HelloEngine,
    /// Every instance: what it is, how far its engine is trusted, the protocol it speaks.
    pub instances: Vec<HelloInstance>,
    /// How long the engine waits for a person to answer a permission ask: the card counts down
    /// from it.
    pub permission_prompt_timeout_secs: u64,
    /// The engine's scenes, as `engine.scene.list` says them.
    #[cfg_attr(feature = "ts", ts(type = "Scene[]"))]
    pub scenes: Vec<Value>,
    /// The engine's commands, as `engine.commands.list` says them.
    #[cfg_attr(feature = "ts", ts(type = "unknown[]"))]
    pub commands: Vec<Value>,
    /// What a client fills in when it names none.
    pub defaults: HelloDefaults,
    /// Every extension and whether it is enabled: the shell activates its compiled-in UI packages
    /// by this before its first draw. What changes later is `harnix.ui`'s.
    pub extensions: Vec<ExtensionEntry>,
    /// What this product is: its brand, what the shell does and does not do.
    pub shell: HelloShell,
    pub limits: Limits,
}

/// The `main` engine as `hello` tells it.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct HelloEngine {
    pub model: Option<String>,
    pub active_scenes: Vec<String>,
    /// The scene a session that names none is created in.
    pub scene: Option<String>,
    pub data_root: String,
    pub has_credentials: bool,
    /// What a session can be switched to; null when the engine could not say.
    pub providers: Option<EngineProviders>,
    /// `engine.daemon.status`'s answer, as the engine gave it.
    #[cfg_attr(feature = "ts", ts(type = "unknown"))]
    pub status: Value,
}

/// The engine's model providers as `hello` tells them: their models only, nothing of how each is
/// reached.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct EngineProviders {
    pub providers: BTreeMap<String, ProviderModels>,
    pub default_provider: Option<String>,
}

/// One provider's default model and the others it allows.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ProviderModels {
    pub default_model: Option<String>,
    pub models: Vec<String>,
}

/// Whose engine an instance runs, as the wire says it: `product` is one the product started or
/// connected to; `extension` is an engine in an extension's own process, under third-party
/// control, whose sessions are rendered as untrusted content.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum EngineTrust {
    Product,
    Extension,
}

impl From<&crate::s1::InstanceTrust> for EngineTrust {
    fn from(trust: &crate::s1::InstanceTrust) -> Self {
        if trust.is_extension() { EngineTrust::Extension } else { EngineTrust::Product }
    }
}

/// `hello.instances`'s row.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct HelloInstance {
    pub id: String,
    pub carrier: InstanceCarrier,
    pub trust: EngineTrust,
    pub status: LinkStatus,
    pub version: Option<String>,
    pub protocol_version: Option<u64>,
}

/// What a client fills in when it names none. `space` is null until the product names one.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct HelloDefaults {
    pub instance: String,
    pub space: Option<String>,
}

/// What the shell is told about this product.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct HelloShell {
    pub brand: Brand,
    /// Whether installed packages' interface modules load.
    pub runtime_modules: bool,
    /// False: the shell draws no "New session" button and ⌘K starts none, in every space.
    pub new_session: bool,
    /// The composer changes a session's model through `harnix.session.setModel`.
    pub model_switch: bool,
    /// How an open right column behaves at ≤ 960px.
    pub narrow_details: NarrowDetails,
    pub state_dir: String,
    pub projects_dir: String,
    /// Who runs this process, for the avatar.
    pub user: Option<String>,
}

/// The product's brand.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Brand {
    pub name: Option<String>,
    /// Where the icon is served, when the profile names one.
    pub icon: Option<String>,
    pub product_id: String,
}

/// `[shell] narrow_details`: `beside` keeps three columns at ≤ 960px instead of maximizing an open
/// right column over the conversation.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum NarrowDetails {
    #[default]
    Over,
    Beside,
}

/// One extension, as `hello.extensions` and `harnix.ui` tell it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ExtensionEntry {
    pub id: String,
    /// `origin` (linked into the product) or `consent` (a package).
    pub trust: String,
    /// `native`, `wasm` or `process`.
    pub carrier: String,
    pub version: Option<String>,
    pub enabled: bool,
    /// In `hello.extensions`: the compiled-in UI package it is drawn by, when that is not its id.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub bundle: Option<String>,
    /// In `harnix.ui` only: what it declared of its interface (`bundle`, `surfaces`, `inline`,
    /// `modules`, `points`), null when nothing.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional, type = "{ bundle?: string; [key: string]: unknown } | null"))]
    pub ui: Option<Value>,
}

/// `harnix.ui`'s value: each extension's entry, and the interface modules to load.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct UiView {
    pub extensions: Vec<ExtensionEntry>,
    #[cfg_attr(feature = "ts", ts(type = "unknown[]"))]
    pub contributions: Vec<Value>,
}

/* ── instances ─────────────────────────────────────────────────────────── */

/// How an instance's engine is carried, for the instance list. Reported, never branched on: what
/// differs between carriers is the carrier's own doing, in the layer that built the link.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum InstanceCarrier {
    /// Built in this process.
    InProcess,
    /// A daemon this process started.
    Spawn,
    /// A daemon somebody else runs.
    Attach,
    /// An engine in an extension's own process, reached over the stream that process was given.
    Adopted,
}

/// How an instance's link to its engine is.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum LinkStatus {
    Starting,
    Ready,
    /// Lost, and being found again. Turns on the far side keep running.
    Degraded { since_ms: u64, reason: String },
    /// Not coming back without someone changing something.
    Down { reason: String },
    /// Let go while idle, with what it answers without its engine kept: the next call that needs
    /// the engine starts it again. How an instance is listed, never what a link says of itself.
    Hibernating { since_ms: u64 },
}

impl LinkStatus {
    pub fn is_ready(&self) -> bool {
        matches!(self, LinkStatus::Ready)
    }

    /// The answer to a call on a link in this state, which cannot reach its engine.
    pub fn unavailable(&self) -> RpcError {
        let why = match self {
            LinkStatus::Starting => "is starting".to_string(),
            LinkStatus::Ready => "lost its engine mid-call".to_string(),
            LinkStatus::Degraded { reason, .. } => format!("is reconnecting ({reason})"),
            LinkStatus::Down { reason } => format!("is down ({reason})"),
            LinkStatus::Hibernating { .. } => "is hibernating".to_string(),
        };
        RpcError::new(codes::INSTANCE_UNAVAILABLE, format!("the instance {why}"))
    }
}

/// An instance, as `harnix.instances.list` and the topic `harnix.instances` list it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct InstanceRow {
    pub id: String,
    pub carrier: InstanceCarrier,
    pub template: Option<String>,
    pub params: BTreeMap<String, String>,
    /// `kernel`, `module:<id>` or `space:<id>`.
    pub owner: String,
    pub status: LinkStatus,
    pub version: Option<String>,
    pub protocol_version: Option<u64>,
    pub data_root: String,
    pub scenes: Vec<String>,
    pub pid: Option<u32>,
    pub created_ms: u64,
    /// Only on an engine in an extension's process.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub trust: Option<EngineTrust>,
    /// Its process is reaped when it is stopped.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub reap: bool,
}

/// `harnix.instances.list`'s answer and the topic `harnix.instances`'s value.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct InstancesList {
    pub instances: Vec<InstanceRow>,
}

/* ── sessions ──────────────────────────────────────────────────────────── */

/// What a session waits on a person for.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum Waiting {
    /// An answer to a question.
    Answer,
    /// A plan's review.
    Plan,
    /// A permission.
    Approval,
}

impl Waiting {
    pub fn as_str(self) -> &'static str {
        match self {
            Waiting::Answer => "answer",
            Waiting::Plan => "plan",
            Waiting::Approval => "approval",
        }
    }
}

/// Who started a session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum StartedBy {
    /// A person, on a device.
    User,
    /// An extension or the kernel.
    Harnix,
}

/// What a session is doing now, in one word.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum SessionActivity {
    Idle,
    Running,
    /// A turn runs and more wait behind it.
    Queued,
}

/// The hub's part of a session's row in `harnix.sessions.list` and `harnix.instances.sessions`:
/// what it folds into the engine's `SessionInfo`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct HubSessionRow {
    pub instance: String,
    /// The space it belongs to: the one recorded for it, else the instance's default.
    pub space: String,
    /// A turn is running.
    pub running: bool,
    /// Messages waiting their turn.
    pub queued: usize,
    /// Its tasks still running: work that can outlive the turn that started it.
    pub tasks: usize,
    pub waiting: Option<Waiting>,
    /// When the running turn began (ms since the epoch).
    pub turn_started_ms: Option<u64>,
    pub started_by: StartedBy,
    pub state: SessionActivity,
    /// Its goal as the hub last heard it (the engine's shape); absent when the hub has heard
    /// nothing of it.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional, type = "Goal | null"))]
    pub goal: Option<Value>,
    /// The scene layer it was created in, when it was: shown in place of `scene`.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub scene_layer: Option<String>,
    /// What the caller added to the end of the system prompt (`harnix.session.setPrompt`).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub prompt_append: Option<String>,
    /// An engine in an extension's process: rendered as untrusted content.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub trust: Option<EngineTrust>,
}

/// `harnix.sessions.list`'s answer: one instance's (`instance`) or one space's (`space`) sessions.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct SessionsList {
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub instance: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub space: Option<String>,
    /// Each the engine's `SessionInfo` with a [`HubSessionRow`] folded in.
    #[cfg_attr(feature = "ts", ts(type = "SessionRow[]"))]
    pub sessions: Vec<Value>,
}

/// A session that is not idle, in the topic `harnix.sessions.status`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct SessionStatusRow {
    pub instance: String,
    pub session_id: String,
    pub state: SessionActivity,
    pub turn_id: Option<String>,
    pub queued: usize,
    /// The running turn was started outside the hub (an extension's engine).
    pub external: bool,
}

/// The topic `harnix.sessions.status`: every session that is not idle, across instances.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct SessionsStatus {
    pub sessions: Vec<SessionStatusRow>,
}

/// A space, as `harnix.spaces.list` and the topic `harnix.spaces` list it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct SpaceRow {
    pub id: String,
    /// The instance its sessions land in, as declared.
    pub instance: String,
    /// The instance built for it, once one is.
    pub built: Option<String>,
    pub scene: Option<String>,
    pub cross_space: bool,
    /// The space a call that claims none is checked as.
    #[serde(rename = "default")]
    pub default_space: bool,
}

/// `harnix.spaces.list`'s answer and the topic `harnix.spaces`'s value.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct SpacesList {
    pub spaces: Vec<SpaceRow>,
}

/// A message waiting its turn. Attachments are in it because the row is the only place a queued
/// message is visible.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct QueueItem {
    pub item_id: String,
    pub message: String,
    #[cfg_attr(feature = "ts", ts(type = "unknown"))]
    pub attachments: Value,
}

/// A frame in the hub's buffer: `seq` is the one anchor for catching up.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ReplayFrame {
    pub seq: u64,
    pub turn_id: String,
    #[cfg_attr(feature = "ts", ts(type = "EngineEvent"))]
    pub event: Value,
}

/// `harnix.session.attach`'s answer.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct AttachResult {
    pub instance: String,
    /// The engine's `SessionInfo`, as `engine.session.get` answers it.
    #[cfg_attr(feature = "ts", ts(type = "SessionInfo"))]
    pub session: Value,
    /// Read the history up to here; `replay` goes on from it. While a turn runs, the count before
    /// it began: the running turn is partly in the transcript and wholly in the buffer.
    pub history_total: u64,
    pub replay: Vec<ReplayFrame>,
    /// The buffer overflowed: frames before `replay` are gone.
    pub truncated: bool,
    /// The questions waiting on a person, as the engine put them.
    #[cfg_attr(feature = "ts", ts(type = "PromptEvent[]"))]
    pub pending_prompts: Vec<Value>,
    pub running_turn: Option<String>,
    pub queue: Vec<QueueItem>,
    pub seq: u64,
    /// An engine in an extension's process: rendered as untrusted content.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub trust: Option<EngineTrust>,
}

/* ── the hub's frames ──────────────────────────────────────────────────── */

/// `harnix.event`: a session event, with the seq the hub gave it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct EventFrame {
    pub instance: String,
    pub session_id: String,
    pub seq: u64,
    pub turn_id: String,
    /// The engine's event (`hello.engine_protocol`).
    #[cfg_attr(feature = "ts", ts(type = "EngineEvent"))]
    pub event: Value,
    /// An engine in an extension's process: rendered as untrusted content.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub trust: Option<EngineTrust>,
}

/// A tool call a turn made, and whether its result was an error once it has one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ToolCallOutcome {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub is_error: Option<bool>,
}

/// How a turn ended, as the engine said when it completed: the points a caller acts on, and what
/// the turn said and called.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct TurnOutcome {
    #[cfg_attr(feature = "ts", ts(type = "string | null"))]
    pub stop_reason: Value,
    #[cfg_attr(feature = "ts", ts(type = "Usage | null"))]
    pub usage: Value,
    /// The answer's text, up to the hub's ceiling; `text_truncated` past it, the whole of it in
    /// the transcript.
    pub text: String,
    pub tool_calls: Vec<ToolCallOutcome>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub text_truncated: bool,
    /// What the turn returned, for a scene that asks for structured output.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional, type = "unknown"))]
    pub structured_output: Option<Value>,
    /// Why it stopped, in words, when the engine says (a hook's reason).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional, type = "string | null"))]
    pub message: Option<Value>,
}

/// `harnix.turn_settled`: a turn ended — well (`result`), badly (`error`) or by interruption.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct TurnSettled {
    pub instance: String,
    pub session_id: String,
    pub turn_id: String,
    /// `run_turn`'s answer, the engine's shape.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional, type = "{ name?: string | null; usage?: Usage; [key: string]: unknown }"))]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub error: Option<RpcError>,
    /// Absent when the hub saw no `turn_complete`.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub outcome: Option<TurnOutcome>,
}

/// `harnix.queue`: the queue's whole value.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct QueueFrame {
    pub instance: String,
    pub session_id: String,
    pub items: Vec<QueueItem>,
}

/// `harnix.session_resync`: the hub rebuilt this session's buffer; attach again.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct SessionResync {
    pub instance: String,
    pub session_id: String,
}

/// `harnix.topic`: a topic's whole value.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct TopicFrame {
    pub topic: String,
    #[cfg_attr(feature = "ts", ts(type = "Record<string, unknown>"))]
    pub params: Value,
    #[cfg_attr(feature = "ts", ts(type = "unknown"))]
    pub value: Value,
}

/// `harnix.topic_ended`: the topic is gone (its extension withdrew it); nothing more is pushed.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct TopicEnded {
    pub topic: String,
    #[cfg_attr(feature = "ts", ts(type = "Record<string, unknown>"))]
    pub params: Value,
    pub reason: String,
}

/* ── answering a session's asks ────────────────────────────────────────── */

/// Which kind of ask an answer is for: the `prompt_type` of the ask's `prompt` event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum AskKind {
    /// May a tool call run — leaving plan mode is one, answered as a plan's review.
    Permission,
    /// A question the model put to the person.
    Question,
    /// A form, or a page to visit, an MCP server asked the person for in a tool call.
    McpElicitation,
}

/// `harnix.session.respond`'s `answer`: to a question (`answer`, `decline`) or an MCP server's ask
/// (`accept`, `decline`, `cancel`). A permission is answered with `decision`, as before.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AskReply {
    /// A question's answer: one of its options' `key`, or words of the person's own.
    Answer { text: String },
    /// Not answered: the person said no. `reason` is for a question; an MCP server is told no more
    /// than that.
    Decline {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        reason: Option<String>,
    },
    /// An MCP server's ask, accepted: a form's fields filled in by its `requested_schema`; no
    /// `content` for a page the person has visited.
    Accept {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional, type = "Record<string, unknown>"))]
        content: Option<serde_json::Map<String, Value>>,
    },
    /// An MCP server's ask, left without a choice.
    Cancel,
}

impl AskReply {
    /// Whether this is an answer to `kind` at all.
    pub fn answers(&self, kind: AskKind) -> bool {
        match kind {
            AskKind::Permission => false,
            AskKind::Question => matches!(self, AskReply::Answer { .. } | AskReply::Decline { .. }),
            AskKind::McpElicitation => matches!(self, AskReply::Accept { .. } | AskReply::Decline { .. } | AskReply::Cancel),
        }
    }
}

/// `harnix.session.respond`'s params. Whoever answers first is the answer; a later one succeeds
/// and changes nothing.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct RespondParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub instance: Option<String>,
    pub session_id: String,
    pub prompt_id: String,
    /// The ask's `prompt_type`; absent: a permission.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub prompt_type: Option<AskKind>,
    /// A permission's answer (the engine's shape).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional, type = "Decision"))]
    pub decision: Option<Value>,
    /// A question's answer, or an MCP server's ask's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub answer: Option<AskReply>,
    /// The call the ask is about: the hub keeps who answered it for that tool's audit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub tool_use_id: Option<String>,
}

impl RespondParams {
    /// The answer fits the kind of ask: a `decision` for a permission, an `answer` of the right
    /// kind otherwise, never both.
    pub fn check(&self) -> Result<AskKind, RpcError> {
        let kind = self.prompt_type.unwrap_or(AskKind::Permission);
        let fits = match (kind, &self.decision, &self.answer) {
            (AskKind::Permission, Some(Value::Object(_)), None) => true,
            (_, None, Some(answer)) => answer.answers(kind),
            _ => false,
        };
        if !fits {
            let wants = match kind {
                AskKind::Permission => "a `decision`",
                AskKind::Question => "an `answer` of type `answer` or `decline`",
                AskKind::McpElicitation => "an `answer` of type `accept`, `decline` or `cancel`",
            };
            return Err(RpcError::invalid_params(format!("this kind of ask is answered with {wants}, and nothing else")));
        }
        Ok(kind)
    }
}

/// The kind of [`AskAnswered`]'s event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum AskAnsweredKind {
    AskAnswered,
}

/// A `harnix.event` the hub adds after the engine took the first answer to an ask through
/// `harnix.session.respond`: what it was, so that every device's card shows it, not only the one
/// that gave it. An MCP server's form goes out without what was filled in.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct AskAnswered {
    pub kind: AskAnsweredKind,
    pub prompt_id: String,
    pub prompt_type: AskKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional, type = "Decision"))]
    pub decision: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub answer: Option<AskReply>,
    /// The extension that answered (`flow`, by its policy); absent when a person did.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub extension: Option<String>,
}

impl AskAnswered {
    pub fn new(params: &RespondParams, kind: AskKind, extension: Option<String>) -> Self {
        let answer = params.answer.clone().map(|answer| match answer {
            AskReply::Accept { .. } => AskReply::Accept { content: None },
            other => other,
        });
        AskAnswered {
            kind: AskAnsweredKind::AskAnswered,
            prompt_id: params.prompt_id.clone(),
            prompt_type: kind,
            decision: params.decision.clone(),
            answer,
            extension,
        }
    }
}

/// Build a frame's `params` from one of these: a structure that does not serialize is a bug, and
/// an empty object is the frame a client can at least parse.
pub fn params<T: Serialize>(frame: &T) -> Value {
    serde_json::to_value(frame).unwrap_or_else(|_| Value::Object(Default::default()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn what_is_absent_is_not_written() {
        let settled = TurnSettled {
            instance: "main".into(),
            session_id: "s".into(),
            turn_id: "t".into(),
            result: Some(json!({"name": null})),
            error: None,
            outcome: None,
        };
        assert_eq!(
            params(&settled),
            json!({"instance": "main", "session_id": "s", "turn_id": "t", "result": {"name": null}})
        );
        let row = InstanceRow {
            id: "main".into(),
            carrier: InstanceCarrier::Spawn,
            template: None,
            params: Default::default(),
            owner: "kernel".into(),
            status: LinkStatus::Degraded { since_ms: 1, reason: "lost".into() },
            version: None,
            protocol_version: None,
            data_root: "/d".into(),
            scenes: Vec::new(),
            pid: None,
            created_ms: 0,
            trust: None,
            reap: false,
        };
        let value = params(&row);
        assert_eq!(value["status"], json!({"state": "degraded", "since_ms": 1, "reason": "lost"}));
        assert!(value.get("trust").is_none() && value.get("reap").is_none());
    }

    fn respond(value: Value) -> Result<AskKind, RpcError> {
        serde_json::from_value::<RespondParams>(value).map_err(|e| RpcError::invalid_params(e.to_string()))?.check()
    }

    #[test]
    fn an_answer_fits_its_kind_of_ask() {
        let key = json!({"session_id": "s", "prompt_id": "p"});
        let with = |extra: Value| {
            let mut v = key.clone();
            v.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
            v
        };
        assert_eq!(respond(with(json!({"decision": {"type": "permit"}}))).unwrap(), AskKind::Permission);
        let question = json!({"prompt_type": "question", "answer": {"type": "answer", "text": "a"}});
        assert_eq!(respond(with(question)).unwrap(), AskKind::Question);
        assert!(respond(with(json!({"prompt_type": "question", "answer": {"type": "decline"}}))).is_ok());
        let form = json!({"prompt_type": "mcp_elicitation", "answer": {"type": "accept", "content": {"color": "blue"}}});
        assert_eq!(respond(with(form)).unwrap(), AskKind::McpElicitation);
        assert!(respond(with(json!({"prompt_type": "mcp_elicitation", "answer": {"type": "cancel"}}))).is_ok());
        for wrong in [
            json!({}),
            json!({"answer": {"type": "answer", "text": "a"}}),
            json!({"prompt_type": "question", "decision": {"type": "permit"}}),
            json!({"prompt_type": "question", "answer": {"type": "cancel"}}),
            json!({"prompt_type": "mcp_elicitation", "answer": {"type": "answer", "text": "a"}}),
            json!({"prompt_type": "mcp_elicitation", "answer": {"type": "accept", "content": "blue"}}),
            json!({"prompt_type": "question", "answer": {"type": "answer"}}),
            json!({"prompt_type": "host_tool_call", "answer": {"type": "answer", "text": "a"}}),
            json!({"decision": {"type": "permit"}, "answer": {"type": "cancel"}}),
        ] {
            assert!(respond(with(wrong.clone())).is_err(), "{wrong}");
        }
    }

    #[test]
    fn an_answer_told_to_every_device_leaves_out_a_forms_content() {
        let given: RespondParams = serde_json::from_value(json!({
            "session_id": "s", "prompt_id": "p", "prompt_type": "mcp_elicitation",
            "answer": {"type": "accept", "content": {"token": "secret"}},
        }))
        .unwrap();
        let told = params(&AskAnswered::new(&given, AskKind::McpElicitation, None));
        assert_eq!(told, json!({"kind": "ask_answered", "prompt_id": "p", "prompt_type": "mcp_elicitation", "answer": {"type": "accept"}}));
    }

    #[test]
    fn the_space_rows_default_flag_is_named_as_the_wire_says() {
        let row = SpaceRow {
            id: "system".into(),
            instance: "main".into(),
            built: None,
            scene: None,
            cross_space: true,
            default_space: true,
        };
        assert_eq!(params(&row)["default"], json!(true));
    }
}
