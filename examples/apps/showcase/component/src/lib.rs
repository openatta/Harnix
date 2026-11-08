//! The showcase's component: a `backend-app` (`harnix:app`, `crates/wasm/wit/app.wit`) — a
//! resident backend, and the agent's tools, each tool call in an instance of its own.
//!
//! # The backend
//!
//! One instance for the whole process, started with the application (`init`) and stopped with it
//! (`stop`). Its methods are `app.example.showcase.<name>`, declared in `harnix-app.toml`:
//!
//! | method | shows |
//! |---|---|
//! | `notes.add {text}` | params checked against the manifest's schema; the key-value store; the `notes` topic |
//! | `notes.list`, `notes.clear` | the same store, read and emptied |
//! | `counter.bump` | state in the instance's own memory, which a recycled instance loses |
//! | `state` | what the backend knows: what its tools ask it for, with `host.call` |
//! | `timer.start {ms, times?}` | `host.set-timer`: each firing publishes `heartbeat` |
//! | `fetch {url}` | `host.http-request`, to a host in `net` only |
//! | `session.start {message, …}` | driving the agent: `session.create` and `harnix.send`, as `module:example.showcase` |
//! | `overreach` | a call its `hub` does not list: refused, and on the record |
//! | `spin`, `crash` | what a timeout and a trap do to the instance |
//! | `card.post {session_id, …}` | an entry in a session's flow (`harnix.entries.append`), which the shell draws as the manifest's card |
//! | `complete {trigger, name, query}` | the candidates of its `/showcase` command and its `@notes` source |
//! | `sessions.group {space, sessions}` | how its space's left column groups sessions: those it put a card in, and the rest |
//!
//! It subscribes to `harnix.sessions.status` in `init` (listed in its `hub` as
//! `topic:harnix.sessions.status`) and counts what arrives at `on-topic`.
//!
//! # The tools
//!
//! | tool | shows |
//! |---|---|
//! | `echo {text}` | a plain answer, with structured content beside it |
//! | `count {key?}` | the application's key-value storage, which outlives the call's instance |
//! | `config` | `host.config()`: the manifest's `[config]`, with the profile's settings over it |
//! | `read_project {path}` | the calling session's project, read-only, at `/project` |
//! | `slow {steps?, ms?}` | progress while it runs (`host.progress`), and being cancelled |
//! | `vetoed` | nothing: the event hook below refuses every call to it |
//! | `board` | the backend's state, asked of it with `host.call` — a tool cannot see its memory |
//!
//! The `events` export sees every tool call of a session that has the application — not only
//! its own tools — and refuses the one its configuration names (`blocked_tool`).

wit_bindgen::generate!({
    path: "../../../../crates/wasm/wit",
    world: "backend-app",
});

use exports::harnix::app::backend::{AppError, Guest as Backend};
use exports::harnix::app::events::{Guest as Events, HookDecision};
use exports::harnix::app::tools::{Guest as Tools, ToolDef, ToolOutput};
use harnix::app::host;
use serde_json::{json, Value};
use std::sync::Mutex;

struct Showcase;

const ID: &str = "example.showcase";
/// Where the notes are kept: the key-value store, so they outlive a recycled instance.
const NOTES: &str = "notes";
/// The sessions it put a card in, for `sessions.group`.
const CARDS: &str = "cards";

/// The backend's memory: gone when the instance is replaced, which is the point of `counter`.
#[derive(Default)]
struct Memory {
    counter: u64,
    ticks: u64,
    /// Firings left of the timer `timer.start` armed.
    ticks_left: u64,
    tick_ms: u64,
    /// Values seen on `harnix.sessions.status`, and how many sessions the last one had running.
    status_seen: u64,
    running_sessions: u64,
    started_ms: u64,
}

static MEMORY: Mutex<Option<Memory>> = Mutex::new(None);

fn memory<T>(f: impl FnOnce(&mut Memory) -> T) -> T {
    let mut guard = MEMORY.lock().unwrap();
    f(guard.get_or_insert_with(Memory::default))
}

fn error(code: i32, message: impl Into<String>) -> AppError {
    AppError {
        code,
        message: message.into(),
        data_json: None,
    }
}

fn config() -> Value {
    serde_json::from_str(&host::config()).unwrap_or(Value::Null)
}

fn notes() -> Vec<Value> {
    host::kv_get(NOTES)
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

/// Keep the notes and tell whoever watches the `notes` topic.
fn save(notes: &[Value]) -> Result<(), AppError> {
    let bytes = serde_json::to_vec(notes).unwrap_or_default();
    host::kv_set(NOTES, &bytes).map_err(|e| error(2, e))?;
    let _ = host::publish("notes", &json!({"notes": notes, "count": notes.len()}).to_string());
    Ok(())
}

fn carded() -> Vec<String> {
    host::kv_get(CARDS)
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

/// `{en, zh-CN}`: the shell shows the reader's.
fn words(en: &str, zh: &str) -> Value {
    json!({"en": en, "zh-CN": zh})
}

/// The `/showcase…` commands and the `@notes` source. A command names the method the shell runs for
/// it — one of the showcase's own, nothing else; a note is put in the composer.
fn complete(params: &Value) -> Value {
    let query = params["query"].as_str().unwrap_or_default().to_lowercase();
    let candidates: Vec<Value> = match (params["trigger"].as_str(), params["name"].as_str()) {
        (Some("/"), Some("showcase")) => vec![
            json!({
                "replacement": "/showcase-bump",
                "description": words("Count one more on the showcase's counter", "在示例应用的计数器上再数一个"),
                "call": {"method": "counter.bump"},
                "done": words("Counted one more", "又数了一个"),
            }),
            json!({
                "replacement": "/showcase-clear",
                "description": words("Take every note off the showcase's board", "撤下示例应用留言板上的所有留言"),
                "call": {"method": "notes.clear"},
                "done": words("The board is empty", "留言板空了"),
            }),
        ],
        (Some("@"), Some("notes")) => notes()
            .iter()
            .filter_map(|n| n["text"].as_str())
            .filter(|text| query.is_empty() || text.to_lowercase().contains(&query))
            .take(10)
            .map(|text| {
                json!({
                    "replacement": format!("“{text}” "),
                    "label": format!("notes:{text}"),
                    "description": words("A note on the showcase's board", "示例应用留言板上的留言"),
                })
            })
            .collect(),
        _ => Vec::new(),
    };
    json!({"candidates": candidates})
}

fn state() -> Value {
    let (counter, ticks, status_seen, running, started) = memory(|m| {
        (m.counter, m.ticks, m.status_seen, m.running_sessions, m.started_ms)
    });
    json!({
        "notes": notes().len(),
        "counter": counter,
        "ticks": ticks,
        "status_seen": status_seen,
        "running_sessions": running,
        "started_ms": started,
        "secret_set": host::secret("SHOWCASE_TOKEN").is_some(),
    })
}

impl Backend for Showcase {
    fn init(config: String) -> Result<(), String> {
        let config: Value = serde_json::from_str(&config).unwrap_or(Value::Null);
        memory(|m| {
            *m = Memory::default();
            m.started_ms = host::now_ms();
        });
        // Every change to which sessions are running arrives at `on-topic`. Not being let watch is
        // no reason not to start: the board works without it.
        if config["watch_sessions"].as_bool().unwrap_or(true) {
            if let Err(e) = host::subscribe("harnix.sessions.status", "{}") {
                host::log("warn", &format!("not watching the sessions: {e}"));
            }
        }
        host::log("info", "the showcase's backend started");
        Ok(())
    }

    fn handle(method: String, params: String, caller: String) -> Result<String, AppError> {
        let params: Value = serde_json::from_str(&params).unwrap_or(Value::Null);
        let answer = match method.as_str() {
            "notes.add" => {
                let caller: Value = serde_json::from_str(&caller).unwrap_or(Value::Null);
                let mut all = notes();
                all.push(json!({
                    "text": params["text"],
                    "by": caller["subject"],
                    "at_ms": host::now_ms(),
                }));
                save(&all)?;
                json!({"count": all.len()})
            }
            "notes.list" => json!({"notes": notes()}),
            "notes.clear" => {
                save(&[])?;
                json!({"count": 0})
            }
            "counter.bump" => json!({"counter": memory(|m| { m.counter += 1; m.counter })}),
            "state" => state(),
            "timer.start" => {
                let ms = params["ms"].as_u64().unwrap_or(500);
                let times = params["times"].as_u64().unwrap_or(1).clamp(1, 100);
                memory(|m| {
                    m.ticks_left = times;
                    m.tick_ms = ms;
                });
                host::set_timer("tick", ms).map_err(|e| error(3, e))?;
                json!({"armed": true, "ms": ms, "times": times})
            }
            "fetch" => {
                let url = params["url"].as_str().unwrap_or_default();
                let response = host::http_request("GET", url, &[], None).map_err(|e| error(4, e))?;
                json!({
                    "status": response.status,
                    "body": String::from_utf8_lossy(&response.body),
                })
            }
            "session.start" => {
                // The agent, driven by the application: a session of its own, and a prompt.
                let mut create = json!({"instance": "system"});
                for key in ["project_root", "options", "space"] {
                    if !params[key].is_null() {
                        create[key] = params[key].clone();
                    }
                }
                let created = host::call("session.create", &create.to_string()).map_err(|e| error(5, e))?;
                let created: Value = serde_json::from_str(&created).unwrap_or(Value::Null);
                let session_id = created["session_id"].clone();
                let send = json!({
                    "instance": "system",
                    "session_id": session_id,
                    "message": params["message"].as_str().unwrap_or("Hello from the showcase's backend"),
                });
                let sent = host::call("harnix.send", &send.to_string()).map_err(|e| error(5, e))?;
                json!({
                    "instance": "system",
                    "session_id": session_id,
                    "sent": serde_json::from_str::<Value>(&sent).unwrap_or(Value::Null),
                })
            }
            "overreach" => {
                // Its `hub` does not list this; the answer is the refusal, as the backend got it.
                match host::call("harnix.apps.uninstall", &json!({"id": ID}).to_string()) {
                    Ok(answer) => json!({"refused": false, "answer": answer}),
                    Err(refusal) => json!({"refused": true, "message": refusal}),
                }
            }
            "spin" => loop {
                std::hint::black_box(memory(|m| m.counter));
            },
            "crash" => panic!("the showcase was asked to crash"),
            "card.post" => {
                // An entry in the session's log, for people: the shell draws its `card` event as the
                // manifest's `[[inline.flow_block]]`, with this payload in its placeholders.
                let caller: Value = serde_json::from_str(&caller).unwrap_or(Value::Null);
                let session_id = params["session_id"].as_str().unwrap_or_default().to_string();
                let instance = params["instance"].as_str().unwrap_or("system");
                let entry = json!({
                    "instance": instance,
                    "session_id": session_id,
                    "event": "card",
                    "payload": {
                        "title": params["title"].as_str().unwrap_or("A card from the showcase"),
                        "body": params["body"].as_str().unwrap_or("The board has **notes**, a counter and a heartbeat."),
                        "notes": notes().len(),
                        "by": caller["subject"],
                    },
                });
                let appended = host::call("harnix.entries.append", &entry.to_string()).map_err(|e| error(6, e))?;
                let mut cards = carded();
                if !cards.contains(&session_id) {
                    cards.push(session_id.clone());
                    let bytes = serde_json::to_vec(&cards).unwrap_or_default();
                    host::kv_set(CARDS, &bytes).map_err(|e| error(2, e))?;
                }
                json!({
                    "session_id": session_id,
                    "appended": serde_json::from_str::<Value>(&appended).unwrap_or(Value::Null),
                })
            }
            "complete" => complete(&params),
            "sessions.group" => {
                // The shell hands over the space's sessions; the rest of them land in its `other`.
                let cards = carded();
                let ids: Vec<Value> = params["sessions"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|s| s["session_id"].as_str())
                    .filter(|id| cards.iter().any(|c| c == id))
                    .map(|id| json!(id))
                    .collect();
                json!({"groups": [{
                    "key": "cards",
                    "title": words("With a card", "有卡片的"),
                    "sessions": ids,
                }]})
            }
            other => return Err(error(-32601, format!("the showcase has no method `{other}`"))),
        };
        Ok(answer.to_string())
    }

    fn on_topic(topic: String, value: String) {
        if topic != "harnix.sessions.status" {
            return;
        }
        let value: Value = serde_json::from_str(&value).unwrap_or(Value::Null);
        let running = value["sessions"].as_array().map_or(0, |s| s.len() as u64);
        memory(|m| {
            m.status_seen += 1;
            m.running_sessions = running;
        });
    }

    fn on_timer(id: String) {
        if id != "tick" {
            return;
        }
        let (ticks, left, ms) = memory(|m| {
            m.ticks += 1;
            m.ticks_left = m.ticks_left.saturating_sub(1);
            (m.ticks, m.ticks_left, m.tick_ms)
        });
        let _ = host::publish(
            "heartbeat",
            &json!({"ticks": ticks, "at_ms": host::now_ms()}).to_string(),
        );
        if left > 0 {
            let _ = host::set_timer("tick", ms);
        }
    }

    fn stop() {
        host::log("info", "the showcase's backend stopped");
    }
}

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
            tool(
                "board",
                "What the showcase's backend holds now: its notes, its counter, its timer",
                object(json!({}), &[]),
                true,
            ),
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
            // This instance is not the backend's: what the backend holds, it has to ask for.
            "board" => match host::call(&format!("app.{ID}.state"), "{}") {
                Ok(state) => {
                    let state: Value = serde_json::from_str(&state).unwrap_or(Value::Null);
                    answer(
                        format!(
                            "the board has {} notes; the counter is at {}",
                            state["notes"], state["counter"]
                        ),
                        Some(state),
                    )
                }
                Err(e) => failure(format!("the backend did not answer: {e}")),
            },
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
