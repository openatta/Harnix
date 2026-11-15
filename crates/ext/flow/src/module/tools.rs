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

use harnix_ext::engine::{Permission, SceneBase, SceneLayer, Tool, ToolCx, ToolError, ToolOutput, ToolSpec};
use harnix_ext::{Entry, SessionRef};
use serde_json::{json, Value};

use super::dispatch;
use super::shared::{Binding, Shared};
use crate::kernel::protocol::STRUCTURED_OUTPUT_TOOL;
use crate::kernel::run::Actor;
use crate::kernel::FlowError;

pub const TOOL_STATE: &str = "flow_state";
pub const TOOL_SUBMIT: &str = "flow_submit";
pub const TOOL_PROPOSE: &str = "flow_propose";

pub(crate) fn wrapped_id(base: &str) -> String {
    format!("flow-{base}")
}

/// An application's scene with the flow's tools added, offered as `flow-<id>`.
///
/// Attended whatever the base scene says: the module answers this session's permission asks (from
/// its task's permissions, or by asking a person in the flow space), so they must reach the host
/// rather than be refused as no one's to answer. And no model call outside its task: naming the
/// session would be one (and, under replay, one that is not recorded and goes to the network).
pub(crate) fn flow_scene(base: SceneBase, sh: Arc<Shared>, requires_project: Option<bool>) -> SceneLayer {
    let layer = SceneLayer::new("flow-{base}", base)
        .name("{base} (flow)")
        .description("{base} — working on a flow task, with flow_state, flow_submit and flow_propose")
        .tool(Arc::new(StateTool::new(sh.clone())))
        .tool(Arc::new(SubmitTool::new(sh.clone())))
        .tool(Arc::new(ProposeTool::new(sh)))
        .interactive(true)
        .auto_name(false)
        .name_prompt(None);
    match requires_project {
        Some(on) => layer.requires_project(on),
        None => layer,
    }
}

/// The id [`structured_scene`] gives a base scene: `structured-<id>`.
pub fn structured_id(base: &str) -> String {
    format!("structured-{base}")
}

/// A scene whose sessions can hand a flow task in by structured output: the base scene with the
/// engine's `StructuredOutput` tool in its whitelist, offered as `structured-<id>`.
///
/// The engine gives a session with an `output_schema` that tool only where its scene permits
/// it, and Agix's harnix scene does not. Register it where a station's session runs — in a
/// template's native contributions (`InstanceTemplate::native`, which a spawned daemon builds
/// again in its own process) and that template's `EngineProfile.scenes`. It holds no state of
/// the module's, so it works in any process. Attended and unnamed, as [`flow_scene`]'s are.
pub fn structured_scene(base: SceneBase) -> SceneLayer {
    SceneLayer::new("structured-{base}", base)
        .name("{base} (structured output)")
        .admit(STRUCTURED_OUTPUT_TOOL)
        .undisallow(STRUCTURED_OUTPUT_TOOL)
        .interactive(true)
        .auto_name(false)
        .name_prompt(None)
}

/// The task a calling session serves, or what to tell it.
fn bound(sh: &Shared, cx: &ToolCx) -> Result<Binding, ToolOutput> {
    sh.binding(cx.session_id()).ok_or_else(|| {
        ToolOutput::error(
            "This session is not working on a flow task (or its task is over): there is nothing \
             to read or hand in here.",
        )
    })
}

fn agent(b: &Binding, cx: &ToolCx) -> Actor {
    Actor::agent(format!("{}/{}", b.instance, cx.session_id()))
}

fn pretty(v: &Value) -> String {
    serde_json::to_string_pretty(v).unwrap_or_else(|_| v.to_string())
}

type ToolOut = Result<ToolOutput, ToolError>;

struct StateTool {
    spec: ToolSpec,
    sh: Arc<Shared>,
}

impl StateTool {
    fn new(sh: Arc<Shared>) -> Self {
        let spec = ToolSpec::new(
            TOOL_STATE,
            "The flow task this session is working on: the run, the station and its round, your \
             task and item, the output contract, what you handed in, and which exit conditions are \
             still not met.",
            json!({"type": "object", "properties": {}}),
        )
        .read_only()
        .concurrency_safe();
        Self { spec, sh }
    }
}

#[async_trait::async_trait]
impl Tool for StateTool {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }
    fn permission(&self, _: &Value) -> Permission {
        Permission::Allow
    }
    async fn call(&self, cx: ToolCx, _input: Value) -> ToolOut {
        let b = match bound(&self.sh, &cx) {
            Ok(b) => b,
            Err(r) => return Ok(r),
        };
        let Ok(engine) = self.sh.engine() else {
            return Ok(ToolOutput::error("the flow module has not started"));
        };
        // The whole record, to tell whether the claim is still this session's; only the
        // fields below leave here, never a token.
        let full = match engine.record(&b.run).await {
            Ok(r) => r,
            Err(e) => return Ok(ToolOutput::error(e.to_string())),
        };
        let current = full
            .stations
            .get(&b.station)
            .and_then(|s| s.task(&b.task))
            .is_some_and(|t| t.lease.as_ref().is_some_and(|l| l.token == b.token));
        let view = crate::kernel::engine::view(full);
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
            "permissions": b.permissions,
            "current": current,
        });
        Ok(ToolOutput::text(pretty(&state)))
    }
}

struct SubmitTool {
    spec: ToolSpec,
    sh: Arc<Shared>,
}

impl SubmitTool {
    /// It writes nothing outside the flow's own record, and a headless session has nobody to ask
    /// for permission: read-only, for scheduling.
    fn new(sh: Arc<Shared>) -> Self {
        let spec = ToolSpec::new(
            TOOL_SUBMIT,
            "Hand in this task's output: one JSON value satisfying the station's output contract. \
             Problems come back for you to fix; submit again until it is accepted. You may submit \
             again until the station passes; the last submission counts.",
            json!({
                "type": "object",
                "required": ["output"],
                "properties": {
                    "output": {"description": "The output, as the flow protocol's contract says."}
                }
            }),
        )
        .read_only();
        Self { spec, sh }
    }
}

#[async_trait::async_trait]
impl Tool for SubmitTool {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }
    fn permission(&self, _: &Value) -> Permission {
        Permission::Allow
    }
    async fn call(&self, cx: ToolCx, input: Value) -> ToolOut {
        let b = match bound(&self.sh, &cx) {
            Ok(b) => b,
            Err(r) => return Ok(r),
        };
        let Ok(engine) = self.sh.engine() else {
            return Ok(ToolOutput::error("the flow module has not started"));
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
                dispatch::apply(&self.sh, Some(&b.run), o).await;
                Ok(ToolOutput::text(format!(
                    "Accepted. Station `{}` is now {}. If it is still working, others' tasks or \
                     its checks are outstanding; your part is done unless you want to improve \
                     your output.",
                    b.station,
                    status.as_str().unwrap_or("unknown")
                )))
            }
            Err(FlowError::Contract(m)) => Ok(ToolOutput::error(format!(
                "Not accepted: {m}. Fix the output and call {TOOL_SUBMIT} again."
            ))),
            Err(e @ FlowError::Invalid(_)) => Ok(ToolOutput::error(format!(
                "Not accepted: {e}. The station is not taking submissions now."
            ))),
            Err(e) => Ok(ToolOutput::error(format!(
                "Not accepted: {e}. Your claim on this task is no longer current; stop working on it."
            ))),
        }
    }
}

struct ProposeTool {
    spec: ToolSpec,
    sh: Arc<Shared>,
}

impl ProposeTool {
    fn new(sh: Arc<Shared>) -> Self {
        let spec = ToolSpec::new(
            TOOL_PROPOSE,
            "Propose that a person rework an upstream station (kind `rework`, `to` the station) or \
             cancel the run (kind `cancel`), with your reason. It is shown to people as a card; \
             nothing happens unless one of them accepts it.",
            json!({
                "type": "object",
                "required": ["kind", "reason"],
                "properties": {
                    "kind": {"type": "string", "enum": ["rework", "cancel"]},
                    "to": {"type": "string", "description": "With `rework`: the station to go back to."},
                    "reason": {"type": "string"}
                }
            }),
        )
        .read_only();
        Self { spec, sh }
    }
}

#[async_trait::async_trait]
impl Tool for ProposeTool {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }
    fn permission(&self, _: &Value) -> Permission {
        Permission::Allow
    }
    async fn call(&self, cx: ToolCx, input: Value) -> ToolOut {
        let b = match bound(&self.sh, &cx) {
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
            return Ok(ToolOutput::error("`kind` is `rework` or `cancel`"));
        }
        if kind == "rework" && to.is_none() {
            return Ok(ToolOutput::error(
                "a rework proposal names the station to go back to in `to`",
            ));
        }
        if reason.is_empty() {
            return Ok(ToolOutput::error("a proposal needs a reason"));
        }
        let Some(module) = self.sh.cx.get() else {
            return Ok(ToolOutput::error("the flow module has not started"));
        };
        let session = match cx.session() {
            s if s.instance.is_empty() => SessionRef::new(b.instance.as_str(), s.session_id),
            s => s,
        };
        let payload = json!({
            "run": b.run, "station": b.station, "task": b.task,
            "kind": kind, "to": to, "reason": reason,
            "session": {"instance": session.instance, "session_id": session.session_id},
        });
        match module
            .sessions()
            .append_entry(&session, Entry::new("proposal", payload))
            .await
        {
            Ok(_) => Ok(ToolOutput::text(
                "Proposed. A person will accept or dismiss it; carry on with your task meanwhile, \
                 or stop if you cannot do it without the change.",
            )),
            Err(e) => Ok(ToolOutput::error(format!(
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

    fn sh() -> Arc<Shared> {
        let (tx, _) = tokio::sync::broadcast::channel(4);
        Arc::new(Shared::new(
            Config {
                space: "flow".into(),
                instance: "main".into(),
                scenes: vec!["flow-harnix".into()],
                programs: None,
                gates: None,
                command_gates: false,
                guarded: vec![],
                recording: None,
                main_recording: None,
                conversation_recording: None,
                main_scene: None,
                project_scenes: Default::default(),
                lease_ms: 60_000,
                tick_ms: 1000,
                ensure_ms: 60_000,
            },
            tx,
        ))
    }

    #[test]
    fn the_wrapped_scene_adds_the_flow_tools_and_is_attended_and_unnamed() {
        let layer = flow_scene(SceneBase::builtin("harnix"), sh(), None);
        let p = layer.parts();
        assert_eq!(p.id, "flow-{base}");
        let names: Vec<&str> = p.tools.iter().map(|t| t.spec().name()).collect();
        assert_eq!(names, [TOOL_STATE, TOOL_SUBMIT, TOOL_PROPOSE]);
        assert_eq!((p.interactive, p.auto_name, p.name_prompt), (Some(true), Some(false), Some(None)));
        assert_eq!(p.requires_project, None, "the base says, unless the application does");
        let p = flow_scene(SceneBase::builtin("code"), sh(), Some(true));
        assert_eq!(p.parts().requires_project, Some(true));
    }

    #[test]
    fn the_structured_scene_lets_a_session_hand_in_by_structured_output() {
        let layer = structured_scene(SceneBase::builtin("harnix"));
        let p = layer.parts();
        assert_eq!((p.id, p.name), ("structured-{base}", Some("{base} (structured output)")));
        assert_eq!((p.admitted, p.undisallowed), (&[STRUCTURED_OUTPUT_TOOL.to_string()][..], &[STRUCTURED_OUTPUT_TOOL.to_string()][..]));
        assert_eq!((p.interactive, p.auto_name), (Some(true), Some(false)));
        assert!(p.tools.is_empty());
    }

    #[test]
    fn its_tools_touch_nothing_and_ask_nothing() {
        let layer = flow_scene(SceneBase::builtin("harnix"), sh(), None);
        for tool in layer.parts().tools {
            assert!(tool.spec().is_read_only(), "{}", tool.spec().name());
            assert_eq!(tool.permission(&json!({})), Permission::Allow);
        }
    }

    #[tokio::test]
    async fn a_session_that_serves_no_task_is_told_so() {
        let layer = flow_scene(SceneBase::builtin("harnix"), sh(), None);
        let out = layer.parts().tools[0].call(ToolCx::new("nobody", "t"), json!({})).await.unwrap();
        assert!(out.is_error() && out.content().contains("not working on a flow task"));
    }
}
