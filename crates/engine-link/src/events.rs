//! What an engine says unasked, as the hub needs to tell it apart.
//!
//! Each event carries the engine's own value (`event`), which watchers are sent as it is — the
//! session events' shape is the engine's and `hello.engine_protocol` names it — and, beside it,
//! what of it the hub acts on, typed. The engine's names for its events are known in one place
//! only, the engine crate's translation; an event the hub has no use for is `Other` and still
//! reaches every watcher.

use serde_json::Value;

/// One frame off a link.
#[non_exhaustive]
#[derive(Debug, Clone)]
pub enum EngineEvent {
    /// About one session: its watchers see it, in order.
    Session {
        session: String,
        /// The turn it belongs to; empty when it belongs to none.
        turn_id: String,
        kind: SessionEventKind,
        event: Value,
    },
    /// About the engine as a whole, or a session it ran without being asked to.
    Daemon { kind: DaemonEventKind, event: Value },
    /// The mark [`crate::EngineLink::mark`] wrote.
    Barrier { id: u64 },
    /// The link lost its engine and found it again: every subscription it had is gone.
    Reconnected,
    /// The engine dropped one session's subscription, because its reader fell behind.
    SubscriptionDropped { session: String },
}

/// What of a session event the hub acts on.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq)]
pub enum SessionEventKind {
    /// Streamed text that joins the fragment before it when both have the same [`Joins`].
    Fragment(Fragment),
    /// A tool call, whole: the fragments its arguments came in are not needed any more.
    ToolCall { id: String, name: String },
    /// A tool call's result. A question about that call is answered by it, either way. `wrote`:
    /// the files it wrote, when the engine says (its file-editing tools do).
    ToolResult { id: String, is_error: bool, wrote: Vec<FileWrite> },
    /// Files a sub-agent's tool call wrote, from the parent's stream: the turn changed them too.
    Wrote(Vec<FileWrite>),
    /// A question for a person: a permission, or an answer the model asked for.
    Asked(Prompt),
    /// A question stopped waiting, as the engine says: answered (here or elsewhere), or
    /// withdrawn on its timeout or by a cancel.
    AskResolved { prompt_id: String, answered: bool },
    /// The session's goal, as it now is (`Null`: none).
    GoalChanged { goal: Value },
    /// Where one of the session's teams stands.
    TeamProgress { team_id: String },
    /// One of the session's tasks started (or finished): work that outlives the turn. `session`
    /// is the sub-agent's own session, when the task runs one: a session the engine made, whose
    /// parent this session is. `live`: it counts as running work now (a standing member between
    /// two messages is idle, and does not).
    Task { task_id: String, started: bool, session: Option<String>, live: bool },
    /// A task that had started changed: it is running or waiting now, or idle, as `live` says.
    TaskChanged { task_id: String, live: bool },
    /// The turn compacted the transcript, which is shorter for it.
    Compacted,
    /// A host rewound the session: the conversation is `messages` long now, and what was drawn
    /// past it is gone. Not a turn's.
    Rewound { messages: u64 },
    /// The turn ended, as the engine says.
    TurnComplete(Completion),
    Other,
}

/// A fragment of streamed text, and where in its event it sits.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq)]
pub struct Fragment {
    pub joins: Joins,
    /// The event's field the text is in: a fragment that joins this one is appended to it.
    pub field: &'static str,
    pub text: String,
}

impl Fragment {
    pub fn new(joins: Joins, field: &'static str, text: impl Into<String>) -> Self {
        Self { joins, field, text: text.into() }
    }
}

/// Which stream a fragment belongs to: fragments of one stream join, of two never.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Joins {
    /// The answer's text: what the turn says.
    Answer,
    /// The model's thinking.
    Thinking,
    /// One tool call's arguments, by the call's id.
    ToolArgs(String),
}

/// A question waiting for a person, as the engine put it.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq)]
pub struct Prompt {
    /// The engine's own value: what a client is sent.
    pub value: Value,
    /// What answers it names.
    pub prompt_id: Option<String>,
    /// The tool call it is about, when it is about one: its result answers it.
    pub tool_use_id: Option<String>,
    pub waiting: Waiting,
}

impl Prompt {
    pub fn new(value: Value, prompt_id: Option<String>, tool_use_id: Option<String>, waiting: Waiting) -> Self {
        Self { value, prompt_id, tool_use_id, waiting }
    }
}

/// What a session waiting on a question waits for, as its row shows it: the wire's word for it,
/// declared with the kernel's wire shapes (`harnix_contract::wire`).
pub use harnix_contract::wire::Waiting;

/// How a turn ended, as the engine said: the points a caller acts on.
#[non_exhaustive]
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Completion {
    pub stop_reason: Value,
    pub usage: Value,
    /// What the turn returned, for a scene that asks for structured output.
    pub structured_output: Option<Value>,
    /// Why it stopped, in words, when it says.
    pub message: Option<Value>,
}

impl Completion {
    pub fn new(stop_reason: Value, usage: Value) -> Self {
        Self { stop_reason, usage, structured_output: None, message: None }
    }

    pub fn with_structured_output(mut self, output: Option<Value>) -> Self {
        self.structured_output = output;
        self
    }

    pub fn with_message(mut self, message: Option<Value>) -> Self {
        self.message = message;
        self
    }
}

/// One file a tool call wrote, as the engine reported it beside the result: what a turn's changes
/// are made of (harnix-app.md, the changes card). Opaque to the hub but for its size: it gathers a
/// turn's into one entry in the session's log, so they are still there after a refresh.
#[non_exhaustive]
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FileWrite {
    /// The absolute path written.
    pub path: String,
    /// It did not exist before.
    pub created: bool,
    /// Lines added and removed, for the whole change even when the diff is cut.
    pub added: u64,
    pub removed: u64,
    /// Its size after the write, when the engine measured it.
    pub bytes: Option<u64>,
    /// The change as a unified diff; `None` when the engine left it out, and `omitted` says why.
    pub diff: Option<String>,
    /// The diff was cut at a hunk boundary.
    pub truncated: bool,
    /// Why there is no diff: `too_large`, `binary`, `redacted` — the engine's word, passed on.
    pub omitted: Option<String>,
}

impl FileWrite {
    pub fn new(path: impl Into<String>, created: bool) -> Self {
        Self { path: path.into(), created, ..Self::default() }
    }

    pub fn lines(mut self, added: u64, removed: u64) -> Self {
        self.added = added;
        self.removed = removed;
        self
    }

    pub fn bytes(mut self, bytes: Option<u64>) -> Self {
        self.bytes = bytes;
        self
    }

    pub fn diff(mut self, diff: Option<String>, truncated: bool, omitted: Option<String>) -> Self {
        self.diff = diff;
        self.truncated = truncated;
        self.omitted = omitted;
        self
    }
}

/// What of an engine-wide event the hub acts on.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq)]
pub enum DaemonEventKind {
    /// A turn started that the hub may not have started: a scheduled one.
    TurnStarted { session: String, turn_id: String },
    /// A turn ended, whoever started it.
    TurnCompleted { session: String, turn_id: String, initiator: Value },
    /// The engine's sessions changed: one was created, resumed, or `ended` — evicted, closed or
    /// deleted, after which it runs nothing.
    SessionsChanged { session: Option<String>, ended: bool },
    /// The engine dropped some of these events: what the hub knows of turns elsewhere may be
    /// stale.
    Lagged,
    /// An MCP server of the engine connected, or `failed` to.
    McpServer { server: String, failed: bool },
    Other,
}

impl DaemonEventKind {
    /// The `kind` a client is told of a turn starting or ending on an engine whose own events are
    /// not passed on: the host event's word, whatever engine is under it. `None` for the rest.
    pub fn told_as(&self) -> Option<&'static str> {
        match self {
            DaemonEventKind::TurnStarted { .. } => Some("turn_started"),
            DaemonEventKind::TurnCompleted { .. } => Some("turn_completed"),
            _ => None,
        }
    }
}
