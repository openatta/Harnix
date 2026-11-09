//! A `backend-app` component that drives every host service from its methods,
//! keeps a counter in its own memory (so a test can tell a replaced instance
//! from a kept one), and can be told to spin or trap. It exports no `events`:
//! the plain world, which is how a component that has nothing to say about
//! tool calls is built.

wit_bindgen::generate!({
    path: "../../../wit",
    world: "backend-app",
});

use exports::harnix::app::backend::{AppError, Guest as Backend};
use exports::harnix::app::tools::{Guest as Tools, ToolDef, ToolOutput};
use harnix::app::host;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

struct Probe;

static COUNTER: AtomicU64 = AtomicU64::new(0);
static CONFIG: Mutex<String> = Mutex::new(String::new());

/// See tools-probe: a flat-object field reader, enough for a probe.
fn field(json: &str, name: &str) -> Option<String> {
    let at = json.find(&format!("\"{name}\""))? + name.len() + 2;
    let rest = json[at..].trim_start().strip_prefix(':')?.trim_start();
    if let Some(s) = rest.strip_prefix('"') {
        return Some(s[..s.find('"')?].to_string());
    }
    let end = rest.find([',', '}']).unwrap_or(rest.len());
    Some(rest[..end].trim().to_string())
}

fn app_error(code: i32, message: impl Into<String>) -> AppError {
    AppError {
        code,
        message: message.into(),
        data_json: None,
    }
}

fn quoted(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

impl Backend for Probe {
    fn init(config: String) -> Result<(), String> {
        if config.contains("fail-init") {
            return Err("this configuration asks init to fail".into());
        }
        if let Some(topic) = field(&config, "init_topic") {
            host::subscribe(&topic, "{}")?;
        }
        *CONFIG.lock().unwrap() = config;
        Ok(())
    }

    fn handle(method: String, params: String, caller: String) -> Result<String, AppError> {
        let p = |name: &str| field(&params, name).unwrap_or_default();
        match method.as_str() {
            "echo" => Ok(params),
            "caller" => Ok(caller),
            "config" => Ok(CONFIG.lock().unwrap().clone()),
            "count" => Ok((COUNTER.fetch_add(1, Ordering::SeqCst) + 1).to_string()),
            "call" => host::call(&p("target"), &params).map_err(|e| app_error(1, e)),
            "publish" => host::publish(&p("topic"), &params)
                .map(|()| "true".into())
                .map_err(|e| app_error(1, e)),
            "kv.put" => {
                let size: usize = p("size").parse().unwrap_or(0);
                host::kv_set(&p("key"), &vec![b'x'; size])
                    .map(|()| "true".into())
                    .map_err(|e| app_error(2, e))
            }
            "kv.get" => host::kv_get(&p("key"))
                .map(|v| v.map_or("null".into(), |v| v.len().to_string()))
                .map_err(|e| app_error(2, e)),
            "kv.delete" => host::kv_delete(&p("key"))
                .map(|()| "true".into())
                .map_err(|e| app_error(2, e)),
            "file.write" => {
                let size: usize = p("size").parse().unwrap_or(0);
                std::fs::write(format!("/data/{}", p("name")), vec![b'x'; size])
                    .map(|()| "true".into())
                    .map_err(|e| app_error(5, e.to_string()))
            }
            "file.delete" => std::fs::remove_file(format!("/data/{}", p("name")))
                .map(|()| "true".into())
                .map_err(|e| app_error(5, e.to_string())),
            "timer" => {
                let ms: u64 = p("ms").parse().unwrap_or(0);
                host::set_timer(&p("id"), ms)
                    .map(|()| "true".into())
                    .map_err(|e| app_error(3, e))
            }
            "subscribe" => host::subscribe(&p("topic"), "{}")
                .map(|()| "true".into())
                .map_err(|e| app_error(4, e)),
            "spin" => {
                let mut n: u64 = 0;
                loop {
                    n = std::hint::black_box(n.wrapping_add(1));
                }
            }
            "trap" => panic!("asked to trap"),
            "fail" => Err(AppError {
                code: 42,
                message: "failing as asked".into(),
                data_json: Some(r#"{"why":"test"}"#.into()),
            }),
            "not-json" => Ok("this is not JSON".into()),
            other => Err(app_error(-32601, format!("no method `{other}`"))),
        }
    }

    fn on_topic(topic: String, value: String) {
        let _ = host::publish(
            "seen",
            &format!(r#"{{"topic":{},"value":{value}}}"#, quoted(&topic)),
        );
    }

    fn on_timer(id: String) {
        let _ = host::publish("timer", &format!(r#"{{"id":{}}}"#, quoted(&id)));
    }

    fn stop() {
        let _ = host::publish("stopped", "{}");
    }
}

impl Tools for Probe {
    fn list_tools() -> Vec<ToolDef> {
        let tool = |name: &str, description: &str| ToolDef {
            name: name.into(),
            description: description.into(),
            doc: None,
            input_schema: r#"{"type":"object"}"#.into(),
            read_only: true,
            concurrency_safe: true,
        };
        vec![
            tool("peek", "The counter, as this instance sees it"),
            tool("explode", "Trap"),
        ]
    }

    /// Runs in an instance of its own, so `peek` sees its own counter (zero),
    /// never the backend's.
    fn call_tool(name: String, _input: String, _call_id: String) -> ToolOutput {
        if name == "explode" {
            panic!("asked to explode");
        }
        ToolOutput {
            content: COUNTER.load(Ordering::SeqCst).to_string(),
            structured: None,
            is_error: false,
        }
    }
}

export!(Probe);
