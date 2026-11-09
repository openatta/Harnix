//! `AppsBridge`: the running applications' tools in every session an in-process instance builds.
//!
//! An Agix plugin (`runtime::plugin::Plugin`) in the instance's assembly. Agix registers it each
//! time it builds a session — sub-agents, team members and background tasks included — and it
//! registers what is running *then*: an application enabled now is in the next session built, and
//! one disabled now is not (a session already built keeps what it was built with until it is
//! rebuilt). Per application:
//!
//! - each tool the host offered for it, as `app__<id>__<tool>`. A call runs in an instance of its
//!   own (`Control::call_tool`), with the calling session's project as its read-only `/project`
//!   when the manifest asks for `${project}`; `host.progress` goes to the tool's progress and to
//!   the host's `harnix.apps.progress` topic, and cancelling the turn stops it
//!   (`Control::cancel`). **Every call asks first**: a tool's `read-only` claim is the component's
//!   own word, so it schedules calls and decides nothing about permission.
//! - its `events` export, as a `tool.around` middleware on the session's tool calls: offered
//!   `tool.before` with the tool's name and input — for its own tools' calls only, unless its
//!   manifest asks for every one (`[events] tools = "all"`, which the installer is told; the host
//!   decides) — it may let the call through, refuse it, or add a note to its result. A note goes
//!   on the result of one of its own tools only, and only so long ([`MAX_NOTE`]): a downloaded
//!   package does not get to write into the result of every tool the model calls. It cannot change
//!   the call. A hook that fails lets the call through: an application's fault is not the user's.

use std::future::Future;
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
use harnix_app_port::{Control, Failure, HookCall, HookReply, ToolDef, ToolReply};
use harnix_contract::codes;
use runtime::plugin::{Plugin, PluginSession, Registrar};
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use crate::runtime::{Loaded, Registry};

/// The plugin's name in every session.
pub const NAME: &str = "harnix-apps";

/// The longest note an application's hook may add to a tool's result, in characters.
pub const MAX_NOTE: usize = 2000;

/// The bridge between the running plugin apps and an instance's sessions. It is an Agix plugin,
/// which is why `plugin` in this crate's code means Agix's kind and never a plugin app.
pub struct AppsBridge {
    registry: Arc<Registry>,
}

/// Run `call` (a tool call or a hook named `call_id`), and cancel it in the host if `cancel` fires
/// first. The call is polled first, so the host has its cancellation before it can be asked to.
pub(crate) async fn cancellable<T>(
    control: &Arc<dyn Control>,
    call_id: &str,
    cancel: &CancellationToken,
    call: impl Future<Output = T>,
) -> T {
    tokio::pin!(call);
    tokio::select! {
        biased;
        out = &mut call => out,
        _ = cancel.cancelled() => {
            let _ = control.cancel(call_id.to_string()).await;
            call.await
        }
    }
}

impl AppsBridge {
    pub fn new(registry: Arc<Registry>) -> Self {
        Self { registry }
    }

    /// The tools a session in `project` is given now: what `register` registers.
    pub fn tools(&self, project: &std::path::Path) -> Vec<Arc<dyn Tool>> {
        let mut tools: Vec<Arc<dyn Tool>> = Vec::new();
        for app in self.registry.loaded() {
            if !app.component {
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
            .filter(|app| app.component)
            .map(|app| {
                Arc::new(AppEvents {
                    app,
                    registry: self.registry.clone(),
                    session_id: session_id.to_string(),
                    depth,
                }) as Arc<dyn ToolMiddleware>
            })
            .collect()
    }
}

impl Plugin for AppsBridge {
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
        let Some(control) = self.registry.control() else {
            return Err(ToolError::NotFound(format!(
                "`{}` is not running",
                self.app.id
            )));
        };
        let call_id = if ctx.tool_use_id.is_empty() {
            progress.tool_use_id().to_string()
        } else {
            ctx.tool_use_id.clone()
        };
        // A tool call's `host.progress`: to the engine's progress channel, and to the host's
        // listeners (`Registry::progress`), which is what a client sees.
        {
            let registry = self.registry.clone();
            let (app, tool, session_id, tool_use_id) = (
                self.app.id.clone(),
                self.name.clone(),
                ctx.session_id.clone(),
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
        let call = harnix_app_port::ToolCall {
            app: self.app.id.clone(),
            tool: self.def.name.clone(),
            input,
            call_id: call_id.clone(),
            project: Some(self.project.clone()),
            progress: true,
        };
        let reply = cancellable(&control, &call_id, &ctx.cancel, control.call_tool(call)).await;
        self.registry.unlisten(&call_id);
        match reply {
            Ok(ToolReply::Output {
                content,
                structured,
                is_error,
            }) => Ok(ToolResult {
                content: ToolResultContent::Text(content),
                is_error,
                structured_content: structured,
                ..ToolResult::default()
            }),
            Ok(ToolReply::Failed {
                failure: Failure::Cancelled,
                ..
            }) => Err(ToolError::Cancelled),
            Ok(ToolReply::Failed { message, .. }) => Ok(ToolResult::error_text(format!(
                "{}: {message}",
                self.app.id
            ))),
            Err(e) if e.code == codes::METHOD_NOT_FOUND => Err(ToolError::NotFound(e.message)),
            Err(e) => Ok(ToolResult::error_text(format!("{}: {}", self.app.id, e.message))),
        }
    }
}

/// An application's `events`, around every tool call of one session.
struct AppEvents {
    app: Arc<Loaded>,
    registry: Arc<Registry>,
    session_id: String,
    depth: u32,
}

/// What a note of the application `app` adds to the result of `tool`: nothing for a tool not its
/// own, and at most [`MAX_NOTE`] characters, under its name.
fn note_for(app: &str, tool: &str, note: &str) -> Option<String> {
    if !tool.starts_with(&crate::tool_name(app, "")) {
        return None;
    }
    let cut = note.chars().count() > MAX_NOTE;
    let mut note: String = note.chars().take(MAX_NOTE).collect();
    if cut {
        note.push('…');
    }
    Some(format!("\n\n[{app}] {note}"))
}

#[async_trait::async_trait]
impl ToolMiddleware for AppEvents {
    async fn around(
        &self,
        call: &ToolCall,
        exec: &mut ToolExec,
        next: NextDispatch<'_>,
    ) -> ToolOutcome {
        let Some(control) = self.registry.control() else {
            return next.run(call, exec).await;
        };
        let payload = json!({
            "tool": call.name,
            "input": call.input,
            "session_id": self.session_id,
            "depth": self.depth,
        });
        let call_id = uuid::Uuid::new_v4().simple().to_string();
        let hook = HookCall {
            app: self.app.id.clone(),
            event: "tool.before".into(),
            payload,
            call_id: call_id.clone(),
        };
        let cancel = exec.cancel().clone();
        let decision = cancellable(&control, &call_id, &cancel, control.hook(hook))
            .await
            .unwrap_or_else(|e| HookReply::Failed(e.message));
        match decision {
            HookReply::Proceed => next.run(call, exec).await,
            HookReply::Block(reason) => {
                tracing::info!(app = %self.app.id, tool = %call.name, %reason, "an application refused a tool call");
                Ok(ToolAnswer::failure(format!(
                    "refused by the application `{}`: {reason}",
                    self.app.id
                )))
            }
            HookReply::AddContext(note) => {
                let mut answer = next.run(call, exec).await?;
                if let Some(note) = note_for(&self.app.id, &call.name, &note) {
                    answer.text.push_str(&note);
                } else {
                    tracing::debug!(app = %self.app.id, tool = %call.name, "a note for another's tool; left out");
                }
                Ok(answer)
            }
            HookReply::Failed(failure) => {
                tracing::warn!(app = %self.app.id, tool = %call.name, %failure, "an application's event hook failed; the call goes on");
                next.run(call, exec).await
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_note_goes_on_its_own_tools_results_only_and_only_so_long() {
        assert_eq!(
            note_for("acme.notes", "app__acme_notes__search", "seen").as_deref(),
            Some("\n\n[acme.notes] seen")
        );
        for tool in ["Bash", "app__acme_other__search", "app__acme_notesx__search"] {
            assert_eq!(note_for("acme.notes", tool, "seen"), None, "{tool}");
        }
        let long = "x".repeat(MAX_NOTE + 10);
        let note = note_for("acme.notes", "app__acme_notes__search", &long).unwrap();
        assert_eq!(note.chars().filter(|c| *c == 'x').count(), MAX_NOTE);
        assert!(note.ends_with('…'));
    }
}
