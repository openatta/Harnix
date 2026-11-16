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
//!
//! **What a package's `events` is shown is decided here**, before anything is sent to it
//! ([`EventScope`], [`offered`]): a call it was not granted to see does not leave the product —
//! not its name, not its input, not that it happened. Its host holds it to its manifest as well;
//! that is the second check, not the one that counts.

use std::future::Future;
use std::sync::Arc;

use harnix_ext::engine::{Cancel, HookDecision, Permission, ToolError, ToolOutput, ToolSpec};
use harnix_ext::{caps, codes, Capability};
use harnix_package::manifest::Manifest;
use harnix_wasm_port::{CallMeta, Control, Failure, HookReply, RpcError, ToolDef, ToolReply};
use serde_json::json;

use crate::registry::Loaded;

/// The provider's name in every session.
pub const NAME: &str = "harnix-packages";

pub use harnix_package::MAX_NOTE;

/// Which of a session's tool calls a package's `events` export is offered, as the product decides
/// it from what was consented to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventScope {
    /// It exports no `events` (or has no component): offered nothing.
    None,
    /// The calls of its own tools: `[events] tools = "own"`, the default, or `"all"` without the
    /// grant.
    Own,
    /// Every call of every session, by name and with its input: `[events] tools = "all"`, and
    /// granted `events.all_tools`.
    All,
}

impl EventScope {
    /// For a package whose component exports `events` (as its host found it: `exports`), whose
    /// manifest is `manifest`, granted `granted`.
    pub fn of(exports: bool, manifest: &Manifest, granted: &[Capability]) -> Self {
        if !exports || manifest.component.is_none() {
            Self::None
        } else if manifest.events.all() && granted.contains(&caps::EVENTS_ALL_TOOLS) {
            Self::All
        } else {
            Self::Own
        }
    }
}

/// Whether `app`'s `events` is offered the call of `tool` (by the name a session calls it): asked
/// before the call is sent to it, by either carrier.
pub fn offered(app: &Loaded, tool: &str) -> bool {
    match app.events {
        EventScope::None => false,
        EventScope::Own => app.tools.iter().any(|def| app.tool_name(&def.name) == tool),
        EventScope::All => true,
    }
}

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
        HookReply::AddContext(note) => match harnix_package::note_for(app, tool, &note) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    use harnix_package::manifest::Interface;
    use harnix_package::signature::Trust;
    use serde_json::Value;

    use crate::registry::DEFAULT_TIMEOUT;

    fn manifest(events: &str) -> Manifest {
        Manifest::parse(&format!(
            "[app]\nid = \"acme.a\"\nversion = \"1.0.0\"\nharnix_api = {}\npublisher = \"acme\"\n\n\
             [[component]]\nname = \"tools\"\nworld = \"harnix:ext/tools-ext@1.0.0\"\n{events}",
            harnix_package::manifest::HARNIX_API
        ))
        .unwrap()
    }

    fn search() -> ToolDef {
        ToolDef {
            name: "search".into(),
            description: "d".into(),
            doc: None,
            input_schema: "{}".into(),
            read_only: true,
            concurrency_safe: true,
        }
    }

    fn loaded(events: EventScope) -> Loaded {
        Loaded {
            id: "acme.a".into(),
            version: "1.0.0".into(),
            title: None,
            root: PathBuf::new(),
            component: true,
            events,
            tools: vec![search()],
            ui: Vec::new(),
            interface: Interface::default(),
            surfaces: Vec::new(),
            trust: Trust::parse("unsigned"),
            backend_spaces: Vec::new(),
            inline: Value::Null,
            timeout: DEFAULT_TIMEOUT,
        }
    }

    #[test]
    fn the_scope_is_what_the_manifest_asks_and_the_grant_allows() {
        let own = manifest("");
        let all = manifest("\n[events]\ntools = \"all\"\n");
        let granted = [caps::EVENTS_ALL_TOOLS];
        assert_eq!(EventScope::of(true, &own, &[]), EventScope::Own);
        assert_eq!(EventScope::of(true, &own, &granted), EventScope::Own, "a grant it did not ask for shows it nothing more");
        assert_eq!(EventScope::of(true, &all, &[]), EventScope::Own, "asked for, not granted");
        assert_eq!(EventScope::of(true, &all, &granted), EventScope::All);
        assert_eq!(EventScope::of(false, &all, &granted), EventScope::None, "no `events` export, nothing");
    }

    #[test]
    fn a_package_is_offered_its_own_tools_calls_unless_granted_every_one() {
        let own_tool = harnix_package::tool_name("acme.a", "search");
        let another_s = harnix_package::tool_name("acme.b", "search");
        let own = loaded(EventScope::Own);
        assert!(offered(&own, &own_tool));
        assert!(!offered(&own, "Bash"));
        assert!(!offered(&own, &another_s));
        assert!(!offered(&own, &harnix_package::tool_name("acme.a", "other")), "only a tool it declared");
        let all = loaded(EventScope::All);
        assert!(offered(&all, "Bash") && offered(&all, &another_s) && offered(&all, &own_tool));
        let none = loaded(EventScope::None);
        assert!(!offered(&none, &own_tool) && !offered(&none, "Bash"));
    }
}
