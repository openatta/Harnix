//! Harnix's [`SessionHost`]: the runner's sessions, through the hub.
//!
//! A session is named `<instance>/<session_id>`, as run records have always held it. Opening one
//! places it as the station said (the flow's space by default, an instance, or an instance of a
//! template made ready first), asks the hub for it under an id derived from the runner's key — so
//! opening the same key again, after a restart too, is the same session — shows the main session
//! where the start said, and names it. Watching one is a hub tap, its frames put into the runner's
//! words: a turn's start, its tool calls and results, its permission questions (the only ones the
//! runner answers), its sub-agents, and how it ended.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use harnix_ext::{Entry, Exposure, OnBusy, RpcError, SessionSpec, Tap, TapEvent};
use serde_json::{json, Value};

use super::lock;
use super::state::{session_name, session_ref, Shared};
use crate::kernel::host::Placement;
use crate::kernel::protocol::ask_rules;
use crate::kernel::run::Usage;
use crate::runner::{Decision, Frame, HostError, Note, OpenSpec, Opened, SessionHost, SessionId, SessionInfo, TurnEnd, Watch};

/// The permission mode a task's session asks for: the most permissive one that still goes
/// through the session's permission rules without a classifier. The engine only ever tightens
/// what a client asks for, so an instance set stricter keeps its own mode; one set to
/// `bypassPermissions` — which ignores a session's rules — is held to this one instead, so the
/// flow's answers to its questions still hold.
pub const TASK_PERMISSION_MODE: &str = "acceptEdits";

pub(crate) struct HarnixSessionHost {
    sh: Arc<Shared>,
    /// What each key was opened with, while this process runs: a second open of a key with
    /// another spec is refused here; across restarts the engine compares the options.
    opened: Mutex<HashMap<String, OpenSpec>>,
}

impl HarnixSessionHost {
    pub(crate) fn new(sh: Arc<Shared>) -> Arc<Self> {
        Arc::new(Self { sh, opened: Mutex::new(HashMap::new()) })
    }

    fn cx(&self) -> Result<&harnix_ext::ExtensionContext, HostError> {
        self.sh.cx.get().ok_or_else(|| HostError::Unavailable("the flow extension has not started".into()))
    }

    /// Where a new session goes: the station's space (the extension's when it names none), its
    /// instance, or an instance of its template made ready first; the extension's own space
    /// without an instance means `main`, another space its own default. `Err`: why there is
    /// nowhere to go, as the task's error.
    async fn place(&self, p: &Placement, run: &str) -> Result<(String, Option<String>), HostError> {
        let cx = self.cx()?;
        let space = p.space.clone().unwrap_or_else(|| self.sh.cfg.space.clone());
        if let (Some(template), Some(name)) = (&p.template, p.template_instance(run)) {
            let params = p.params.iter().filter(|(k, _)| k.as_str() != "name").map(|(k, v)| (k.clone(), v.clone())).collect();
            // Named by the extension, the instance is the run's: recorded before it is made,
            // stopped when the run is over (`run_over`).
            if p.names_its_instance() {
                self.sh.own_instance(&name, run);
            }
            cx.instances()
                .ensure_ready(template, &name, params, Duration::from_millis(self.sh.cfg.ensure_ms))
                .await
                .map_err(|e| HostError::Refused(format!("template {template} gave no instance `{name}`: {}", e.message)))?;
            return Ok((space, Some(name)));
        }
        let instance = match &p.instance {
            Some(i) => Some(i.clone()),
            None if space == self.sh.cfg.space => Some(self.sh.cfg.instance.clone()),
            None => None,
        };
        Ok((space, instance))
    }

    /// After a restart: the instances of runs that ended while the extension was down.
    pub(crate) async fn stop_stale_instances(&self) {
        let Some(engine) = self.sh.engine.get().cloned() else {
            return;
        };
        for run in self.sh.instance_runs() {
            let over = match engine.record(&run).await {
                Ok(rec) => !rec.is_active(),
                Err(crate::FlowError::NotFound(_)) => true,
                Err(_) => false,
            };
            if over {
                self.run_over(&run).await;
            }
        }
    }
}

/// A hub error in the host's words: a session that is not there is `Gone`; the hub or the
/// engine being away `Unavailable`; a refusal `Refused`.
fn host_error(e: RpcError) -> HostError {
    use harnix_ext::codes;
    match e.code {
        codes::NOT_FOUND => HostError::Gone(e.message),
        codes::INSTANCE_UNAVAILABLE => HostError::Unavailable(e.message),
        codes::REFUSED | codes::INVALID_PARAMS | codes::CONFLICT => HostError::Refused(e.message),
        _ => HostError::Other(e.message),
    }
}

/// The session id a key is opened under: a v5 UUID of the key, written as the engine writes its
/// own ids (BASE58), so the same key is the same session across restarts.
pub(crate) fn session_id_of(key: &str) -> String {
    let ns = uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_DNS, b"flow.harnix");
    base58(uuid::Uuid::new_v5(&ns, key.as_bytes()).as_bytes())
}

/// BASE58 (the Bitcoin alphabet), as the engine writes a UUID.
pub(crate) fn base58(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 58] = b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
    let zeros = bytes.iter().take_while(|b| **b == 0).count();
    let mut digits: Vec<u8> = vec![];
    for &b in &bytes[zeros..] {
        let mut carry = b as u32;
        for d in digits.iter_mut() {
            carry += (*d as u32) << 8;
            *d = (carry % 58) as u8;
            carry /= 58;
        }
        while carry > 0 {
            digits.push((carry % 58) as u8);
            carry /= 58;
        }
    }
    let mut out = String::with_capacity(zeros + digits.len());
    out.extend(std::iter::repeat_n('1', zeros));
    out.extend(digits.iter().rev().map(|&d| ALPHABET[d as usize] as char));
    out
}

#[async_trait::async_trait]
impl SessionHost for HarnixSessionHost {
    async fn open(&self, spec: OpenSpec) -> Result<Opened, HostError> {
        let cx = self.cx()?;
        if let Some(had) = lock(&self.opened).get(&spec.key) {
            if *had != spec {
                return Err(HostError::Refused(format!("key {} is open with another spec", spec.key)));
            }
        }
        let (space, instance) = self.place(&spec.placement, &spec.run).await?;
        let mut s = SessionSpec::new()
            .space(&space)
            .scene(&spec.profile)
            .permission_mode(TASK_PERMISSION_MODE)
            .permission_rules(ask_rules(&spec.asks))
            .session_id(&session_id_of(&spec.key));
        if let Some(i) = &instance {
            s = s.instance(i);
        }
        if let Some(root) = &spec.project_root {
            s = s.project_root(root);
        }
        if let Some(m) = &spec.placement.model {
            s = s.model(m);
        }
        if let Some(p) = &spec.placement.provider {
            s = s.provider(p);
        }
        if let Some(e) = &spec.placement.effort {
            s = s.effort(e);
        }
        if let Some(schema) = &spec.output_schema {
            s = s.output_schema(schema.clone());
        }
        if let Some(r) = &spec.recording {
            s = s.recording(r);
        }
        let created = cx.sessions().open_or_create(s).await.map_err(host_error)?;
        let session = created.session;
        if spec.main && !created.existed {
            // A task started from where a project's sessions are listed is shown there too, to be
            // talked to: the session stays the flow's, in the flow's space. A space the application
            // does not have is the hub's to refuse; the run goes on, and the task opens from the
            // flow's side.
            if let Some(shown) = spec.shown_in.as_deref().filter(|shown| *shown != space) {
                if let Err(e) = cx.sessions().expose(&session, shown, Exposure::interactive()).await {
                    tracing::warn!(run = spec.run, space = shown, error = %e.message, "the main session is not shown in the space the start named");
                }
            }
            if let Some(title) = &spec.title {
                let _ = cx
                    .call(
                        "engine.session.rename",
                        json!({"instance": session.instance, "session_id": session.session_id, "name": title}),
                    )
                    .await;
            }
        }
        lock(&self.opened).insert(spec.key.clone(), spec);
        Ok(Opened { id: session_name(&session), existed: created.existed })
    }

    async fn lookup(&self, id: &SessionId) -> Result<Option<SessionInfo>, HostError> {
        let cx = self.cx()?;
        let s = session_ref(id);
        match cx.call("engine.session.get", json!({"instance": s.instance, "session_id": s.session_id})).await {
            Ok(v) => Ok(Some(info_of(&v))),
            Err(e) if e.code == harnix_ext::codes::NOT_FOUND => Ok(None),
            Err(e) => Err(host_error(e)),
        }
    }

    async fn watch(&self, id: &SessionId) -> Result<Watch, HostError> {
        let cx = self.cx()?;
        let s = session_ref(id);
        let tap = cx.sessions().tap(&s).await.map_err(host_error)?;
        let info = cx
            .call("engine.session.get", json!({"instance": s.instance, "session_id": s.session_id}))
            .await
            .map(|v| info_of(&v))
            .unwrap_or_default();
        Ok(Watch { frames: Box::pin(frames_of(tap, s.instance.clone())), info })
    }

    async fn send(&self, id: &SessionId, text: &str) -> Result<(), HostError> {
        let cx = self.cx()?;
        cx.sessions().send(&session_ref(id), text, OnBusy::Queue).await.map(|_| ()).map_err(host_error)
    }

    async fn interrupt(&self, id: &SessionId) -> Result<(), HostError> {
        let cx = self.cx()?;
        cx.sessions().interrupt(&session_ref(id)).await.map(|_| ()).map_err(host_error)
    }

    async fn close(&self, id: &SessionId) -> Result<(), HostError> {
        let cx = self.cx()?;
        cx.sessions().close(&session_ref(id)).await.map(|_| ()).map_err(host_error)
    }

    async fn answer(&self, id: &SessionId, ask: &str, decision: Decision) -> Result<(), HostError> {
        let cx = self.cx()?;
        let s = session_ref(id);
        let d = match decision {
            Decision::Allow => json!({"type": "permit"}),
            Decision::Deny { reason } => json!({"type": "deny", "reason": reason}),
        };
        cx.call(
            "harnix.session.respond",
            json!({
                "instance": s.instance, "session_id": s.session_id,
                "prompt_id": ask, "prompt_type": "permission", "decision": d,
            }),
        )
        .await
        .map(|_| ())
        .map_err(host_error)
    }

    async fn note(&self, id: &SessionId, note: Note) -> Result<(), HostError> {
        let cx = self.cx()?;
        let s = session_ref(id);
        let mut payload = note.payload;
        // The card names the session it is in, for the interface to act on it.
        if let Some(obj) = payload.as_object_mut() {
            obj.entry("session").or_insert_with(|| json!({"instance": s.instance, "session_id": s.session_id}));
        }
        cx.sessions().append_entry(&s, Entry::new(note.event, payload)).await.map(|_| ()).map_err(host_error)
    }

    /// Whether the engine says `root` is trusted (`engine.project.status`, which `engine.control`
    /// reaches). Before the extension has started, as last heard.
    async fn project_trusted(&self, root: &Path) -> Result<bool, HostError> {
        let root = root.to_string_lossy();
        let Some(cx) = self.sh.cx.get() else {
            return Ok(self.sh.projects.trusted(&root));
        };
        let status = cx
            .call("engine.project.status", json!({"instance": super::FLOW_INSTANCE, "project_root": root}))
            .await
            .map_err(host_error)?;
        Ok(status.get("trust").and_then(Value::as_str) == Some("trusted"))
    }

    /// Stop the instances the extension made for `run` under its own names: a spawned daemon per
    /// run would otherwise outlive it, and a template's `max` run out.
    async fn run_over(&self, run: &str) {
        let Some(cx) = self.sh.cx.get() else { return };
        for name in self.sh.take_instances(run) {
            match cx.instances().stop(&name).await {
                Ok(_) => tracing::debug!(run, instance = name, "the run's instance stopped"),
                Err(e) => tracing::info!(run, instance = name, error = %e, "the run's instance not stopped"),
            }
        }
    }
}

/// What `engine.session.get` says, in the host's words.
fn info_of(v: &Value) -> SessionInfo {
    SessionInfo {
        project_root: v.get("project_root").and_then(Value::as_str).map(std::path::PathBuf::from),
        turn_running: v.get("turn_state").and_then(Value::as_str).is_some_and(|s| s != "idle"),
    }
}

/// Every frame of a tap, in the runner's words; what was missed in a gap is caught up first, and
/// the gap said.
fn frames_of(tap: Tap, instance: String) -> impl futures::Stream<Item = Frame> + Send {
    let start = (tap, std::collections::VecDeque::<Frame>::new(), Translator::new(instance));
    futures::stream::unfold(start, |(mut tap, mut queued, mut seen)| async move {
        loop {
            if let Some(f) = queued.pop_front() {
                return Some((f, (tap, queued, seen)));
            }
            match tap.next().await? {
                TapEvent::Frame(f) => queued.extend(seen.translate(&f)),
                TapEvent::Lagged { .. } => {
                    let _ = tap.catch_up_settled().await;
                    queued.push_back(Frame::Lagged);
                }
                _ => {}
            }
        }
    })
}

/// Hub frames into the runner's: `{method: harnix.event, params: {turn_id, event: {kind, …}}}`
/// and `{method: harnix.turn_settled, params: {turn_id, result | error, outcome}}`.
#[derive(Default)]
pub(crate) struct Translator {
    /// The instance the session is on: what a sub-agent's session is named under.
    instance: String,
    /// The sub-agents seen started.
    agents: HashSet<String>,
}

impl Translator {
    pub(crate) fn new(instance: String) -> Self {
        Self { instance, agents: HashSet::new() }
    }

    /// The runner's frames for one hub frame: none for what the runner does not read.
    pub(crate) fn translate(&mut self, frame: &Value) -> Vec<Frame> {
        let turn = frame.pointer("/params/turn_id").and_then(Value::as_str).unwrap_or_default().to_string();
        match frame.get("method").and_then(Value::as_str) {
            Some("harnix.turn_settled") => {
                let params = frame.get("params").cloned().unwrap_or(Value::Null);
                vec![Frame::TurnSettled {
                    turn,
                    end: turn_end(&params),
                    output: params.pointer("/outcome/structured_output").filter(|v| !v.is_null()).cloned(),
                    usage: params.pointer("/outcome/usage").filter(|u| u.is_object()).map(Usage::of_turn),
                }]
            }
            Some("harnix.event") => {
                let Some(ev) = frame.pointer("/params/event") else { return vec![] };
                // A sub-agent's calls and questions come wrapped in its progress.
                let (inner, agent) = match ev.get("kind").and_then(Value::as_str) {
                    Some("subagent_progress") => {
                        let Some(id) = ev.get("agent_session_id").and_then(Value::as_str) else { return vec![] };
                        (ev.get("event").unwrap_or(ev), Some(format!("{}/{id}", self.instance)))
                    }
                    _ => (ev, None),
                };
                let mut out = vec![];
                if let Some(agent) = &agent {
                    if self.agents.insert(agent.clone()) {
                        out.push(Frame::AgentStarted {
                            turn: turn.clone(),
                            call: ev.get("parent_tool_use_id").and_then(Value::as_str).map(String::from),
                            agent: agent.clone(),
                            agent_type: ev.get("agent_type").and_then(Value::as_str).map(String::from),
                        });
                    }
                }
                let str_of = |k: &str| inner.get(k).and_then(Value::as_str).map(String::from);
                match inner.get("kind").and_then(Value::as_str) {
                    Some("user_message") => {
                        if agent.is_none() {
                            out.push(Frame::TurnStarted { turn, text: str_of("text").unwrap_or_default() });
                        }
                    }
                    Some("tool_use") => {
                        if let Some(call) = str_of("id") {
                            out.push(Frame::ToolCall {
                                turn,
                                call,
                                tool: str_of("name").unwrap_or_default(),
                                input: inner.get("input").cloned().unwrap_or(Value::Null),
                                agent,
                            });
                        }
                    }
                    Some("tool_result") => {
                        // The engine names the call `id`; a hub may say `tool_use_id`.
                        if let Some(call) = str_of("id").or_else(|| str_of("tool_use_id")) {
                            out.push(Frame::ToolResult { turn, call, agent });
                        }
                    }
                    // Only a permission question is the runner's: one for words (the model's, an
                    // MCP server's) is a person's.
                    Some("prompt") if inner.get("prompt_type").and_then(Value::as_str).is_none_or(|t| t == "permission") => {
                        if let Some(ask) = str_of("prompt_id") {
                            out.push(Frame::Asked {
                                turn,
                                ask,
                                call: str_of("tool_use_id"),
                                tool: str_of("tool_name").unwrap_or_default(),
                                paths: inner
                                    .get("paths")
                                    .and_then(Value::as_array)
                                    .map(|a| a.iter().filter_map(|p| p.as_str().map(String::from)).collect())
                                    .unwrap_or_default(),
                                agent,
                            });
                        }
                    }
                    Some("turn_complete") if agent.is_some() => {
                        out.push(Frame::AgentEnded { turn, agent: agent.unwrap_or_default(), ok: true });
                    }
                    Some("error") if agent.is_some() => {
                        out.push(Frame::AgentEnded { turn, agent: agent.unwrap_or_default(), ok: false });
                    }
                    _ => {}
                }
                out
            }
            _ => vec![],
        }
    }
}

/// How a turn ended, from `harnix.turn_settled`'s params: an `error` (the engine's kind and
/// whether a retry may work ride in its `data`), an interrupted `stop_reason`, else done.
fn turn_end(params: &Value) -> TurnEnd {
    if let Some(e) = params.get("error").filter(|e| !e.is_null()) {
        let data = e.get("data").cloned().unwrap_or(Value::Null);
        return TurnEnd::Error {
            retryable: data.get("retryable").and_then(Value::as_bool).unwrap_or(true),
            kind: data.get("kind").and_then(Value::as_str).unwrap_or("error").to_string(),
            message: e.get("message").and_then(Value::as_str).unwrap_or_default().to_string(),
        };
    }
    match params.pointer("/outcome/stop_reason").and_then(Value::as_str) {
        Some("interrupted") => TurnEnd::Interrupted,
        _ => TurnEnd::Done,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_is_the_same_session_id_every_time_written_as_the_engine_writes_one() {
        let a = session_id_of("flow:r-1@main");
        assert_eq!(a, session_id_of("flow:r-1@main"));
        assert_ne!(a, session_id_of("flow:r-2@main"));
        assert!(a.len() >= 21 && a.len() <= 22 && a.chars().all(|c| c.is_ascii_alphanumeric()), "{a}");
        // BASE58 of a UUID, with the alphabet the engine uses.
        assert_eq!(base58(&[0u8; 16]), "1111111111111111");
        assert_eq!(base58(&[0x00, 0xff]), "15Q");
        let u = uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_DNS, b"flow:r-1@main");
        assert_eq!(u.to_string(), "74bb92f5-ded8-5413-a9d3-0a4e80ba0c4c");
        assert_eq!(base58(u.as_bytes()), "FR3tkA8wTXcvtWsBd1V66P");
    }

    /// The frames a hub tap gives, as the LIVE recordings show them, in the runner's words.
    #[test]
    fn hub_frames_are_put_into_the_runners_words() {
        let mut t = Translator::new("main".into());
        let ev = |turn: &str, event: Value| json!({"method": "harnix.event", "params": {"turn_id": turn, "seq": 1, "event": event}});
        assert_eq!(
            t.translate(&ev("t1", json!({"kind": "user_message", "text": "Do it."}))),
            [Frame::TurnStarted { turn: "t1".into(), text: "Do it.".into() }]
        );
        assert_eq!(
            t.translate(&ev("t1", json!({"kind": "tool_use", "id": "c1", "name": "Write", "input": {"file_path": "a.md"}}))),
            [Frame::ToolCall { turn: "t1".into(), call: "c1".into(), tool: "Write".into(), input: json!({"file_path": "a.md"}), agent: None }]
        );
        assert_eq!(
            t.translate(&ev("t1", json!({"kind": "prompt", "prompt_type": "permission", "prompt_id": "p1", "tool_name": "Write", "tool_use_id": "c1", "paths": ["/p/a.md"]}))),
            [Frame::Asked { turn: "t1".into(), ask: "p1".into(), call: Some("c1".into()), tool: "Write".into(), paths: vec!["/p/a.md".into()], agent: None }]
        );
        assert!(t.translate(&ev("t1", json!({"kind": "prompt", "prompt_type": "question", "prompt_id": "q1", "tool_name": "AskUserQuestion"}))).is_empty(), "words are a person's");
        assert_eq!(
            t.translate(&ev("t1", json!({"kind": "tool_result", "tool_use_id": "c1"}))),
            [Frame::ToolResult { turn: "t1".into(), call: "c1".into(), agent: None }]
        );
        assert!(t.translate(&ev("t1", json!({"kind": "text_delta", "text": "x"}))).is_empty());
        // A sub-agent's progress: started once, its calls and questions under its name, its end.
        let sub = |event: Value| json!({"kind": "subagent_progress", "agent_session_id": "a1", "agent_type": "general", "parent_tool_use_id": "call_Agent_1", "event": event});
        assert_eq!(
            t.translate(&ev("t1", sub(json!({"kind": "tool_use", "id": "c9", "name": "Bash", "input": {"command": "ls"}})))),
            [
                Frame::AgentStarted { turn: "t1".into(), call: Some("call_Agent_1".into()), agent: "main/a1".into(), agent_type: Some("general".into()) },
                Frame::ToolCall { turn: "t1".into(), call: "c9".into(), tool: "Bash".into(), input: json!({"command": "ls"}), agent: Some("main/a1".into()) },
            ]
        );
        assert_eq!(
            t.translate(&ev("t1", sub(json!({"kind": "prompt", "prompt_type": "permission", "prompt_id": "p9", "tool_name": "Bash"})))),
            [Frame::Asked { turn: "t1".into(), ask: "p9".into(), call: None, tool: "Bash".into(), paths: vec![], agent: Some("main/a1".into()) }]
        );
        assert_eq!(
            t.translate(&ev("t1", sub(json!({"kind": "turn_complete", "stop_reason": "end_turn"})))),
            [Frame::AgentEnded { turn: "t1".into(), agent: "main/a1".into(), ok: true }]
        );
        // How a turn ended.
        let settled = |params: Value| json!({"method": "harnix.turn_settled", "params": params});
        assert_eq!(
            t.translate(&settled(json!({"turn_id": "t1", "result": "done", "outcome": {"stop_reason": "end_turn", "structured_output": {"n": 1}, "usage": {"input_tokens": 3, "output_tokens": 2}}}))),
            [Frame::TurnSettled { turn: "t1".into(), end: TurnEnd::Done, output: Some(json!({"n": 1})), usage: Some(Usage::of_turn(&json!({"input_tokens": 3, "output_tokens": 2}))) }]
        );
        assert!(matches!(
            t.translate(&settled(json!({"turn_id": "t2", "result": "done", "outcome": {"stop_reason": "interrupted"}})))[0],
            Frame::TurnSettled { end: TurnEnd::Interrupted, .. }
        ));
        let Frame::TurnSettled { end, .. } = &t.translate(&settled(json!({"turn_id": "t3", "error": {"code": -32000, "message": "no_credentials: none for acme", "data": {"kind": "no_credentials", "retryable": false}}})))[0] else { panic!() };
        assert_eq!(end, &TurnEnd::Error { retryable: false, kind: "no_credentials".into(), message: "no_credentials: none for acme".into() });
        let Frame::TurnSettled { end, .. } = &t.translate(&settled(json!({"turn_id": "t4", "error": {"code": -32000, "message": "boom"}})))[0] else { panic!() };
        assert!(matches!(end, TurnEnd::Error { retryable: true, .. }), "an error that says nothing may pass");
    }

    #[test]
    fn what_the_hub_says_of_a_session_is_read() {
        let info = info_of(&json!({"project_root": "/p", "turn_state": "running"}));
        assert_eq!(info, SessionInfo { project_root: Some("/p".into()), turn_running: true });
        assert_eq!(info_of(&json!({"turn_state": "idle"})), SessionInfo::default());
    }
}
