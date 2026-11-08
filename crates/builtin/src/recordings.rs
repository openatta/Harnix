//! The request envelope, read back out of the engine's recording.
//!
//! What the model was sent, minus the messages: the assembled system blocks,
//! the whole tool catalog, and the call configuration. Agix does not send
//! this over the wire — it writes every call to a recording
//! (`agix/docs/recorder_design.md`), and this crate reads that directory.
//!
//! Reading files rather than receiving frames is what makes the envelope
//! survive a reload: the browser can ask for it at any point, including for a
//! session whose turns ran in a previous process. The cost is the boundary in
//! §4 — a recording covers the current *run* of a session, because the writer
//! truncates when a resumed session makes its first call.
//!
//! Only turn calls are folded into the timeline. Compaction, memory extraction
//! and titling assemble their own envelopes, so including them would report a
//! change on every one and a change back afterwards, none of which describes
//! the conversation.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use base::interface::model::{ModelEvent, ToolDef};
use base::interface::prompt::PromptBlock;
use harnix_contract::{codes, RpcError};
use serde_json::{json, Value};
use telemetry::recorder::blob::{BlobId, BlobStore};
use telemetry::recorder::format::{CallRecord, RecordedParams};
use telemetry::recorder::reader;

use crate::{require_session_id, Builtin};

impl Builtin {
    /// Where an instance writes a session's recording.
    pub(crate) fn recording_dir(&self, instance: &str, session_id: &str) -> Result<PathBuf, RpcError> {
        let handle = self.instance(instance).ok_or_else(|| {
            RpcError::new(
                codes::INSTANCE_NOT_FOUND,
                format!("there is no instance `{instance}`"),
            )
        })?;
        Ok(handle.recordings_root.join(session_id))
    }

    /// `harnix.requestHeaders {instance?, session_id}` — the envelope timeline of this
    /// session's current run, oldest first, one entry per change.
    pub(crate) async fn request_headers(&self, params: Value) -> Result<Value, RpcError> {
        let session_id = require_session_id(&params)?;
        let instance = params
            .get("instance")
            .and_then(Value::as_str)
            .unwrap_or(harnix_contract::SYSTEM_INSTANCE);
        envelopes_in(&self.recording_dir(instance, &session_id)?)
    }
    /// `harnix.system.trace {instance?, session_id}` — every model call of this session's current
    /// run, oldest first, each with what went in and what came out, and the tool calls it asked
    /// for with their results (found in the next call's messages). The system space's calls tab.
    pub(crate) async fn trace(&self, params: Value) -> Result<Value, RpcError> {
        let session_id = require_session_id(&params)?;
        let instance = params
            .get("instance")
            .and_then(Value::as_str)
            .unwrap_or(harnix_contract::SYSTEM_INSTANCE);
        trace_in(&self.recording_dir(instance, &session_id)?)
    }

    /// `harnix.system.turns {instance?, session_id}` — the trace summed up per turn: how many calls,
    /// the time to first token, model and tool time, tokens in and out, the cache's share of what
    /// went in (by tokens, so one cold call is not hidden by warm ones), and how much the context
    /// grew since the turn before. The system space's per-turn tab.
    pub(crate) async fn turns(&self, params: Value) -> Result<Value, RpcError> {
        let session_id = require_session_id(&params)?;
        let instance = params
            .get("instance")
            .and_then(Value::as_str)
            .unwrap_or(harnix_contract::SYSTEM_INSTANCE);
        let trace = trace_in(&self.recording_dir(instance, &session_id)?)?;
        let calls = trace["calls"].as_array().cloned().unwrap_or_default();
        Ok(json!({"recording": trace["recording"], "turns": turns_of(&calls)}))
    }

    /// `harnix.system.trace.call {instance?, session_id, index}` — one call in full: every system
    /// block, the tool catalog, the messages as sent, and the whole answer.
    pub(crate) async fn trace_call(&self, params: Value) -> Result<Value, RpcError> {
        let session_id = require_session_id(&params)?;
        let instance = params
            .get("instance")
            .and_then(Value::as_str)
            .unwrap_or(harnix_contract::SYSTEM_INSTANCE);
        let index = params.get("index").and_then(Value::as_u64).ok_or_else(|| {
            RpcError::invalid_params("`index` is the call's position in the trace")
        })?;
        call_in(&self.recording_dir(instance, &session_id)?, index as usize)
    }
}

/// Clip a text for a list row; the call's detail has all of it.
fn clip(text: &str, chars: usize) -> String {
    let mut out: String = text.chars().take(chars).collect();
    if text.chars().count() > chars {
        out.push('…');
    }
    out
}

/// What a call answered: its text, how much thinking, and the tool calls it asked for.
fn answer(events: &[ModelEvent]) -> (String, usize, Vec<Value>) {
    let mut text = String::new();
    let mut thinking = 0;
    let mut tools = Vec::new();
    for event in events {
        match event {
            ModelEvent::TextDelta { text: delta } => text.push_str(delta),
            ModelEvent::ThinkingDelta { text: delta } => thinking += delta.chars().count(),
            ModelEvent::ToolUse { id, name, input } => {
                tools.push(json!({"id": id, "name": name, "input": input}))
            }
            _ => {}
        }
    }
    (text, thinking, tools)
}

/// Every `tool_result` in a call's messages, by the tool call it answers.
fn results(messages: &[Value]) -> std::collections::HashMap<String, (String, bool)> {
    let mut found = std::collections::HashMap::new();
    for message in messages {
        for block in message
            .get("content")
            .and_then(Value::as_array)
            .map_or(&[][..], |v| v)
        {
            if block.get("type").and_then(Value::as_str) == Some("tool_result") {
                let id = block
                    .get("tool_use_id")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let content = block
                    .get("content")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let error = block
                    .get("is_error")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                found.insert(id.to_string(), (content.to_string(), error));
            }
        }
    }
    found
}

fn outcome(call: &reader::RecordedCall) -> Value {
    match &call.end {
        None => json!({"status": "unfinished"}),
        Some(end) => serde_json::to_value(&end.outcome).unwrap_or(json!(null)),
    }
}

/// When a call was sent, its first and last streamed chunk, and when it ended (milliseconds).
#[derive(Debug, Default, Clone, Copy, PartialEq)]
struct Timing {
    sent: u64,
    first: Option<u64>,
    last: Option<u64>,
    end: Option<u64>,
}

/// Each call's times, in the order of its call records — read off `calls.jsonl` itself, since the
/// reader folds the chunks into events and keeps none of their times. A packed run of chunks
/// (`ts0` and the gaps `dt`) spans from its first member to its last.
fn timings(dir: &Path) -> Vec<Timing> {
    let Ok(text) = std::fs::read_to_string(dir.join("calls.jsonl")) else {
        return Vec::new();
    };
    let mut order = Vec::new();
    let mut by_call: HashMap<u64, Timing> = HashMap::new();
    for line in text.lines() {
        let Ok(record) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        match record["type"].as_str() {
            Some("recording") => {}
            Some("call") => {
                if let (Some(seq), Some(ts)) = (record["seq"].as_u64(), record["ts"].as_u64()) {
                    order.push(seq);
                    by_call.insert(seq, Timing { sent: ts, ..Timing::default() });
                }
            }
            Some("end") => {
                if let (Some(call), Some(ts)) = (record["call"].as_u64(), record["ts"].as_u64()) {
                    if let Some(timing) = by_call.get_mut(&call) {
                        timing.end = Some(ts);
                    }
                }
            }
            _ => {
                let Some(call) = record["call"].as_u64() else {
                    continue;
                };
                let span = match (record["ts0"].as_u64(), record["ts"].as_u64()) {
                    (Some(ts0), _) => {
                        let gaps: i64 = record["dt"]
                            .as_array()
                            .map(|dt| dt.iter().filter_map(Value::as_i64).sum())
                            .unwrap_or(0);
                        (ts0, ts0.saturating_add(gaps.max(0) as u64))
                    }
                    (None, Some(ts)) => (ts, ts),
                    (None, None) => continue,
                };
                if let Some(timing) = by_call.get_mut(&call) {
                    timing.first = Some(timing.first.map_or(span.0, |t| t.min(span.0)));
                    timing.last = Some(timing.last.map_or(span.1, |t| t.max(span.1)));
                }
            }
        }
    }
    order.iter().filter_map(|seq| by_call.get(seq).copied()).collect()
}

/// A call's `timing`: when it was sent, how long to its first chunk, how long it streamed, and
/// the output rate over that stretch. Absent parts are left out rather than guessed.
fn timing_json(timing: &Timing, output_tokens: Option<u64>) -> Value {
    let ttft = timing.first.map(|first| first.saturating_sub(timing.sent));
    let generate = match (timing.first, timing.last) {
        (Some(first), Some(last)) => Some(last.saturating_sub(first)),
        _ => None,
    };
    let rate = match (output_tokens, generate) {
        (Some(tokens), Some(ms)) if ms > 0 && tokens > 0 => Some((tokens as f64) * 1000.0 / ms as f64),
        _ => None,
    };
    json!({
        "sent_at": timing.sent,
        "ttft_ms": ttft,
        "generate_ms": generate,
        "tokens_per_s": rate.map(|r| (r * 10.0).round() / 10.0),
    })
}

/// Tokens that went in: uncached, read from the cache, written to it — the three do not overlap.
fn tokens_in(usage: &Value) -> u64 {
    ["input_tokens", "cache_read_input_tokens", "cache_creation_input_tokens"]
        .iter()
        .filter_map(|k| usage[*k].as_u64())
        .sum()
}

/// The trace's turn calls summed per turn, in the order the turns ran (see `Builtin::turns`).
/// Auxiliary calls (compaction, memory, titles) are counted beside a turn, not inside its figures.
fn turns_of(calls: &[Value]) -> Vec<Value> {
    let mut order: Vec<u64> = Vec::new();
    let mut by_turn: HashMap<u64, Vec<&Value>> = HashMap::new();
    let mut auxiliary: HashMap<u64, u64> = HashMap::new();
    for call in calls {
        let turn = call["turn"].as_u64().unwrap_or(0);
        if !call["purpose"].is_null() {
            *auxiliary.entry(turn).or_default() += 1;
            continue;
        }
        if !by_turn.contains_key(&turn) {
            order.push(turn);
        }
        by_turn.entry(turn).or_default().push(call);
    }
    let mut previous_context: Option<u64> = None;
    let mut out = Vec::new();
    for turn in order {
        let steps = &by_turn[&turn];
        let sent = |c: &Value| c["timing"]["sent_at"].as_u64();
        let duration = |c: &Value| c["duration_ms"].as_u64();
        let model_ms: u64 = steps.iter().filter_map(|c| duration(c)).sum();
        // Between one call's end and the next one's start, the turn ran its tools.
        let tool_ms: u64 = steps
            .windows(2)
            .filter_map(|pair| {
                let ended = sent(pair[0])? + duration(pair[0])?;
                Some(sent(pair[1])?.saturating_sub(ended))
            })
            .sum();
        let input: u64 = steps.iter().map(|c| tokens_in(&c["usage"])).sum();
        let cache_read: u64 = steps
            .iter()
            .filter_map(|c| c["usage"]["cache_read_input_tokens"].as_u64())
            .sum();
        let output: u64 = steps
            .iter()
            .filter_map(|c| c["usage"]["output_tokens"].as_u64())
            .sum();
        let generate_ms: u64 = steps
            .iter()
            .filter_map(|c| c["timing"]["generate_ms"].as_u64())
            .sum();
        let context = steps.last().map(|c| tokens_in(&c["usage"])).filter(|n| *n > 0);
        let started = steps.first().and_then(|c| sent(c));
        let ended = steps.last().and_then(|c| Some(sent(c)? + duration(c)?));
        out.push(json!({
            "turn": turn,
            "calls": steps.iter().map(|c| c["index"].clone()).collect::<Vec<_>>(),
            "auxiliary": auxiliary.get(&turn).copied().unwrap_or(0),
            "started_at": started,
            "duration_ms": started.zip(ended).map(|(s, e)| e.saturating_sub(s)),
            "ttft_ms": steps.first().map(|c| c["timing"]["ttft_ms"].clone()),
            "model_ms": model_ms,
            "tool_ms": tool_ms,
            "input_tokens": input,
            "output_tokens": output,
            "cache_hit": (input > 0).then(|| ((cache_read as f64 / input as f64) * 1000.0).round() / 1000.0),
            "tokens_per_s": (generate_ms > 0 && output > 0).then(|| ((output as f64 * 1000.0 / generate_ms as f64) * 10.0).round() / 10.0),
            "context_tokens": context,
            "context_delta": match (context, previous_context) {
                (Some(now), Some(before)) => Some(now as i64 - before as i64),
                _ => None,
            },
        }));
        if context.is_some() {
            previous_context = context;
        }
    }
    out
}

fn trace_in(dir: &std::path::Path) -> Result<Value, RpcError> {
    let recording = match reader::load(dir) {
        Ok(recording) => recording,
        Err(reader::ReadError::NotFound(_)) => return Ok(json!({"recording": false, "calls": []})),
        Err(e) => {
            return Err(RpcError::new(
                codes::INTERNAL_ERROR,
                format!("unreadable recording: {e}"),
            ))
        }
    };
    let blobs = BlobStore::new(dir);
    // Times only when every call record was found: otherwise they could be paired with the wrong call.
    let times = Some(timings(dir)).filter(|t| t.len() == recording.calls.len());
    let messages: Vec<Vec<Value>> = recording
        .calls
        .iter()
        .map(|call| {
            call.request
                .messages
                .iter()
                .filter_map(|id| read::<Value>(&blobs, id))
                .collect()
        })
        .collect();
    let mut calls = Vec::new();
    for (index, call) in recording.calls.iter().enumerate() {
        let request = &call.request;
        let (text, thinking, tool_uses) = answer(&call.response);
        // A tool call's result is in the next turn call: the engine sends it back.
        let next = recording.calls[index + 1..]
            .iter()
            .position(|c| c.request.purpose.is_none())
            .map(|offset| results(&messages[index + 1 + offset]))
            .unwrap_or_default();
        let tools: Vec<Value> = tool_uses
            .iter()
            .map(|tool| {
                let id = tool["id"].as_str().unwrap_or_default();
                let result = next.get(id);
                json!({
                    "id": id,
                    "name": tool["name"],
                    "input": clip(&tool["input"].to_string(), 160),
                    "result": result.map(|(text, _)| clip(text.lines().next().unwrap_or_default(), 160)),
                    "result_chars": result.map(|(text, _)| text.chars().count()),
                    "is_error": result.map(|(_, error)| *error),
                })
            })
            .collect();
        let system: Vec<Value> = request
            .system
            .iter()
            .map(|id| match read::<PromptBlock>(&blobs, id) {
                Some(block) => {
                    json!({"source": block.name, "chars": block.content.chars().count()})
                }
                None => json!({"source": null, "chars": 0}),
            })
            .collect();
        let tool_count = read::<Vec<ToolDef>>(&blobs, &request.tools).map_or(0, |t| t.len());
        let end = call.end.as_ref();
        let output_tokens = end.and_then(|e| e.usage.as_ref()).map(|u| u.output_tokens as u64);
        let timing = times
            .as_ref()
            .and_then(|t| t.get(index))
            .map(|t| timing_json(t, output_tokens))
            .unwrap_or(Value::Null);
        calls.push(json!({
            "index": index,
            "turn": request.turn,
            "step": request.step,
            "purpose": request.purpose,
            "ts": request.ts,
            "provider": request.provider,
            "model": request.params.model,
            "input": {
                "system": system,
                "tools": tool_count,
                "messages": request.messages.len(),
            },
            "output": {
                "text": clip(&text, 240),
                "chars": text.chars().count(),
                "thinking_chars": thinking,
                "tool_uses": tool_uses.len(),
            },
            "tools": tools,
            "outcome": outcome(call),
            "stop_reason": end.and_then(|e| e.stop_reason.clone()),
            "usage": end.and_then(|e| e.usage.as_ref()).and_then(|u| serde_json::to_value(u).ok()),
            "duration_ms": end.map(|e| e.duration_ms),
            "timing": timing,
        }));
    }
    Ok(json!({"recording": true, "calls": calls, "damaged": recording.damaged}))
}

fn call_in(dir: &std::path::Path, index: usize) -> Result<Value, RpcError> {
    let recording =
        reader::load(dir).map_err(|e| RpcError::not_found(format!("no recording: {e}")))?;
    let call = recording.calls.get(index).ok_or_else(|| {
        RpcError::not_found(format!("this run has {} calls", recording.calls.len()))
    })?;
    let blobs = BlobStore::new(dir);
    let request = &call.request;
    let envelope = envelope(request, "detail", &blobs);
    let messages: Vec<Value> = request
        .messages
        .iter()
        .filter_map(|id| read::<Value>(&blobs, id))
        .collect();
    let (text, thinking, tool_uses) = answer(&call.response);
    let end = call.end.as_ref();
    Ok(json!({
        "index": index,
        "input": {
            "system": envelope["system"],
            "tools": envelope["tools"],
            "messages": messages,
            "model": request.params.model,
            "max_tokens": request.params.max_tokens,
            "thinking_mode": request.params.thinking_mode,
            "provider": request.provider,
            "purpose": request.purpose,
            "turn": request.turn,
            "step": request.step,
        },
        "output": {
            "text": text,
            "thinking_chars": thinking,
            "tool_uses": tool_uses,
            "outcome": outcome(call),
            "stop_reason": end.and_then(|e| e.stop_reason.clone()),
            "usage": end.and_then(|e| e.usage.as_ref()).and_then(|u| serde_json::to_value(u).ok()),
            "duration_ms": end.map(|e| e.duration_ms),
        },
    }))
}

/// Fold one recording directory into the changes of its envelope.
///
/// A session that has not called the model yet has no recording at all; that
/// reports as `recording: false` rather than as an error, because it is the
/// ordinary state of a session nobody has spoken to.
fn envelopes_in(dir: &std::path::Path) -> Result<Value, RpcError> {
    let recording = match reader::load(dir) {
        Ok(recording) => recording,
        Err(reader::ReadError::NotFound(_)) => {
            return Ok(json!({
                "recording": false, "headers": [], "calls": 0,
                "auxiliary": 0, "damaged": 0,
            }))
        }
        Err(e) => {
            return Err(RpcError::new(
                codes::INTERNAL_ERROR,
                format!("unreadable recording: {e}"),
            ))
        }
    };

    let blobs = BlobStore::new(dir);
    let mut headers: Vec<Value> = Vec::new();
    let mut previous: Option<Fingerprint> = None;
    let mut turn_calls = 0u64;
    let mut auxiliary = 0u64;

    for call in &recording.calls {
        let request = &call.request;
        if request.purpose.is_some() {
            auxiliary += 1;
            continue;
        }
        turn_calls += 1;
        let fingerprint = Fingerprint::of(request);
        if previous.as_ref() == Some(&fingerprint) {
            continue;
        }
        let reason = if previous.is_none() {
            "initial"
        } else {
            "changed"
        };
        previous = Some(fingerprint);
        headers.push(envelope(request, reason, &blobs));
    }

    Ok(json!({
        "recording": true,
        "headers": headers,
        "calls": turn_calls,
        "auxiliary": auxiliary,
        "damaged": recording.damaged,
    }))
}

/// What makes two calls carry the same envelope.
///
/// Blob ids, not contents: a blob id *is* a hash of the content, so equal ids
/// mean equal bytes and the comparison costs no reads. This is the same
/// identity replay uses to detect divergence.
#[derive(PartialEq)]
struct Fingerprint {
    params: RecordedParams,
    system: Vec<String>,
    tools: String,
}

impl Fingerprint {
    fn of(request: &CallRecord) -> Self {
        Self {
            params: request.params.clone(),
            system: request.system.iter().map(|id| id.0.clone()).collect(),
            tools: request.tools.0.clone(),
        }
    }
}

/// One envelope in the shape the browser renders.
///
/// `text` and `cache` rather than `content` and `cache_strategy`: the flow row
/// and the details pane read a block, not a `PromptBlock`, and `source` is the
/// field that lets them say *which* block — the skills inventory, the memory
/// recall, a named scene section — instead of numbering them.
fn envelope(request: &CallRecord, reason: &str, blobs: &BlobStore) -> Value {
    let system: Vec<Value> = request
        .system
        .iter()
        .map(|id| match read::<PromptBlock>(blobs, id) {
            Some(block) => json!({
                "role": block.role,
                "cache": block.cache_strategy,
                // Two answers, not one. Agix splits what used to be a
                // single `source` into `name` — which block this is
                // (`skills.catalog`, `memory.session`), stable enough for an
                // extension to address — and `origin`, who put it there. The
                // details pane wants both: "what am I looking at" and "who
                // is responsible for it" are different questions.
                "source": block.name,
                "origin": block.origin,
                "text": block.content,
            }),
            None => json!({
                "role": "system", "cache": null, "source": null, "origin": null,
                "text": missing(id),
            }),
        })
        .collect();
    let tools = read::<Vec<ToolDef>>(blobs, &request.tools).unwrap_or_default();

    json!({
        "reason": reason,
        "turn": request.turn,
        "step": request.step,
        "ts": request.ts,
        "provider": request.provider,
        "model": request.params.model,
        "max_tokens": request.params.max_tokens,
        "thinking_mode": request.params.thinking_mode,
        "system": system,
        "tools": tools,
    })
}

/// A blob a recording references but does not have.
///
/// Only reachable for a hand-edited or partially copied directory, and it
/// stands in for one block rather than failing the whole read — the rest of
/// the envelope still answers most of what a reader came for.
fn missing(id: &BlobId) -> String {
    format!("<missing blob {id}>")
}

fn read<T: serde::de::DeserializeOwned>(blobs: &BlobStore, id: &BlobId) -> Option<T> {
    match blobs.get::<T>(id) {
        Ok(value) => value,
        Err(e) => {
            tracing::warn!(blob = %id, error = %e, "unreadable recording blob");
            None
        }
    }
}

/// Written against real recordings produced by the recorder's own writer,
/// rather than against hand-written JSON: the format belongs to Agix, and
/// a fixture we typed ourselves would keep passing after Agix changed it.
#[cfg(test)]
mod tests {
    use super::*;

    /// Two calls of one turn with a tool between them, and an auxiliary call after: what the
    /// timings read off the file, and what a turn sums to.
    #[test]
    fn times_are_read_off_the_file_and_summed_per_turn() {
        let dir = tempfile::tempdir().unwrap();
        let lines = [
            r#"{"type":"recording","version":1}"#,
            r#"{"type":"call","seq":10,"ts":1000,"turn":0,"step":0}"#,
            r#"{"type":"chunk","seq":11,"ts":1300,"call":10,"chunk":{}}"#,
            r#"{"type":"text_chunks","seq0":12,"ts0":1310,"call":10,"texts":["a","b","c"],"dt":[100,90]}"#,
            r#"{"type":"end","seq":15,"ts":1600,"call":10,"outcome":{"status":"ok"},"duration_ms":600}"#,
            r#"{"type":"call","seq":20,"ts":2100,"turn":0,"step":1}"#,
            r#"{"type":"chunk","seq":21,"ts":2150,"call":20,"chunk":{}}"#,
            r#"{"type":"end","seq":22,"ts":2350,"call":20,"outcome":{"status":"ok"},"duration_ms":250}"#,
            r#"{"type":"call","seq":30,"ts":2400,"turn":0,"step":0,"purpose":"memory"}"#,
            "not json",
        ];
        std::fs::write(dir.path().join("calls.jsonl"), lines.join("\n")).unwrap();
        let t = timings(dir.path());
        assert_eq!(t.len(), 3, "one per call record, the damaged line skipped");
        assert_eq!(t[0], Timing { sent: 1000, first: Some(1300), last: Some(1500), end: Some(1600) });
        assert_eq!(t[1], Timing { sent: 2100, first: Some(2150), last: Some(2150), end: Some(2350) });
        assert_eq!(t[2].first, None, "a call that streamed nothing has no first chunk");

        let timing = timing_json(&t[0], Some(100));
        assert_eq!(timing["ttft_ms"], 300);
        assert_eq!(timing["generate_ms"], 200);
        assert_eq!(timing["tokens_per_s"], 500.0);
        assert!(timing_json(&t[2], None)["ttft_ms"].is_null());

        let usage = |input: u64, read: u64, output: u64| json!({"input_tokens": input, "cache_read_input_tokens": read, "output_tokens": output});
        let calls = vec![
            json!({"index": 0, "turn": 0, "purpose": null, "duration_ms": 600, "usage": usage(100, 900, 100), "timing": timing_json(&t[0], Some(100))}),
            json!({"index": 1, "turn": 0, "purpose": null, "duration_ms": 250, "usage": usage(50, 1050, 20), "timing": timing_json(&t[1], Some(20))}),
            json!({"index": 2, "turn": 0, "purpose": "memory", "duration_ms": 10, "usage": usage(5, 0, 1), "timing": timing_json(&t[2], Some(1))}),
            json!({"index": 3, "turn": 1, "purpose": null, "duration_ms": 100, "usage": usage(10, 1200, 5), "timing": {"sent_at": 3000, "ttft_ms": 40, "generate_ms": 50}}),
        ];
        let turns = turns_of(&calls);
        assert_eq!(turns.len(), 2);
        let first = &turns[0];
        assert_eq!(first["calls"], json!([0, 1]));
        assert_eq!(first["auxiliary"], 1, "the memory call is beside the turn, not in it");
        assert_eq!(first["model_ms"], 850);
        assert_eq!(first["tool_ms"], 500, "from the first call's end (1600) to the second's start (2100)");
        assert_eq!(first["ttft_ms"], 300);
        assert_eq!(first["input_tokens"], 2100);
        assert_eq!(first["output_tokens"], 120);
        assert_eq!(first["cache_hit"], 0.929, "1950 of 2100 tokens in came from the cache");
        assert_eq!(first["context_tokens"], 1100, "what the last call carried");
        assert!(first["context_delta"].is_null(), "nothing before the first turn");
        assert_eq!(turns[1]["context_delta"], 110);
        assert_eq!(first["duration_ms"], 1350);
    }
    use base::interface::model::ToolDef;
    use base::interface::prompt::{BlockRole, CacheStrategy, PromptBlock};
    use base::interface::settings::ThinkingMode;
    use base::provider::ApiType;
    use telemetry::recorder::format::{Header, Record, RecordedParams};
    use telemetry::recorder::writer::RecordingWriter;

    fn block(content: &str, source: &str) -> PromptBlock {
        PromptBlock {
            role: BlockRole::System,
            content: content.to_string(),
            cache_strategy: Some(CacheStrategy::Ephemeral),
            name: Some(source.to_string()),
            origin: base::prompt::BlockOrigin::Kernel,
        }
    }

    fn tool(name: &str, source: &str) -> ToolDef {
        ToolDef {
            name: name.to_string(),
            description: format!("does {name}"),
            input_schema: json!({"type": "object"}),
            source: Some(source.to_string()),
        }
    }

    struct Fixture {
        root: tempfile::TempDir,
        writer: RecordingWriter,
    }

    impl Fixture {
        fn new() -> Self {
            let root = tempfile::tempdir().expect("tempdir");
            let writer = RecordingWriter::create(
                root.path(),
                Header {
                    version: telemetry::recorder::format::FORMAT_VERSION,
                    name: "S-1".into(),
                    session_id: "S-1".into(),
                    parent: None,
                    agent_type: None,
                    created_at: 0,
                    engine_version: "test".into(),
                },
            );
            Self { root, writer }
        }

        fn dir(&self) -> PathBuf {
            self.root.path().join("S-1")
        }

        fn call(
            &self,
            turn: u32,
            model: &str,
            system: &[PromptBlock],
            tools: &[ToolDef],
            purpose: Option<&str>,
        ) {
            let blobs = self.writer.blobs();
            let record = CallRecord {
                seq: self.writer.next_seq(),
                ts: 0,
                session_id: Some("S-1".into()),
                parent_session_id: None,
                agent_type: None,
                turn,
                step: 0,
                purpose: purpose.map(str::to_string),
                provider: "anthropic".into(),
                api_type: ApiType::Anthropic,
                params: RecordedParams {
                    model: model.to_string(),
                    max_tokens: 8192,
                    thinking_mode: ThinkingMode::Off,
                    effort: None,
                    fallback_model: None,
                    cache_edits: vec![],
                    search_max_uses: None,
                },
                system: system.iter().map(|b| blobs.put(b).expect("blob")).collect(),
                tools: blobs.put(&tools.to_vec()).expect("blob"),
                messages: vec![],
                input_map: None,
            };
            self.writer
                .append(&Record::Call(Box::new(record)))
                .expect("append");
            self.writer.flush().expect("flush");
        }
    }

    #[test]
    fn a_session_that_never_called_the_model_is_not_an_error() {
        let root = tempfile::tempdir().expect("tempdir");
        let answer = envelopes_in(&root.path().join("S-never")).expect("ok");
        assert_eq!(answer["recording"], json!(false));
        assert_eq!(answer["headers"], json!([]));
    }

    #[test]
    fn identical_envelopes_fold_to_one() {
        let fixture = Fixture::new();
        let system = [block("you are a coding agent", "scene")];
        let tools = [tool("Bash", "builtin")];
        for turn in 1..=3 {
            fixture.call(turn, "claude-sonnet-5", &system, &tools, None);
        }

        let answer = envelopes_in(&fixture.dir()).expect("ok");
        assert_eq!(answer["calls"], json!(3));
        assert_eq!(answer["headers"].as_array().expect("headers").len(), 1);
        assert_eq!(answer["headers"][0]["reason"], json!("initial"));
    }

    #[test]
    fn a_changed_catalog_is_a_new_envelope_and_carries_its_sources() {
        let fixture = Fixture::new();
        let system = [block("you are a coding agent", "scene")];
        fixture.call(
            1,
            "claude-sonnet-5",
            &system,
            &[tool("Bash", "builtin")],
            None,
        );
        fixture.call(
            2,
            "claude-sonnet-5",
            &system,
            &[tool("Bash", "builtin"), tool("Grep", "mcp:files")],
            None,
        );

        let answer = envelopes_in(&fixture.dir()).expect("ok");
        let headers = answer["headers"].as_array().expect("headers");
        assert_eq!(headers.len(), 2);
        assert_eq!(headers[1]["reason"], json!("changed"));
        // The blobs were hydrated, not just referenced: the pane renders text
        // and provenance, and neither survives as a blob id.
        assert_eq!(
            headers[1]["system"][0]["text"],
            json!("you are a coding agent")
        );
        assert_eq!(headers[1]["system"][0]["source"], json!("scene"));
        assert_eq!(headers[1]["system"][0]["cache"], json!("ephemeral"));
        assert_eq!(headers[1]["tools"][1]["source"], json!("mcp:files"));
    }

    /// Compaction and memory extraction assemble envelopes of their own. Left
    /// in, each one would report a change and then a change back, and a
    /// session that compacted twice would show four envelopes it never sent to
    /// the conversation.
    #[test]
    fn auxiliary_calls_are_counted_but_not_folded() {
        let fixture = Fixture::new();
        let system = [block("you are a coding agent", "scene")];
        let tools = [tool("Bash", "builtin")];
        fixture.call(1, "claude-sonnet-5", &system, &tools, None);
        fixture.call(
            0,
            "claude-haiku-4-5",
            &[block("summarize", "compact")],
            &[],
            Some("compact"),
        );
        fixture.call(2, "claude-sonnet-5", &system, &tools, None);

        let answer = envelopes_in(&fixture.dir()).expect("ok");
        assert_eq!(answer["calls"], json!(2));
        assert_eq!(answer["auxiliary"], json!(1));
        assert_eq!(answer["headers"].as_array().expect("headers").len(), 1);
    }
}
