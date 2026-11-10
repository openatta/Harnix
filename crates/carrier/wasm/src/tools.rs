//! `WasmTools`: the running packages' tools in every session an in-process instance builds.
//!
//! A tool provider (`harnix_ext::engine::ToolProvider`) the component gives every engine built in
//! this process (`engine_everywhere`). The engine asks it each time it builds a session —
//! sub-agents, team members and background tasks included — and it registers what is running
//! *then*: a package enabled now is in the next session built, and one disabled now is not (a
//! session already built keeps what it was built with until it is rebuilt). Per package:
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
//!   decides) — it may let the call through, refuse it, or add a note to its result. A note goes
//!   on the result of one of its own tools only, and only so long ([`MAX_NOTE`]): a downloaded
//!   package does not get to write into the result of every tool the model calls. It cannot change
//!   the call. A hook that fails lets the call through: a package's fault is not the user's.

use std::future::Future;
use std::path::PathBuf;
use std::sync::Arc;

use harnix_ext::codes;
use harnix_ext::engine::{
    Cancel, HookDecision, Permission, SessionFacts, Tool, ToolCallFacts, ToolCx, ToolError, ToolHook, ToolOutput,
    ToolProvider, ToolSpec,
};
use harnix_wasm_port::{CallMeta, Control, Failure, HookCall, HookReply, ToolDef, ToolReply};
use serde_json::{json, Value};

use crate::registry::{Loaded, Registry};

/// The provider's name in every session.
pub const NAME: &str = "harnix-packages";

/// The longest note an application's hook may add to a tool's result, in characters.
pub const MAX_NOTE: usize = 2000;

/// The running packages' tools and hooks, for an instance's sessions: a provider of the
/// instance's engine.
pub struct WasmTools {
    registry: Arc<Registry>,
}

/// Run `call` (a tool call or a hook named `call_id`), and cancel it in the host if `cancel` fires
/// first. The call is polled first, so the host has its cancellation before it can be asked to.
pub(crate) async fn cancellable<T>(
    control: &Arc<dyn Control>,
    call_id: &str,
    cancel: &Cancel,
    call: impl Future<Output = T>,
) -> T {
    tokio::pin!(call);
    tokio::select! {
        biased;
        out = &mut call => out,
        _ = cancel.cancelled() => {
            let _ = control.cancel(CallMeta::default(), call_id.to_string()).await;
            call.await
        }
    }
}

impl WasmTools {
    pub fn new(registry: Arc<Registry>) -> Self {
        Self { registry }
    }

    /// The tools a session in `project` is given now: what the engine registers.
    pub fn tools_for(&self, project: &std::path::Path) -> Vec<Arc<dyn Tool>> {
        let mut tools: Vec<Arc<dyn Tool>> = Vec::new();
        for app in self.registry.loaded() {
            if !app.component {
                continue;
            }
            for def in &app.tools {
                let name = app.tool_name(&def.name);
                let schema = serde_json::from_str(&def.input_schema).unwrap_or_else(|_| json!({"type": "object"}));
                let mut spec = ToolSpec::new(&name, &def.description, schema).source(&format!("app:{}", app.id));
                if let Some(doc) = &def.doc {
                    spec = spec.doc(doc);
                }
                if def.read_only {
                    spec = spec.read_only();
                }
                if def.concurrency_safe {
                    spec = spec.concurrency_safe();
                }
                tools.push(Arc::new(AppTool {
                    spec,
                    def: def.clone(),
                    app: app.clone(),
                    registry: self.registry.clone(),
                    project: project.to_path_buf(),
                }));
            }
        }
        tools
    }

    /// The hooks for one session: each running package's `events`.
    pub fn hooks_for(&self, session_id: &str, depth: u32) -> Vec<Arc<dyn ToolHook>> {
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
                }) as Arc<dyn ToolHook>
            })
            .collect()
    }
}

impl ToolProvider for WasmTools {
    fn name(&self) -> &str {
        NAME
    }

    fn tools(&self, session: &SessionFacts) -> Vec<Arc<dyn Tool>> {
        self.tools_for(session.project())
    }

    fn hooks(&self, session: &SessionFacts) -> Vec<Arc<dyn ToolHook>> {
        self.hooks_for(session.session_id(), session.depth())
    }
}

/// One package's tool, as a session has it.
struct AppTool {
    spec: ToolSpec,
    def: ToolDef,
    app: Arc<Loaded>,
    registry: Arc<Registry>,
    /// The session's project: `/project`, for a manifest that asks for `${project}`.
    project: PathBuf,
}

#[async_trait::async_trait]
impl Tool for AppTool {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    fn permission(&self, _: &Value) -> Permission {
        Permission::Ask(format!(
            "The package `{}` wants to run its tool `{}`",
            self.app.title.as_deref().unwrap_or(&self.app.id),
            self.def.name
        ))
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
            project: Some(self.project.clone()),
            progress: true,
        };
        // On behalf of the session's turn: what the tool calls carries the turn's chain.
        let meta = CallMeta::default().chain(cx.chain().iter().cloned());
        let reply = cancellable(&control, &call_id, cx.cancel(), control.call_tool(meta, call)).await;
        self.registry.unlisten(&call_id);
        match reply {
            Ok(ToolReply::Output { content, structured, is_error }) => {
                let out = if is_error { ToolOutput::error(content) } else { ToolOutput::text(content) };
                Ok(out.with_structured(structured))
            }
            Ok(ToolReply::Failed { failure: Failure::Cancelled, .. }) => Err(ToolError::Cancelled),
            Ok(ToolReply::Failed { message, .. }) => Ok(ToolOutput::error(format!("{}: {message}", self.app.id))),
            Err(e) if e.code == codes::METHOD_NOT_FOUND => Err(ToolError::NotFound(e.message)),
            Err(e) => Ok(ToolOutput::error(format!("{}: {}", self.app.id, e.message))),
        }
    }
}

/// A package's `events`, before every tool call of one session.
struct AppEvents {
    app: Arc<Loaded>,
    registry: Arc<Registry>,
    session_id: String,
    depth: u32,
}

/// What a note of the package `app` adds to the result of `tool`: nothing for a tool not its own,
/// and at most [`MAX_NOTE`] characters, under its name.
fn note_for(app: &str, tool: &str, note: &str) -> Option<String> {
    if !tool.starts_with(&crate::registry::tool_name(app, "")) {
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
impl ToolHook for AppEvents {
    async fn before(&self, call: &ToolCallFacts) -> HookDecision {
        let Some(control) = self.registry.control() else {
            return HookDecision::Proceed;
        };
        let payload = json!({
            "tool": call.name(),
            "input": call.input(),
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
        let decision = cancellable(&control, &call_id, call.cancel(), control.hook(CallMeta::default(), hook))
            .await
            .unwrap_or_else(|e| HookReply::Failed(e.message));
        match decision {
            HookReply::Proceed => HookDecision::Proceed,
            HookReply::Block(reason) => {
                tracing::info!(package = %self.app.id, tool = %call.name(), %reason, "a package refused a tool call");
                HookDecision::Block(format!("refused by the package `{}`: {reason}", self.app.id))
            }
            HookReply::AddContext(note) => match note_for(&self.app.id, call.name(), &note) {
                Some(note) => HookDecision::AddContext(note),
                None => {
                    tracing::debug!(package = %self.app.id, tool = %call.name(), "a note for another's tool; left out");
                    HookDecision::Proceed
                }
            },
            HookReply::Failed(failure) => {
                tracing::warn!(package = %self.app.id, tool = %call.name(), %failure, "a package's event hook failed; the call goes on");
                HookDecision::Proceed
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
            note_for("acme.notes", "ext__acme_notes__search", "seen").as_deref(),
            Some("\n\n[acme.notes] seen")
        );
        for tool in ["Bash", "ext__acme_other__search", "ext__acme_notesx__search"] {
            assert_eq!(note_for("acme.notes", tool, "seen"), None, "{tool}");
        }
        let long = "x".repeat(MAX_NOTE + 10);
        let note = note_for("acme.notes", "ext__acme_notes__search", &long).unwrap();
        assert_eq!(note.chars().filter(|c| *c == 'x').count(), MAX_NOTE);
        assert!(note.ends_with('…'));
    }
}
