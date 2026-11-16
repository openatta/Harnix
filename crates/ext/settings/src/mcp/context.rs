//! The MCP part of what a session's next model call carries, read off the engine's account of it
//! (`engine.session.inspectContext`): each server's tools, and the instructions it gave, as the
//! model reads them. The right column's context tab draws it.
//!
//! The engine names a server's tool's origin `mcp:<server>`, and puts every server's instructions
//! into one system block, `mcp.instructions`, a `## <server>` heading over each.

use std::collections::BTreeMap;

use serde_json::Value;

use crate::wire::{McpContext, McpContextServer, McpContextTool, McpStatus};

/// The system block the engine puts the servers' instructions in.
const INSTRUCTIONS_BLOCK: &str = "mcp.instructions";
/// How the engine says a tool is a server's.
const MCP_SOURCE: &str = "mcp:";

/// `inspect` is `engine.session.inspectContext`'s answer; `statuses` what the engine says of its
/// servers, by name.
pub(crate) fn context_of(inspect: &Value, statuses: &BTreeMap<String, McpStatus>) -> McpContext {
    let mut servers: BTreeMap<String, McpContextServer> = BTreeMap::new();
    for tool in inspect.get("tools").and_then(Value::as_array).into_iter().flatten() {
        let Some(server) = tool.get("source").and_then(Value::as_str).and_then(|s| s.strip_prefix(MCP_SOURCE)) else { continue };
        let number = |key: &str| tool.get(key).and_then(Value::as_u64).unwrap_or(0);
        entry(&mut servers, server).tools.push(McpContextTool {
            name: tool.get("name").and_then(Value::as_str).unwrap_or_default().into(),
            tokens: number("tokens"),
            schema_bytes: number("schema_bytes"),
        });
    }
    for name in statuses.keys() {
        entry(&mut servers, name);
    }
    let block = inspect
        .get("system")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .find(|b| b.get("name").and_then(Value::as_str) == Some(INSTRUCTIONS_BLOCK));
    let instructions_tokens = block.and_then(|b| b.get("tokens")).and_then(Value::as_u64);
    if let Some(text) = block.and_then(|b| b.get("text")).and_then(Value::as_str) {
        let known: Vec<String> = servers.keys().cloned().collect();
        for (name, said) in sections(text, &known) {
            entry(&mut servers, &name).instructions = Some(said);
        }
    }
    for (name, server) in servers.iter_mut() {
        server.status = statuses.get(name).cloned();
    }
    McpContext { servers: servers.into_values().collect(), instructions_tokens }
}

fn entry<'a>(servers: &'a mut BTreeMap<String, McpContextServer>, name: &str) -> &'a mut McpContextServer {
    servers.entry(name.to_string()).or_insert_with(|| McpContextServer { name: name.into(), tools: Vec::new(), instructions: None, status: None })
}

/// Each server's part of the instructions block: what is under its `## <name>` heading. A heading
/// counts when it names a server known otherwise (its tools, the engine's list), so a server's own
/// `## ` line is part of what it said; with none known, every `## ` heading does.
fn sections(text: &str, known: &[String]) -> Vec<(String, String)> {
    let mut out: Vec<(String, Vec<&str>)> = Vec::new();
    for line in text.lines() {
        let heading = line.strip_prefix("## ").map(str::trim).filter(|name| known.is_empty() || known.iter().any(|k| k == name));
        match (heading, out.last_mut()) {
            (Some(name), _) => out.push((name.to_string(), Vec::new())),
            (None, Some((_, lines))) => lines.push(line),
            (None, None) => {}
        }
    }
    out.into_iter().map(|(name, lines)| (name, lines.join("\n").trim().to_string())).filter(|(_, said)| !said.is_empty()).collect()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::wire::McpAuthStatus;

    #[test]
    fn each_servers_tools_and_instructions_are_read_off_the_request() {
        let inspect = json!({
            "system": [
                {"name": "skills.catalog", "tokens": 50, "text": "## not a server\n"},
                {"name": "mcp.instructions", "tokens": 40, "text":
                    "# MCP Server Instructions\n\nThe following MCP servers have provided instructions for how to use their tools and resources:\n\n## docs\nSearch before you read.\n## Notes\nA heading of its own.\n\n## files\nPaths are relative.\n\n"},
            ],
            "tools": [
                {"name": "Bash", "source": "builtin", "tokens": 300, "schema_bytes": 900},
                {"name": "mcp__docs__search", "source": "mcp:docs", "tokens": 120, "schema_bytes": 400},
                {"name": "mcp__files__read", "source": "mcp:files", "tokens": 80, "schema_bytes": 200},
                {"name": "plugin__x__y", "source": "plugin:x", "tokens": 1, "schema_bytes": 1},
            ],
        });
        let status = McpStatus { state: "auth_required".into(), tool_count: 0, error: None, limited: None, auth: McpAuthStatus { state: "required".into(), ..Default::default() } };
        let statuses = BTreeMap::from([("remote".to_string(), status.clone())]);
        let context = context_of(&inspect, &statuses);
        assert_eq!(context.instructions_tokens, Some(40));
        let names: Vec<&str> = context.servers.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["docs", "files", "remote"]);
        let docs = &context.servers[0];
        assert_eq!(docs.tools, [McpContextTool { name: "mcp__docs__search".into(), tokens: 120, schema_bytes: 400 }]);
        assert_eq!(docs.instructions.as_deref(), Some("Search before you read.\n## Notes\nA heading of its own."));
        assert_eq!(context.servers[1].instructions.as_deref(), Some("Paths are relative."));
        // A server the engine lists with nothing in the request: there, with what the engine says.
        assert!(context.servers[2].tools.is_empty() && context.servers[2].instructions.is_none());
        assert_eq!(context.servers[2].status.as_ref(), Some(&status));
    }

    #[test]
    fn a_session_with_no_mcp_has_no_servers() {
        let context = context_of(&json!({"system": [], "tools": [{"name": "Read", "source": "builtin"}]}), &BTreeMap::new());
        assert!(context.servers.is_empty() && context.instructions_tokens.is_none());
        // Instructions from servers known by nothing else: every heading is one.
        let context = context_of(&json!({"system": [{"name": "mcp.instructions", "text": "# MCP\n\n## a\nx\n## b\ny\n"}]}), &BTreeMap::new());
        let said: Vec<(&str, Option<&str>)> = context.servers.iter().map(|s| (s.name.as_str(), s.instructions.as_deref())).collect();
        assert_eq!(said, [("a", Some("x")), ("b", Some("y"))]);
    }
}
