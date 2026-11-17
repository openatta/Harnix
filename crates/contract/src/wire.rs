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
    /// The other spaces it is shown in (`harnix.session.expose`), each with whether a call
    /// claiming that space may also talk to it. Its own space is `space`, unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub exposed_to: Option<Vec<ExposureRow>>,
    /// In a space's list (`harnix.sessions.list {space}`): set on a row that is there because
    /// the session is exposed to that space, naming the space it belongs to (`space`, again), so a
    /// list can tell a lent session from its own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub exposed_from: Option<String>,
}

/// One space a session is exposed to (`HubSessionRow::exposed_to`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ExposureRow {
    pub space: String,
    /// A call claiming the space may talk to the session too (send, respond, queue, interrupt,
    /// rewind, feedback), not only look at it.
    pub interact: bool,
}

/// `harnix.session.expose` and `.unexpose`'s answer: the session, its own space, the space the
/// call was about, and what it is now.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct SessionExposed {
    pub instance: String,
    pub session_id: String,
    /// The space it belongs to.
    pub space: String,
    /// The space the call exposed it to, or took it from.
    pub to: String,
    /// Whether it is exposed to `to` now.
    pub exposed: bool,
    /// With `exposed`: whether a call claiming `to` may talk to it.
    pub interact: bool,
    /// The call changed something: `false` when it was already so.
    pub changed: bool,
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

/// The method and the topic that list what waits on a person across sessions.
pub const INBOX: &str = "harnix.inbox";

/// One ask a session is waiting on a person for, in `harnix.inbox`: a permission, a plan's review,
/// a question or an MCP server's form. A session waiting on two has two.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct InboxRow {
    pub instance: String,
    pub session_id: String,
    /// The space the session belongs to.
    pub space: String,
    pub waiting: Waiting,
    /// The ask as the engine put it: what `harnix.session.attach` hands a client as one of its
    /// `pending_prompts`, answered with `harnix.session.respond`.
    #[cfg_attr(feature = "ts", ts(type = "PromptEvent"))]
    pub prompt: Value,
    /// The session's name, when it has one.
    pub name: Option<String>,
    /// The project it works in, when it has one.
    pub project_root: Option<String>,
    pub started_by: StartedBy,
    /// An engine in an extension's process: rendered as untrusted content.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub trust: Option<EngineTrust>,
}

/// `harnix.inbox`'s answer and the topic's value: every ask waiting on a person, in the spaces the
/// caller sees, by instance and session.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Inbox {
    pub items: Vec<InboxRow>,
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

/// `harnix.session.outline`'s answer: a session's turns, one row each, oldest first — what a turn
/// rail draws and a trajectory groups its rows by.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct SessionOutline {
    pub instance: String,
    pub session_id: String,
    pub turns: Vec<OutlineTurn>,
    /// How many messages the transcript held when it was read: `engine.session.history`'s
    /// `total`, the coordinate each turn's `seq` is in.
    pub total: u64,
}

/// One turn of a [`SessionOutline`]: where it starts, and what was asked and answered, cut short.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct OutlineTurn {
    /// The turn's id, as its live frames carry it; null in a log written before turns had ids.
    pub turn_id: Option<String>,
    /// Where the person's message that started it is in the transcript: an `engine.session.history`
    /// `offset`. Reading history from here on shows the turn.
    pub seq: u64,
    /// When it was asked, as the engine wrote it (ISO 8601); null when the log does not say.
    pub at: Option<String>,
    /// What the person said, on one line, cut to about a line's worth of characters (ending `…`
    /// when cut).
    pub prompt: String,
    /// The turn's last text reply, on one line, cut to about three lines' worth; empty for a turn
    /// that has said nothing yet.
    pub reply: String,
    /// What else is known to happen at this turn, for a rail or a trajectory to mark: a flow's
    /// station starting, say. The hub writes none; absent when there are none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub marks: Option<Vec<OutlineMark>>,
}

/// A mark on an [`OutlineTurn`]: `kind` says what it is (`station` for a flow's station), `label`
/// is what it is called.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct OutlineMark {
    pub kind: String,
    pub label: String,
}

/* ── harnix.session.rewind ─────────────────────────────────────────────── */

/// Where a rewind goes back to: `"start"`, or one of `{turn}`, `{before_turn}`, `{tool_use}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum RewindTo {
    /// Before the first turn.
    Start,
    /// Just after this turn ended.
    Turn(String),
    /// Just before this turn began: where the turn before it ended, or the start. Going back to
    /// a message a person sent, which this turn answered.
    BeforeTurn(String),
    /// Just before this tool call wrote anything. Files only: a conversation cut in the middle of a
    /// turn would leave calls without results.
    ToolUse(String),
}

/// What a rewind puts back.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum RewindScope {
    /// The files the session's file tools wrote; the conversation stays.
    Files,
    /// The conversation; the files stay as they are.
    Conversation,
    Both,
}

/// `harnix.session.rewind`'s params.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct RewindParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub instance: Option<String>,
    pub session_id: String,
    /// The space the caller claims; a device rewinds only a session of the space it claims.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub space: Option<String>,
    pub to: RewindTo,
    /// `both` when absent; `files` for a tool use, the only one it takes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub scope: Option<RewindScope>,
    /// Work out the plan and its issues; change nothing.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub dry_run: bool,
    /// Fork the session as it is first, so the turns rewound stay in the fork.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub fork_first: bool,
    /// Restore over files changed since the session last wrote them (`modified`).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub force: bool,
    /// Leave out the files that cannot be restored rather than refusing.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub allow_partial: bool,
}

/// A turn a rewind takes out of the conversation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct RemovedTurn {
    pub turn_id: String,
    /// The text the turn was started with: what goes back into the composer.
    pub prompt: String,
}

/// What a rewind does to the conversation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct RewoundConversation {
    pub messages_before: u64,
    pub messages_after: u64,
    /// Oldest first.
    pub removed_turns: Vec<RemovedTurn>,
}

/// A file a rewind restores.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct RestoredFile {
    pub path: String,
    /// `file`: its earlier contents; `absent`: deleted, as it did not exist then.
    pub to: String,
    /// A write outside the project, approved when it was made.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub outside_project: bool,
    /// The session's last write of it left no record to check it against: restored with `force`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub unverified: bool,
}

/// A file a rewind cannot restore as asked.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct RewindIssue {
    pub path: String,
    /// `modified` (changed since the session wrote it), `unavailable`, `denied`, `symlink`,
    /// `worktree_removed`; more may come.
    pub issue: String,
    pub detail: String,
    /// What lets it through: `force` (a `modified` file is overwritten) or `allow_partial` (it is
    /// skipped).
    pub overridable_by: String,
}

/// What a rewind does to the files.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct RewoundFiles {
    pub restored: Vec<RestoredFile>,
    /// Skipped (`allow_partial`); on a dry run, every issue.
    pub skipped: Vec<RewindIssue>,
    /// Restored over a change made outside the session (`force`).
    pub overwrote_external: Vec<String>,
}

/// `harnix.session.rewind`'s answer: what a rewind did, or on a dry run would do.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct RewindResult {
    pub instance: String,
    pub session_id: String,
    pub rewind_id: String,
    pub dry_run: bool,
    pub scope: RewindScope,
    /// Where it went back to: a `before_turn` as the turn it came to, or the start.
    pub to: RewindTo,
    /// Absent when the conversation was not part of it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub conversation: Option<RewoundConversation>,
    /// Absent when the files were not part of it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub files: Option<RewoundFiles>,
    /// The fork made first (`fork_first`): the turns rewound are still in it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub fork_session_id: Option<String>,
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

/* ── feedback on an answer ─────────────────────────────────────────────── */

/// The namespace a person's feedback is filed under in a session's log: the hub's own, beside
/// `harnix.changes`. An entry, so it is never the model's (concept §3.2.1).
pub const FEEDBACK_NS: &str = "harnix.feedback";
/// The one event in it.
pub const FEEDBACK_EVENT: &str = "rating";
/// The topic a holder of `feedback.read` hears each piece of feedback on, as it is given.
pub const FEEDBACK_TOPIC: &str = "harnix.feedback";
/// The longest note one piece of feedback carries, in characters.
pub const FEEDBACK_NOTE_MAX_CHARS: usize = 4000;

/// Thumbs up or down.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum Rating {
    Up,
    Down,
}

/// What was wrong with (or about) an answer, when a person says.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum FeedbackCategory {
    Inaccurate,
    Unhelpful,
    Unsafe,
    Slow,
    Other,
}

/// One piece of feedback on a turn's answer: the payload of a `harnix.feedback` / `rating` entry.
///
/// Every change is a new entry; the last one for a turn (and call) is what stands, and the ones
/// before it stay in the log. `rating: null` takes the feedback back.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Feedback {
    /// The turn whose answer it is about: the `turn_id` of its frames, its transcript messages and
    /// its recorded calls.
    pub turn_id: String,
    /// One call in the turn, when the caller rated that rather than the turn's answer; opaque to
    /// the hub. The conversation's thumbs leave it out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub call_id: Option<String>,
    /// `null`: taken back.
    pub rating: Option<Rating>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub category: Option<FeedbackCategory>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub note: Option<String>,
    /// When, in ms since the epoch.
    pub at: u64,
    /// Who: the subject's label (`device:<id>`).
    pub by: String,
}

impl Feedback {
    /// The feedback an entry of the session's log holds, when it holds some.
    pub fn of_entry(entry: &Value) -> Option<Self> {
        if entry.get("ns").and_then(Value::as_str) != Some(FEEDBACK_NS)
            || entry.get("event").and_then(Value::as_str) != Some(FEEDBACK_EVENT)
        {
            return None;
        }
        serde_json::from_value(entry.get("payload")?.clone()).ok()
    }

    /// What it is about: a turn, or one call in it.
    pub fn target(&self) -> (&str, Option<&str>) {
        (&self.turn_id, self.call_id.as_deref())
    }
}

/// The feedback that stands in a session's log, `entries` oldest first: the last word on each turn
/// (and call), in the order each was first rated; what was taken back is left out.
pub fn current_feedback<'a>(entries: impl IntoIterator<Item = &'a Value>) -> Vec<Feedback> {
    let mut standing: Vec<Feedback> = Vec::new();
    for feedback in entries.into_iter().filter_map(Feedback::of_entry) {
        match standing.iter_mut().find(|f| f.target() == feedback.target()) {
            Some(slot) => *slot = feedback,
            None => standing.push(feedback),
        }
    }
    standing.retain(|f| f.rating.is_some());
    standing
}

/// `harnix.session.feedback`'s answer: what was filed, and the entry it was filed as (the
/// engine's shape).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct FeedbackFiled {
    pub feedback: Feedback,
    #[cfg_attr(feature = "ts", ts(type = "{ id: string; ts?: string; ns: string; event: string; payload: Feedback; at: number }"))]
    pub entry: Value,
}

/// The topic `harnix.feedback`'s value: the last piece of feedback given, on which session. Pushed
/// once per piece; the session's log is the record.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct FeedbackNotice {
    pub instance: String,
    pub session_id: String,
    /// The space the session belongs to.
    pub space: String,
    /// The entry's id in the session's log.
    pub entry_id: String,
    pub feedback: Feedback,
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

/* ── what a session sees: its skills and hooks ─────────────────────────── */

/// Where a skill a session sees comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum SkillTier {
    /// An extension package's skills, which Harnix lists for the session's space.
    Space,
    /// The project's own (`.agents/skills`).
    Project,
    /// An engine plugin package's.
    Plugin,
    /// The engine's user tiers: its global skills or its scene's — the engine does not say which.
    User,
    /// Built into the engine.
    Builtin,
}

/// One skill a session sees: `harnix.session.skills`'s row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct SkillRow {
    pub name: String,
    pub description: String,
    pub tier: SkillTier,
    /// The directory it was read from, when it can be said: a space tier's package directory, the
    /// project's `.agents/skills`, a plugin package's root.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub dir: Option<String>,
    /// The extension package that brought it (the space tier).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub package: Option<String>,
    /// The engine plugin package that brought it (the plugin tier).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub plugin: Option<String>,
    /// Held back in part until the project is trusted: its inline commands are not run, and its
    /// `allowed-tools` grant nothing.
    pub withheld: bool,
}

/// A skill of a listed directory that another of the same name hides.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ShadowedSkill {
    pub skill: String,
    /// The tier whose skill holds the name, as the engine says it: `user`, `project` or `host`.
    pub by: String,
}

/// One skill directory Harnix lists for the session (a package's, by the session's space).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct SkillDirRow {
    pub path: String,
    /// The package whose directory it is, when Harnix knows it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub package: Option<String>,
    /// `ok`, `missing` (not there yet) or `refused` (did not pass the engine's checks on resume);
    /// absent for a session not in the engine's memory.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub state: Option<String>,
    /// The skills it gives the session.
    pub skills: Vec<String>,
    /// Its skills another of the same name hides.
    pub shadowed: Vec<ShadowedSkill>,
}

/// `harnix.session.skills`: the skills a session sees, and where each comes from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct SessionSkills {
    pub instance: String,
    pub session_id: String,
    /// Whether the engine listed them: it lists a session's skills only while the session is in
    /// its memory. False, and `skills` is empty, for one that is only on disk.
    pub listed: bool,
    /// The project's trust, as the engine says it (`trusted`, `untrusted`, `none`); absent with no
    /// project, or when the engine could not say.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub trust: Option<String>,
    pub skills: Vec<SkillRow>,
    pub dirs: Vec<SkillDirRow>,
}

/// Where a hook a session runs comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum HookSource {
    /// The engine's global settings.
    Global,
    /// The session's scene's settings.
    Scene,
    /// The project's settings.
    Project,
    /// Registered by this process on its connection to the engine: what extensions answer.
    Host,
}

/// One hook: `harnix.session.hooks`'s row. Never what it runs — no command, URL, header or
/// prompt text: those can carry credentials, and the settings page shows only how many there are.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct HookRow {
    /// The engine's event (`PreToolUse`, `Stop`, …).
    pub event: String,
    pub source: HookSource,
    /// `command`, `prompt`, `http` or `agent` for a settings hook; `host` for one this process
    /// answers; `other` for a type this version does not know.
    pub kind: String,
    /// What it applies to: a settings hook's `if`, a host hook's tool patterns; cut short.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub matcher: Option<String>,
    /// A short label safe to show: the kind and the matcher.
    pub label: String,
    /// What a failure does: `allow` or `block`, when it says.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub on_error: Option<String>,
    /// Whether it runs: not a project's while the project is untrusted, nor one whose event a
    /// higher tier also configures.
    pub in_force: bool,
    /// Held back until the project is trusted.
    pub withheld: bool,
    /// The tier whose list for the same event replaces this one's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub replaced_by: Option<HookSource>,
    /// A host hook's providers: whose hooks before a tool call it asks; empty for any other.
    #[serde(default)]
    pub providers: Vec<String>,
}

/// `harnix.session.hooks`: the hooks a session's turns run, by where each comes from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct SessionHooks {
    pub instance: String,
    pub session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub trust: Option<String>,
    pub hooks: Vec<HookRow>,
    /// The settings tiers that could not be read: their hooks are unknown, not absent.
    pub unread: Vec<HookSource>,
}

/// The most characters of a matcher a [`HookRow`] carries.
pub const HOOK_MATCHER_CHARS: usize = 80;

impl HookRow {
    /// A hook in force, said by its kind and its matcher: the matcher cut to
    /// [`HOOK_MATCHER_CHARS`], an ellipsis where it was cut, and blank taken as none.
    pub fn new(event: impl Into<String>, source: HookSource, kind: impl Into<String>, matcher: Option<&str>, on_error: Option<String>) -> Self {
        let kind = kind.into();
        let matcher = matcher.map(str::trim).filter(|m| !m.is_empty()).map(|m| match m.char_indices().nth(HOOK_MATCHER_CHARS) {
            Some((at, _)) => format!("{}…", &m[..at]),
            None => m.to_string(),
        });
        let label = match &matcher {
            Some(m) => format!("{kind} · {m}"),
            None => kind.clone(),
        };
        Self {
            event: event.into(),
            source,
            kind,
            matcher,
            label,
            on_error,
            in_force: true,
            withheld: false,
            replaced_by: None,
            providers: Vec::new(),
        }
    }
}

/* ── extension logs ─────────────────────────────────────────────────────── */

/// How a line came to be one extension's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum ExtensionLogOrigin {
    /// Logged while the extension answered a call, computed a topic, started or stopped.
    Call,
    /// A WebAssembly package's own log, or its carrier's line about it.
    Package,
    /// A line its extension process wrote to stderr.
    Process,
    /// The host's line about it.
    Host,
}

/// One line of an extension's log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ExtensionLogLine {
    /// Rising across the process: what a reader asks after.
    pub seq: u64,
    pub at_ms: u64,
    pub extension: String,
    /// `error`, `warn`, `info`, `debug` or `trace`.
    pub level: String,
    pub origin: ExtensionLogOrigin,
    /// Where it was logged from (the tracing target).
    pub target: String,
    /// The message, then its other fields as `name=value`; cut short.
    pub message: String,
}

/// How many lines one extension has kept.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ExtensionLogCount {
    pub extension: String,
    pub lines: usize,
}

/// `harnix.extensions.log` (and `system.extensions.log`): the lines kept, oldest first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ExtensionLog {
    pub lines: Vec<ExtensionLogLine>,
    /// The newest `seq` in the log, whether or not it was asked for: the next read's `after`.
    pub last: u64,
    /// Every extension with lines kept.
    pub extensions: Vec<ExtensionLogCount>,
    /// Lines let go, across the process, since it started: the ring keeps only the newest.
    pub dropped: u64,
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

    fn rated(turn: &str, call: Option<&str>, rating: Option<Rating>, at: u64) -> Value {
        let mut payload = json!({"turn_id": turn, "rating": rating, "at": at, "by": "device:d"});
        if let Some(call) = call {
            payload["call_id"] = json!(call);
        }
        json!({"id": format!("e{at}"), "ns": FEEDBACK_NS, "event": FEEDBACK_EVENT, "payload": payload, "at": 2})
    }

    #[test]
    fn the_last_word_on_a_turn_stands_and_a_retraction_leaves_none() {
        let entries = vec![
            rated("t1", None, Some(Rating::Up), 1),
            json!({"id": "x", "ns": "harnix.changes", "event": "turn", "payload": {"turn_id": "t1"}}),
            rated("t2", None, Some(Rating::Down), 2),
            rated("t1", None, Some(Rating::Down), 3),
            rated("t2", None, None, 4),
            rated("t1", Some("c1"), Some(Rating::Up), 5),
            json!({"id": "y", "ns": FEEDBACK_NS, "event": FEEDBACK_EVENT, "payload": {"rating": "sideways"}}),
        ];
        let standing = current_feedback(&entries);
        let seen: Vec<_> = standing.iter().map(|f| (f.turn_id.as_str(), f.call_id.as_deref(), f.rating, f.at)).collect();
        // t1's turn changed its mind; t2 was taken back; a call of t1 is a target of its own; what
        // does not read as feedback is skipped.
        assert_eq!(seen, vec![("t1", None, Some(Rating::Down), 3), ("t1", Some("c1"), Some(Rating::Up), 5)]);
    }

    #[test]
    fn feedback_is_written_as_the_wire_says() {
        let given = Feedback {
            turn_id: "t".into(),
            call_id: None,
            rating: Some(Rating::Down),
            category: Some(FeedbackCategory::Slow),
            note: None,
            at: 7,
            by: "device:d".into(),
        };
        assert_eq!(params(&given), json!({"turn_id": "t", "rating": "down", "category": "slow", "at": 7, "by": "device:d"}));
        let taken_back = Feedback { rating: None, category: None, ..given };
        assert_eq!(params(&taken_back)["rating"], Value::Null);
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
    fn where_a_rewind_goes_is_a_word_or_one_field() {
        let to = |v: serde_json::Value| serde_json::from_value::<RewindTo>(v).unwrap();
        assert_eq!(to(json!("start")), RewindTo::Start);
        assert_eq!(to(json!({"before_turn": "t2"})), RewindTo::BeforeTurn("t2".into()));
        assert_eq!(params(&RewindTo::ToolUse("u1".into())), json!({"tool_use": "u1"}));
        let ask: RewindParams = serde_json::from_value(json!({"session_id": "s", "to": {"turn": "t1"}, "dry_run": true})).unwrap();
        assert!(ask.dry_run && !ask.fork_first && ask.scope.is_none());
        assert_eq!(params(&ask), json!({"session_id": "s", "to": {"turn": "t1"}, "dry_run": true}));
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
