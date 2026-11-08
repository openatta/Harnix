//! The showcase's tools: a `tools-app` component (`harnix:app`, `crates/wasm/wit/app.wit`).
//!
//! | tool | shows |
//! |---|---|
//! | `echo {text}` | a plain answer, with structured content beside it |
//! | `count {key?}` | the application's key-value storage, which outlives the call's instance |
//! | `config` | `host.config()`: the manifest's `[config]`, with the profile's settings over it |
//! | `read_project {path}` | the calling session's project, read-only, at `/project` |
//! | `slow {steps?, ms?}` | progress while it runs (`host.progress`), and being cancelled |
//! | `vetoed` | nothing: the event hook below refuses every call to it |
//!
//! The `events` export sees every tool call of a session that has the application — not only
//! its own tools — and refuses the one its configuration names (`blocked_tool`).

wit_bindgen::generate!({
    path: "../../../../crates/wasm/wit",
    world: "tools-app",
});

use exports::harnix::app::events::{Guest as Events, HookDecision};
use exports::harnix::app::tools::{Guest as Tools, ToolDef, ToolOutput};
use harnix::app::host;
use serde_json::{json, Value};

struct Showcase;

fn tool(name: &str, description: &str, schema: Value, read_only: bool) -> ToolDef {
    ToolDef {
        name: name.into(),
        description: description.into(),
        doc: None,
        input_schema: schema.to_string(),
        read_only,
        concurrency_safe: read_only,
    }
}

fn answer(content: impl Into<String>, structured: Option<Value>) -> ToolOutput {
    ToolOutput {
        content: content.into(),
        structured: structured.map(|v| v.to_string()),
        is_error: false,
    }
}

fn failure(content: impl Into<String>) -> ToolOutput {
    ToolOutput {
        content: content.into(),
        structured: None,
        is_error: true,
    }
}

fn config() -> Value {
    serde_json::from_str(&host::config()).unwrap_or(Value::Null)
}

impl Tools for Showcase {
    fn list_tools() -> Vec<ToolDef> {
        let object = |props: Value, required: &[&str]| json!({"type": "object", "properties": props, "required": required});
        let mut echo = tool(
            "echo",
            "Say `text` back, exactly",
            object(
                json!({"text": {"type": "string", "description": "What to say back"}}),
                &["text"],
            ),
            true,
        );
        echo.doc = Some(
            "Answers with the text it was given, and the same text as structured content.".into(),
        );
        vec![
            echo,
            tool(
                "count",
                "Count one more for `key` (default `visits`) and say the total",
                object(json!({"key": {"type": "string"}}), &[]),
                false,
            ),
            tool("config", "Show the showcase's configuration", object(json!({}), &[]), true),
            tool(
                "read_project",
                "Read a file of the current project, by its path relative to the project",
                object(json!({"path": {"type": "string"}}), &["path"]),
                true,
            ),
            tool(
                "slow",
                "Work for a while, reporting each step: `steps` (default 5) of `ms` (default 400) each",
                object(json!({"steps": {"type": "integer"}, "ms": {"type": "integer"}}), &[]),
                true,
            ),
            tool("vetoed", "A tool the showcase's own event hook refuses", object(json!({}), &[]), true),
        ]
    }

    fn call_tool(name: String, input: String, call_id: String) -> ToolOutput {
        let input: Value = serde_json::from_str(&input).unwrap_or(Value::Null);
        match name.as_str() {
            "echo" => {
                let text = input["text"].as_str().unwrap_or_default().to_string();
                answer(text.clone(), Some(json!({"echoed": text})))
            }
            "count" => {
                let key = format!("count:{}", input["key"].as_str().unwrap_or("visits"));
                let n = host::kv_get(&key)
                    .and_then(|v| String::from_utf8(v).ok())
                    .and_then(|s| s.parse::<u64>().ok())
                    .unwrap_or(0)
                    + 1;
                match host::kv_set(&key, n.to_string().as_bytes()) {
                    Ok(()) => answer(format!("count = {n}"), Some(json!({"count": n}))),
                    Err(e) => failure(format!("could not count: {e}")),
                }
            }
            "config" => {
                let config = config();
                answer(config.to_string(), Some(config))
            }
            "read_project" => {
                let path = input["path"]
                    .as_str()
                    .unwrap_or_default()
                    .trim_start_matches('/');
                match std::fs::read_to_string(format!("/project/{path}")) {
                    Ok(text) => answer(text, None),
                    Err(e) => failure(format!("cannot read `{path}` in the project: {e}")),
                }
            }
            "slow" => {
                let steps = input["steps"].as_u64().unwrap_or(5).clamp(1, 100);
                let ms = input["ms"].as_u64().unwrap_or(400).min(10_000);
                for step in 1..=steps {
                    host::progress(&call_id, &format!("step {step} of {steps}"));
                    std::thread::sleep(std::time::Duration::from_millis(ms));
                }
                answer(format!("worked through {steps} steps"), None)
            }
            "vetoed" => answer(
                "the event hook let this through, and it should not have",
                None,
            ),
            other => failure(format!("the showcase has no tool `{other}`")),
        }
    }
}

impl Events for Showcase {
    fn on_event(event: String, payload: String) -> HookDecision {
        if event != "tool.before" {
            return HookDecision::Proceed;
        }
        let payload: Value = serde_json::from_str(&payload).unwrap_or(Value::Null);
        let blocked = config()["blocked_tool"].as_str().map(str::to_string);
        match (payload["tool"].as_str(), blocked) {
            (Some(tool), Some(blocked)) if tool == blocked => HookDecision::Block(format!(
                "`{tool}` is refused by the showcase's event hook (its `blocked_tool`)"
            )),
            _ => HookDecision::Proceed,
        }
    }
}

export!(Showcase);
