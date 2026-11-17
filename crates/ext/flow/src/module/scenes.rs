//! The scenes flow sessions run in: an application's scene offered as `flow-<id>` — the same
//! prompt, budget and tools, plus the flow's three native tools (`flow_state`, `flow_submit`,
//! `flow_propose`), one set shared by every `flow-*` scene. Each call goes to the runner's
//! `FlowTools` under the calling session's name, which finds the task it serves.

use std::sync::Arc;

use harnix_ext::engine::{Permission, SceneBase, SceneLayer, Tool, ToolCx, ToolError, ToolOutput, ToolSpec};
use serde_json::Value;

use super::state::Shared;
use crate::runner::ToolSpec as FlowToolSpec;

pub use crate::runner::{TOOL_PROPOSE, TOOL_STATE, TOOL_SUBMIT};

pub(crate) fn wrapped_id(base: &str) -> String {
    format!("flow-{base}")
}

/// The flow's three tools, made once: a tool by one name is one object in every scene that has it.
#[derive(Clone)]
pub(crate) struct FlowTools {
    tools: Vec<Arc<dyn Tool>>,
}

impl FlowTools {
    pub(crate) fn new(sh: Arc<Shared>) -> Self {
        let specs = crate::runner::tool_specs();
        Self { tools: specs.into_iter().map(|spec| Arc::new(FlowTool::new(spec, sh.clone())) as Arc<dyn Tool>).collect() }
    }
}

/// An application's scene with the flow's tools added, offered as `flow-<id>`.
///
/// Attended whatever the base scene says: the runner answers this session's permission asks (from
/// its task's permissions, or by asking a person in the flow space), so they must reach the host
/// rather than be refused as no one's to answer. And no model call outside its task: naming the
/// session would be one (and, under replay, one that is not recorded and goes to the network).
///
/// No one answers a task's questions: its sessions are the extension's own, and a session an
/// extension opens has no `AskUserQuestion` (the hub lists questions among the prompts it answers
/// only for a person's sessions). A person is reached through a station's `confirm`.
pub(crate) fn flow_scene(base: SceneBase, tools: &FlowTools, requires_project: Option<bool>) -> SceneLayer {
    let mut layer = SceneLayer::new("flow-{base}", base)
        .name("{base} (flow)")
        .description("{base} — working on a flow task, with flow_state, flow_submit and flow_propose");
    for t in &tools.tools {
        layer = layer.tool(t.clone());
    }
    let layer = layer.interactive(true).auto_name(false);
    match requires_project {
        Some(on) => layer.requires_project(on),
        None => layer,
    }
}

/// One of the flow's tools as the engine sees it: its spec from the runner, its calls routed to
/// the runner under the calling session's name (`<instance>/<session_id>`).
struct FlowTool {
    spec: ToolSpec,
    name: String,
    sh: Arc<Shared>,
}

impl FlowTool {
    fn new(spec: FlowToolSpec, sh: Arc<Shared>) -> Self {
        let mut told = ToolSpec::new(&spec.name, &spec.description, spec.input_schema);
        if spec.read_only {
            told = told.read_only();
        }
        if spec.name == TOOL_STATE {
            told = told.concurrency_safe();
        }
        Self { spec: told, name: spec.name, sh }
    }
}

#[async_trait::async_trait]
impl Tool for FlowTool {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }
    /// It writes nothing outside the flow's own record, and a headless session has nobody to ask
    /// for permission.
    fn permission(&self, _: &Value) -> Permission {
        Permission::Allow
    }
    async fn call(&self, cx: ToolCx, input: Value) -> Result<ToolOutput, ToolError> {
        let Ok(rn) = self.sh.runner() else {
            return Ok(ToolOutput::error("the flow extension has not started"));
        };
        // The session the call is made in, on the instance its engine carries (the flow space's
        // when an engine does not say).
        let me = cx.session();
        let instance = if me.instance.is_empty() { self.sh.cfg.instance.as_str() } else { me.instance.as_str() };
        let session = format!("{instance}/{}", me.session_id);
        let reply = rn.tools().call(&session, &self.name, input).await;
        Ok(if reply.is_error { ToolOutput::error(reply.text) } else { ToolOutput::text(reply.text) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sh() -> Arc<Shared> {
        Arc::new(Shared::for_tests())
    }

    #[test]
    fn the_wrapped_scene_adds_the_flow_tools_and_is_attended_and_unnamed() {
        let tools = FlowTools::new(sh());
        let layer = flow_scene(SceneBase::builtin("harnix"), &tools, None);
        let p = layer.parts();
        assert_eq!(p.id, "flow-{base}");
        let names: Vec<&str> = p.tools.iter().map(|t| t.spec().name()).collect();
        assert_eq!(names, [TOOL_STATE, TOOL_SUBMIT, TOOL_PROPOSE]);
        assert_eq!((p.interactive, p.auto_name), (Some(true), Some(false)));
        assert_eq!(p.requires_project, None, "the base says, unless the application does");
        let code = flow_scene(SceneBase::builtin("code"), &tools, Some(true));
        assert_eq!(code.parts().requires_project, Some(true));
        // One set of tools in every wrapped scene: a tool by one name is one object.
        for (a, b) in p.tools.iter().zip(code.parts().tools) {
            assert!(Arc::ptr_eq(a, b), "{}", a.spec().name());
        }
    }

    #[test]
    fn its_tools_touch_nothing_and_ask_nothing() {
        let layer = flow_scene(SceneBase::builtin("harnix"), &FlowTools::new(sh()), None);
        for tool in layer.parts().tools {
            assert!(tool.spec().is_read_only(), "{}", tool.spec().name());
            assert_eq!(tool.permission(&json!({})), Permission::Allow);
        }
    }

    #[tokio::test]
    async fn before_the_extension_starts_a_tool_says_so() {
        let layer = flow_scene(SceneBase::builtin("harnix"), &FlowTools::new(sh()), None);
        let out = layer.parts().tools[0].call(ToolCx::new("nobody", "t"), json!({})).await.unwrap();
        assert!(out.is_error() && out.content().contains("has not started"));
    }
}
