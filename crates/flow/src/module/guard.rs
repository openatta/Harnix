//! `writes`: in a session working on a flow task, a tool that writes is refused unless the
//! station lists it. Installed on every session of a wrapped scene; a session not bound to a
//! task (between tasks, or never given one) is left alone.

use std::sync::Arc;

use base::interface::tool_middleware::{
    NextDispatch, ToolAnswer, ToolCall, ToolExec, ToolMiddleware, ToolOutcome,
};

use super::shared::Shared;

/// The engine's tools that change something outside the conversation: the shell, the file
/// editors, the notebook editor, the interpreters.
pub const DEFAULT_GUARDED: &[&str] = &[
    "Bash",
    "PowerShell",
    "REPL",
    "Write",
    "Edit",
    "NotebookEdit",
];

/// Whether `name` matches any of `patterns` (`prefix*` matches every name with that prefix).
pub fn matches_any(patterns: &[String], name: &str) -> bool {
    patterns.iter().any(|p| match p.strip_suffix('*') {
        Some(prefix) => name.starts_with(prefix),
        None => p == name,
    })
}

/// Why a write is refused, or `None` when it may go ahead.
pub(crate) fn refusal(
    guarded: &[String],
    binding: Option<&super::shared::Binding>,
    tool: &str,
) -> Option<String> {
    let b = binding?;
    if !matches_any(guarded, tool) || matches_any(&b.writes, tool) {
        return None;
    }
    let allowed = if b.writes.is_empty() {
        "it allows no writes".to_string()
    } else {
        format!("it allows only {}", b.writes.join(", "))
    };
    Some(format!(
        "`{tool}` writes, and this session is working on station `{}` of run `{}`: {allowed}. \
         If the work needs it, say so with `flow_propose`.",
        b.station, b.run
    ))
}

pub(crate) struct Writes {
    sh: Arc<Shared>,
    session_id: String,
}

impl Writes {
    pub fn new(sh: Arc<Shared>, session_id: &str) -> Self {
        Self {
            sh,
            session_id: session_id.to_string(),
        }
    }
}

#[async_trait::async_trait]
impl ToolMiddleware for Writes {
    async fn around(
        &self,
        call: &ToolCall,
        exec: &mut ToolExec,
        next: NextDispatch<'_>,
    ) -> ToolOutcome {
        let binding = self.sh.binding(&self.session_id);
        if let Some(why) = refusal(&self.sh.cfg.guarded, binding.as_ref(), &call.name) {
            return Ok(ToolAnswer::failure(why));
        }
        next.run(call, exec).await
    }
}

#[cfg(test)]
mod tests {
    use super::super::shared::Binding;
    use super::*;

    fn guarded() -> Vec<String> {
        DEFAULT_GUARDED.iter().map(|s| s.to_string()).collect()
    }

    fn bound(writes: &[&str]) -> Binding {
        Binding {
            instance: "system".into(),
            run: "demo-1".into(),
            station: "analyze".into(),
            round: 1,
            task: "core".into(),
            token: "t".into(),
            writes: writes.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn patterns_match_names_and_prefixes() {
        let p = vec!["Bash".to_string(), "mcp__db__*".to_string()];
        assert!(matches_any(&p, "Bash"));
        assert!(matches_any(&p, "mcp__db__insert"));
        assert!(!matches_any(&p, "mcp__web__get"));
        assert!(!matches_any(&p, "BashX"));
    }

    #[test]
    fn a_bound_session_writes_only_what_its_station_allows() {
        let g = guarded();
        let read_only = bound(&[]);
        assert!(refusal(&g, Some(&read_only), "Bash").is_some());
        assert!(
            refusal(&g, Some(&read_only), "Read").is_none(),
            "reads are not guarded"
        );
        assert!(refusal(&g, Some(&read_only), "flow_submit").is_none());
        let edits = bound(&["Edit", "Write"]);
        assert!(refusal(&g, Some(&edits), "Edit").is_none());
        let why = refusal(&g, Some(&edits), "Bash").unwrap();
        assert!(
            why.contains("analyze") && why.contains("Edit, Write"),
            "{why}"
        );
        assert!(
            refusal(&g, None, "Bash").is_none(),
            "an unbound session is left alone"
        );
    }
}
