//! The scenes flow sessions run in, and the tools they add.
//!
//! An application's scene is offered as `flow-<id>`: the same prompt, budget and tools, plus
//! three native tools. A tool finds the task it serves by the caller's session id; a session
//! that serves none is told so.
//!
//! | tool | does |
//! |---|---|
//! | `flow_state` | the run as far as this task is concerned: the station, what is still unmet |
//! | `flow_submit {output}` | hands the output in under the session's claim; problems come back to fix |
//! | `flow_propose {kind, to?, reason}` | writes a `proposal` entry; a person accepts it or not |

use std::sync::Arc;

use base::interface::prompt::PromptBlock;
use base::interface::scene::{
    AgentScene, ExecutionParams, ReminderContext, ScenePromptContext, TokenBudget,
};
use base::tool::{ProgressSender, Tool, ToolContext, ToolResult};
use harnix_module::{Entry, SessionRef};
use serde_json::{json, Value};

use super::dispatch;
use super::shared::{Binding, Shared};
use crate::kernel::run::Actor;
use crate::kernel::FlowError;

pub const TOOL_STATE: &str = "flow_state";
pub const TOOL_SUBMIT: &str = "flow_submit";
pub const TOOL_PROPOSE: &str = "flow_propose";
const OURS: [&str; 3] = [TOOL_STATE, TOOL_SUBMIT, TOOL_PROPOSE];

pub(crate) fn wrapped_id(base: &str) -> String {
    format!("flow-{base}")
}

/// An application's scene with the flow's tools added.
pub struct FlowScene {
    inner: Arc<dyn AgentScene>,
    id: String,
    name: String,
    description: String,
    sh: Arc<Shared>,
}

impl FlowScene {
    pub(crate) fn new(inner: Arc<dyn AgentScene>, sh: Arc<Shared>) -> Self {
        Self {
            id: wrapped_id(inner.id()),
            name: format!("{} (flow)", inner.name()),
            description: format!(
                "{} — working on a flow task, with flow_state, flow_submit and flow_propose",
                inner.description()
            ),
            inner,
            sh,
        }
    }
}

impl AgentScene for FlowScene {
    fn id(&self) -> &str {
        &self.id
    }
    fn name(&self) -> &str {
        &self.name
    }
    fn description(&self) -> &str {
        &self.description
    }
    fn build_system_prompt(&self, ctx: &ScenePromptContext) -> Vec<PromptBlock> {
        self.inner.build_system_prompt(ctx)
    }
    /// The base whitelist plus ours; an empty one (everything) stays empty.
    fn tools(&self) -> Vec<String> {
        let mut tools = self.inner.tools();
        if !tools.is_empty() {
            tools.extend(OURS.iter().map(|s| s.to_string()));
        }
        tools
    }
    fn requires_project(&self) -> bool {
        self.inner.requires_project()
    }
    fn supports_team(&self) -> bool {
        self.inner.supports_team()
    }
    fn extra_tools(&self) -> Vec<Arc<dyn Tool>> {
        let mut tools = self.inner.extra_tools();
        tools.push(Arc::new(StateTool(self.sh.clone())));
        tools.push(Arc::new(SubmitTool(self.sh.clone())));
        tools.push(Arc::new(ProposeTool(self.sh.clone())));
        tools
    }
    fn token_budget(&self) -> TokenBudget {
        self.inner.token_budget()
    }
    fn build_system_reminder(&self, ctx: &ReminderContext) -> String {
        self.inner.build_system_reminder(ctx)
    }
    fn disallowed_tools(&self) -> Vec<String> {
        self.inner.disallowed_tools()
    }
    fn permits_tool(&self, name: &str) -> bool {
        OURS.contains(&name) || self.inner.permits_tool(name)
    }
    fn deferred_tools(&self) -> Vec<String> {
        self.inner.deferred_tools()
    }
    fn execution_params(&self) -> ExecutionParams {
        self.inner.execution_params()
    }
    fn auto_name_session(&self) -> bool {
        self.inner.auto_name_session()
    }
    fn session_name_prompt(&self, first_message: &str) -> Option<String> {
        self.inner.session_name_prompt(first_message)
    }
    fn memory_extraction_prompt(&self) -> Option<String> {
        self.inner.memory_extraction_prompt()
    }
}

/// The task a calling session serves, or what to tell it.
fn bound(sh: &Shared, cx: &ToolContext) -> Result<Binding, ToolResult> {
    sh.binding(&cx.session_id).ok_or_else(|| {
        ToolResult::error_text(
            "This session is not working on a flow task (or its task is over): there is nothing \
             to read or hand in here.",
        )
    })
}

fn agent(b: &Binding, cx: &ToolContext) -> Actor {
    Actor::new("agent", format!("{}/{}", b.instance, cx.session_id))
}

fn pretty(v: &Value) -> String {
    serde_json::to_string_pretty(v).unwrap_or_else(|_| v.to_string())
}

type ToolOut = Result<ToolResult, base::error::ToolError>;

struct StateTool(Arc<Shared>);

#[async_trait::async_trait]
impl Tool for StateTool {
    fn name(&self) -> &str {
        TOOL_STATE
    }
    fn description(&self) -> &str {
        "The flow task this session is working on: the run, the station and its round, your \
         task and item, the output contract, what you handed in, and which exit conditions are \
         still not met."
    }
    fn input_schema(&self) -> Value {
        json!({"type": "object", "properties": {}})
    }
    fn is_read_only(&self, _: &Value) -> bool {
        true
    }
    fn is_concurrency_safe(&self, _: &Value) -> bool {
        true
    }
    async fn call(&self, _input: Value, cx: ToolContext, _p: ProgressSender) -> ToolOut {
        let b = match bound(&self.0, &cx) {
            Ok(b) => b,
            Err(r) => return Ok(r),
        };
        let Ok(engine) = self.0.engine() else {
            return Ok(ToolResult::error_text("the flow module has not started"));
        };
        let view = match engine.get(&b.run).await {
            Ok(v) => v,
            Err(e) => return Ok(ToolResult::error_text(e.to_string())),
        };
        let rec = &view.record;
        let st = rec.stations.get(&b.station);
        let task = st.and_then(|s| s.task(&b.task));
        let sdef = rec.def.station(&b.station);
        let state = json!({
            "run": rec.id,
            "flow": rec.flow,
            "subject": rec.subject,
            "run_status": rec.status,
            "station": b.station,
            "round": st.map(|s| s.round),
            "station_status": st.map(|s| s.status),
            "task": b.task,
            "task_status": task.map(|t| t.status),
            "item": task.and_then(|t| t.item.clone()),
            "submitted": task.and_then(|t| t.output.clone()),
            "contract": sdef.and_then(|s| s.output.clone()),
            "exits": sdef.map(|s| s.exit.iter().map(|e| e.label()).collect::<Vec<_>>()),
            "unmet": view.unmet.get(&b.station),
            "writes": b.writes,
            "current": task.is_some_and(|t| t.lease.as_ref().is_some_and(|l| l.token == b.token)),
        });
        Ok(ToolResult::text(pretty(&state)))
    }
}

struct SubmitTool(Arc<Shared>);

#[async_trait::async_trait]
impl Tool for SubmitTool {
    fn name(&self) -> &str {
        TOOL_SUBMIT
    }
    fn description(&self) -> &str {
        "Hand in this task's output: one JSON value satisfying the station's output contract. \
         Problems come back for you to fix; submit again until it is accepted. You may submit \
         again until the station passes; the last submission counts."
    }
    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["output"],
            "properties": {
                "output": {"description": "The output, as the flow protocol's contract says."}
            }
        })
    }
    /// It writes nothing outside the flow's own record, and a headless session has nobody
    /// to ask for permission.
    fn is_read_only(&self, _: &Value) -> bool {
        true
    }
    async fn call(&self, input: Value, cx: ToolContext, _p: ProgressSender) -> ToolOut {
        let b = match bound(&self.0, &cx) {
            Ok(b) => b,
            Err(r) => return Ok(r),
        };
        let Ok(engine) = self.0.engine() else {
            return Ok(ToolResult::error_text("the flow module has not started"));
        };
        let output = input.get("output").cloned().unwrap_or(Value::Null);
        let result = engine
            .submit(
                &b.run,
                &b.station,
                &b.task,
                &b.token,
                output,
                agent(&b, &cx),
            )
            .await;
        match result {
            Ok(o) => {
                let status = o.value.get("station").cloned().unwrap_or(Value::Null);
                dispatch::apply(&self.0, Some(&b.run), o).await;
                Ok(ToolResult::text(format!(
                    "Accepted. Station `{}` is now {}. If it is still working, others' tasks or \
                     its checks are outstanding; your part is done unless you want to improve \
                     your output.",
                    b.station,
                    status.as_str().unwrap_or("unknown")
                )))
            }
            Err(FlowError::Invalid(m)) => Ok(ToolResult::error_text(format!(
                "Not accepted: {m}. Fix the output and call {TOOL_SUBMIT} again."
            ))),
            Err(e) => Ok(ToolResult::error_text(format!(
                "Not accepted: {e}. Your claim on this task is no longer current; stop working on it."
            ))),
        }
    }
}

struct ProposeTool(Arc<Shared>);

#[async_trait::async_trait]
impl Tool for ProposeTool {
    fn name(&self) -> &str {
        TOOL_PROPOSE
    }
    fn description(&self) -> &str {
        "Propose that a person rework an upstream station (kind `rework`, `to` the station) or \
         cancel the run (kind `cancel`), with your reason. It is shown to people as a card; \
         nothing happens unless one of them accepts it."
    }
    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["kind", "reason"],
            "properties": {
                "kind": {"type": "string", "enum": ["rework", "cancel"]},
                "to": {"type": "string", "description": "With `rework`: the station to go back to."},
                "reason": {"type": "string"}
            }
        })
    }
    fn is_read_only(&self, _: &Value) -> bool {
        true
    }
    async fn call(&self, input: Value, cx: ToolContext, _p: ProgressSender) -> ToolOut {
        let b = match bound(&self.0, &cx) {
            Ok(b) => b,
            Err(r) => return Ok(r),
        };
        let kind = input.get("kind").and_then(Value::as_str).unwrap_or("");
        let reason = input
            .get("reason")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string();
        let to = input
            .get("to")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(String::from);
        if !matches!(kind, "rework" | "cancel") {
            return Ok(ToolResult::error_text("`kind` is `rework` or `cancel`"));
        }
        if kind == "rework" && to.is_none() {
            return Ok(ToolResult::error_text(
                "a rework proposal names the station to go back to in `to`",
            ));
        }
        if reason.is_empty() {
            return Ok(ToolResult::error_text("a proposal needs a reason"));
        }
        let Some(module) = self.0.cx.get() else {
            return Ok(ToolResult::error_text("the flow module has not started"));
        };
        let session = SessionRef {
            instance: b.instance.clone(),
            session_id: cx.session_id.clone(),
        };
        let payload = json!({
            "run": b.run, "station": b.station, "task": b.task,
            "kind": kind, "to": to, "reason": reason,
        });
        match module
            .sessions()
            .append_entry(&session, Entry::new("proposal", payload))
            .await
        {
            Ok(_) => Ok(ToolResult::text(
                "Proposed. A person will accept or dismiss it; carry on with your task meanwhile, \
                 or stop if you cannot do it without the change.",
            )),
            Err(e) => Ok(ToolResult::error_text(format!(
                "The proposal was not recorded: {}",
                e.message
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::module::shared::Config;

    struct Base(Vec<String>);

    impl AgentScene for Base {
        fn id(&self) -> &str {
            "chat"
        }
        fn name(&self) -> &str {
            "Chat"
        }
        fn description(&self) -> &str {
            "talk"
        }
        fn build_system_prompt(&self, _: &ScenePromptContext) -> Vec<PromptBlock> {
            vec![]
        }
        fn tools(&self) -> Vec<String> {
            self.0.clone()
        }
        fn token_budget(&self) -> TokenBudget {
            TokenBudget {
                compact_threshold: 0,
                compact_keep_recent: 0,
            }
        }
    }

    fn sh() -> Arc<Shared> {
        let (tx, _) = tokio::sync::broadcast::channel(4);
        Arc::new(Shared::new(
            Config {
                space: "flow".into(),
                instance: "system".into(),
                scenes: vec!["flow-chat".into()],
                programs: None,
                gates: None,
                hooks: None,
                guarded: vec![],
                recording: None,
                lease_ms: 60_000,
                tick_ms: 1000,
            },
            tx,
        ))
    }

    #[test]
    fn the_wrapped_scene_adds_the_flow_tools() {
        let s = FlowScene::new(Arc::new(Base(vec!["Read".into()])), sh());
        assert_eq!(s.id(), "flow-chat");
        assert_eq!(s.tools(), ["Read", TOOL_STATE, TOOL_SUBMIT, TOOL_PROPOSE]);
        let names: Vec<String> = s
            .extra_tools()
            .iter()
            .map(|t| t.name().to_string())
            .collect();
        assert_eq!(names, [TOOL_STATE, TOOL_SUBMIT, TOOL_PROPOSE]);
        assert!(s.permits_tool(TOOL_SUBMIT) && s.permits_tool("Read") && !s.permits_tool("Bash"));
    }

    #[test]
    fn an_open_whitelist_stays_open() {
        let s = FlowScene::new(Arc::new(Base(vec![])), sh());
        assert!(
            s.tools().is_empty(),
            "empty means every tool; adding ours would narrow it"
        );
        assert!(s.permits_tool("Bash") && s.permits_tool(TOOL_STATE));
    }
}
