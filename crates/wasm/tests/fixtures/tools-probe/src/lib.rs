//! A `tools-app` component whose tools exist to be provoked: one echoes, one
//! never returns, one eats memory, one reaches for the network and a secret,
//! one touches files, one traps. The host's tests check that each ends the
//! way the design says it should.

wit_bindgen::generate!({
    path: "../../../wit",
    world: "tools-app",
});

use exports::harnix::app::events::{Guest as Events, HookDecision};
use exports::harnix::app::tools::{Guest as Tools, ToolDef, ToolOutput};
use harnix::app::host;

struct Probe;

fn tool(name: &str, description: &str) -> ToolDef {
    ToolDef {
        name: name.into(),
        description: description.into(),
        doc: None,
        input_schema: r#"{"type":"object"}"#.into(),
        read_only: true,
        concurrency_safe: true,
    }
}

fn ok(content: impl Into<String>) -> ToolOutput {
    ToolOutput {
        content: content.into(),
        structured: None,
        is_error: false,
    }
}

fn err(content: impl Into<String>) -> ToolOutput {
    ToolOutput {
        content: content.into(),
        structured: None,
        is_error: true,
    }
}

/// The string value of `"name":"…"` in a flat JSON object. Enough for a probe;
/// a JSON parser would be most of the component.
fn field(json: &str, name: &str) -> Option<String> {
    let at = json.find(&format!("\"{name}\""))? + name.len() + 2;
    let rest = json[at..].trim_start().strip_prefix(':')?.trim_start();
    if let Some(s) = rest.strip_prefix('"') {
        return Some(s[..s.find('"')?].to_string());
    }
    let end = rest.find([',', '}']).unwrap_or(rest.len());
    Some(rest[..end].trim().to_string())
}

impl Tools for Probe {
    fn list_tools() -> Vec<ToolDef> {
        let mut echo = tool("echo", "Echo `text` back");
        echo.doc = Some("Returns `text` verbatim, plus a structured copy.".into());
        echo.input_schema =
            r#"{"type":"object","properties":{"text":{"type":"string"}},"required":["text"]}"#
                .into();
        vec![
            echo,
            tool("slow", "Never return"),
            tool("hog", "Allocate `mb` MiB and touch every page"),
            tool(
                "fetch",
                "GET `url` through the host, and read the secret `secret`",
            ),
            tool("read-file", "Read the file at `path`"),
            tool("write-file", "Write `text` to the file at `path`"),
            tool("explode", "Trap"),
            tool("timer", "Try to arm a timer, which a tool may not"),
            tool("config", "Return the application's configuration"),
        ]
    }

    fn call_tool(name: String, input: String, call_id: String) -> ToolOutput {
        match name.as_str() {
            "echo" => {
                let text = field(&input, "text").unwrap_or_default();
                host::progress(&call_id, "echoing");
                ToolOutput {
                    structured: Some(format!(r#"{{"echoed":"{text}"}}"#)),
                    ..ok(text)
                }
            }
            "slow" => {
                let mut n: u64 = 0;
                loop {
                    n = std::hint::black_box(n.wrapping_add(1));
                }
            }
            "hog" => {
                let mb: usize = field(&input, "mb")
                    .and_then(|m| m.parse().ok())
                    .unwrap_or(1);
                let mut v: Vec<u8> = Vec::with_capacity(mb << 20);
                v.resize(mb << 20, 1);
                ok(format!("held {} bytes", std::hint::black_box(&v).len()))
            }
            "fetch" => {
                let mut out = String::new();
                if let Some(key) = field(&input, "secret") {
                    out += &format!(
                        "secret={} ",
                        host::secret(&key).unwrap_or_else(|| "none".into())
                    );
                }
                let url = field(&input, "url").unwrap_or_default();
                match host::http_request("GET", &url, &[("x-probe".into(), "1".into())], None) {
                    Ok(r) => ok(format!(
                        "{out}status={} body={}",
                        r.status,
                        String::from_utf8_lossy(&r.body)
                    )),
                    Err(e) => err(format!("{out}refused: {e}")),
                }
            }
            "read-file" => match std::fs::read_to_string(field(&input, "path").unwrap_or_default())
            {
                Ok(text) => ok(text),
                Err(e) => err(e.to_string()),
            },
            "write-file" => {
                let path = field(&input, "path").unwrap_or_default();
                match std::fs::write(&path, field(&input, "text").unwrap_or_default()) {
                    Ok(()) => ok("written"),
                    Err(e) => err(e.to_string()),
                }
            }
            "explode" => panic!("asked to explode"),
            "config" => ok(host::config()),
            "timer" => match host::set_timer("t", 10) {
                Ok(()) => ok("armed"),
                Err(e) => err(e),
            },
            other => err(format!("no tool `{other}`")),
        }
    }
}

impl Events for Probe {
    /// Blocks any use of the tool `forbidden`; annotates `note` events.
    fn on_event(event: String, payload: String) -> HookDecision {
        if event == "tool.before" && field(&payload, "tool").as_deref() == Some("forbidden") {
            return HookDecision::Block("`forbidden` is not allowed here".into());
        }
        if event == "note" {
            return HookDecision::AddContext(format!("noted: {payload}"));
        }
        HookDecision::Proceed
    }
}

export!(Probe);
