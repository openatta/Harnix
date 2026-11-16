//! A hook a link registered, called: every provider's tool hooks before a tool call, every gate on
//! a message, answered with `session.respondHook`.
//!
//! The hook before a tool call comes before the engine checks the call or asks about it, so a hook
//! also sees calls that are refused after. What a hook adds reaches the model as a reminder before
//! its next step, once the call is let through.

use std::sync::Arc;

use harnix_engine_api::{Cancel, HookDecision, PromptDecision, PromptFacts, ToolCallFacts};

use crate::host::{HostSide, Wired};
use crate::translate::{self, HookEvent, HookVerdict, HostHookCall};

/// The most a hook's answer may add, in characters: the engine's, for one hook.
const MAX_CONTEXT: usize = 10_000;

pub(crate) async fn run(host: Arc<HostSide>, wire: Arc<dyn Wired>, call: HostHookCall, cancel: Cancel) {
    let verdict = match call.event {
        HookEvent::PreToolUse => before_tool(&host, &call, &cancel).await,
        HookEvent::UserPromptSubmit => on_message(&host, &call).await,
        // Not registered for: let through.
        HookEvent::Other(_) => HookVerdict::Proceed { context: None },
    };
    // One that does not go fails as the hook says it fails (`on_error`) when the engine times it out.
    let _ = host.answer(&*wire, translate::SESSION_RESPOND_HOOK, translate::hook_answer(&call.session, &call.call_id, verdict)).await;
    host.done(&call.call_id);
}

/// Every provider's hooks, asked now — one switched off since has none — in order: the first that
/// blocks decides, and what the others add is joined.
async fn before_tool(host: &HostSide, call: &HostHookCall, cancel: &Cancel) -> HookVerdict {
    let (tool, input, session, cwd) = translate::pre_tool_use(&call.input);
    let facts = ToolCallFacts::new(&tool, input, cancel.clone())
        .with_session(session.as_deref().unwrap_or(&call.session), call.agent_session.is_some() as u32)
        .with_cwd(cwd.unwrap_or_default());
    let mut notes = Vec::new();
    for hook in host.providers().iter().flat_map(|p| p.hooks()) {
        match hook.before(&facts).await {
            HookDecision::Block(why) => return HookVerdict::Block(why),
            HookDecision::AddContext(note) if !note.trim().is_empty() => notes.push(note.trim().to_string()),
            _ => {}
        }
        if cancel.is_cancelled() {
            return HookVerdict::Proceed { context: None };
        }
    }
    HookVerdict::Proceed { context: joined(notes) }
}

async fn on_message(host: &HostSide, call: &HostHookCall) -> HookVerdict {
    let (content, delivery, session) = translate::user_prompt(&call.input);
    let facts = PromptFacts::new(session.as_deref().unwrap_or(&call.session), call.turn_id.as_deref().unwrap_or(""), &content, delivery);
    for gate in host.gates() {
        if let PromptDecision::Block(message) = gate.inspect(&facts).await {
            return HookVerdict::Block(message);
        }
    }
    HookVerdict::Proceed { context: None }
}

/// The notes as one, cut at the most one hook may add.
fn joined(notes: Vec<String>) -> Option<String> {
    if notes.is_empty() {
        return None;
    }
    let all = notes.join("\n\n");
    if all.chars().count() <= MAX_CONTEXT {
        return Some(all);
    }
    let mut cut: String = all.chars().take(MAX_CONTEXT - 1).collect();
    cut.push('…');
    Some(cut)
}

#[cfg(test)]
mod tests {
    use super::joined;

    #[test]
    fn notes_are_joined_and_cut_at_what_one_hook_may_add() {
        assert_eq!(joined(vec![]), None);
        assert_eq!(joined(vec!["a".into(), "b".into()]).as_deref(), Some("a\n\nb"));
        let long = joined(vec!["x".repeat(20_000)]).unwrap();
        assert_eq!(long.chars().count(), 10_000);
        assert!(long.ends_with('…'));
    }
}
