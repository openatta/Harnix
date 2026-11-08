//! `AppsPlugin`: the running applications' tools in every session an in-process instance builds.
//!
//! An Agix plugin (`runtime::plugin::Plugin`) in the instance's assembly. Agix registers it each
//! time it builds a session — sub-agents, team members and background tasks included — and it
//! registers what is running *then*: an application enabled now is in the next session built, and
//! one disabled now is not (a session already built keeps what it was built with until it is
//! rebuilt). Per application:
//!
//! - each tool the component listed, as `app__<id>__<tool>`. A call runs in an instance of its own
//!   (`AppVm::call_tool`), with the calling session's project as its read-only `/project` when the
//!   manifest asks for `${project}`; `host.progress` goes to the tool's progress and to the
//!   host's `harnix.apps.progress` topic, and cancelling the turn stops it. **Every call asks first**: a tool's `read-only` claim is the component's
//!   own word, so it schedules calls and decides nothing about permission.
//! - its `events` export, as a `tool.around` middleware on every tool call of the session (not
//!   only its own tools): offered `tool.before` with the tool's name and input, it may let the call
//!   through, refuse it, or add a note to its result. It cannot change the call. A hook that fails
//!   lets the call through: an application's fault is not the user's.

use std::path::PathBuf;
use std::sync::Arc;

use base::error::ToolError;
use base::interface::tool::{
    PermissionDecision, ProgressSender, PromptContext, Tool, ToolContext, ToolResult,
    ToolResultContent,
};
use base::interface::tool_middleware::{
    NextDispatch, ToolAnswer, ToolCall, ToolExec, ToolMiddleware, ToolOutcome,
};
use harnix_wasm::{CallFailure, HookDecision, ProgressSink, ToolDef};
use runtime::plugin::{Plugin, PluginSession, Registrar};
use serde_json::{json, Value};

use crate::runtime::{Loaded, Registry};

/// The plugin's name in every session.
pub const NAME: &str = "harnix-apps";

pub struct AppsPlugin {
    registry: Arc<Registry>,
}

impl AppsPlugin {
    pub fn new(registry: Arc<Registry>) -> Self {
        Self { registry }
    }

    /// The tools a session in `project` is given now: what `register` registers.
    pub fn tools(&self, project: &std::path::Path) -> Vec<Arc<dyn Tool>> {
        let mut tools: Vec<Arc<dyn Tool>> = Vec::new();
        for app in self.registry.loaded() {
            if app.vm.is_none() {
                continue;
            }
            for def in &app.tools {
                tools.push(Arc::new(AppTool {
                    name: app.tool_name(&def.name),
                    schema: serde_json::from_str(&def.input_schema)
                        .unwrap_or_else(|_| json!({"type": "object"})),
                    def: def.clone(),
                    app: app.clone(),
                    registry: self.registry.clone(),
                    project: project.to_path_buf(),
                }));
            }
        }
        tools
    }

    /// The `tool.around` middleware for one session: each running application's `events`.
    pub fn middleware(&self, session_id: &str, depth: u32) -> Vec<Arc<dyn ToolMiddleware>> {
        self.registry
            .loaded()
            .into_iter()
            .filter(|app| app.vm.is_some())
            .map(|app| {
                Arc::new(AppEvents {
                    app,
                    session_id: session_id.to_string(),
                    depth,
                }) as Arc<dyn ToolMiddleware>
            })
            .collect()
    }
}

impl Plugin for AppsPlugin {
    fn name(&self) -> &str {
        NAME
    }

    fn register(&self, session: &PluginSession<'_>, r: &mut Registrar) {
        for tool in self.tools(session.project_root) {
            r.tool(tool);
        }
        for middleware in self.middleware(session.session_id, session.depth) {
            r.tool_middleware(middleware);
        }
    }
}

/// One application tool, as a session has it.
struct AppTool {
    name: String,
    def: ToolDef,
    schema: Value,
    app: Arc<Loaded>,
    registry: Arc<Registry>,
    /// The session's project: `/project`, for a manifest that asks for `${project}`.
    project: PathBuf,
}

/// A tool call's `host.progress`: to the engine's progress channel, and to the host's listeners
/// (`Registry::progress`), which is what a client sees.
struct Progress {
    sender: ProgressSender,
    registry: Arc<Registry>,
    app: String,
    tool: String,
    session_id: String,
    tool_use_id: String,
}

impl ProgressSink for Progress {
    fn on_progress(&self, _call_id: &str, text: &str) {
        self.sender.send(text);
        self.registry.progress(json!({
            "app": self.app,
            "tool": self.tool,
            "session_id": self.session_id,
            "tool_use_id": self.tool_use_id,
            "text": text,
        }));
    }
}

#[async_trait::async_trait]
impl Tool for AppTool {
    fn name(&self) -> &str {
        &self.name
    }

    fn description(&self) -> &str {
        &self.def.description
    }

    fn input_schema(&self) -> Value {
        self.schema.clone()
    }

    fn source(&self) -> std::borrow::Cow<'_, str> {
        std::borrow::Cow::Owned(format!("app:{}", self.app.id))
    }

    async fn prompt(&self, _: &PromptContext) -> String {
        match &self.def.doc {
            Some(doc) => format!("{}\n\n{doc}", self.def.description),
            None => self.def.description.clone(),
        }
    }

    fn is_read_only(&self, _: &Value) -> bool {
        self.def.read_only
    }

    fn is_concurrency_safe(&self, _: &Value) -> bool {
        self.def.concurrency_safe
    }

    async fn check_permissions(&self, _: &Value, _: &ToolContext) -> PermissionDecision {
        PermissionDecision::ask(format!(
            "The application `{}` wants to run its tool `{}`",
            self.app.title.as_deref().unwrap_or(&self.app.id),
            self.def.name
        ))
    }

    async fn call(
        &self,
        input: Value,
        ctx: ToolContext,
        progress: ProgressSender,
    ) -> Result<ToolResult, ToolError> {
        let Some(vm) = &self.app.vm else {
            return Err(ToolError::NotFound(format!(
                "`{}` has no component",
                self.app.id
            )));
        };
        let call_id = if ctx.tool_use_id.is_empty() {
            progress.tool_use_id().to_string()
        } else {
            ctx.tool_use_id.clone()
        };
        let sink: Arc<dyn ProgressSink> = Arc::new(Progress {
            sender: progress,
            registry: self.registry.clone(),
            app: self.app.id.clone(),
            tool: self.name.clone(),
            session_id: ctx.session_id.clone(),
            tool_use_id: call_id.clone(),
        });
        match vm
            .call_tool(
                &self.def.name,
                &input,
                &call_id,
                Some(&self.project),
                Some(sink),
                &ctx.cancel,
            )
            .await
        {
            Ok(out) => Ok(ToolResult {
                content: ToolResultContent::Text(out.content),
                is_error: out.is_error,
                structured_content: out.structured,
                ..ToolResult::default()
            }),
            Err(CallFailure::Cancelled) => Err(ToolError::Cancelled),
            Err(failure) => Ok(ToolResult::error_text(format!(
                "{}: {failure}",
                self.app.id
            ))),
        }
    }
}

/// An application's `events`, around every tool call of one session.
struct AppEvents {
    app: Arc<Loaded>,
    session_id: String,
    depth: u32,
}

#[async_trait::async_trait]
impl ToolMiddleware for AppEvents {
    async fn around(
        &self,
        call: &ToolCall,
        exec: &mut ToolExec,
        next: NextDispatch<'_>,
    ) -> ToolOutcome {
        let Some(vm) = &self.app.vm else {
            return next.run(call, exec).await;
        };
        let payload = json!({
            "tool": call.name,
            "input": call.input,
            "session_id": self.session_id,
            "depth": self.depth,
        });
        match vm.on_event("tool.before", &payload, exec.cancel()).await {
            Ok(HookDecision::Proceed) => next.run(call, exec).await,
            Ok(HookDecision::Block(reason)) => {
                tracing::info!(app = %self.app.id, tool = %call.name, %reason, "an application refused a tool call");
                Ok(ToolAnswer::failure(format!(
                    "refused by the application `{}`: {reason}",
                    self.app.id
                )))
            }
            Ok(HookDecision::AddContext(note)) => {
                let mut answer = next.run(call, exec).await?;
                answer
                    .text
                    .push_str(&format!("\n\n[{}] {note}", self.app.id));
                Ok(answer)
            }
            Err(failure) => {
                tracing::warn!(app = %self.app.id, tool = %call.name, %failure, "an application's event hook failed; the call goes on");
                next.run(call, exec).await
            }
        }
    }
}
