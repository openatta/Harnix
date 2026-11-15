//! What the hub asks an engine, one method each.
//!
//! The hub spells none of the engine's method names: it asks for a session to be created, read,
//! subscribed to, run, and the link turns each into the engine's own call. The answers come back
//! as the engine gave them — a client is sent them as they are, and `hello.engine_protocol` names
//! their shape — with the few points the hub acts on read off them here.
//!
//! An engine method a client reaches through the hub without the hub doing anything around it
//! goes by [`EngineOps::passthrough`], under the name the hub's row gives it.

use std::collections::BTreeSet;
use std::path::PathBuf;

use harnix_contract::RpcError;
use serde_json::{json, Value};

use crate::events::Prompt;

pub type Answer<T> = Result<T, RpcError>;

/// The engine's error codes the hub tells apart. The engine's own numbers: the engine crate
/// checks them against its engine's.
pub mod codes {
    /// No such session — or not in memory, for a call that needs it there.
    pub const SESSION_NOT_FOUND: i32 = -32000;
    /// The session has a turn running.
    pub const SESSION_BUSY: i32 = -32015;
}

/// The longest session title an extension's engine may give: past it, the hub cuts it.
pub const TITLE_MAX: usize = 200;

/// The session options only the hub fills — from the session's space and preset — and a caller
/// never gives: the tools and skills a session sees, whether someone answers its questions, its
/// naming, and its prompt.
pub const HUB_OWNED_OPTIONS: [&str; 5] = ["host_tools", "skill_dirs", "attended", "auto_name", "prompt"];

/// The turn states a session can be in: a turn is running, or the engine holds the session to
/// rebuild it or to operate on its context.
const TURN_STATES: &[&str] = &["idle", "running", "rebuilding", "operating"];

#[async_trait::async_trait]
pub trait EngineOps: Send + Sync {
    /// A new session. `params` are the engine's own, as the caller gave them.
    async fn create(&self, params: Value) -> Answer<SessionCreated>;

    /// One session, live or only on disk.
    async fn get(&self, session: &str) -> Answer<SessionInfo>;

    /// The engine's top-level sessions.
    async fn list(&self) -> Answer<Vec<SessionInfo>>;

    /// Subscribe this link to a session's events. `None`: the session is not in memory, and has
    /// to be resumed first.
    async fn subscribe(&self, session: &str) -> Answer<Option<Subscribed>>;

    /// Bring a session that exists only on disk back into memory, as `from` (its
    /// [`EngineOps::get`]) describes it.
    async fn resume(&self, session: &str, from: &SessionInfo) -> Answer<()>;

    /// How many messages the session's transcript holds.
    async fn history_len(&self, session: &str) -> Answer<u64>;

    /// The first `limit` messages of the session's transcript, as text.
    async fn transcript(&self, session: &str, limit: usize) -> Answer<Vec<Said>>;

    /// The session's tasks; none for a session not in memory.
    async fn tasks(&self, session: &str) -> Answer<Tasks>;

    /// Run one turn, and answer when it has ended — or, steered into a running turn, when that
    /// turn took it.
    async fn run_turn(&self, session: &str, input: TurnInput) -> Answer<Value>;

    async fn interrupt(&self, session: &str) -> Answer<Value>;

    /// Give the session a title: the engine's name for it, the one every client lists.
    async fn rename(&self, session: &str, title: &str) -> Answer<Value>;

    /// Change the model the session's next turn calls, and its provider when named.
    async fn set_model(&self, session: &str, model: &str, provider: Option<&str>) -> Answer<Value>;

    /// Replace the host skill directories the session lists, as a whole (none: it lists none).
    /// The next turn sees them; a session not in memory has them from when it is resumed.
    async fn set_skill_dirs(&self, session: &str, dirs: &[PathBuf]) -> Answer<Value>;

    /// Close the session, or `delete` it. `params` are the engine's own (a dry run, say).
    async fn close(&self, params: Value, delete: bool) -> Answer<Value>;

    /// A new session from one that exists. `params` are the engine's own.
    async fn fork(&self, params: Value) -> Answer<SessionCreated>;

    /// An entry in the session's log, for people and never for the model.
    async fn append_entry(&self, session: &str, entry: Entry) -> Answer<Value>;

    /// Answer a question the session asked. `params` are the engine's own.
    async fn respond(&self, params: Value) -> Answer<Value>;

    /// Any other engine method, under the name the hub's row gives it; the answer as it is.
    async fn passthrough(&self, method: &str, params: Value) -> Answer<Value>;
}

/// A session just made.
#[non_exhaustive]
#[derive(Debug, Clone)]
pub struct SessionCreated {
    pub session_id: Option<String>,
    /// The engine's answer, as it gave it.
    pub value: Value,
}

impl SessionCreated {
    pub fn new(value: Value) -> Self {
        let session_id = value.get("session_id").and_then(Value::as_str).map(str::to_string);
        Self { session_id, value }
    }
}

/// One session as the engine describes it: its row in a list, or the whole of it.
#[non_exhaustive]
#[derive(Debug, Clone, Default)]
pub struct SessionInfo {
    value: Value,
}

impl SessionInfo {
    pub fn new(value: Value) -> Self {
        Self { value }
    }

    pub fn session_id(&self) -> Option<&str> {
        self.value.get("session_id").and_then(Value::as_str)
    }

    /// What it is called.
    pub fn title(&self) -> Option<&str> {
        self.value.get("name").and_then(Value::as_str)
    }

    /// A turn is running in it.
    pub fn running(&self) -> bool {
        self.turn_state() == Some("running")
    }

    /// Nothing is running in it.
    pub fn idle(&self) -> bool {
        self.turn_state() == Some("idle")
    }

    /// The engine holds it with no turn to interrupt: it is rebuilding in place, or an operation
    /// on its context has it alone.
    pub fn held(&self) -> bool {
        matches!(self.turn_state(), Some("rebuilding" | "operating"))
    }

    fn turn_state(&self) -> Option<&str> {
        self.value.get("turn_state").and_then(Value::as_str)
    }

    /// It is in the engine's memory, not only on disk.
    pub fn loaded(&self) -> bool {
        self.value.get("status").and_then(Value::as_str) == Some("active")
    }

    /// The turn running in it, when one is.
    pub fn current_turn(&self) -> Option<&str> {
        self.value.get("current_turn_id").and_then(Value::as_str)
    }

    /// How many times its transcript has been compacted; zero when it does not say.
    pub fn compactions(&self) -> u64 {
        self.value.get("compact_count").and_then(Value::as_u64).unwrap_or(0)
    }

    /// The scene it runs in.
    pub fn scene(&self) -> Option<&str> {
        self.value.get("scene").and_then(Value::as_str)
    }

    /// The project it works in.
    pub fn project_root(&self) -> Option<&Value> {
        self.value.get("project_root").filter(|r| !r.is_null())
    }

    /// Held to what a session can say, for an engine under third-party control: its title cut to
    /// [`TITLE_MAX`] characters, a turn state outside the known ones dropped.
    pub fn harden(&mut self) {
        let Some(obj) = self.value.as_object_mut() else { return };
        if let Some(Value::String(title)) = obj.get_mut("name") {
            if let Some((cut, _)) = title.char_indices().nth(TITLE_MAX) {
                title.truncate(cut);
            }
        }
        let known = obj.get("turn_state").and_then(Value::as_str).is_none_or(|s| TURN_STATES.contains(&s));
        if !known {
            obj.insert("turn_state".into(), Value::Null);
        }
    }

    pub fn value(&self) -> &Value {
        &self.value
    }

    pub fn into_value(self) -> Value {
        self.value
    }
}

/// A subscription taken: where the engine's sequence stands, and what is waiting on a person.
#[non_exhaustive]
#[derive(Debug, Clone, Default)]
pub struct Subscribed {
    pub last_seq: Option<u64>,
    pub pending: Vec<Prompt>,
}

impl Subscribed {
    pub fn new(last_seq: Option<u64>, pending: Vec<Prompt>) -> Self {
        Self { last_seq, pending }
    }
}

/// A session's tasks: the engine's answer, and which of them are running or waiting.
#[non_exhaustive]
#[derive(Debug, Clone, Default)]
pub struct Tasks {
    pub value: Value,
    pub live: BTreeSet<String>,
}

impl Tasks {
    pub fn new(value: Value, live: BTreeSet<String>) -> Self {
        Self { value, live }
    }
}

/// One message of a transcript, as text.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Said {
    pub role: String,
    pub text: String,
}

impl Said {
    pub fn new(role: impl Into<String>, text: impl Into<String>) -> Self {
        Self { role: role.into(), text: text.into() }
    }
}

/// What a turn is given.
#[non_exhaustive]
#[derive(Debug, Clone)]
pub struct TurnInput {
    pub turn_id: String,
    pub message: String,
    pub attachments: Value,
    /// Handed to the running turn, which reads it at its next step boundary, rather than run as
    /// a turn of its own.
    pub steer: bool,
}

impl TurnInput {
    pub fn new(turn_id: impl Into<String>, message: impl Into<String>, attachments: Value) -> Self {
        Self { turn_id: turn_id.into(), message: message.into(), attachments, steer: false }
    }

    /// A message for the running turn. Attachments do not steer.
    pub fn steer(turn_id: impl Into<String>, message: impl Into<String>) -> Self {
        Self { turn_id: turn_id.into(), message: message.into(), attachments: json!([]), steer: true }
    }
}

/// An entry for a session's log.
#[non_exhaustive]
#[derive(Debug, Clone)]
pub struct Entry {
    /// Whose: an extension writes under its own.
    pub ns: String,
    pub event: String,
    pub payload: Value,
}

impl Entry {
    pub fn new(ns: impl Into<String>, event: impl Into<String>, payload: Value) -> Self {
        Self { ns: ns.into(), event: event.into(), payload }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn hardened(state: &str) -> SessionInfo {
        let mut info = SessionInfo::new(json!({"turn_state": state}));
        info.harden();
        info
    }

    #[test]
    fn a_session_the_engine_holds_is_neither_running_nor_free() {
        let rebuilding = hardened("rebuilding");
        assert!(rebuilding.held() && !rebuilding.running() && !rebuilding.idle());
        assert!(hardened("operating").held());
        assert!(!hardened("running").held());
        assert!(!hardened("weird").held() && hardened("weird").value()["turn_state"].is_null());
    }
}
