//! `harnix.system.usage`: what each Agent of a session has used, read off its recording.
//!
//! A session's recording holds every model call its Agent made, and its sub-agents' too — each call
//! names the session it was made for and that session's parent — so one pass over `calls.jsonl`
//! gives the whole tree: per Agent, its calls and turns, the models it used, its tokens and the
//! time the model took. Only the call and end lines are read, never the blobs, and the answer is
//! kept until the file changes: the system space asks for many sessions at once, again and again.
//!
//! Like the trace, it covers the session's current run (see `recordings.rs`).

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use harnix_contract::RpcError;
use serde_json::{json, Value};

use crate::Builtin;

/// How many sessions one call may ask about.
const MAX_SESSIONS: usize = 200;

/// One recording's summary, and the size and time of the file it was read from.
pub(crate) type UsageCache = HashMap<PathBuf, (u64, SystemTime, Value)>;

#[derive(Default)]
struct Agent {
    parent: Option<String>,
    calls: u64,
    auxiliary: u64,
    turns: BTreeSet<u64>,
    models: BTreeSet<String>,
    input: u64,
    cache_read: u64,
    cache_write: u64,
    output: u64,
    model_ms: u64,
    errors: u64,
    first_at: Option<u64>,
    last_at: Option<u64>,
}

impl Builtin {
    /// `harnix.system.usage {instance?, session_ids}` → `{sessions: {<id>: {recording, agents}}}`;
    /// `agents` is the session's own figures first, then each sub-agent's with its parent.
    pub(crate) async fn usage(&self, params: Value) -> Result<Value, RpcError> {
        let instance = params
            .get("instance")
            .and_then(Value::as_str)
            .unwrap_or(harnix_contract::SYSTEM_INSTANCE);
        let ids: Vec<&str> = params
            .get("session_ids")
            .and_then(Value::as_array)
            .ok_or_else(|| RpcError::invalid_params("missing session_ids"))?
            .iter()
            .filter_map(Value::as_str)
            .collect();
        if ids.len() > MAX_SESSIONS {
            return Err(RpcError::invalid_params(format!(
                "at most {MAX_SESSIONS} sessions at once, not {}",
                ids.len()
            )));
        }
        let mut out = serde_json::Map::new();
        for id in ids {
            let dir = self.recording_dir(instance, id)?;
            out.insert(id.to_string(), self.usage_of(&dir, id));
        }
        Ok(json!({"instance": instance, "sessions": out}))
    }

    fn usage_of(&self, dir: &Path, session_id: &str) -> Value {
        let file = dir.join("calls.jsonl");
        let Some((len, modified)) = std::fs::metadata(&file)
            .ok()
            .map(|m| (m.len(), m.modified().unwrap_or(SystemTime::UNIX_EPOCH)))
        else {
            return json!({"recording": false, "agents": []});
        };
        let mut cache = self.usage.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((l, m, value)) = cache.get(&file) {
            if *l == len && *m == modified {
                return value.clone();
            }
        }
        let value = std::fs::read_to_string(&file)
            .map(|text| summary(&text, session_id))
            .unwrap_or_else(|_| json!({"recording": false, "agents": []}));
        cache.insert(file, (len, modified, value.clone()));
        value
    }
}

/// The recording's calls summed per Agent, the session's own first.
fn summary(text: &str, root: &str) -> Value {
    let mut agents: HashMap<String, Agent> = HashMap::new();
    let mut order: Vec<String> = Vec::new();
    // seq of a call → the Agent it was made for.
    let mut owner: HashMap<u64, String> = HashMap::new();
    for line in text.lines() {
        let Ok(record) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        match record["type"].as_str() {
            Some("call") => {
                let id = record["session_id"].as_str().unwrap_or(root).to_string();
                if !agents.contains_key(&id) {
                    order.push(id.clone());
                }
                let agent = agents.entry(id.clone()).or_default();
                if agent.parent.is_none() {
                    agent.parent = record["parent_session_id"].as_str().map(str::to_string);
                }
                if record["purpose"].is_null() {
                    agent.calls += 1;
                    if let Some(turn) = record["turn"].as_u64() {
                        agent.turns.insert(turn);
                    }
                } else {
                    agent.auxiliary += 1;
                }
                if let Some(model) = record["params"]["model"].as_str() {
                    agent.models.insert(model.to_string());
                }
                if let Some(ts) = record["ts"].as_u64() {
                    agent.first_at = Some(agent.first_at.map_or(ts, |t| t.min(ts)));
                    agent.last_at = Some(agent.last_at.map_or(ts, |t| t.max(ts)));
                }
                if let Some(seq) = record["seq"].as_u64() {
                    owner.insert(seq, id);
                }
            }
            Some("end") => {
                let Some(agent) = record["call"]
                    .as_u64()
                    .and_then(|call| owner.get(&call))
                    .and_then(|id| agents.get_mut(id))
                else {
                    continue;
                };
                let usage = &record["usage"];
                let n = |k: &str| usage[k].as_u64().unwrap_or(0);
                agent.input += n("input_tokens");
                agent.cache_read += n("cache_read_input_tokens");
                agent.cache_write += n("cache_creation_input_tokens");
                agent.output += n("output_tokens");
                agent.model_ms += record["duration_ms"].as_u64().unwrap_or(0);
                if record["outcome"]["status"].as_str().is_some_and(|s| s != "ok") {
                    agent.errors += 1;
                }
                if let Some(ts) = record["ts"].as_u64() {
                    agent.last_at = Some(agent.last_at.map_or(ts, |t| t.max(ts)));
                }
            }
            _ => {}
        }
    }
    // The session's own figures first, even when a sub-agent happened to call first.
    order.sort_by_key(|id| id != root);
    let agents: Vec<Value> = order
        .iter()
        .map(|id| {
            let a = &agents[id];
            json!({
                "session_id": id,
                "parent_session_id": a.parent,
                "calls": a.calls,
                "auxiliary": a.auxiliary,
                "turns": a.turns.len(),
                "models": a.models,
                "input_tokens": a.input,
                "cache_read_tokens": a.cache_read,
                "cache_write_tokens": a.cache_write,
                "output_tokens": a.output,
                "model_ms": a.model_ms,
                "errors": a.errors,
                "first_at": a.first_at,
                "last_at": a.last_at,
            })
        })
        .collect();
    json!({"recording": true, "agents": agents})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_recording_is_summed_per_agent_with_the_session_first() {
        let text = [
            r#"{"type":"recording","version":1,"session_id":"S"}"#,
            r#"{"type":"call","seq":1,"ts":100,"session_id":"A","parent_session_id":"S","turn":0,"params":{"model":"m-small"}}"#,
            r#"{"type":"end","seq":2,"ts":150,"call":1,"outcome":{"status":"ok"},"usage":{"input_tokens":10,"output_tokens":2},"duration_ms":50}"#,
            r#"{"type":"call","seq":3,"ts":200,"session_id":"S","turn":0,"params":{"model":"m-big"}}"#,
            r#"{"type":"end","seq":4,"ts":300,"call":3,"outcome":{"status":"ok"},"usage":{"input_tokens":5,"cache_read_input_tokens":95,"output_tokens":7},"duration_ms":100}"#,
            r#"{"type":"call","seq":5,"ts":400,"session_id":"S","turn":1,"params":{"model":"m-big"}}"#,
            r#"{"type":"end","seq":6,"ts":450,"call":5,"outcome":{"status":"error"},"usage":{},"duration_ms":50}"#,
            r#"{"type":"call","seq":7,"ts":460,"session_id":"S","turn":1,"purpose":"title","params":{"model":"m-small"}}"#,
            "not json",
        ]
        .join("\n");
        let v = summary(&text, "S");
        let agents = v["agents"].as_array().unwrap();
        assert_eq!(agents.len(), 2);
        let s = &agents[0];
        assert_eq!(s["session_id"], "S");
        assert_eq!(s["parent_session_id"], Value::Null);
        assert_eq!((s["calls"].as_u64(), s["auxiliary"].as_u64(), s["turns"].as_u64()), (Some(2), Some(1), Some(2)));
        assert_eq!(s["models"], json!(["m-big", "m-small"]));
        assert_eq!((s["input_tokens"].as_u64(), s["cache_read_tokens"].as_u64(), s["output_tokens"].as_u64()), (Some(5), Some(95), Some(7)));
        assert_eq!((s["model_ms"].as_u64(), s["errors"].as_u64()), (Some(150), Some(1)));
        assert_eq!((s["first_at"].as_u64(), s["last_at"].as_u64()), (Some(200), Some(460)));
        let a = &agents[1];
        assert_eq!((a["session_id"].as_str(), a["parent_session_id"].as_str()), (Some("A"), Some("S")));
        assert_eq!((a["calls"].as_u64(), a["output_tokens"].as_u64()), (Some(1), Some(2)));
    }
}
