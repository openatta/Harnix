//! What the runner asks of its host about sessions: [`SessionHost`].
//!
//! The runner knows runs, stations, tasks, profiles (the scene names definitions write), how a
//! task hands in and what a permission question is; it does not know what a session is made of.
//! The host does — Harnix opens one through its hub, another host through an engine's RPC — and
//! answers these calls in the runner's words. A session is an opaque id the host gives in
//! [`SessionHost::open`] and reads back everywhere else; the runner keeps it in the run record
//! and never looks inside it.
//!
//! What a host promises (the embedding guide, `docs/embed.en.md` §2, has the whole contract):
//!
//! - **Frames in order.** A session's frames arrive as they happened: `TurnStarted(t)` before
//!   anything of `t`, `TurnSettled(t)` last. A host that falls behind may drop frames in between
//!   (it says so with [`Frame::Lagged`]); it never drops a `TurnSettled`.
//! - **`open` is idempotent.** The same `key` opened again answers the same id with `existed:
//!   true`; the same key with another spec is `Refused`.
//! - **`send` queues.** A turn that is running is not cut off; one send starts exactly one turn,
//!   whose `TurnStarted.text` is the text sent.
//! - **`interrupt`** on an idle session is `Ok`; on a running turn a `TurnSettled { end:
//!   Interrupted }` follows.
//! - **`answer`** for a question whose turn is over is `Ok` or `Gone`; the runner minds neither.
//! - **`close`**: the streams of its watchers end; a `send` after it is `Gone`.
//! - Only permission questions come as [`Frame::Asked`]: a question for words (the model's, an
//!   MCP server's) is never the runner's to answer.

use std::path::{Path, PathBuf};
use std::pin::Pin;

use futures::Stream;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::kernel::host::{Delivery, Placement};
use crate::kernel::run::Usage;

/// A session as the host names it. The runner keeps it in run records and tells it back to the
/// host; it means nothing to the runner.
pub type SessionId = String;

/// Everything a session is opened with.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct OpenSpec {
    /// Opening the same key again is opening the same session (a host may derive the session's
    /// id from it). The runner gives a task's own session `flow:<run>/<station>/<task>@<token>`,
    /// a shared one `flow:<run>#<key>`, the main session `flow:<run>@main`.
    pub key: String,
    pub run: String,
    /// The run's main session: the one a person opens to watch and talk to the run.
    pub main: bool,
    /// The scene the definition names (`flow-harnix`, `code`…), which the host maps to its own
    /// scene, prompt and tools.
    pub profile: String,
    /// What to call the session where sessions are listed (the main session's).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// The project the session works in; none for a session of no project.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_root: Option<PathBuf>,
    /// The model and where the session goes, as the definition gave it: the host interprets
    /// `instance`, `template`, `params` and `space`, and refuses one it does not know rather
    /// than ignore it.
    pub placement: Placement,
    /// Tool patterns each call of which must come to the runner as [`Frame::Asked`] to be
    /// judged (the input of `ask_rules`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub asks: Vec<String>,
    /// The output contract when the session hands in by structured output; `None` with
    /// `flow_submit`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_schema: Option<Value>,
    pub delivery: Delivery,
    /// Where else the host shows the session (the start's `shown_in`); the host's word.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shown_in: Option<String>,
    /// The recording the session replays when the host runs in replay.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recording: Option<String>,
    /// The flow's own tools this session needs (`flow_state`, `flow_submit`, `flow_propose`),
    /// for the host to register; it answers each call through `FlowTools::call`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<ToolSpec>,
}

impl OpenSpec {
    /// A spec with what every session has; the rest is set field by field.
    pub fn new(key: impl Into<String>, run: impl Into<String>, profile: impl Into<String>, delivery: Delivery) -> Self {
        Self {
            key: key.into(),
            run: run.into(),
            main: false,
            profile: profile.into(),
            title: None,
            project_root: None,
            placement: Placement::default(),
            asks: vec![],
            output_schema: None,
            delivery,
            shown_in: None,
            recording: None,
            tools: vec![],
        }
    }
}

/// One of the flow's tools, as the host registers it to a session.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
    /// It changes nothing outside the flow's own record: a host may run it without asking.
    pub read_only: bool,
}

/// What [`SessionHost::open`] answers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Opened {
    pub id: SessionId,
    /// The key was open already: this is that session, not a new one.
    pub existed: bool,
}

/// What is known of a session when it is looked up or watched.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionInfo {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_root: Option<PathBuf>,
    /// A turn is running right now.
    pub turn_running: bool,
}

/// A session's frames from the moment of watching on, with its state at that moment — so a turn
/// that settles between asking and watching is not missed.
pub struct Watch {
    pub frames: Pin<Box<dyn Stream<Item = Frame> + Send>>,
    pub info: SessionInfo,
}

/// What a session's turn does, as the host reports it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum Frame {
    /// A turn began, with the message that started it (the runner tells its own turns by it).
    TurnStarted { turn: String, text: String },
    /// The model called a tool; `agent` names the sub-agent when it was one's.
    ToolCall {
        turn: String,
        call: String,
        tool: String,
        #[serde(default)]
        input: Value,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        agent: Option<SessionId>,
    },
    /// A tool call ended.
    ToolResult {
        turn: String,
        call: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        agent: Option<SessionId>,
    },
    /// A permission question: about a call of a tool the spec's `asks` named, or one the host
    /// holds back on its own (a write outside the project, into a reserved directory). `call` is
    /// the tool call it is about, when the host knows; `paths` what the call touches.
    Asked {
        turn: String,
        ask: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        call: Option<String>,
        tool: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        paths: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        agent: Option<SessionId>,
    },
    /// The main Agent started a sub-agent; `call` is the `ToolCall` it came from.
    AgentStarted {
        turn: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        call: Option<String>,
        agent: SessionId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        agent_type: Option<String>,
    },
    AgentEnded { turn: String, agent: SessionId, ok: bool },
    /// The turn ended. Exactly one for each `TurnStarted`, never dropped.
    TurnSettled {
        turn: String,
        end: TurnEnd,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        output: Option<Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        usage: Option<Usage>,
    },
    /// Frames were dropped here (never a `TurnSettled`).
    Lagged,
}

impl Frame {
    /// The turn a frame belongs to; none for [`Frame::Lagged`].
    pub fn turn(&self) -> Option<&str> {
        match self {
            Frame::TurnStarted { turn, .. }
            | Frame::ToolCall { turn, .. }
            | Frame::ToolResult { turn, .. }
            | Frame::Asked { turn, .. }
            | Frame::AgentStarted { turn, .. }
            | Frame::AgentEnded { turn, .. }
            | Frame::TurnSettled { turn, .. } => Some(turn),
            Frame::Lagged => None,
        }
    }
}

/// How a turn ended.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "how", rename_all = "snake_case")]
pub enum TurnEnd {
    Done,
    Interrupted,
    /// The turn failed: `retryable` when another may go through (a rate limit, an overload),
    /// not when nothing will change (no credentials).
    Error { retryable: bool, kind: String, message: String },
}

/// The runner's answer to a permission question.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Decision {
    Allow,
    Deny { reason: String },
}

/// A note written into a session's record for people to see (a proposal card, say): never for
/// the model.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Note {
    pub event: String,
    pub payload: Value,
}

/// Why the host could not.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HostError {
    /// The session is not there (deleted, or never known).
    Gone(String),
    /// The host will not: no such profile, model or placement; the project cannot be used.
    Refused(String),
    /// The host cannot right now (not started, stopping).
    Unavailable(String),
    Other(String),
}

impl std::fmt::Display for HostError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HostError::Gone(m) => write!(f, "the session is gone: {m}"),
            HostError::Refused(m) => write!(f, "refused: {m}"),
            HostError::Unavailable(m) => write!(f, "the host is unavailable: {m}"),
            HostError::Other(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for HostError {}

/// The sessions a host provides to the runner.
#[async_trait::async_trait]
pub trait SessionHost: Send + Sync {
    /// Open the session `spec` describes, or the one its `key` already names.
    async fn open(&self, spec: OpenSpec) -> Result<Opened, HostError>;
    /// Whether a session a run record names is still there, brought back when it was only on
    /// disk. `None`: it is not.
    async fn lookup(&self, id: &SessionId) -> Result<Option<SessionInfo>, HostError>;
    /// Its frames from now on, and its state now.
    async fn watch(&self, id: &SessionId) -> Result<Watch, HostError>;
    /// Send a message, queued behind a turn that is running.
    async fn send(&self, id: &SessionId, text: &str) -> Result<(), HostError>;
    /// Stop the turn that is running, if one is.
    async fn interrupt(&self, id: &SessionId) -> Result<(), HostError>;
    async fn close(&self, id: &SessionId) -> Result<(), HostError>;
    /// Answer a permission question ([`Frame::Asked`]'s `ask`).
    async fn answer(&self, id: &SessionId, ask: &str, decision: Decision) -> Result<(), HostError>;
    /// Write a note into the session's record, for people.
    async fn note(&self, id: &SessionId, note: Note) -> Result<(), HostError>;
    /// Whether the project at `root` is trusted (an `Err` counts as not).
    async fn project_trusted(&self, root: &Path) -> Result<bool, HostError>;
    /// The run is over: whatever the host made for it alone can go. Nothing by default.
    async fn run_over(&self, _run: &str) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn frames_are_tagged_by_kind_on_the_wire() {
        let f = Frame::TurnSettled { turn: "t1".into(), end: TurnEnd::Done, output: Some(json!({"n": 1})), usage: None };
        let v = serde_json::to_value(&f).unwrap();
        assert_eq!(v, json!({"kind": "turn_settled", "turn": "t1", "end": {"how": "done"}, "output": {"n": 1}}));
        assert_eq!(serde_json::from_value::<Frame>(v).unwrap(), f);
        let asked: Frame = serde_json::from_value(json!({"kind": "asked", "turn": "t", "ask": "a", "tool": "Write"})).unwrap();
        assert_eq!(asked.turn(), Some("t"));
        assert_eq!(Frame::Lagged.turn(), None);
        let err = TurnEnd::Error { retryable: false, kind: "no_credentials".into(), message: "none".into() };
        assert_eq!(serde_json::to_value(&err).unwrap(), json!({"how": "error", "retryable": false, "kind": "no_credentials", "message": "none"}));
    }

    #[test]
    fn a_spec_starts_bare_and_a_decision_says_why_not() {
        let s = OpenSpec::new("flow:r@main", "r", "flow-harnix", Delivery::FlowSubmit);
        assert!(!s.main && s.asks.is_empty() && s.output_schema.is_none());
        let v = serde_json::to_value(&s).unwrap();
        assert!(v.get("title").is_none() && v.get("tools").is_none(), "{v}");
        assert_eq!(
            serde_json::to_value(Decision::Deny { reason: "no".into() }).unwrap(),
            json!({"kind": "deny", "reason": "no"})
        );
        assert_eq!(HostError::Gone("s1".into()).to_string(), "the session is gone: s1");
    }
}
