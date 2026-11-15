//! `WasmTools`: the running packages' tools in every session of an instance this process builds
//! or spawns.
//!
//! A tool provider (`harnix_ext::engine::ToolProvider`) the component gives every engine this
//! process builds or spawns (`engine_everywhere`). Its tools are offered on the instance's link to
//! the engine — sub-agents, team members and background tasks have them too — and offered again
//! each time a package starts or stops: a package enabled now is in the next session built, and one
//! disabled now is not (a session already built keeps what it was built with until it is rebuilt).
//! Per package:
//!
//! - each tool the host offered for it, as `ext__<id>__<tool>`. A call runs in an instance of its
//!   own (`Control::call_tool`), with the calling session's project as its read-only `/project`
//!   when the manifest asks for `${project}`; `host.progress` goes to the tool's progress and to
//!   the component's `packages.progress` topic, and cancelling the turn stops it
//!   (`Control::cancel`). **Every call asks first**: a tool's `read-only` claim is the component's
//!   own word, so it schedules calls and decides nothing about permission.
//! - its `events` export, as a hook before each of the session's tool calls: offered
//!   `tool.before` with the tool's name and input — for its own tools' calls only, unless its
//!   manifest asks for every one (`[events] tools = "all"`, which the installer is told; the host
//!   decides) — it may let the call through, refuse it, or add a note, which the model is handed as
//!   a reminder before its next step. A hook is asked before the engine checks the call or asks
//!   about it. A note is about one of its own tools only, and only so long ([`MAX_NOTE`]): a
//!   downloaded package does not get to write into what the model reads of every tool it calls. It
//!   cannot change the call. A hook that fails lets the call through: a package's fault is not the
//!   user's.

use std::sync::Arc;

use harnix_carrier_shared::tools::{cancellable, hook_decision, permission, tool_output, tool_spec};
use harnix_carrier_shared::Loaded;
use harnix_ext::engine::{
    HookDecision, Permission, Tool, ToolCallFacts, ToolCx, ToolError, ToolHook, ToolOutput, ToolProvider, ToolSpec,
    ToolsChanged,
};
use harnix_wasm_port::{CallMeta, HookCall, HookReply, ToolDef};
use serde_json::{json, Value};

use crate::registry::Registry;

pub use harnix_carrier_shared::tools::{MAX_NOTE, NAME};

/// The running packages' tools and hooks, for an instance's sessions: a provider of the
/// instance's engine.
pub struct WasmTools {
    registry: Arc<Registry>,
}

impl WasmTools {
    pub fn new(registry: Arc<Registry>) -> Self {
        Self { registry }
    }

    /// The tools the engine's sessions are given now.
    pub fn tools(&self) -> Vec<Arc<dyn Tool>> {
        let mut tools: Vec<Arc<dyn Tool>> = Vec::new();
        for app in self.registry.loaded() {
            if !app.component {
                continue;
            }
            for def in &app.tools {
                let spec = tool_spec(&app, def);
                tools.push(Arc::new(AppTool {
                    spec,
                    def: def.clone(),
                    app: app.clone(),
                    registry: self.registry.clone(),
                }));
            }
        }
        tools
    }

    /// The hooks before every session's tool calls: each running package's `events`.
    pub fn hooks(&self) -> Vec<Arc<dyn ToolHook>> {
        self.registry
            .loaded()
            .into_iter()
            .filter(|app| app.component)
            .map(|app| {
                Arc::new(AppEvents { app, registry: self.registry.clone() }) as Arc<dyn ToolHook>
            })
            .collect()
    }
}

impl ToolProvider for WasmTools {
    fn name(&self) -> &str {
        NAME
    }

    fn tools(&self) -> Vec<Arc<dyn Tool>> {
        WasmTools::tools(self)
    }

    fn hooks(&self) -> Vec<Arc<dyn ToolHook>> {
        WasmTools::hooks(self)
    }

    fn changes(&self) -> Vec<ToolsChanged> {
        vec![self.registry.changed()]
    }
}

/// One package's tool, as a session has it.
struct AppTool {
    spec: ToolSpec,
    def: ToolDef,
    app: Arc<Loaded>,
    registry: Arc<Registry>,
}

#[async_trait::async_trait]
impl Tool for AppTool {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    fn permission(&self, _: &Value) -> Permission {
        permission(&self.app, &self.def.name)
    }

    async fn call(&self, cx: ToolCx, input: Value) -> Result<ToolOutput, ToolError> {
        let Some(control) = self.registry.control() else {
            return Err(ToolError::NotFound(format!("`{}` is not running", self.app.id)));
        };
        let call_id = cx.tool_use_id().to_string();
        // A tool call's `host.progress`: to the engine's progress channel, and to the host's
        // listeners (`Registry::progress`), which is what a client sees.
        {
            let registry = self.registry.clone();
            let progress = cx.progress().clone();
            let (app, tool, session_id, tool_use_id) = (
                self.app.id.clone(),
                self.spec.name().to_string(),
                cx.session_id().to_string(),
                call_id.clone(),
            );
            self.registry.listen(
                &call_id,
                Arc::new(move |text: &str| {
                    progress.send(text);
                    registry.progress(json!({
                        "app": app,
                        "tool": tool,
                        "session_id": session_id,
                        "tool_use_id": tool_use_id,
                        "text": text,
                    }));
                }),
            );
        }
        let call = harnix_wasm_port::ToolCall {
            app: self.app.id.clone(),
            tool: self.def.name.clone(),
            input,
            call_id: call_id.clone(),
            project: (!cx.project().as_os_str().is_empty()).then(|| cx.project().to_path_buf()),
            progress: true,
        };
        // On behalf of the session's turn: what the tool calls carries the turn's chain.
        let meta = CallMeta::default().chain(cx.chain().iter().cloned());
        let reply = cancellable(&control, &call_id, cx.cancel(), control.call_tool(meta, call)).await;
        self.registry.unlisten(&call_id);
        tool_output(&self.app.id, reply)
    }
}

/// A package's `events`, before every tool call of one session.
struct AppEvents {
    app: Arc<Loaded>,
    registry: Arc<Registry>,
}

#[async_trait::async_trait]
impl ToolHook for AppEvents {
    async fn before(&self, call: &ToolCallFacts) -> HookDecision {
        let Some(control) = self.registry.control() else {
            return HookDecision::Proceed;
        };
        let payload = json!({
            "tool": call.name(),
            "input": call.input(),
            "session_id": call.session_id(),
            "depth": call.depth(),
        });
        let call_id = uuid::Uuid::new_v4().simple().to_string();
        let hook = HookCall {
            app: self.app.id.clone(),
            event: "tool.before".into(),
            payload,
            call_id: call_id.clone(),
        };
        let decision = cancellable(&control, &call_id, call.cancel(), control.hook(CallMeta::default(), hook))
            .await
            .unwrap_or_else(|e| HookReply::Failed(e.message));
        hook_decision(&self.app.id, call.name(), decision)
    }
}
