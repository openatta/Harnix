//! Agix's JSON-RPC, in the engine interface's terms: the one place that knows what Agix calls its
//! methods and its events.
//!
//! A link speaks Agix's protocol underneath — a call into an engine in this process, or a line
//! on a daemon's socket — and the hub sees only [`EngineOps`] and [`EngineEvent`]. Every name the
//! hub would otherwise have had to spell is a constant here, and `crates/harnix/tests/layering.rs`
//! reads them to check that the hub spells none of them.

use std::collections::BTreeSet;
use std::sync::Arc;

use harnix_contract::RpcError;
use harnix_engine_api::events::{Completion, DaemonEventKind, EngineEvent, Fragment, Joins, Prompt, SessionEventKind, Waiting};
use harnix_engine_api::ops::{codes, Answer, EngineOps, Entry, Said, SessionCreated, SessionInfo, Subscribed, Tasks, TurnInput};
use serde_json::{json, Value};

pub const SESSION_CREATE: &str = "session.create";
pub const SESSION_GET: &str = "session.get";
pub const SESSION_LIST: &str = "session.list";
pub const SESSION_SUBSCRIBE: &str = "session.subscribe";
pub const SESSION_RESUME: &str = "session.resume";
pub const SESSION_HISTORY: &str = "session.history";
pub const SESSION_TASKS: &str = "session.tasks";
pub const SESSION_RUN_TURN: &str = "session.run_turn";
pub const SESSION_INTERRUPT: &str = "session.interrupt";
pub const SESSION_RENAME: &str = "session.rename";
pub const SESSION_SET_MODEL: &str = "session.setModel";
pub const SESSION_CLOSE: &str = "session.close";
pub const SESSION_DELETE: &str = "session.delete";
pub const SESSION_FORK: &str = "session.fork";
pub const SESSION_APPEND_ENTRY: &str = "session.appendEntry";
pub const SESSION_RESPOND: &str = "session.respondToPrompt";
pub const CONFIG_RELOAD: &str = "config.reload";
pub const DAEMON_SUBSCRIBE_EVENTS: &str = "daemon.subscribeEvents";
pub const DAEMON_PING: &str = "daemon.ping";
pub const DAEMON_AUTH: &str = "daemon.auth";

/// The two notifications an engine sends.
pub const SESSION_EVENT: &str = "session.event";
pub const DAEMON_EVENT: &str = "daemon.event";

// What a session's events are called.
const TEXT_DELTA: &str = "text_delta";
const THINKING_DELTA: &str = "thinking_delta";
const TOOL_ARGS_DELTA: &str = "tool_args_delta";
const TOOL_USE: &str = "tool_use";
const TOOL_RESULT: &str = "tool_result";
const PROMPT: &str = "prompt";
const GOAL_CHANGED: &str = "goal_changed";
const TEAM_PROGRESS: &str = "team_progress";
const TASK_STARTED: &str = "task_started";
const TASK_FINISHED: &str = "task_finished";
const COMPACT: &str = "compact";
const TURN_COMPLETE: &str = "turn_complete";
const SUBSCRIPTION_DROPPED: &str = "subscription_dropped";

// What the engine's own events are called.
const TURN_STARTED: &str = "turn_started";
const TURN_COMPLETED: &str = "turn_completed";
const SESSION_CREATED: &str = "session_created";
const SESSION_RESUMED: &str = "session_resumed";
const SESSION_EVICTED: &str = "session_evicted";
const SESSION_CLOSED: &str = "session_closed";
const SESSION_DELETED: &str = "session_deleted";
const EVENTS_LAGGED: &str = "events_lagged";

/// A question whose answer is words, not a permission.
const QUESTION: &str = "question";
/// The tool a plan is handed in by: leaving plan mode is asked as a permission, and the plan waits
/// on a review.
const PLAN_TOOL: &str = "ExitPlanMode";

/// One of Agix's calls, as a link makes it over its stream.
#[async_trait::async_trait]
pub(crate) trait Rpc: Send + Sync + 'static {
    async fn call(&self, method: &str, params: Value) -> Result<Value, RpcError>;
}

/// [`EngineOps`] over Agix's calls.
pub(crate) struct AgixOps(pub(crate) Arc<dyn Rpc>);

impl AgixOps {
    async fn call(&self, method: &str, params: Value) -> Result<Value, RpcError> {
        self.0.call(method, params).await
    }
}

#[async_trait::async_trait]
impl EngineOps for AgixOps {
    async fn create(&self, params: Value) -> Answer<SessionCreated> {
        self.call(SESSION_CREATE, params).await.map(SessionCreated::new)
    }

    async fn get(&self, session: &str) -> Answer<SessionInfo> {
        self.call(SESSION_GET, json!({"session_id": session})).await.map(SessionInfo::new)
    }

    async fn list(&self) -> Answer<Vec<SessionInfo>> {
        let listed = self.call(SESSION_LIST, json!({})).await?;
        let rows = match listed.get("sessions") {
            Some(Value::Array(rows)) => rows.iter().cloned().map(SessionInfo::new).collect(),
            _ => Vec::new(),
        };
        Ok(rows)
    }

    async fn subscribe(&self, session: &str) -> Answer<Option<Subscribed>> {
        match self.call(SESSION_SUBSCRIBE, json!({"session_id": session})).await {
            Ok(answer) => {
                let pending = match answer.get("pending_prompts") {
                    Some(Value::Array(asks)) => asks.iter().cloned().map(prompt).collect(),
                    _ => Vec::new(),
                };
                Ok(Some(Subscribed::new(answer.get("last_seq").and_then(Value::as_u64), pending)))
            }
            // Only live sessions have subscribers.
            Err(e) if e.code == codes::SESSION_NOT_FOUND => Ok(None),
            Err(e) => Err(e),
        }
    }

    async fn resume(&self, session: &str, from: &SessionInfo) -> Answer<()> {
        let mut params = json!({"session_id": session});
        if let Some(scene) = from.scene() {
            params["scene"] = json!(scene);
        }
        if let Some(root) = from.project_root() {
            params["project_root"] = root.clone();
        }
        self.call(SESSION_RESUME, params).await.map(|_| ())
    }

    async fn history_len(&self, session: &str) -> Answer<u64> {
        let page = self.call(SESSION_HISTORY, json!({"session_id": session, "limit": 0})).await?;
        Ok(page.get("total").and_then(Value::as_u64).unwrap_or(0))
    }

    async fn transcript(&self, session: &str, limit: usize) -> Answer<Vec<Said>> {
        let page = self
            .call(SESSION_HISTORY, json!({"session_id": session, "offset": 0, "limit": limit}))
            .await?;
        let said = page
            .get("messages")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .map(|m| Said::new(m.get("role").and_then(Value::as_str).unwrap_or("?"), message_text(m)))
            .collect();
        Ok(said)
    }

    async fn tasks(&self, session: &str) -> Answer<Tasks> {
        let value = match self.call(SESSION_TASKS, json!({"session_id": session})).await {
            Ok(value) => value,
            // Not in memory: nothing of it is running.
            Err(e) if e.code == codes::SESSION_NOT_FOUND => json!({"active": false, "tasks": []}),
            Err(e) => return Err(e),
        };
        let live: BTreeSet<String> = value
            .get("tasks")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|t| matches!(t.get("status").and_then(Value::as_str), Some("running" | "waiting" | "pending")))
            .filter_map(|t| t.get("task_id").and_then(Value::as_str).map(str::to_string))
            .collect();
        Ok(Tasks::new(value, live))
    }

    async fn run_turn(&self, session: &str, input: TurnInput) -> Answer<Value> {
        let params = if input.steer {
            json!({"session_id": session, "turn_id": input.turn_id, "message": input.message, "on_busy": "steer"})
        } else {
            json!({"session_id": session, "turn_id": input.turn_id, "message": input.message, "attachments": input.attachments})
        };
        self.call(SESSION_RUN_TURN, params).await
    }

    async fn interrupt(&self, session: &str) -> Answer<Value> {
        self.call(SESSION_INTERRUPT, json!({"session_id": session})).await
    }

    async fn rename(&self, session: &str, title: &str) -> Answer<Value> {
        self.call(SESSION_RENAME, json!({"session_id": session, "name": title})).await
    }

    async fn set_model(&self, session: &str, model: &str, provider: Option<&str>) -> Answer<Value> {
        let mut params = json!({"session_id": session, "model": model});
        if let Some(provider) = provider {
            params["provider"] = json!(provider);
        }
        self.call(SESSION_SET_MODEL, params).await
    }

    async fn close(&self, params: Value, delete: bool) -> Answer<Value> {
        self.call(if delete { SESSION_DELETE } else { SESSION_CLOSE }, params).await
    }

    async fn fork(&self, params: Value) -> Answer<SessionCreated> {
        self.call(SESSION_FORK, params).await.map(SessionCreated::new)
    }

    async fn append_entry(&self, session: &str, entry: Entry) -> Answer<Value> {
        let params = json!({"session_id": session, "ns": entry.ns, "event": entry.event, "payload": entry.payload});
        self.call(SESSION_APPEND_ENTRY, params).await
    }

    async fn respond(&self, params: Value) -> Answer<Value> {
        self.call(SESSION_RESPOND, params).await
    }

    async fn passthrough(&self, method: &str, params: Value) -> Answer<Value> {
        self.call(agix_method(method), params).await
    }
}

/// The Agix method a row of the hub's passes through to: its name without the `engine.` the
/// kernel's names put in front of the engine's.
fn agix_method(row: &str) -> &str {
    row.strip_prefix("engine.").unwrap_or(row)
}

/// One frame an engine sent unasked, as the hub reads it; `None` for one that is not an event.
pub(crate) fn event(frame: Value) -> Option<EngineEvent> {
    let Value::Object(mut frame) = frame else { return None };
    let params = frame.remove("params").unwrap_or(Value::Null);
    match frame.get("method").and_then(Value::as_str) {
        Some(SESSION_EVENT) => session_event(params),
        Some(DAEMON_EVENT) => Some(daemon_event(params)),
        _ => None,
    }
}

fn session_event(mut params: Value) -> Option<EngineEvent> {
    let session = params.get("session_id").and_then(Value::as_str)?.to_string();
    let event = params.get_mut("event").map(Value::take).unwrap_or(Value::Null);
    let kind_name = event.get("kind").and_then(Value::as_str).unwrap_or("");
    if kind_name == SUBSCRIPTION_DROPPED {
        return Some(EngineEvent::SubscriptionDropped { session });
    }
    let turn_id = params.get("turn_id").and_then(Value::as_str).unwrap_or("").to_string();
    let field = |k: &str| event.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let kind = match kind_name {
        TEXT_DELTA => SessionEventKind::Fragment(Fragment::new(Joins::Answer, "text", field("text"))),
        THINKING_DELTA => SessionEventKind::Fragment(Fragment::new(Joins::Thinking, "text", field("text"))),
        TOOL_ARGS_DELTA => {
            SessionEventKind::Fragment(Fragment::new(Joins::ToolArgs(field("id")), "partial_json", field("partial_json")))
        }
        TOOL_USE => SessionEventKind::ToolCall { id: field("id"), name: field("name") },
        TOOL_RESULT => SessionEventKind::ToolResult {
            id: field("id"),
            is_error: event.get("is_error").and_then(Value::as_bool).unwrap_or(false),
        },
        PROMPT => SessionEventKind::Asked(prompt(event.clone())),
        GOAL_CHANGED => SessionEventKind::GoalChanged { goal: event.get("goal").cloned().unwrap_or(Value::Null) },
        TEAM_PROGRESS => match event.get("team_id").and_then(Value::as_str) {
            Some(team) => SessionEventKind::TeamProgress { team_id: team.to_string() },
            None => SessionEventKind::Other,
        },
        TASK_STARTED | TASK_FINISHED => {
            let task = event.get("task");
            match task.and_then(|t| t.get("task_id")).and_then(Value::as_str) {
                Some(id) => SessionEventKind::Task {
                    task_id: id.to_string(),
                    started: kind_name == TASK_STARTED,
                    session: task
                        .and_then(|t| t.pointer("/detail/agent_session_id"))
                        .and_then(Value::as_str)
                        .map(str::to_string),
                },
                None => SessionEventKind::Other,
            }
        }
        COMPACT => SessionEventKind::Compacted,
        TURN_COMPLETE => SessionEventKind::TurnComplete(completion(&event)),
        _ => SessionEventKind::Other,
    };
    Some(EngineEvent::Session { session, turn_id, kind, event })
}

fn daemon_event(params: Value) -> EngineEvent {
    let field = |k: &str| params.get(k).and_then(Value::as_str).map(str::to_string);
    let session = field("session_id");
    let turn_id = field("turn_id").unwrap_or_default();
    let kind = match (params.get("kind").and_then(Value::as_str).unwrap_or(""), session) {
        (TURN_STARTED, Some(session)) => DaemonEventKind::TurnStarted { session, turn_id },
        (TURN_COMPLETED, Some(session)) => DaemonEventKind::TurnCompleted {
            session,
            turn_id,
            initiator: params.get("initiator").cloned().unwrap_or(Value::Null),
        },
        (SESSION_CREATED | SESSION_RESUMED, session) => DaemonEventKind::SessionsChanged { session, ended: false },
        (SESSION_EVICTED | SESSION_CLOSED | SESSION_DELETED, session) => {
            DaemonEventKind::SessionsChanged { session, ended: true }
        }
        (EVENTS_LAGGED, _) => DaemonEventKind::Lagged,
        _ => DaemonEventKind::Other,
    };
    EngineEvent::Daemon { kind, event: params }
}

/// A question as the engine put it, with what it waits for.
fn prompt(value: Value) -> Prompt {
    let field = |k: &str| value.get(k).and_then(Value::as_str).map(str::to_string);
    let waiting = match (field("prompt_type").as_deref(), field("tool_name").as_deref()) {
        (Some(QUESTION), _) => Waiting::Answer,
        (_, Some(PLAN_TOOL)) => Waiting::Plan,
        _ => Waiting::Approval,
    };
    let (prompt_id, tool_use_id) = (field("prompt_id"), field("tool_use_id"));
    Prompt::new(value, prompt_id, tool_use_id, waiting)
}

fn completion(event: &Value) -> Completion {
    let present = |k: &str| event.get(k).filter(|v| !v.is_null()).cloned();
    Completion::new(
        event.get("stop_reason").cloned().unwrap_or(Value::Null),
        event.get("usage").cloned().unwrap_or(Value::Null),
    )
    .with_structured_output(present("structured_output"))
    .with_message(present("message"))
}

/// The text of one transcript message, whether its content is a string or blocks.
fn message_text(message: &Value) -> String {
    match message.get("content") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(parts)) => parts
            .iter()
            .filter_map(|p| p.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(event: Value) -> EngineEvent {
        event_of(json!({"jsonrpc": "2.0", "method": SESSION_EVENT, "params": {"session_id": "s", "turn_id": "t", "event": event}}))
    }

    fn event_of(frame: Value) -> EngineEvent {
        super::event(frame).expect("an event")
    }

    fn kind(e: EngineEvent) -> SessionEventKind {
        match e {
            EngineEvent::Session { kind, .. } => kind,
            other => panic!("not a session event: {other:?}"),
        }
    }

    /// The engine's error codes the hub tells apart are the engine's own numbers.
    #[test]
    fn the_codes_the_hub_tells_apart_are_the_engines() {
        assert_eq!(codes::SESSION_NOT_FOUND, daemon::rpc::codes::SESSION_NOT_FOUND);
        assert_eq!(codes::SESSION_BUSY, daemon::rpc::codes::SESSION_BUSY);
    }

    #[test]
    fn a_session_event_keeps_the_engines_value_and_says_what_it_is() {
        let e = session(json!({"kind": "text_delta", "text": "hel"}));
        let EngineEvent::Session { session, turn_id, kind, event } = e else { panic!() };
        assert_eq!((session.as_str(), turn_id.as_str()), ("s", "t"));
        assert_eq!(kind, SessionEventKind::Fragment(Fragment::new(Joins::Answer, "text", "hel")));
        assert_eq!(event, json!({"kind": "text_delta", "text": "hel"}));

        let args = kind_of(json!({"kind": "tool_args_delta", "id": "c", "partial_json": "{"}));
        assert_eq!(args, SessionEventKind::Fragment(Fragment::new(Joins::ToolArgs("c".into()), "partial_json", "{")));
        assert_eq!(
            kind_of(json!({"kind": "task_finished", "task": {"task_id": "k"}})),
            SessionEventKind::Task { task_id: "k".into(), started: false, session: None }
        );
        assert_eq!(
            kind_of(json!({"kind": "task_started", "task": {"task_id": "k", "detail": {"agent_session_id": "sub"}}})),
            SessionEventKind::Task { task_id: "k".into(), started: true, session: Some("sub".into()) }
        );
        assert_eq!(kind_of(json!({"kind": "assistant_message"})), SessionEventKind::Other);
    }

    fn kind_of(event: Value) -> SessionEventKind {
        kind(session(event))
    }

    #[test]
    fn a_question_says_what_it_waits_for() {
        let asked = |e: Value| match kind_of(e) {
            SessionEventKind::Asked(p) => p,
            other => panic!("{other:?}"),
        };
        let p = asked(json!({"kind": "prompt", "prompt_id": "p", "tool_use_id": "u", "tool_name": "Bash"}));
        assert_eq!((p.prompt_id.as_deref(), p.tool_use_id.as_deref(), p.waiting), (Some("p"), Some("u"), Waiting::Approval));
        assert_eq!(asked(json!({"kind": "prompt", "prompt_type": "question"})).waiting, Waiting::Answer);
        assert_eq!(asked(json!({"kind": "prompt", "tool_name": "ExitPlanMode"})).waiting, Waiting::Plan);
    }

    #[test]
    fn a_dropped_subscription_and_the_engines_own_events_are_told_apart() {
        assert!(matches!(
            session(json!({"kind": "subscription_dropped"})),
            EngineEvent::SubscriptionDropped { session } if session == "s"
        ));
        let daemon = |p: Value| match event_of(json!({"jsonrpc": "2.0", "method": DAEMON_EVENT, "params": p})) {
            EngineEvent::Daemon { kind, .. } => kind,
            other => panic!("{other:?}"),
        };
        assert_eq!(
            daemon(json!({"kind": "turn_completed", "session_id": "s", "turn_id": "t", "initiator": "schedule"})),
            DaemonEventKind::TurnCompleted { session: "s".into(), turn_id: "t".into(), initiator: json!("schedule") }
        );
        assert_eq!(
            daemon(json!({"kind": "session_deleted", "session_id": "s"})),
            DaemonEventKind::SessionsChanged { session: Some("s".into()), ended: true }
        );
        assert_eq!(daemon(json!({"kind": "events_lagged"})), DaemonEventKind::Lagged);
        assert!(super::event(json!({"jsonrpc": "2.0", "id": 3, "result": {}})).is_none(), "a reply is no event");
    }

    #[test]
    fn a_completion_is_the_points_a_caller_acts_on() {
        let SessionEventKind::TurnComplete(c) = kind_of(json!({
            "kind": "turn_complete", "stop_reason": "end_turn", "usage": {"input_tokens": 1},
            "structured_output": null, "message": "said",
        })) else {
            panic!()
        };
        assert_eq!(c.stop_reason, json!("end_turn"));
        assert_eq!(c.structured_output, None, "null is not an output");
        assert_eq!(c.message, Some(json!("said")));
    }
}
