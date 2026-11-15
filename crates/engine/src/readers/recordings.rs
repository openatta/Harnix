//! A session's recording, read back: its calls, each call in full, its turns, its envelopes.
//!
//! What the model was sent and what came back: the assembled system blocks, the whole tool
//! catalog, the call configuration, the messages. Agix does not send this over the wire — it
//! writes every call to a recording (Agix's recorder), and this reads that directory.
//!
//! Reading files rather than receiving frames is what makes it survive a reload: the browser can
//! ask at any point, including for a session whose turns ran in a previous process. The cost is a
//! boundary: a recording covers the current *run* of a session, because the writer truncates when
//! a resumed session makes its first call.
//!
//! Only turn calls are folded into the envelope timeline. Compaction, memory extraction and
//! titling assemble their own envelopes, so including them would report a change on every one and
//! a change back afterwards, none of which describes the conversation.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::SystemTime;

use agix::contracts::model::{ModelEvent, ToolDef};
use agix::contracts::prompt::PromptBlock;
use agix::recordings::{reader, BlobId, BlobStore, CallRecord, RecordedParams};
use harnix_engine_api::readers::{
    BlockSize, CallDetail, CallDetailInput, CallDetailOutput, CallInput, CallOutput, CallSummary, Envelope, Envelopes,
    ReadError, Recordings, SessionUsage, SystemBlock, Timing, ToolUseSummary, Trace, TurnSummary, Turns,
};
use serde_json::{json, Value};

use super::off_the_runtime;
use super::usage::UsageCache;

/// The recordings under one root, one directory per session id: where an engine writes them.
/// What it read is kept until the file it read changes.
pub struct RecordingsDir {
    root: PathBuf,
    traces: Arc<Traces>,
    usage: Arc<Mutex<UsageCache>>,
}

impl RecordingsDir {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into(), traces: Arc::default(), usage: Arc::default() }
    }

    fn dir(&self, session: &str) -> PathBuf {
        self.root.join(session)
    }
}

#[async_trait::async_trait]
impl Recordings for RecordingsDir {
    async fn calls(&self, session: &str) -> Result<Trace, ReadError> {
        let (dir, traces) = (self.dir(session), self.traces.clone());
        off_the_runtime(move || traces.trace(&dir)).await
    }

    async fn call(&self, session: &str, index: usize) -> Result<CallDetail, ReadError> {
        let dir = self.dir(session);
        off_the_runtime(move || call_in(&dir, index)).await
    }

    async fn turns(&self, session: &str) -> Result<Turns, ReadError> {
        let (dir, traces) = (self.dir(session), self.traces.clone());
        off_the_runtime(move || {
            let trace = traces.trace(&dir)?;
            Ok(Turns::new(trace.recording, turns_of(&trace.calls)))
        })
        .await
    }

    async fn envelopes(&self, session: &str) -> Result<Envelopes, ReadError> {
        let dir = self.dir(session);
        off_the_runtime(move || envelopes_in(&dir)).await
    }

    async fn usage(&self, sessions: &[String]) -> Result<BTreeMap<String, SessionUsage>, ReadError> {
        let dirs: Vec<(PathBuf, String)> = sessions.iter().map(|id| (self.dir(id), id.clone())).collect();
        let cache = self.usage.clone();
        off_the_runtime(move || {
            Ok(dirs.into_iter().map(|(dir, id)| {
                let usage = super::usage::usage_of(&cache, &dir, &id);
                (id, usage)
            }).collect())
        })
        .await
    }
}

pub(super) fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// How many recordings' traces are kept. Past it, the one asked about longest ago goes.
const TRACES_KEPT: usize = 64;

/// Each recording's trace, until its `calls.jsonl` changes size or time. The system space asks
/// for the trace and the turns of the session it shows, again on every change; reading the
/// recording is the expensive part of both.
#[derive(Default)]
struct Traces(Mutex<TraceCache>);

#[derive(Default)]
struct TraceCache {
    by_file: HashMap<PathBuf, Kept>,
    clock: u64,
}

struct Kept {
    len: u64,
    modified: Option<SystemTime>,
    asked: u64,
    trace: Trace,
}

impl Traces {
    /// The trace of the recording in `dir`: the kept one when its file has not changed. The lock
    /// is not held while the recording is read.
    fn trace(&self, dir: &Path) -> Result<Trace, ReadError> {
        let file = dir.join("calls.jsonl");
        let stat = std::fs::metadata(&file).ok().map(|m| (m.len(), m.modified().ok()));
        {
            let mut cache = lock(&self.0);
            cache.clock += 1;
            let now = cache.clock;
            match (stat, cache.by_file.get_mut(&file)) {
                (Some((len, modified)), Some(kept)) if kept.len == len && kept.modified == modified => {
                    kept.asked = now;
                    return Ok(kept.trace.clone());
                }
                (None, _) => {
                    cache.by_file.remove(&file);
                }
                _ => {}
            }
        }
        let trace = trace_in(dir)?;
        if let Some((len, modified)) = stat {
            let mut cache = lock(&self.0);
            let asked = cache.clock;
            cache.by_file.insert(file, Kept { len, modified, asked, trace: trace.clone() });
            if cache.by_file.len() > TRACES_KEPT {
                if let Some(oldest) = cache.by_file.iter().min_by_key(|(_, k)| k.asked).map(|(f, _)| f.clone()) {
                    cache.by_file.remove(&oldest);
                }
            }
        }
        Ok(trace)
    }

    #[cfg(test)]
    fn kept(&self) -> usize {
        lock(&self.0).by_file.len()
    }
}

/// A session's recording, or `None` when it has none — the ordinary state of a session that has
/// not called the model yet. One that is there and cannot be read is the same error to every
/// reader.
fn load(dir: &Path) -> Result<Option<reader::Recording>, ReadError> {
    match reader::load(dir) {
        Ok(recording) => Ok(Some(recording)),
        Err(reader::ReadError::NotFound(_)) => Ok(None),
        Err(e) => Err(ReadError::Unreadable(format!("unreadable recording: {e}"))),
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
            ModelEvent::ToolUse { id, name, input } => tools.push(json!({"id": id, "name": name, "input": input})),
            _ => {}
        }
    }
    (text, thinking, tools)
}

/// Every `tool_result` in a call's messages, by the tool call it answers.
fn results(messages: &[Value]) -> HashMap<String, (String, bool)> {
    let mut found = HashMap::new();
    for message in messages {
        for block in message.get("content").and_then(Value::as_array).map_or(&[][..], |v| v) {
            if block.get("type").and_then(Value::as_str) == Some("tool_result") {
                let id = block.get("tool_use_id").and_then(Value::as_str).unwrap_or_default();
                let content = block.get("content").and_then(Value::as_str).unwrap_or_default();
                let error = block.get("is_error").and_then(Value::as_bool).unwrap_or(false);
                found.insert(id.to_string(), (content.to_string(), error));
            }
        }
    }
    found
}

/// The results of the tool calls `ids`, from the call that sent them back. They are in its last
/// messages, so its messages are read from the end and only until every one is found: a list row
/// does not need the conversation's every message decoded.
fn results_of(ids: &[&str], answering: &CallRecord, blobs: &BlobStore) -> HashMap<String, (String, bool)> {
    let mut found = HashMap::new();
    for id in answering.messages.iter().rev() {
        if ids.iter().all(|i| found.contains_key(*i)) {
            break;
        }
        if let Some(message) = read::<Value>(blobs, id) {
            for (k, v) in results(std::slice::from_ref(&message)) {
                found.entry(k).or_insert(v);
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
struct Times {
    sent: u64,
    first: Option<u64>,
    last: Option<u64>,
    end: Option<u64>,
}

/// Each call's times, in the order of its call records — read off `calls.jsonl` itself, since the
/// reader folds the chunks into events and keeps none of their times. A packed run of chunks
/// (`ts0` and the gaps `dt`) spans from its first member to its last.
fn timings(dir: &Path) -> Vec<Times> {
    let Ok(text) = std::fs::read_to_string(dir.join("calls.jsonl")) else {
        return Vec::new();
    };
    let mut order = Vec::new();
    let mut by_call: HashMap<u64, Times> = HashMap::new();
    for line in text.lines() {
        let Ok(record) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        match record["type"].as_str() {
            Some("recording") => {}
            Some("call") => {
                if let (Some(seq), Some(ts)) = (record["seq"].as_u64(), record["ts"].as_u64()) {
                    order.push(seq);
                    by_call.insert(seq, Times { sent: ts, ..Times::default() });
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
                        let gaps: i64 = record["dt"].as_array().map(|dt| dt.iter().filter_map(Value::as_i64).sum()).unwrap_or(0);
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

/// A call's timing: when it was sent, how long to its first chunk, how long it streamed, and the
/// output rate over that stretch. Absent parts are left out rather than guessed.
fn timing_of(times: &Times, output_tokens: Option<u64>) -> Timing {
    let ttft = times.first.map(|first| first.saturating_sub(times.sent));
    let generate = match (times.first, times.last) {
        (Some(first), Some(last)) => Some(last.saturating_sub(first)),
        _ => None,
    };
    let rate = match (output_tokens, generate) {
        (Some(tokens), Some(ms)) if ms > 0 && tokens > 0 => Some((tokens as f64) * 1000.0 / ms as f64),
        _ => None,
    };
    Timing::new(times.sent, ttft, generate, rate.map(|r| (r * 10.0).round() / 10.0))
}

/// Tokens that went in: uncached, read from the cache, written to it — the three do not overlap.
fn tokens_in(usage: Option<&Value>) -> u64 {
    let Some(usage) = usage else {
        return 0;
    };
    ["input_tokens", "cache_read_input_tokens", "cache_creation_input_tokens"]
        .iter()
        .filter_map(|k| usage[*k].as_u64())
        .sum()
}

fn usage_of(call: &CallSummary, key: &str) -> Option<u64> {
    call.usage.as_ref().and_then(|u| u[key].as_u64())
}

/// The trace's turn calls summed per turn, in the order the turns ran. Auxiliary calls
/// (compaction, memory, titles) are counted beside a turn, not inside its figures.
fn turns_of(calls: &[CallSummary]) -> Vec<TurnSummary> {
    let mut order: Vec<u32> = Vec::new();
    let mut by_turn: HashMap<u32, Vec<&CallSummary>> = HashMap::new();
    let mut auxiliary: HashMap<u32, u64> = HashMap::new();
    for call in calls {
        let turn = call.turn;
        if call.purpose.is_some() {
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
        let sent = |c: &CallSummary| c.timing.as_ref().map(|t| t.sent_at);
        let duration = |c: &CallSummary| c.duration_ms;
        let model_ms: u64 = steps.iter().filter_map(|c| duration(c)).sum();
        // Between one call's end and the next one's start, the turn ran its tools.
        let tool_ms: u64 = steps
            .windows(2)
            .filter_map(|pair| {
                let ended = sent(pair[0])? + duration(pair[0])?;
                Some(sent(pair[1])?.saturating_sub(ended))
            })
            .sum();
        let input: u64 = steps.iter().map(|c| tokens_in(c.usage.as_ref())).sum();
        let cache_read: u64 = steps.iter().filter_map(|c| usage_of(c, "cache_read_input_tokens")).sum();
        let output: u64 = steps.iter().filter_map(|c| usage_of(c, "output_tokens")).sum();
        let generate_ms: u64 = steps.iter().filter_map(|c| c.timing.as_ref().and_then(|t| t.generate_ms)).sum();
        let context = steps.last().map(|c| tokens_in(c.usage.as_ref())).filter(|n| *n > 0);
        let started = steps.first().and_then(|c| sent(c));
        let ended = steps.last().and_then(|c| Some(sent(c)? + duration(c)?));
        let mut summary = TurnSummary::new(turn);
        summary.calls = steps.iter().map(|c| c.index).collect();
        summary.auxiliary = auxiliary.get(&turn).copied().unwrap_or(0);
        summary.started_at = started;
        summary.duration_ms = started.zip(ended).map(|(s, e)| e.saturating_sub(s));
        summary.ttft_ms = steps.first().and_then(|c| c.timing.as_ref()).and_then(|t| t.ttft_ms);
        summary.model_ms = model_ms;
        summary.tool_ms = tool_ms;
        summary.input_tokens = input;
        summary.output_tokens = output;
        summary.cache_hit = (input > 0).then(|| ((cache_read as f64 / input as f64) * 1000.0).round() / 1000.0);
        summary.tokens_per_s =
            (generate_ms > 0 && output > 0).then(|| ((output as f64 * 1000.0 / generate_ms as f64) * 10.0).round() / 10.0);
        summary.context_tokens = context;
        summary.context_delta = match (context, previous_context) {
            (Some(now), Some(before)) => Some(now as i64 - before as i64),
            _ => None,
        };
        out.push(summary);
        if context.is_some() {
            previous_context = context;
        }
    }
    out
}

fn trace_in(dir: &Path) -> Result<Trace, ReadError> {
    let Some(recording) = load(dir)? else {
        return Ok(Trace::new(false, Vec::new(), None));
    };
    let blobs = BlobStore::new(dir);
    // Times only when every call record was found: otherwise they could be paired with the wrong call.
    let times = Some(timings(dir)).filter(|t| t.len() == recording.calls.len());
    let mut calls = Vec::new();
    for (index, call) in recording.calls.iter().enumerate() {
        let request = &call.request;
        let (text, thinking, tool_uses) = answer(&call.response);
        // A tool call's result is in the next turn call: the engine sends it back.
        let ids: Vec<&str> = tool_uses.iter().filter_map(|t| t["id"].as_str()).collect();
        let next = if ids.is_empty() {
            HashMap::new()
        } else {
            recording.calls[index + 1..]
                .iter()
                .find(|c| c.request.purpose.is_none())
                .map(|answering| results_of(&ids, &answering.request, &blobs))
                .unwrap_or_default()
        };
        let tools: Vec<ToolUseSummary> = tool_uses
            .iter()
            .map(|tool| {
                let id = tool["id"].as_str().unwrap_or_default();
                let result = next.get(id).map(|(text, error)| {
                    (clip(text.lines().next().unwrap_or_default(), 160), text.chars().count(), *error)
                });
                ToolUseSummary::new(id.to_string(), tool["name"].clone(), clip(&tool["input"].to_string(), 160), result)
            })
            .collect();
        let system: Vec<BlockSize> = request
            .system
            .iter()
            .map(|id| match read::<PromptBlock>(&blobs, id) {
                Some(block) => BlockSize::new(block.name, block.content.chars().count()),
                None => BlockSize::new(None, 0),
            })
            .collect();
        let tool_count = read::<Vec<ToolDef>>(&blobs, &request.tools).map_or(0, |t| t.len());
        let end = call.end.as_ref();
        let output_tokens = end.and_then(|e| e.usage.as_ref()).map(|u| u.output_tokens as u64);
        let timing = times.as_ref().and_then(|t| t.get(index)).map(|t| timing_of(t, output_tokens));
        let summary = CallSummary::new(
            index,
            request.turn,
            request.step,
            request.purpose.clone(),
            request.ts,
            request.provider.clone(),
            request.params.model.clone(),
            CallInput::new(system, tool_count, request.messages.len()),
            CallOutput::new(clip(&text, 240), text.chars().count(), thinking, tool_uses.len()),
            tools,
        )
        .ended(
            outcome(call),
            end.and_then(|e| e.stop_reason.clone()),
            end.and_then(|e| e.usage.as_ref()).and_then(|u| serde_json::to_value(u).ok()),
            end.map(|e| e.duration_ms),
        )
        .timed(timing);
        calls.push(summary);
    }
    Ok(Trace::new(true, calls, Some(recording.damaged)))
}

fn call_in(dir: &Path, index: usize) -> Result<CallDetail, ReadError> {
    let recording =
        load(dir)?.ok_or_else(|| ReadError::Missing("this session has no recording: it has not called the model".into()))?;
    let call = recording
        .calls
        .get(index)
        .ok_or_else(|| ReadError::Missing(format!("this run has {} calls", recording.calls.len())))?;
    let blobs = BlobStore::new(dir);
    let request = &call.request;
    let envelope = envelope(request, "detail", &blobs);
    let messages: Vec<Value> = request.messages.iter().filter_map(|id| read::<Value>(&blobs, id)).collect();
    let (text, thinking, tool_uses) = answer(&call.response);
    let end = call.end.as_ref();
    Ok(CallDetail::new(
        index,
        CallDetailInput::new(
            envelope.system,
            envelope.tools,
            messages,
            request.params.model.clone(),
            request.params.max_tokens,
            json!(request.params.thinking_mode),
            request.provider.clone(),
            request.purpose.clone(),
            request.turn,
            request.step,
        ),
        CallDetailOutput::new(
            text,
            thinking,
            tool_uses,
            outcome(call),
            end.and_then(|e| e.stop_reason.clone()),
            end.and_then(|e| e.usage.as_ref()).and_then(|u| serde_json::to_value(u).ok()),
            end.map(|e| e.duration_ms),
        ),
    ))
}

/// Fold one recording directory into the changes of its envelope.
///
/// A session that has not called the model yet has no recording at all; that reports as
/// `recording: false` rather than as an error, because it is the ordinary state of a session
/// nobody has spoken to.
fn envelopes_in(dir: &Path) -> Result<Envelopes, ReadError> {
    let Some(recording) = load(dir)? else {
        return Ok(Envelopes::new(false, Vec::new(), 0, 0, 0));
    };

    let blobs = BlobStore::new(dir);
    let mut headers = Vec::new();
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
        let reason = if previous.is_none() { "initial" } else { "changed" };
        previous = Some(fingerprint);
        headers.push(envelope(request, reason, &blobs));
    }

    Ok(Envelopes::new(true, headers, turn_calls, auxiliary, recording.damaged))
}

/// What makes two calls carry the same envelope.
///
/// Blob ids, not contents: a blob id *is* a hash of the content, so equal ids mean equal bytes and
/// the comparison costs no reads. This is the same identity replay uses to detect divergence.
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
/// `text` and `cache` rather than `content` and `cache_strategy`: the flow row and the details
/// pane read a block, not a `PromptBlock`, and `source` is the field that lets them say *which*
/// block — the skills inventory, the memory recall, a named scene section — instead of numbering
/// them.
fn envelope(request: &CallRecord, reason: &str, blobs: &BlobStore) -> Envelope {
    let system: Vec<SystemBlock> = request
        .system
        .iter()
        .map(|id| match read::<PromptBlock>(blobs, id) {
            // Two answers, not one. Agix splits what used to be a single `source` into `name` —
            // which block this is (`skills.catalog`, `memory.session`), stable enough for an
            // extension to address — and `origin`, who put it there. The details pane wants both:
            // "what am I looking at" and "who is responsible for it" are different questions.
            Some(block) => SystemBlock::new(
                json!(block.role),
                json!(block.cache_strategy),
                json!(block.name),
                json!(block.origin),
                block.content,
            ),
            None => SystemBlock::new(json!("system"), Value::Null, Value::Null, Value::Null, missing(id)),
        })
        .collect();
    let tools = read::<Vec<ToolDef>>(blobs, &request.tools).unwrap_or_default();

    Envelope::new(
        reason,
        request.turn,
        request.step,
        request.ts,
        request.provider.clone(),
        request.params.model.clone(),
        request.params.max_tokens,
        json!(request.params.thinking_mode),
        system,
        tools.iter().map(|t| json!(t)).collect(),
    )
}

/// A blob a recording references but does not have.
///
/// Only reachable for a hand-edited or partially copied directory, and it stands in for one block
/// rather than failing the whole read — the rest of the envelope still answers most of what a
/// reader came for.
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
        assert_eq!(t[0], Times { sent: 1000, first: Some(1300), last: Some(1500), end: Some(1600) });
        assert_eq!(t[1], Times { sent: 2100, first: Some(2150), last: Some(2150), end: Some(2350) });
        assert_eq!(t[2].first, None, "a call that streamed nothing has no first chunk");

        let timing = timing_of(&t[0], Some(100));
        assert_eq!((timing.ttft_ms, timing.generate_ms, timing.tokens_per_s), (Some(300), Some(200), Some(500.0)));
        assert!(timing_of(&t[2], None).ttft_ms.is_none());

        let usage = |input: u64, read: u64, output: u64| json!({"input_tokens": input, "cache_read_input_tokens": read, "output_tokens": output});
        let call = |index: usize, turn: u32, purpose: Option<&str>, ms: u64, usage: Value, timing: Timing| {
            let input = CallInput::new(Vec::new(), 0, 0);
            let output = CallOutput::new(String::new(), 0, 0, 0);
            CallSummary::new(index, turn, 0, purpose.map(str::to_string), 0, String::new(), String::new(), input, output, Vec::new())
                .ended(Value::Null, None, Some(usage), Some(ms))
                .timed(Some(timing))
        };
        let calls = vec![
            call(0, 0, None, 600, usage(100, 900, 100), timing_of(&t[0], Some(100))),
            call(1, 0, None, 250, usage(50, 1050, 20), timing_of(&t[1], Some(20))),
            call(2, 0, Some("memory"), 10, usage(5, 0, 1), timing_of(&t[2], Some(1))),
            call(3, 1, None, 100, usage(10, 1200, 5), Timing::new(3000, Some(40), Some(50), None)),
        ];
        let turns: Vec<Value> = turns_of(&calls).iter().map(|t| serde_json::to_value(t).unwrap()).collect();
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
    use agix::contracts::model::ToolDef;
    use agix::contracts::prompt::{BlockOrigin, BlockRole, CacheStrategy, PromptBlock};
    use agix::recordings::{Header, Record, RecordedParams, RecordingWriter, FORMAT_VERSION};
    use agix::settings::{ApiType, ThinkingMode};

    fn block(content: &str, source: &str) -> PromptBlock {
        PromptBlock {
            role: BlockRole::System,
            content: content.to_string(),
            cache_strategy: Some(CacheStrategy::Ephemeral),
            name: Some(source.to_string()),
            origin: BlockOrigin::Kernel,
        }
    }

    fn tool(name: &str, source: &str) -> ToolDef {
        ToolDef::new(name, format!("does {name}"), json!({"type": "object"})).with_source(source)
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
                    version: FORMAT_VERSION,
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
                turn_id: None,
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

    /// The trace is kept until the recording's file changes, and read again once it does; a
    /// session with no recording is not kept at all. Every reader says the same of a recording
    /// that is not there: none of them an error, or the one that asks for a call by index a
    /// `NOT_FOUND` — not "there is no such method".
    #[test]
    fn a_trace_is_kept_until_its_recording_changes() {
        let f = Fixture::new();
        let traces = Traces::default();
        f.call(0, "m-1", &[block("a", "scene")], &[], None);
        let first = traces.trace(&f.dir()).unwrap();
        assert_eq!(first.calls.len(), 1);
        assert_eq!(traces.kept(), 1);
        // Same file: the same answer, without reading it.
        assert_eq!(traces.trace(&f.dir()).unwrap(), first);
        f.call(1, "m-1", &[block("a", "scene")], &[], None);
        assert_eq!(traces.trace(&f.dir()).unwrap().calls.len(), 2);
        assert_eq!(traces.kept(), 1);

        let never = f.root.path().join("S-never");
        let none = traces.trace(&never).unwrap();
        assert_eq!(serde_json::to_value(&none).unwrap(), json!({"recording": false, "calls": []}));
        assert_eq!(traces.kept(), 1, "a session with no recording is not kept");
        assert!(matches!(call_in(&never, 0), Err(ReadError::Missing(_))));
        let past = call_in(&f.dir(), 2).unwrap_err();
        assert_eq!(past, ReadError::Missing("this run has 2 calls".into()));
    }

    #[test]
    fn a_session_that_never_called_the_model_is_not_an_error() {
        let root = tempfile::tempdir().expect("tempdir");
        let answer = serde_json::to_value(envelopes_in(&root.path().join("S-never")).expect("ok")).unwrap();
        assert_eq!(answer, json!({"recording": false, "headers": [], "calls": 0, "auxiliary": 0, "damaged": 0}));
    }

    #[test]
    fn identical_envelopes_fold_to_one() {
        let fixture = Fixture::new();
        let system = [block("you are a coding agent", "scene")];
        let tools = [tool("Bash", "builtin")];
        for turn in 1..=3 {
            fixture.call(turn, "claude-sonnet-5", &system, &tools, None);
        }

        let answer = serde_json::to_value(envelopes_in(&fixture.dir()).expect("ok")).unwrap();
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

        let answer = serde_json::to_value(envelopes_in(&fixture.dir()).expect("ok")).unwrap();
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

        let answer = serde_json::to_value(envelopes_in(&fixture.dir()).expect("ok")).unwrap();
        assert_eq!(answer["calls"], json!(2));
        assert_eq!(answer["auxiliary"], json!(1));
        assert_eq!(answer["headers"].as_array().expect("headers").len(), 1);
    }
}
