//! What a package's tools look like to a session, whichever carrier runs them: the provider's
//! name, the tool's spec and permission question, a reply as the tool's output, a hook's reply as
//! a decision, and the note a hook may add. How a call or a hook reaches the package is the
//! carrier's (`WasmTools`, `ProcTools`).
//!
//! **Every call asks first**: a tool's `read-only` claim is the package's own word, so it
//! schedules calls and decides nothing about permission. A note goes on the result of one of its
//! own tools only, and only so long ([`MAX_NOTE`]): a downloaded package does not get to write
//! into the result of every tool the model calls. A hook that fails lets the call through: a
//! package's fault is not the user's.

use std::future::Future;
use std::sync::Arc;

use harnix_ext::codes;
use harnix_ext::engine::{Cancel, HookDecision, Permission, ToolError, ToolOutput, ToolSpec};
use harnix_wasm_port::{CallMeta, Control, Failure, HookReply, RpcError, ToolDef, ToolReply};
use serde_json::json;

use crate::registry::Loaded;

/// The provider's name in every session.
pub const NAME: &str = "harnix-packages";

/// The longest note a package's hook may add to a tool's result, in characters.
pub const MAX_NOTE: usize = 2000;

/// Run `call` (a tool call or a hook named `call_id`), and cancel it in the host if `cancel` fires
/// first. The call is polled first, so the host has its cancellation before it can be asked to.
pub async fn cancellable<T>(
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

/// What the engine registers for one of `app`'s tools.
pub fn tool_spec(app: &Loaded, def: &ToolDef) -> ToolSpec {
    let name = app.tool_name(&def.name);
    let schema = serde_json::from_str(&def.input_schema).unwrap_or_else(|_| json!({"type": "object"}));
    let mut spec = ToolSpec::new(&name, &def.description, schema).source(&format!("ext:{}", app.id));
    if let Some(doc) = &def.doc {
        spec = spec.doc(doc);
    }
    if def.read_only {
        spec = spec.read_only();
    }
    if def.concurrency_safe {
        spec = spec.concurrency_safe();
    }
    spec
}

/// What the person is asked before one of `app`'s tools runs.
pub fn permission(app: &Loaded, tool: &str) -> Permission {
    Permission::Ask(format!(
        "The package `{}` wants to run its tool `{tool}`",
        app.title.as_deref().unwrap_or(&app.id)
    ))
}

/// A package's reply to a tool call, as the tool's output.
pub fn tool_output(app: &str, reply: Result<ToolReply, RpcError>) -> Result<ToolOutput, ToolError> {
    match reply {
        Ok(ToolReply::Output { content, structured, is_error }) => {
            let out = if is_error { ToolOutput::error(content) } else { ToolOutput::text(content) };
            Ok(out.with_structured(structured))
        }
        Ok(ToolReply::Failed { failure: Failure::Cancelled, .. }) => Err(ToolError::Cancelled),
        Ok(ToolReply::Failed { message, .. }) => Ok(ToolOutput::error(format!("{app}: {message}"))),
        Err(e) if e.code == codes::METHOD_NOT_FOUND => Err(ToolError::NotFound(e.message)),
        Err(e) => Ok(ToolOutput::error(format!("{app}: {}", e.message))),
    }
}

/// A package's `events` reply to `tool.before` for the tool `tool`, as the engine's decision.
pub fn hook_decision(app: &str, tool: &str, reply: HookReply) -> HookDecision {
    match reply {
        HookReply::Proceed => HookDecision::Proceed,
        HookReply::Block(reason) => {
            tracing::info!(package = %app, tool = %tool, %reason, "a package refused a tool call");
            HookDecision::Block(format!("refused by the package `{app}`: {reason}"))
        }
        HookReply::AddContext(note) => match note_for(app, tool, &note) {
            Some(note) => HookDecision::AddContext(note),
            None => {
                tracing::debug!(package = %app, tool = %tool, "a note for another's tool; left out");
                HookDecision::Proceed
            }
        },
        HookReply::Failed(failure) => {
            tracing::warn!(package = %app, tool = %tool, %failure, "a package's event hook failed; the call goes on");
            HookDecision::Proceed
        }
    }
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
