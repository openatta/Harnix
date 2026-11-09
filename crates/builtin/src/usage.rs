//! `harnix.system.usage`: what each Agent of a session has used, read off its recording.
//!
//! A session's recording holds every model call its Agent made, and its sub-agents' too — each call
//! names the session it was made for and that session's parent — so one pass over `calls.jsonl`
//! gives the whole tree: per Agent, its calls and turns, the models it used, its tokens and the
//! time the model took. Only the call and end lines are parsed, never the chunks or the blobs.
//!
//! The system space asks for many sessions at once, again and again, and an active session's file
//! changes between every two asks. So what was folded is kept with the offset it reached, and the
//! next ask reads only what was appended since; a file that shrank (the writer truncates when a
//! resumed session makes its first call) is read again from the start. The reading is done on the
//! blocking pool, and at most [`CACHED`] recordings are kept.
//!
//! Like the trace, it covers the session's current run (see `recordings.rs`).

use std::collections::{BTreeSet, HashMap};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use harnix_contract::sync::lock;
use harnix_contract::RpcError;
use serde_json::{json, Value};

use crate::Builtin;

/// How many sessions one call may ask about.
const MAX_SESSIONS: usize = 200;

/// How many recordings' folds are kept. Past it, the one asked about longest ago goes.
const CACHED: usize = 256;

/// Each recording's fold, by its `calls.jsonl`.
#[derive(Default)]
pub(crate) struct UsageCache {
    by_file: HashMap<PathBuf, Tail>,
    /// Bumped on every ask: which fold was asked about longest ago.
    clock: u64,
}

/// One recording read up to `offset`, and the size and time of the file when it was.
#[derive(Clone, Default)]
struct Tail {
    len: u64,
    modified: Option<SystemTime>,
    offset: u64,
    fold: Fold,
    asked: u64,
}

/// What the lines read so far add up to.
#[derive(Clone, Default)]
struct Fold {
    agents: HashMap<String, Agent>,
    order: Vec<String>,
    /// seq of a call → the Agent it was made for.
    owner: HashMap<u64, String>,
}

#[derive(Clone, Default)]
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
        let ids: Vec<String> = params
            .get("session_ids")
            .and_then(Value::as_array)
            .ok_or_else(|| RpcError::invalid_params("missing session_ids"))?
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect();
        if ids.len() > MAX_SESSIONS {
            return Err(RpcError::invalid_params(format!(
                "at most {MAX_SESSIONS} sessions at once, not {}",
                ids.len()
            )));
        }
        let mut dirs = Vec::with_capacity(ids.len());
        for id in ids {
            dirs.push((self.recording_dir(instance, &id)?, id));
        }
        let cache = self.usage.clone();
        let out = tokio::task::spawn_blocking(move || {
            let mut out = serde_json::Map::new();
            for (dir, id) in dirs {
                let value = usage_of(&cache, &dir, &id);
                out.insert(id, value);
            }
            out
        })
        .await
        .map_err(|e| RpcError::new(harnix_contract::codes::INTERNAL_ERROR, e.to_string()))?;
        Ok(json!({"instance": instance, "sessions": out}))
    }
}

/// One recording's summary: from the cache when its file has not changed, else folded on from
/// where the last read stopped. The cache's lock is not held while the file is read.
fn usage_of(cache: &Arc<Mutex<UsageCache>>, dir: &Path, session_id: &str) -> Value {
    let file = dir.join("calls.jsonl");
    let Ok(meta) = std::fs::metadata(&file) else {
        lock(cache).by_file.remove(&file);
        return json!({"recording": false, "agents": []});
    };
    let (len, modified) = (meta.len(), meta.modified().ok());
    let kept = {
        let mut cache = lock(cache);
        cache.clock += 1;
        let now = cache.clock;
        cache.by_file.get_mut(&file).map(|tail| {
            tail.asked = now;
            tail.clone()
        })
    };
    let tail = match kept {
        Some(tail) if tail.len == len && tail.modified == modified => tail,
        // Only appended to: read on from where the last read stopped.
        Some(tail) if len >= tail.offset => match read_on(&file, tail) {
            Some(tail) => tail,
            None => return json!({"recording": false, "agents": []}),
        },
        // New, or shrank: from the start.
        _ => match read_on(&file, Tail::default()) {
            Some(tail) => tail,
            None => return json!({"recording": false, "agents": []}),
        },
    };
    let value = tail.fold.summary(session_id);
    let mut cache = lock(cache);
    let asked = cache.clock;
    cache.by_file.insert(file, Tail { asked, ..tail });
    if cache.by_file.len() > CACHED {
        if let Some(oldest) = cache.by_file.iter().min_by_key(|(_, t)| t.asked).map(|(f, _)| f.clone()) {
            cache.by_file.remove(&oldest);
        }
    }
    value
}

/// Fold the lines appended to `file` since `tail.offset`. A last line without its newline is
/// being written: it is left for the next read.
fn read_on(file: &Path, mut tail: Tail) -> Option<Tail> {
    let mut f = std::fs::File::open(file).ok()?;
    let meta = f.metadata().ok()?;
    f.seek(SeekFrom::Start(tail.offset)).ok()?;
    let mut bytes = Vec::new();
    f.read_to_end(&mut bytes).ok()?;
    let complete = bytes.iter().rposition(|b| *b == b'\n').map_or(0, |at| at + 1);
    let text = String::from_utf8_lossy(&bytes[..complete]);
    for line in text.lines() {
        tail.fold.line(line);
    }
    tail.offset += complete as u64;
    tail.len = meta.len();
    tail.modified = meta.modified().ok();
    Some(tail)
}

/// The recording's calls summed per Agent, the session's own first.
#[cfg(test)]
fn summary(text: &str, root: &str) -> Value {
    let mut fold = Fold::default();
    for line in text.lines() {
        fold.line(line);
    }
    fold.summary(root)
}

impl Fold {
    /// Fold one line of `calls.jsonl`. A chunk is most of the file and says nothing used here, so
    /// a line that cannot be a call or an end is not parsed at all.
    fn line(&mut self, line: &str) {
        if !line.contains(r#""type":"call""#) && !line.contains(r#""type":"end""#) {
            return;
        }
        let Ok(record) = serde_json::from_str::<Value>(line) else {
            return;
        };
        match record["type"].as_str() {
            Some("call") => {
                // A call names its own Agent; one that does not is the session's (`root`), which
                // is not known while folding: it is filed under "" and resolved in `summary`.
                let id = record["session_id"].as_str().unwrap_or("").to_string();
                if !self.agents.contains_key(&id) {
                    self.order.push(id.clone());
                }
                let agent = self.agents.entry(id.clone()).or_default();
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
                    self.owner.insert(seq, id);
                }
            }
            Some("end") => {
                let Some(agent) = record["call"]
                    .as_u64()
                    .and_then(|call| self.owner.get(&call))
                    .and_then(|id| self.agents.get_mut(id))
                else {
                    return;
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

    /// The figures per Agent, the session's own (`root`) first, even when a sub-agent happened to
    /// call first.
    fn summary(&self, root: &str) -> Value {
        let named = |id: &str| if id.is_empty() { root.to_string() } else { id.to_string() };
        // Calls that named no session and calls that named the root are the same Agent's.
        let mut merged: Vec<(String, Agent)> = Vec::new();
        for id in &self.order {
            let name = named(id);
            let a = &self.agents[id];
            match merged.iter_mut().find(|(n, _)| *n == name) {
                Some((_, into)) => into.absorb(a),
                None => merged.push((name, a.clone())),
            }
        }
        merged.sort_by_key(|(id, _)| id != root);
        let agents: Vec<Value> = merged
            .iter()
            .map(|(id, a)| {
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
}

impl Agent {
    fn absorb(&mut self, other: &Agent) {
        self.parent = self.parent.take().or_else(|| other.parent.clone());
        self.calls += other.calls;
        self.auxiliary += other.auxiliary;
        self.turns.extend(other.turns.iter().copied());
        self.models.extend(other.models.iter().cloned());
        self.input += other.input;
        self.cache_read += other.cache_read;
        self.cache_write += other.cache_write;
        self.output += other.output;
        self.model_ms += other.model_ms;
        self.errors += other.errors;
        self.first_at = match (self.first_at, other.first_at) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        self.last_at = match (self.last_at, other.last_at) {
            (Some(a), Some(b)) => Some(a.max(b)),
            (a, b) => a.or(b),
        };
    }
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

    fn write(path: &Path, lines: &[&str]) {
        let mut text = lines.join("\n");
        text.push('\n');
        std::fs::write(path, text).unwrap();
    }

    fn append(path: &Path, line: &str) {
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new().append(true).open(path).unwrap();
        f.write_all(line.as_bytes()).unwrap();
    }

    const CALL1: &str = r#"{"type":"call","seq":1,"ts":100,"session_id":"S","turn":0,"params":{"model":"m"}}"#;
    const END1: &str = r#"{"type":"end","seq":2,"ts":150,"call":1,"outcome":{"status":"ok"},"usage":{"output_tokens":2},"duration_ms":50}"#;
    const CALL2: &str = r#"{"type":"call","seq":3,"ts":200,"session_id":"S","turn":1,"params":{"model":"m"}}"#;
    const END2: &str = r#"{"type":"end","seq":4,"ts":260,"call":3,"outcome":{"status":"ok"},"usage":{"output_tokens":3},"duration_ms":60}"#;

    /// A session that is running grows its file between asks: each ask reads only what was
    /// appended, a half-written last line waits for its newline, and a file that shrank — the
    /// writer starting the run again — is read from the start.
    #[test]
    fn a_growing_recording_is_read_on_from_where_the_last_ask_stopped() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("calls.jsonl");
        let cache = Arc::new(Mutex::new(UsageCache::default()));
        let output = |v: &Value| v["agents"][0]["output_tokens"].as_u64();
        let calls = |v: &Value| v["agents"][0]["calls"].as_u64();

        write(&file, &[CALL1, END1]);
        let v = usage_of(&cache, dir.path(), "S");
        assert_eq!((calls(&v), output(&v)), (Some(1), Some(2)));
        let offset = lock(&cache).by_file[&file].offset;
        assert_eq!(offset, std::fs::metadata(&file).unwrap().len());

        // The next call's line, then half of its end.
        append(&file, &format!("{CALL2}\n{}", &END2[..20]));
        let v = usage_of(&cache, dir.path(), "S");
        assert_eq!((calls(&v), output(&v)), (Some(2), Some(2)), "the torn end is not read yet");
        append(&file, &format!("{}\n", &END2[20..]));
        let v = usage_of(&cache, dir.path(), "S");
        assert_eq!((calls(&v), output(&v)), (Some(2), Some(5)));

        // Truncated and started again.
        write(&file, &[CALL1]);
        let v = usage_of(&cache, dir.path(), "S");
        assert_eq!((calls(&v), output(&v)), (Some(1), Some(0)));

        std::fs::remove_file(&file).unwrap();
        assert_eq!(usage_of(&cache, dir.path(), "S")["recording"], false);
        assert!(lock(&cache).by_file.is_empty(), "a recording that is gone is not kept");
    }

    #[test]
    fn at_most_so_many_recordings_are_kept() {
        let root = tempfile::tempdir().unwrap();
        let cache = Arc::new(Mutex::new(UsageCache::default()));
        for n in 0..CACHED + 3 {
            let dir = root.path().join(format!("s{n}"));
            std::fs::create_dir(&dir).unwrap();
            write(&dir.join("calls.jsonl"), &[CALL1]);
            usage_of(&cache, &dir, "S");
        }
        let kept = lock(&cache);
        assert_eq!(kept.by_file.len(), CACHED);
        assert!(!kept.by_file.contains_key(&root.path().join("s0/calls.jsonl")), "the oldest went");
    }
}
