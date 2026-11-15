//! The readers against an engine that answers from a script.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use harnix_contract::sync::lock;
use harnix_contract::{codes, RpcError};
use harnix_engine_api::ops::SessionInfo;
use harnix_engine_api::readers::{ReadError, Recordings, SearchLimits, Transcripts};
use serde_json::{json, Value};

use super::calls::Calls;
use super::wire::{CallList, Detail, SearchAnswer};
use super::{trace, ProtocolReader};
use crate::activity::Activity;
use crate::gate::WakeGate;
use crate::translate::{Rpc, Stir};

/// An engine with one recording per root, answering the protocol's reads.
struct Engine {
    /// The rows of the recording of root `S`.
    rows: Mutex<Vec<Value>>,
    generation: Mutex<u64>,
    /// Each session's `parent_session_id`, for `session.get`.
    parents: Mutex<Vec<(String, String)>>,
    /// Which sessions have a recording: the roots.
    roots: Vec<String>,
    asked: Mutex<Vec<(String, Value)>>,
    /// Detail of a call, by index: the request's messages and the response's tool uses.
    details: Mutex<Vec<(usize, Value)>>,
    blobs: Mutex<Vec<(String, Value)>>,
    /// Refuse every call as the instance being unavailable.
    down: Mutex<bool>,
}

impl Engine {
    fn new(roots: &[&str]) -> Arc<Self> {
        Arc::new(Self {
            rows: Mutex::new(Vec::new()),
            generation: Mutex::new(1),
            parents: Mutex::new(Vec::new()),
            roots: roots.iter().map(|s| s.to_string()).collect(),
            asked: Mutex::new(Vec::new()),
            details: Mutex::new(Vec::new()),
            blobs: Mutex::new(Vec::new()),
            down: Mutex::new(false),
        })
    }

    fn asked(&self, method: &str) -> Vec<Value> {
        lock(&self.asked).iter().filter(|(m, _)| m == method).map(|(_, p)| p.clone()).collect()
    }
}

fn unavailable() -> RpcError {
    RpcError::new(codes::INSTANCE_UNAVAILABLE, "the instance is down")
}

#[async_trait::async_trait]
impl Rpc for Engine {
    async fn call(&self, method: &str, params: Value) -> Result<Value, RpcError> {
        lock(&self.asked).push((method.to_string(), params.clone()));
        if *lock(&self.down) {
            return Err(unavailable());
        }
        let session = params["session_id"].as_str().unwrap_or_default().to_string();
        match method {
            "session.calls" => {
                if !self.roots.contains(&session) {
                    return Ok(json!({"session_id": session, "recording": null, "calls": [], "next": 0, "total": 0, "damaged": 0}));
                }
                let generation = *lock(&self.generation);
                if let Some(g) = params["generation"].as_u64() {
                    if g != generation {
                        return Err(RpcError { code: codes::INVALID_PARAMS, message: "the recording was rewritten".into(), data: Some(json!({"generation": generation})) });
                    }
                }
                let rows = lock(&self.rows).clone();
                let from = params["from"].as_u64().unwrap_or(0) as usize;
                let limit = (params["limit"].as_u64().unwrap_or(200) as usize).min(500);
                let page: Vec<Value> = rows.iter().skip(from).take(limit).cloned().collect();
                let next = page.iter().position(|r| r["status"] == "in_flight").map_or(from + page.len(), |p| from + p);
                Ok(json!({
                    "session_id": session,
                    "recording": {"name": session, "created_at": "2025-11-15T09:46:40Z", "covers": "latest_run", "generation": generation},
                    "calls": page, "next": next, "total": rows.len(), "damaged": 0
                }))
            }
            "session.get" => {
                let parent = lock(&self.parents).iter().find(|(c, _)| *c == session).map(|(_, p)| p.clone());
                let mut info = json!({"session_id": session});
                if let Some(parent) = parent {
                    info["parent_session_id"] = json!(parent);
                }
                Ok(info)
            }
            "session.call" => {
                let index = params["index"].as_u64().unwrap_or(0) as usize;
                let generation = *lock(&self.generation);
                let found = lock(&self.details).iter().find(|(i, _)| *i == index).map(|(_, d)| d.clone());
                match found {
                    Some(detail) => Ok(json!({"session_id": session, "index": index, "generation": generation,
                        "request": detail["request"], "response": detail["response"]})),
                    None => Err(RpcError::invalid_params("no such call")),
                }
            }
            "session.callBlob" => {
                let blob = params["blob"].as_str().unwrap_or_default().to_string();
                match lock(&self.blobs).iter().find(|(b, _)| *b == blob) {
                    Some((b, v)) => Ok(json!({"blob": b, "bytes": 1, "value": v})),
                    None => Err(RpcError::invalid_params("no such blob")),
                }
            }
            "session.search" => Ok(json!({
                "hits": [{"session_id": "a", "name": null, "seq": 12, "origin": "steer", "ts": "2025-11-15T09:46:40Z", "snippet": "…x…"},
                         {"session_id": "b", "name": null, "seq": 3, "origin": "assistant", "ts": "2025-11-15T09:46:41Z", "snippet": "y"}],
                "scanned": 2, "truncated": false
            })),
            other => panic!("unexpected call {other}"),
        }
    }
}

/// One row of a recording, as `session.calls` gives it.
fn row(index: usize, session: &str, parent: Option<&str>, turn: u32, purpose: Option<&str>) -> Value {
    json!({
        "index": index, "session_id": session, "parent_session_id": parent, "agent_type": null,
        "turn_id": format!("t{turn}"), "turn": turn, "step": 0, "purpose": purpose,
        "ts": "2025-11-15T09:46:40Z", "provider": "anthropic", "model": "m",
        "input": {"system": [{"source": "identity", "chars": 10}], "tools": 3, "messages": 2},
        "output": {"chars": 5, "thinking_chars": 0, "tool_uses": []},
        "status": "ok", "stop_reason": "end_turn",
        "usage": {"input_tokens": 10, "output_tokens": 20, "cache_creation_input_tokens": 0, "cache_read_input_tokens": 0},
        "timing": {"sent_at": "2025-11-15T09:46:40Z", "ttft_ms": 800, "duration_ms": 4200},
        "error": null
    })
}

fn reader(engine: &Arc<Engine>) -> (ProtocolReader, Arc<Activity>) {
    let activity = Arc::new(Activity::default());
    let mut reader = ProtocolReader::new(engine.clone(), None, activity.clone());
    // Nothing is held back by the one-second rule: the activity decides.
    reader.calls = Calls::new(Duration::ZERO);
    (reader, activity)
}

/// What the protocol's own examples say, read as the readers read them.
#[test]
fn the_protocols_examples_are_read() {
    let list: CallList = serde_json::from_value(json!({
        "session_id": "s", "recording": {"name": "s", "created_at": "2025-11-15T09:46:40Z", "covers": "latest_run", "generation": 1763200000000u64},
        "calls": [{"index": 0, "session_id": "s", "parent_session_id": null, "agent_type": null, "turn_id": "t", "turn": 1, "step": 0,
            "purpose": null, "ts": "2025-11-15T09:46:40Z", "provider": "anthropic", "model": "m",
            "input": {"system": [{"source": "identity", "chars": 1234}], "tools": 23, "messages": 8},
            "output": {"chars": 456, "thinking_chars": 0, "tool_uses": [{"id": "x", "name": "Read"}]},
            "status": "ok", "stop_reason": "tool_use",
            "usage": {"input_tokens": 10, "output_tokens": 20, "cache_creation_input_tokens": 0, "cache_read_input_tokens": 0},
            "timing": {"sent_at": "2025-11-15T09:46:40Z", "ttft_ms": 800, "duration_ms": 4200}, "error": null}],
        "next": 1, "total": 1, "damaged": 0
    }))
    .unwrap();
    let run = list.recording.unwrap();
    assert_eq!((run.created_at.as_str(), run.generation), ("2025-11-15T09:46:40Z", 1763200000000));
    let row = &list.calls[0];
    assert_eq!((row.input.tools, row.input.messages, row.output.tool_uses[0].name.as_str()), (Some(23), 8, "Read"));
    assert_eq!(row.input.system[0].chars, Some(1234));
    assert_eq!(row.timing.ttft_ms, Some(800));

    let detail: Detail = serde_json::from_value(json!({
        "session_id": "s", "index": 0, "generation": 1,
        "request": {"session_id": "s", "turn": 1, "step": 0, "purpose": null, "provider": "anthropic", "model": "m", "max_tokens": 8192,
            "thinking_mode": "off", "effort": null, "system": [{"blob": "s.0123456789abcdef", "bytes": 70000}],
            "tools": [{"name": "Read"}], "messages": [{"role": "user", "content": [{"type": "text", "text": "hi"}]}]},
        "response": {"status": "ok", "text": "t", "thinking_chars": 0, "tool_uses": [{"id": "x", "name": "Read", "input": {"path": "/x"}}],
            "stop_reason": "tool_use", "usage": {}, "ttft_ms": 800, "duration_ms": 4200, "error": null}
    }))
    .unwrap();
    assert_eq!((detail.request.max_tokens, detail.response.tool_uses.len()), (8192, 1));

    let found: SearchAnswer = serde_json::from_value(json!({
        "hits": [{"session_id": "s", "name": null, "seq": 12, "origin": "user", "ts": "2025-11-15T09:46:40Z", "snippet": "…postgres…"}],
        "scanned": 37, "truncated": false
    }))
    .unwrap();
    assert_eq!((found.hits[0].seq, found.scanned), (12, 37));
}

/// 1200 calls come in three pages; the next ask, once the session moved, goes on from where the
/// last one stopped; a call that still streams is read again, and a session nothing was seen of
/// is not asked about again.
#[tokio::test]
async fn pages_are_read_in_turn_and_only_what_moved_is_read_on() {
    let engine = Engine::new(&["S"]);
    *lock(&engine.rows) = (0..1200).map(|i| row(i, "S", None, i as u32 / 10, None)).collect();
    let (reader, activity) = reader(&engine);
    let trace = reader.calls("S").await.unwrap();
    assert_eq!(trace.calls.len(), 1200);
    let froms: Vec<u64> = engine.asked("session.calls").iter().map(|p| p["from"].as_u64().unwrap()).collect();
    assert_eq!(froms, [0, 500, 1000]);
    assert_eq!(trace.run.as_ref().map(|r| r.created_at.as_str()), Some("2025-11-15T09:46:40Z"));

    reader.calls("S").await.unwrap();
    assert_eq!(engine.asked("session.calls").len(), 3, "nothing was seen of it: not asked again");

    // The session runs: a call is appended, and the next ask reads from where it stopped.
    lock(&engine.rows).push(row(1200, "S", None, 121, None));
    activity.saw("S", Stir::TurnStarted);
    assert_eq!(reader.calls("S").await.unwrap().calls.len(), 1201);
    let asked = engine.asked("session.calls");
    assert_eq!(asked.len(), 4);
    assert_eq!(asked[3]["from"], 1200);
    assert_eq!(asked[3]["generation"], 1);
}

#[tokio::test]
async fn a_call_still_streaming_is_read_again_and_does_not_end_the_loop() {
    let engine = Engine::new(&["S"]);
    let mut open = row(1, "S", None, 0, None);
    open["status"] = json!("in_flight");
    *lock(&engine.rows) = vec![row(0, "S", None, 0, None), open];
    let (reader, activity) = reader(&engine);
    activity.saw("S", Stir::TurnStarted);
    let first = reader.calls("S").await.unwrap();
    assert_eq!(first.calls[1].status, "in_flight");
    // It ended meanwhile.
    lock(&engine.rows)[1] = row(1, "S", None, 0, None);
    let second = reader.calls("S").await.unwrap();
    assert_eq!((second.calls.len(), second.calls[1].status.as_str()), (2, "ok"), "overwritten by index");
    let froms: Vec<u64> = engine.asked("session.calls").iter().map(|p| p["from"].as_u64().unwrap()).collect();
    // (The first ask reads twice: once to find the root, with nothing held back by the clock here.)
    assert_eq!(froms, [0, 1, 1, 1], "the open call is read again from where it began");
}

/// The recording rewritten under the reader: what it held is dropped, it reads from the start,
/// and says an earlier run went. Twice in a row is an error, not a loop.
#[tokio::test]
async fn a_rewritten_recording_is_read_from_the_start() {
    let engine = Engine::new(&["S"]);
    *lock(&engine.rows) = (0..3).map(|i| row(i, "S", None, 0, None)).collect();
    let (reader, activity) = reader(&engine);
    assert!(!reader.calls("S").await.unwrap().run.unwrap().earlier_run);
    *lock(&engine.generation) = 2;
    *lock(&engine.rows) = vec![row(0, "S", None, 0, None)];
    activity.saw("S", Stir::TurnStarted);
    let trace = reader.calls("S").await.unwrap();
    assert_eq!(trace.calls.len(), 1);
    let run = trace.run.unwrap();
    assert_eq!((run.generation, run.earlier_run), (2, true));
    let json = serde_json::to_value(&run).unwrap();
    assert_eq!(json["earlier_run"], true);
    let froms: Vec<u64> = engine.asked("session.calls").iter().map(|p| p["from"].as_u64().unwrap()).collect();
    assert_eq!(froms, [0, 3, 0]);

    // An engine that refuses the generation it just gave, every time.
    struct Fickle;
    #[async_trait::async_trait]
    impl Rpc for Fickle {
        async fn call(&self, _: &str, params: Value) -> Result<Value, RpcError> {
            if params.get("generation").is_some() {
                return Err(RpcError { code: codes::INVALID_PARAMS, message: "again".into(), data: Some(json!({"generation": 9})) });
            }
            Ok(json!({"recording": {"created_at": "", "generation": 1}, "calls": [row(0, "S", None, 0, None)], "next": 1, "total": 2}))
        }
    }
    let reader = ProtocolReader::new(Arc::new(Fickle), None, Arc::new(Activity::default()));
    let err = reader.calls("S").await.unwrap_err();
    assert!(matches!(err, ReadError::Unreadable(_)), "started over once, then an error rather than a loop: {err:?}");
}

#[tokio::test]
async fn a_session_that_never_called_the_model_is_no_error() {
    let engine = Engine::new(&[]);
    let (reader, _) = reader(&engine);
    let trace = reader.calls("S").await.unwrap();
    assert!(!trace.recording && trace.calls.is_empty() && trace.run.is_none());
    assert!(!reader.turns("S").await.unwrap().recording);
    assert!(!reader.envelopes("S").await.unwrap().recording);
}

/// A sub-agent's id has no recording of its own: its root's is read, and only its own calls come
/// back; the root's list has none of the sub-agent's; `index` is the place in the recording.
#[tokio::test]
async fn a_sub_agent_is_read_from_its_roots_recording() {
    let engine = Engine::new(&["S"]);
    *lock(&engine.parents) = vec![("A".into(), "S".into())];
    *lock(&engine.rows) = vec![row(0, "S", None, 0, None), row(1, "A", Some("S"), 0, None), row(2, "S", None, 1, None)];
    let (reader, activity) = reader(&engine);
    let root = reader.calls("S").await.unwrap();
    assert_eq!(root.calls.iter().map(|c| c.index).collect::<Vec<_>>(), [0, 2], "the index has a gap where the sub-agent called");
    assert!(root.calls.iter().all(|c| c.session_id == "S"));
    let before = engine.asked("session.get").len();
    let sub = reader.calls("A").await.unwrap();
    assert_eq!(sub.calls.iter().map(|c| (c.index, c.parent_session_id.clone())).collect::<Vec<_>>(), [(1, Some("S".to_string()))]);
    assert_eq!(engine.asked("session.get").len(), before + 1);
    activity.saw("A", Stir::Other);
    reader.calls("A").await.unwrap();
    assert_eq!(engine.asked("session.get").len(), before + 1, "its root is remembered");
    assert_eq!(reader.turns("A").await.unwrap().turns.len(), 1);
}

/// The turn summing of the calls, from rows: a turn with two calls and a tool between them, and
/// an auxiliary call beside it.
#[test]
fn rows_are_summed_per_turn_and_timed() {
    let at = |s: u64| format!("2025-11-15T09:46:{:02}Z", 40 + s);
    let make = |index: usize, turn: u32, purpose: Option<&str>, sent: u64, ttft: u64, ms: u64, usage: Value| {
        let mut r = row(index, "S", None, turn, purpose);
        r["timing"] = json!({"sent_at": at(sent), "ttft_ms": ttft, "duration_ms": ms});
        r["ts"] = json!(at(sent));
        r["usage"] = usage;
        r
    };
    let usage = |input: u64, read: u64, output: u64| json!({"input_tokens": input, "cache_read_input_tokens": read, "output_tokens": output});
    let engine_rows = vec![
        make(0, 0, None, 0, 300, 600, usage(100, 900, 100)),
        make(1, 0, None, 2, 50, 250, usage(50, 1050, 20)),
        make(2, 0, Some("memory"), 3, 0, 10, usage(5, 0, 1)),
        make(3, 1, None, 8, 40, 100, usage(10, 1200, 5)),
    ];
    let mut kept = super::calls::Kept::default();
    kept.run = Some(super::wire::RunInfo { created_at: "2025-11-15T09:46:40Z".into(), generation: 1 });
    kept.rows = engine_rows.into_iter().map(|r| serde_json::from_value(r).unwrap()).collect();
    let t = trace::trace(&kept, "S");
    let first = &t.calls[0];
    let timing = first.timing.as_ref().unwrap();
    assert_eq!((timing.ttft_ms, timing.generate_ms, timing.tokens_per_s), (Some(300), Some(300), Some(333.3)), "generate is the end less the first chunk");
    assert_eq!(first.ts, 1_763_200_000_000);
    let turns: Vec<Value> = trace::turns(&kept, "S").turns.iter().map(|t| serde_json::to_value(t).unwrap()).collect();
    assert_eq!(turns.len(), 2);
    let first = &turns[0];
    assert_eq!(first["calls"], json!([0, 1]));
    assert_eq!(first["auxiliary"], 1, "the memory call is beside the turn, not in it");
    assert_eq!(first["model_ms"], 850);
    assert_eq!(first["tool_ms"], 1400, "from the first call's end (0.6s) to the second's start (2s)");
    assert_eq!(first["ttft_ms"], 300);
    assert_eq!(first["input_tokens"], 2100);
    assert_eq!(first["output_tokens"], 120);
    assert_eq!(first["cache_hit"], 0.929, "1950 of 2100 tokens in came from the cache");
    assert_eq!(first["context_tokens"], 1100, "what the last call carried");
    assert!(first["context_delta"].is_null(), "nothing before the first turn");
    assert_eq!(turns[1]["context_delta"], 110);
    assert_eq!(first["duration_ms"], 2250);
}

/// A tree's rows summed per Agent: the session first, a sub-agent with its parent, errors counting
/// cancellations; asking for the sub-agent gets it alone.
#[tokio::test]
async fn usage_is_summed_per_agent_with_the_session_first() {
    let engine = Engine::new(&["S"]);
    *lock(&engine.parents) = vec![("A".into(), "S".into())];
    let mut a = row(0, "A", Some("S"), 0, None);
    a["model"] = json!("m-small");
    a["usage"] = json!({"input_tokens": 10, "output_tokens": 2});
    let mut s1 = row(1, "S", None, 0, None);
    s1["model"] = json!("m-big");
    s1["usage"] = json!({"input_tokens": 5, "cache_read_input_tokens": 95, "output_tokens": 7});
    let mut s2 = row(2, "S", None, 1, None);
    s2["status"] = json!("cancelled");
    s2["model"] = json!("m-big");
    s2["usage"] = Value::Null;
    s2["timing"]["duration_ms"] = json!(50);
    let mut title = row(3, "S", None, 1, Some("title"));
    title["model"] = json!("m-small");
    title["usage"] = Value::Null;
    *lock(&engine.rows) = vec![a, s1, s2, title];
    let (reader, _) = reader(&engine);
    let usage = reader.usage(&["S".into(), "A".into()]).await.unwrap();
    let s = &usage["S"];
    assert!(s.recording);
    assert_eq!(s.agents.iter().map(|a| a.session_id.as_str()).collect::<Vec<_>>(), ["S", "A"]);
    let own = &s.agents[0];
    assert_eq!((own.calls, own.auxiliary, own.turns), (2, 1, 2));
    assert_eq!(own.models, ["m-big", "m-small"]);
    assert_eq!((own.input_tokens, own.cache_read_tokens, own.output_tokens), (5, 95, 7));
    assert_eq!((own.model_ms, own.errors), (4200 * 2 + 50, 1));
    assert!(own.first_at.is_some() && own.last_at >= own.first_at);
    let sub = &s.agents[1];
    assert_eq!((sub.parent_session_id.as_deref(), sub.calls, sub.output_tokens), (Some("S"), 1, 2));
    assert_eq!(usage["A"].agents.iter().map(|a| a.session_id.as_str()).collect::<Vec<_>>(), ["A"]);
    assert_eq!(s.run.as_ref().map(|r| r.generation), Some(1));
    assert!(!s.stale);
}

/// A gate that sleeps or not, counting wakes.
struct Gate {
    asleep: Mutex<bool>,
    woken: Mutex<u32>,
}

#[async_trait::async_trait]
impl WakeGate for Gate {
    async fn awake(&self) -> Result<(), RpcError> {
        if std::mem::take(&mut *lock(&self.asleep)) {
            *lock(&self.woken) += 1;
        }
        Ok(())
    }
    fn asleep_list(&self) -> Option<Vec<SessionInfo>> {
        (*lock(&self.asleep)).then(Vec::new)
    }
    fn listed(&self, _: &[SessionInfo]) {}
}

/// A sleeping instance is woken for a trace and not for usage, which gives what it read before,
/// marked stale — and a recording of none when it read nothing.
#[tokio::test]
async fn usage_wakes_nothing_and_a_trace_does() {
    let engine = Engine::new(&["S"]);
    *lock(&engine.rows) = vec![row(0, "S", None, 0, None)];
    let gate = Arc::new(Gate { asleep: Mutex::new(false), woken: Mutex::new(0) });
    let activity = Arc::new(Activity::default());
    let mut reader = ProtocolReader::new(engine.clone(), Some(gate.clone()), activity.clone());
    reader.calls = Calls::new(Duration::ZERO);

    *lock(&gate.asleep) = true;
    let cold = reader.usage(&["S".into()]).await.unwrap();
    assert!(!cold["S"].recording && !cold["S"].stale, "nothing read before");
    assert!(engine.asked("session.calls").is_empty());
    assert_eq!(*lock(&gate.woken), 0);

    *lock(&gate.asleep) = false;
    let awake = reader.usage(&["S".into()]).await.unwrap();
    assert!(awake["S"].recording && !awake["S"].stale);
    let reads = engine.asked("session.calls").len();

    *lock(&gate.asleep) = true;
    activity.saw("S", Stir::TurnStarted);
    let asleep = reader.usage(&["S".into()]).await.unwrap();
    assert!(asleep["S"].recording && asleep["S"].stale, "what was read before");
    assert_eq!(engine.asked("session.calls").len(), reads);
    assert_eq!(*lock(&gate.woken), 0);

    reader.calls("S").await.unwrap();
    assert_eq!(*lock(&gate.woken), 1, "a trace wakes it");
}

#[tokio::test]
async fn an_instance_that_is_down_is_unavailable_not_empty() {
    let engine = Engine::new(&["S"]);
    *lock(&engine.down) = true;
    let (reader, _) = reader(&engine);
    assert!(matches!(reader.calls("S").await, Err(ReadError::Unavailable(_))));
    assert!(matches!(reader.usage(&["S".into()]).await, Err(ReadError::Unavailable(_))));
    assert!(matches!(reader.search(&["S".into()], "x", &SearchLimits::default()).await, Err(ReadError::Unavailable(_))));
}

/// Two conversation calls that look alike are one envelope; one whose system block changed length
/// is another; an auxiliary call is none; asking again takes nothing from the engine.
#[tokio::test]
async fn an_envelope_is_taken_in_full_only_where_the_rows_say_it_changed() {
    let engine = Engine::new(&["S"]);
    let mut changed = row(2, "S", None, 1, None);
    changed["input"] = json!({"system": [{"source": "identity", "chars": 99}], "tools": 3, "messages": 2});
    *lock(&engine.rows) = vec![row(0, "S", None, 0, None), row(1, "S", None, 0, None), changed, row(3, "S", None, 1, Some("title"))];
    let detail = json!({"request": {"turn": 0, "step": 0, "provider": "anthropic", "model": "m", "max_tokens": 100, "thinking_mode": "off",
        "system": [{"role": "base", "cache": null, "source": "identity", "origin": "kernel", "text": "you"}], "tools": [{"name": "Read"}], "messages": []},
        "response": {}});
    *lock(&engine.details) = vec![(0, detail.clone()), (2, detail)];
    let (reader, _) = reader(&engine);
    let e = reader.envelopes("S").await.unwrap();
    assert_eq!((e.calls, e.auxiliary), (3, 1));
    assert_eq!(e.headers.iter().map(|h| h.reason.as_str()).collect::<Vec<_>>(), ["initial", "changed"]);
    assert_eq!(e.headers[0].system[0].text, "you");
    assert_eq!(engine.asked("session.call").len(), 2);
    reader.envelopes("S").await.unwrap();
    assert_eq!(engine.asked("session.call").len(), 2, "kept by index");
}

/// A reference in a system block is fetched, one the recording lost is a placeholder; a tool's
/// result is found in the last message of the session's next conversation call.
#[tokio::test]
async fn a_call_in_full_has_its_references_fetched_and_its_tool_results() {
    let engine = Engine::new(&["S", "other"]);
    let mut first = row(0, "S", None, 0, None);
    first["output"]["tool_uses"] = json!([{"id": "u1", "name": "Read"}, {"id": "u2", "name": "Bash"}]);
    *lock(&engine.rows) = vec![first, row(1, "S", None, 0, Some("title")), row(2, "S", None, 0, None)];
    *lock(&engine.blobs) = vec![("s.0123456789abcdef".into(), json!({"role": "base", "cache": null, "source": "identity", "origin": "plugin:p1", "text": "from a blob"}))];
    *lock(&engine.details) = vec![
        (0, json!({
            "request": {"session_id": "S", "turn": 0, "step": 0, "provider": "anthropic", "model": "m", "max_tokens": 100, "thinking_mode": "off", "effort": "high",
                "system": [{"blob": "s.0123456789abcdef", "bytes": 1}, {"missing": "s.dead"}],
                "tools": {"blob": "t.aaaaaaaaaaaaaaaa", "bytes": 99999999999u64},
                "messages": [{"missing": "m.1"}, {"role": "user", "content": []}]},
            "response": {"status": "ok", "text": "hi", "thinking_chars": 0, "stop_reason": "tool_use", "duration_ms": 5,
                "tool_uses": [{"id": "u1", "name": "Read", "input": {}}, {"id": "u2", "name": "Bash", "input": {}}]}})),
        (2, json!({
            "request": {"session_id": "S", "turn": 0, "step": 1, "provider": "anthropic", "model": "m", "max_tokens": 100, "thinking_mode": "off",
                "system": [], "tools": [], "messages": [
                    {"role": "user", "content": [{"type": "text", "text": "go"}]},
                    {"role": "user", "content": [
                        {"type": "tool_result", "tool_use_id": "u1", "content": "file text"},
                        {"type": "tool_result", "tool_use_id": "u2", "content": [{"type": "text", "text": "a"}, {"type": "text", "text": "b"}], "is_error": true}]}]},
            "response": {}})),
    ];
    let (reader, _) = reader(&engine);
    let detail = reader.call("S", 0).await.unwrap();
    assert_eq!(detail.input.system[0].text, "from a blob");
    assert_eq!(detail.input.system[0].origin, json!("plugin:p1"));
    assert_eq!(detail.input.system[1].text, "<missing blob s.dead>");
    assert!(detail.input.tools.is_empty(), "a table too large to fetch, or not a list, is empty here");
    assert_eq!(detail.input.messages[0], json!({"role": "user", "content": []}));
    assert_eq!(detail.input.effort, json!("high"));
    let results = &detail.output.tool_results;
    assert_eq!(results.iter().map(|r| (r.id.as_str(), r.text.as_str(), r.is_error)).collect::<Vec<_>>(), [("u1", "file text", false), ("u2", "a b", true)]);
    assert_eq!(engine.asked("session.call").iter().map(|p| p["index"].as_u64().unwrap()).collect::<Vec<_>>(), [0, 2], "the title call is skipped");

    // A call of another session of the tree, or one that is not there, is missing.
    assert!(matches!(reader.call("S", 9).await, Err(ReadError::Missing(_))));
    *lock(&engine.rows) = vec![row(0, "A", Some("S"), 0, None)];
    *lock(&engine.parents) = vec![("A".into(), "S".into())];
    let (fresh, _) = self::reader(&engine);
    assert!(matches!(fresh.call("S", 0).await, Err(ReadError::Missing(_))), "call 0 is the sub-agent's");
}

#[tokio::test]
async fn a_search_is_clamped_asked_and_read() {
    let engine = Engine::new(&[]);
    let (reader, _) = reader(&engine);
    let sessions: Vec<String> = (0..300).map(|n| format!("s{n}")).collect();
    let mut limits = SearchLimits::default();
    limits.snippet = 9000;
    limits.deadline = Duration::from_secs(60);
    let found = reader.search(&sessions, "  needle ", &limits).await.unwrap();
    let asked = &engine.asked("session.search")[0];
    assert_eq!((asked["query"].as_str(), asked["snippet_chars"].as_u64(), asked["deadline_ms"].as_u64()), (Some("needle"), Some(500), Some(10_000)));
    assert_eq!(asked["session_ids"].as_array().unwrap().len(), 200);
    assert_eq!(found.hits.iter().map(|h| (h.role.as_str(), h.seq)).collect::<Vec<_>>(), [("user", Some(12)), ("assistant", Some(3))]);
    assert!(found.truncated, "more sessions than it was to look at");
    let none = reader.search(&sessions, "   ", &limits).await.unwrap();
    assert!(none.hits.is_empty());
    assert_eq!(engine.asked("session.search").len(), 1, "an empty query is not sent");
}
