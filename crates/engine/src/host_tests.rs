//! A link's tools, hooks and gates against a daemon that is a test: registered before the link is
//! ready, called and answered on the link without the hub seeing the calls, registered again on
//! the next connection and after a change.

use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use harnix_contract::sync::lock;
use harnix_engine_api::link::{EngineControl, EngineLink, EventReceiver, InstanceCarrier};
use harnix_engine_api::{
    EngineContribution, EngineEvent, HookDecision, Permission, PromptDecision, PromptFacts, PromptGate, SceneBase, SceneLayer, SessionEventKind,
    Tool, ToolCallFacts, ToolCx, ToolError, ToolHook, ToolOutput, ToolProvider, ToolSpec, ToolsChanged,
};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, DuplexStream, ReadHalf, WriteHalf};

use crate::daemon_link::{Connection, DaemonLink, Refusal, Taken};
use crate::host::{Chains, HostSide};
use crate::translate;

const WAIT: Duration = Duration::from_secs(5);

/// The daemon's end of a link's connection: what it was asked, and what it answers or pushes.
struct HostFake {
    lines: tokio::io::Lines<BufReader<ReadHalf<DuplexStream>>>,
    writer: WriteHalf<DuplexStream>,
}

impl HostFake {
    /// The next request: its id, method and params.
    async fn next(&mut self) -> (u64, String, Value) {
        let line = tokio::time::timeout(WAIT, self.lines.next_line()).await.expect("a request in time").unwrap().expect("a request");
        let v: Value = serde_json::from_str(&line).unwrap();
        (v["id"].as_u64().unwrap(), v["method"].as_str().unwrap().to_string(), v["params"].clone())
    }

    async fn expect(&mut self, method: &str) -> (u64, Value) {
        let (id, got, params) = self.next().await;
        assert_eq!(got, method, "{params}");
        (id, params)
    }

    async fn write(&mut self, frame: Value) {
        self.writer.write_all(format!("{frame}\n").as_bytes()).await.unwrap();
    }

    async fn answer(&mut self, id: u64, result: Value) {
        self.write(json!({"jsonrpc": "2.0", "id": id, "result": result})).await;
    }

    async fn fail(&mut self, id: u64, code: i32, message: &str) {
        self.write(json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})).await;
    }

    /// A session event, unasked.
    async fn push(&mut self, session: &str, event: Value) {
        self.write(json!({"jsonrpc": "2.0", "method": "session.event", "params": {"session_id": session, "turn_id": "t1", "event": event}}))
            .await;
    }

    /// Answer the ping with `capabilities`, and the subscription.
    async fn greet(&mut self, capabilities: &[&str]) {
        let (id, _) = self.expect(translate::DAEMON_PING).await;
        self.answer(id, json!({"version": "x", "protocol_version": crate::AGIX_PROTOCOL, "capabilities": capabilities})).await;
        let (id, _) = self.expect(translate::DAEMON_SUBSCRIBE_EVENTS).await;
        self.answer(id, json!({})).await;
    }

    /// Take the registration a link with tools, tool hooks and a gate makes, accepting it.
    async fn registered(&mut self) -> (Value, Value) {
        let (id, tools) = self.expect(translate::DAEMON_SET_HOST_TOOLS).await;
        let names: Vec<Value> = tools["host_tools"].as_array().unwrap().iter().map(|t| t["name"].clone()).collect();
        self.answer(id, json!({"host_tools": names, "refused": []})).await;
        let (id, hooks) = self.expect(translate::DAEMON_SET_HOOKS).await;
        self.answer(id, json!({"hooks": [], "refused": []})).await;
        (tools, hooks)
    }
}

fn pair() -> (Connection, HostFake) {
    let (ours, theirs) = tokio::io::duplex(1 << 16);
    let (r, w) = tokio::io::split(ours);
    let (fr, fw) = tokio::io::split(theirs);
    (Connection::new(r, w), HostFake { lines: BufReader::new(fr).lines(), writer: fw })
}

fn taking(link: &Arc<DaemonLink>, connection: Connection) -> tokio::task::JoinHandle<Result<Taken, Refusal>> {
    let link = link.clone();
    tokio::spawn(async move { link.take(connection, None).await })
}

/// Says back its session, project and input; fails, is missing, reports progress or waits to
/// be cancelled when its input says so.
struct Echo(ToolSpec);

#[async_trait::async_trait]
impl Tool for Echo {
    fn spec(&self) -> &ToolSpec {
        &self.0
    }
    fn permission(&self, _: &Value) -> Permission {
        Permission::Allow
    }
    async fn call(&self, cx: ToolCx, input: Value) -> Result<ToolOutput, ToolError> {
        if input["fail"].as_bool() == Some(true) {
            return Err(ToolError::Failed("it broke".into()));
        }
        if input["missing"].as_bool() == Some(true) {
            return Err(ToolError::NotFound("nothing there".into()));
        }
        if input["wait"].as_bool() == Some(true) {
            cx.cancel().cancelled().await;
            return Err(ToolError::Cancelled);
        }
        if input["progress"].as_bool() == Some(true) {
            for step in ["one", "two", "three"] {
                cx.progress().send(step);
            }
        }
        let said = format!("{} {} {} {}", cx.session_id(), cx.project().display(), cx.depth(), input["text"]);
        Ok(ToolOutput::text(said).with_structured(Some(json!(1))))
    }
}

fn echo(name: &str) -> Arc<dyn Tool> {
    Arc::new(Echo(ToolSpec::new(name, "says it back", json!({"type": "object"})).doc("at length")))
}

struct Note(HookDecision);

#[async_trait::async_trait]
impl ToolHook for Note {
    async fn before(&self, call: &ToolCallFacts) -> HookDecision {
        assert_eq!((call.name(), call.session_id(), call.cwd()), ("Bash", "s1", std::path::Path::new("/w")));
        self.0.clone()
    }
}

struct Provider {
    tools: StdMutex<Vec<Arc<dyn Tool>>>,
    hooks: Vec<Arc<dyn ToolHook>>,
    changed: ToolsChanged,
}

impl ToolProvider for Provider {
    fn name(&self) -> &str {
        "test"
    }
    fn tools(&self) -> Vec<Arc<dyn Tool>> {
        lock(&self.tools).clone()
    }
    fn hooks(&self) -> Vec<Arc<dyn ToolHook>> {
        self.hooks.clone()
    }
    fn changes(&self) -> Vec<ToolsChanged> {
        vec![self.changed.clone()]
    }
}

struct Words;

#[async_trait::async_trait]
impl PromptGate for Words {
    async fn inspect(&self, message: &PromptFacts) -> PromptDecision {
        if message.content().contains("blocked-word") {
            PromptDecision::Block("not that word".into())
        } else {
            PromptDecision::Proceed
        }
    }
}

fn provider(hooks: Vec<HookDecision>) -> Arc<Provider> {
    Arc::new(Provider {
        tools: StdMutex::new(vec![echo("echo")]),
        hooks: hooks.into_iter().map(|d| Arc::new(Note(d)) as Arc<dyn ToolHook>).collect(),
        changed: ToolsChanged::new(),
    })
}

fn host(provider: &Arc<Provider>) -> Arc<HostSide> {
    let mut contrib = EngineContribution::new();
    contrib.provider(provider.clone()).prompt_gate(Arc::new(Words));
    contrib.scene(SceneLayer::new("probe", SceneBase::builtin("harnix")).tool(echo("layer_echo")));
    HostSide::new("main", Chains::default(), &contrib).unwrap()
}

/// A link that has taken a connection, its tools and hooks registered, and its hub's events.
async fn ready(provider: &Arc<Provider>) -> (Arc<DaemonLink>, HostFake, Taken, EventReceiver) {
    let link = DaemonLink::with_host(InstanceCarrier::InProcess, "test", host(provider));
    let events = link.take_events().unwrap();
    let (connection, mut fake) = pair();
    let taken = taking(&link, connection);
    fake.greet(crate::REQUIRED_CAPABILITIES).await;
    fake.registered().await;
    let taken = taken.await.unwrap().map_err(|r| format!("{r:?}")).unwrap();
    (link, fake, taken, events)
}

async fn first_event(events: &mut EventReceiver) -> EngineEvent {
    tokio::time::timeout(WAIT, events.recv()).await.expect("an event in time").expect("an event")
}

fn tool_call(call_id: &str, input: Value) -> Value {
    json!({"kind": "host_tool_call", "call_id": call_id, "tool_use_id": "u1", "name": "echo", "input": input,
           "timeout_ms": 600000, "task_id": null, "agent_session_id": null})
}

#[tokio::test]
async fn a_link_with_tools_declares_them_before_it_is_ready() {
    let provider = provider(vec![HookDecision::Proceed]);
    let link = DaemonLink::with_host(InstanceCarrier::InProcess, "test", host(&provider));
    let (connection, mut fake) = pair();
    let taken = taking(&link, connection);
    fake.greet(crate::REQUIRED_CAPABILITIES).await;
    let (id, tools) = fake.expect(translate::DAEMON_SET_HOST_TOOLS).await;
    assert!(!link.status().borrow().is_ready(), "ready before its tools were declared");
    let decl = &tools["host_tools"][0];
    assert_eq!((&decl["name"], &decl["description"]), (&json!("echo"), &json!("says it back\n\nat length")));
    assert_eq!((&decl["timeout_ms"], &decl["subagents"]), (&json!(600000), &json!(true)));
    fake.answer(id, json!({"host_tools": ["echo"], "refused": []})).await;
    let (id, hooks) = fake.expect(translate::DAEMON_SET_HOOKS).await;
    assert!(!link.status().borrow().is_ready(), "ready before its hooks were registered");
    let hooks = hooks["hooks"].as_array().unwrap().clone();
    assert_eq!(hooks.len(), 2);
    assert_eq!((&hooks[0]["events"], &hooks[0]["tools"], &hooks[0]["on_error"]), (&json!(["PreToolUse"]), &json!(["*"]), &json!("allow")));
    assert_eq!((&hooks[1]["events"], &hooks[1]["on_error"]), (&json!(["UserPromptSubmit"]), &json!("block")));
    assert!(hooks.iter().all(|h| h["timeout_ms"] == 30000));
    fake.answer(id, json!({"hooks": [], "refused": []})).await;
    let _taken = taken.await.unwrap().map_err(|r| format!("{r:?}")).unwrap();
    assert!(link.status().borrow().is_ready());
}

#[tokio::test]
async fn a_host_tool_call_is_answered_by_its_tool_and_not_handed_to_the_hub() {
    let (_link, mut fake, _taken, mut events) = ready(&provider(vec![])).await;
    fake.push("s1", tool_call("c1", json!({"text": "hi"}))).await;
    fake.push("s1", json!({"kind": "text_delta", "text": "after"})).await;
    let (id, asked) = fake.expect(translate::SESSION_GET).await;
    assert_eq!(asked["session_id"], "s1");
    fake.answer(id, json!({"session_id": "s1", "project_root": "/p"})).await;
    let (id, answer) = fake.expect(translate::SESSION_RESPOND_TOOL_CALL).await;
    assert_eq!((&answer["session_id"], &answer["call_id"]), (&json!("s1"), &json!("c1")));
    assert_eq!(answer["result"], json!({"content": "s1 /p 0 \"hi\"", "is_error": false}), "the structured part is left out");
    fake.answer(id, json!({"call_id": "c1"})).await;
    match first_event(&mut events).await {
        EngineEvent::Session { kind: SessionEventKind::Fragment(f), .. } => assert_eq!(f, harnix_engine_api::Fragment::new(harnix_engine_api::Joins::Answer, "text", "after")),
        other => panic!("the hub was handed {other:?} first"),
    }
}

/// A session's own tool, from the scene layer it was created in, is not declared on the connection
/// and is answered on the link as the connection's are.
#[tokio::test]
async fn a_scene_layers_tool_is_answered_on_the_link() {
    let provider = provider(vec![]);
    let (link, mut fake, _taken, mut events) = ready(&provider).await;
    assert!(link.scene_layers().get("probe").is_some());
    let mut call = tool_call("c1", json!({"text": "own"}));
    call["name"] = json!("layer_echo");
    fake.push("s1", call).await;
    fake.push("s1", json!({"kind": "text_delta", "text": "after"})).await;
    let (id, _) = fake.expect(translate::SESSION_GET).await;
    fake.answer(id, json!({"project_root": "/p"})).await;
    let (_, answer) = fake.expect(translate::SESSION_RESPOND_TOOL_CALL).await;
    assert_eq!(answer["result"]["content"], "s1 /p 0 \"own\"");
    assert!(matches!(first_event(&mut events).await, EngineEvent::Session { kind: SessionEventKind::Fragment(_), .. }), "the hub saw the call");
}

/// A delegate's call is the delegate's session's, one deep, and is answered for the root.
#[tokio::test]
async fn a_delegates_call_is_its_own_and_answered_for_the_root() {
    let (_link, mut fake, _taken, _events) = ready(&provider(vec![])).await;
    let mut call = tool_call("c1", json!({"text": "hi"}));
    call["agent_session_id"] = json!("sub");
    fake.push("s1", call).await;
    let (id, _) = fake.expect(translate::SESSION_GET).await;
    fake.answer(id, json!({"project_root": "/p"})).await;
    let (_, answer) = fake.expect(translate::SESSION_RESPOND_TOOL_CALL).await;
    assert_eq!(answer["session_id"], "s1");
    assert_eq!(answer["result"]["content"], "sub /p 1 \"hi\"");
}

#[tokio::test]
async fn a_tool_that_fails_or_is_missing_answers_as_the_engine_reads_it() {
    let (_link, mut fake, _taken, mut events) = ready(&provider(vec![])).await;
    fake.push("s1", tool_call("c1", json!({"fail": true}))).await;
    let (id, _) = fake.expect(translate::SESSION_GET).await;
    fake.answer(id, json!({"project_root": "/p"})).await;
    let (_, failed) = fake.expect(translate::SESSION_RESPOND_TOOL_CALL).await;
    assert_eq!(failed["result"], json!({"content": "it broke", "is_error": true}));
    // The project is asked once per session.
    fake.push("s1", tool_call("c2", json!({"missing": true}))).await;
    let (_, missing) = fake.expect(translate::SESSION_RESPOND_TOOL_CALL).await;
    assert_eq!(missing["error"], json!({"message": "nothing there"}));
    assert!(missing.get("result").is_none());

    let mut other = tool_call("c3", json!({}));
    other["name"] = json!("not_ours");
    fake.push("s1", other).await;
    match first_event(&mut events).await {
        EngineEvent::Session { kind: SessionEventKind::Other, event, .. } => assert_eq!(event["call_id"], "c3"),
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn progress_goes_to_report_tool_progress() {
    let (_link, mut fake, _taken, _events) = ready(&provider(vec![])).await;
    fake.push("s1", tool_call("c1", json!({"progress": true}))).await;
    let (id, _) = fake.expect(translate::SESSION_GET).await;
    fake.answer(id, json!({"project_root": "/p"})).await;
    let (mut reports, mut answered) = (Vec::new(), false);
    while !(answered && reports.last() == Some(&json!("three"))) {
        let (id, method, params) = fake.next().await;
        match method.as_str() {
            translate::SESSION_REPORT_TOOL_PROGRESS => {
                assert_eq!((&params["session_id"], &params["call_id"]), (&json!("s1"), &json!("c1")));
                reports.push(params["message"].clone());
            }
            translate::SESSION_RESPOND_TOOL_CALL => answered = true,
            other => panic!("{other}"),
        }
        fake.answer(id, json!({"call_id": "c1"})).await;
    }
    assert!(!reports.is_empty() && reports.len() <= 3, "{reports:?}");
}

#[tokio::test]
async fn a_withdrawn_call_cancels_the_tool() {
    let (_link, mut fake, _taken, _events) = ready(&provider(vec![])).await;
    fake.push("s1", tool_call("c1", json!({"wait": true}))).await;
    let (id, _) = fake.expect(translate::SESSION_GET).await;
    fake.answer(id, json!({"project_root": "/p"})).await;
    fake.push("s1", json!({"kind": "host_tool_call_resolved", "call_id": "c1", "resolution": "withdrawn", "reason": "cancelled"})).await;
    let (_, answer) = fake.expect(translate::SESSION_RESPOND_TOOL_CALL).await;
    assert_eq!(answer["error"], json!({"message": "cancelled"}));
}

fn hook_call(call_id: &str, event: &str, input: Value) -> Value {
    json!({"kind": "hook_call", "call_id": call_id, "hook_event": event, "input": input, "timeout_ms": 30000,
           "task_id": null, "agent_session_id": null})
}

fn bash() -> Value {
    json!({"hook_event_name": "PreToolUse", "session_id": "s1", "cwd": "/w", "tool_name": "Bash", "tool_input": {"command": "ls"}})
}

#[tokio::test]
async fn a_pre_tool_use_hook_blocks_or_adds_context() {
    let (_link, mut fake, _taken, mut events) =
        ready(&provider(vec![HookDecision::AddContext("  look first ".into()), HookDecision::Proceed])).await;
    fake.push("s1", hook_call("h1", "PreToolUse", bash())).await;
    fake.push("s1", json!({"kind": "text_delta", "text": "after"})).await;
    let (_, answer) = fake.expect(translate::SESSION_RESPOND_HOOK).await;
    assert_eq!(answer, json!({"session_id": "s1", "call_id": "h1", "decision": "proceed", "additional_context": "look first"}));
    assert!(matches!(first_event(&mut events).await, EngineEvent::Session { kind: SessionEventKind::Fragment(_), .. }), "the hub saw the hook");

    let (_link, mut fake, _taken, _events) =
        ready(&provider(vec![HookDecision::Block("not here".into()), HookDecision::AddContext("unseen".into())])).await;
    fake.push("s1", hook_call("h2", "PreToolUse", bash())).await;
    let (_, answer) = fake.expect(translate::SESSION_RESPOND_HOOK).await;
    assert_eq!((&answer["decision"], &answer["message"]), (&json!("block"), &json!("not here")));
    assert!(answer.get("additional_context").is_none());
}

#[tokio::test]
async fn a_prompt_gate_answers_user_prompt_submit() {
    let (_link, mut fake, _taken, _events) = ready(&provider(vec![])).await;
    let said = |text: &str| json!({"hook_event_name": "UserPromptSubmit", "session_id": "s1", "user_prompt": text, "delivery": "turn"});
    fake.push("s1", hook_call("h1", "UserPromptSubmit", said("a blocked-word here"))).await;
    let (_, blocked) = fake.expect(translate::SESSION_RESPOND_HOOK).await;
    assert_eq!((&blocked["decision"], &blocked["message"]), (&json!("block"), &json!("not that word")));
    fake.push("s1", hook_call("h2", "UserPromptSubmit", said("hello"))).await;
    let (_, passed) = fake.expect(translate::SESSION_RESPOND_HOOK).await;
    assert_eq!(passed["decision"], "proceed");
}

#[tokio::test]
async fn a_reconnection_registers_again_before_the_hub_hears_of_it() {
    let provider = provider(vec![HookDecision::Proceed]);
    let (link, fake, taken, mut events) = ready(&provider).await;
    drop(fake);
    taken.lost().await;
    link.disconnect().await;
    link.degrade("the connection closed".into());
    let (connection, mut fake) = pair();
    let taking = taking(&link, connection);
    fake.greet(crate::REQUIRED_CAPABILITIES).await;
    let (id, tools) = fake.expect(translate::DAEMON_SET_HOST_TOOLS).await;
    assert_eq!(tools["host_tools"][0]["name"], "echo");
    assert!(events.try_recv().is_err(), "the hub heard of the reconnection before the tools were back");
    fake.answer(id, json!({"host_tools": ["echo"], "refused": []})).await;
    let (id, _) = fake.expect(translate::DAEMON_SET_HOOKS).await;
    fake.answer(id, json!({"hooks": [], "refused": []})).await;
    let _taken = taking.await.unwrap().map_err(|r| format!("{r:?}")).unwrap();
    assert!(matches!(first_event(&mut events).await, EngineEvent::Reconnected));
}

#[tokio::test]
async fn a_change_declares_the_tools_again() {
    let provider = provider(vec![]);
    let (_link, mut fake, _taken, _events) = ready(&provider).await;
    lock(&provider.tools).push(echo("second"));
    provider.changed.notify();
    let (id, tools) = fake.expect(translate::DAEMON_SET_HOST_TOOLS).await;
    let names: Vec<&str> = tools["host_tools"].as_array().unwrap().iter().filter_map(|t| t["name"].as_str()).collect();
    assert_eq!(names, ["echo", "second"]);
    fake.answer(id, json!({"host_tools": names, "refused": []})).await;
}

#[tokio::test]
async fn a_daemon_without_the_capabilities_is_refused_for_good() {
    let link = DaemonLink::with_host(InstanceCarrier::InProcess, "test", host(&provider(vec![])));
    let (connection, mut fake) = pair();
    let taken = taking(&link, connection);
    let lacking: Vec<&str> = crate::REQUIRED_CAPABILITIES.iter().copied().filter(|c| *c != "host_hooks").collect();
    let (id, _) = fake.expect(translate::DAEMON_PING).await;
    fake.answer(id, json!({"version": "x", "protocol_version": crate::AGIX_PROTOCOL, "capabilities": lacking})).await;
    match taken.await.unwrap() {
        Err(Refusal::Fatal(why)) => assert!(why.contains("host_hooks"), "{why}"),
        other => panic!("{:?}", other.map(|_| ())),
    }
    assert!(!link.status().borrow().is_ready());
}

#[tokio::test]
async fn a_refused_gate_hook_refuses_the_connection() {
    let link = DaemonLink::with_host(InstanceCarrier::InProcess, "test", host(&provider(vec![])));
    let (connection, mut fake) = pair();
    let taken = taking(&link, connection);
    fake.greet(crate::REQUIRED_CAPABILITIES).await;
    let (id, _) = fake.expect(translate::DAEMON_SET_HOST_TOOLS).await;
    fake.answer(id, json!({"host_tools": ["echo"], "refused": []})).await;
    let (id, _) = fake.expect(translate::DAEMON_SET_HOOKS).await;
    fake.fail(id, -32602, "eight connections have hooks already").await;
    assert!(matches!(taken.await.unwrap(), Err(Refusal::Retry(_))));
    assert!(!link.status().borrow().is_ready());
}

/// A link that offers nothing asks nothing beyond the handshake.
#[tokio::test]
async fn a_link_that_offers_nothing_registers_nothing() {
    let link = DaemonLink::new(InstanceCarrier::InProcess, "test");
    let (connection, mut fake) = pair();
    let taken = taking(&link, connection);
    fake.greet(crate::REQUIRED_CAPABILITIES).await;
    let _taken = taken.await.unwrap().map_err(|r| format!("{r:?}")).unwrap();
    assert!(link.status().borrow().is_ready());
    link.shutdown().await;
}
