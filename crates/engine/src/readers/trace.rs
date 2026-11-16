//! The calls of a recording as the system space shows them: a list of rows, one call in full, and
//! the rows summed per turn.

use std::collections::HashMap;

use harnix_engine_api::readers::{
    BlockSize, CallDetail, CallDetailInput, CallDetailOutput, CallInput, CallOutput, CallSummary, ReadError, RecordingRun, SystemBlock, Timing,
    ToolResultView, ToolUseSummary, Trace, TurnSummary, Turns,
};
use serde_json::{json, Value};

use super::calls::Kept;
use super::time;
use super::wire::{self, Blob, Detail, Row};
use super::ProtocolReader;
use crate::translate::{SESSION_CALL, SESSION_CALL_BLOB};

/// Fetched references add up to at most this many bytes per call; past it, a reference stays, for
/// the page to say it is too large to show here.
const BLOBS_MAX: usize = 16 * 1024 * 1024;
/// A tool result's text is cut here (`chars` has the whole length).
const RESULT_CLIP: usize = 64 * 1024;

pub(super) fn run_of(kept: &Kept) -> Option<RecordingRun> {
    kept.run.as_ref().map(|r| RecordingRun::new(r.created_at.clone(), r.generation, kept.earlier_run))
}

/// A row of the list: a session's call, as its recording says it.
fn summary(row: &Row) -> CallSummary {
    let sent = time::ms(&row.timing.sent_at).unwrap_or_else(|| time::ms(&row.ts).unwrap_or(0));
    // From the first chunk to the call's end: the engine does not say when the last chunk came.
    let generate = row.timing.duration_ms.zip(row.timing.ttft_ms).map(|(d, t)| d.saturating_sub(t));
    let output = row.usage.as_ref().and_then(|u| u["output_tokens"].as_u64());
    let rate = match (output, generate) {
        (Some(n), Some(ms)) if n > 0 && ms > 0 => Some(((n as f64 * 1000.0 / ms as f64) * 10.0).round() / 10.0),
        _ => None,
    };
    let system = row
        .input
        .system
        .iter()
        .map(|b| BlockSize::new(b.source.clone(), b.chars.unwrap_or(0)).with_digest(b.digest.clone()))
        .collect();
    CallSummary::new(
        row.index,
        row.session_id.clone(),
        row.parent_session_id.clone(),
        row.agent_type.clone(),
        row.turn_id.clone(),
        row.turn,
        row.step,
        row.purpose.clone(),
        time::ms(&row.ts).unwrap_or(sent),
        row.provider.clone(),
        row.model.clone(),
        CallInput::new(system, row.input.tools.unwrap_or(0), row.input.messages).with_tools_digest(row.input.tools_digest.clone()),
        CallOutput::new(row.output.chars, row.output.thinking_chars, row.output.tool_uses.len())
            .with_text_preview(row.output.text_preview.clone()),
        row.output
            .tool_uses
            .iter()
            .map(|t| ToolUseSummary::new(t.id.clone(), t.name.clone()).with_preview(t.input_preview.clone(), t.result_is_error))
            .collect(),
    )
    .ended(row.status.clone(), row.error.clone(), row.stop_reason.clone(), row.usage.clone(), row.timing.duration_ms)
    .timed(Some(Timing::new(sent, row.timing.ttft_ms, generate, rate)))
}

/// `session`'s own calls: not those of the sub-agents the recording of its tree also holds.
fn calls_of(kept: &Kept, session: &str) -> Vec<CallSummary> {
    kept.rows.iter().filter(|r| r.session_id == session).map(summary).collect()
}

pub(super) fn trace(kept: &Kept, session: &str) -> Trace {
    Trace::new(kept.run.is_some(), calls_of(kept, session), Some(kept.damaged).filter(|d| *d > 0)).run(run_of(kept))
}

pub(super) fn turns(kept: &Kept, session: &str) -> Turns {
    Turns::new(kept.run.is_some(), turns_of(&calls_of(kept, session))).run(run_of(kept))
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

/// What the engine put in a system block's, a tool table's or a message's place that is not the
/// part itself: its text read back from the recording on its own.
fn missing(part: &Value) -> Option<&str> {
    part.get("missing").and_then(Value::as_str)
}

fn clip(text: &str, chars: usize) -> String {
    let mut out: String = text.chars().take(chars).collect();
    if text.chars().count() > chars {
        out.push('…');
    }
    out
}

fn system_block(part: &Value) -> SystemBlock {
    let field = |k: &str| part.get(k).cloned().unwrap_or(Value::Null);
    SystemBlock::new(field("role"), field("cache"), field("source"), field("origin"), part.get("text").and_then(Value::as_str).unwrap_or_default().to_string())
}

/// A system block that is gone from the recording, in its place.
fn gone(id: &str) -> SystemBlock {
    SystemBlock::new(Value::Null, Value::Null, Value::Null, Value::Null, format!("<missing blob {id}>"))
}

impl ProtocolReader {
    /// A part as the engine sent it: inline as it came, or a reference fetched with
    /// `session.callBlob` while `budget` lasts (past it the reference stays).
    pub(super) async fn resolve(&self, root: &str, part: Value, budget: &mut usize) -> Value {
        let Some(blob) = part.get("blob").and_then(Value::as_str) else { return part };
        let bytes = part.get("bytes").and_then(Value::as_u64).unwrap_or(0) as usize;
        if bytes > *budget {
            return part;
        }
        match self.ask::<Blob>(SESSION_CALL_BLOB, json!({"session_id": root, "blob": blob})).await {
            Ok(fetched) => {
                *budget -= bytes;
                fetched.value
            }
            Err(e) => {
                tracing::warn!(blob, error = %e, "a part of a call was not fetched");
                part
            }
        }
    }

    /// The system blocks of a request, references fetched, blocks the recording lost in place.
    pub(super) async fn system_of(&self, root: &str, parts: Vec<Value>, budget: &mut usize) -> Vec<SystemBlock> {
        let mut out = Vec::with_capacity(parts.len());
        for part in parts {
            let part = self.resolve(root, part, budget).await;
            out.push(match missing(&part) {
                Some(id) => gone(id),
                None => system_block(&part),
            });
        }
        out
    }

    /// The tool table of a request: an array, however it came.
    pub(super) async fn tools_of(&self, root: &str, table: Value, budget: &mut usize) -> Vec<Value> {
        match self.resolve(root, table, budget).await {
            Value::Array(tools) => tools,
            other => {
                if !other.is_null() && missing(&other).is_none() {
                    tracing::warn!("a tool table is not a list: shown empty");
                }
                Vec::new()
            }
        }
    }

    /// One call in full, with what its tool calls got back.
    pub(super) async fn detail(&self, session: &str, index: usize) -> Result<CallDetail, ReadError> {
        let root = self.calls.root_of(self, session, true).await?;
        let kept = self.calls.read(self, &root, true).await?;
        let Some(run) = kept.run.clone() else {
            return Err(ReadError::Missing("this session has no recording: it has not called the model".into()));
        };
        if !kept.rows.get(index).is_some_and(|r| r.session_id == session) {
            return Err(ReadError::Missing(format!("this run has no call {index} of this session")));
        }
        let answer = self.send(SESSION_CALL, json!({"session_id": root, "index": index, "generation": run.generation})).await;
        let detail: Detail = match answer {
            Ok(answer) => wire::parse(answer, SESSION_CALL)?,
            Err(e) if wire::restarted(&e).is_some() => {
                self.calls.forget(&root);
                return Err(ReadError::Missing("the recording was started over: read the calls again".into()));
            }
            Err(e) => return Err(wire::read_error(e)),
        };
        let Detail { request, response } = detail;
        let mut budget = BLOBS_MAX;
        let system = self.system_of(&root, request.system, &mut budget).await;
        let tools = self.tools_of(&root, request.tools, &mut budget).await;
        let mut messages = Vec::with_capacity(request.messages.len());
        for message in request.messages {
            let message = self.resolve(&root, message, &mut budget).await;
            messages.push(match missing(&message) {
                Some(_) => json!({"role": "user", "content": []}),
                None => message,
            });
        }
        let ids: Vec<String> = response.tool_uses.iter().filter_map(|t| t["id"].as_str().map(str::to_string)).collect();
        let results = self.tool_results(&root, &kept, session, index, &ids).await;
        let input = CallDetailInput::new(
            request.session_id,
            system,
            tools,
            messages,
            request.model,
            request.max_tokens,
            request.thinking_mode,
            request.effort,
            request.provider,
            request.purpose,
            request.turn,
            request.step,
        );
        let output = CallDetailOutput::new(
            response.text,
            response.thinking_chars,
            response.tool_uses,
            response.status,
            response.error,
            response.stop_reason,
            response.usage,
            response.duration_ms,
        )
        .results(results);
        Ok(CallDetail::new(index, input, output))
    }

    /// What the tool calls `ids` of call `index` got back: the engine sends it in its next
    /// conversation call, in the last messages, which are read from the end and only until every
    /// one is found. Not made yet, or not read: none, said once.
    async fn tool_results(&self, root: &str, kept: &Kept, session: &str, index: usize, ids: &[String]) -> Vec<ToolResultView> {
        let Some(generation) = kept.run.as_ref().map(|r| r.generation) else { return Vec::new() };
        if ids.is_empty() {
            return Vec::new();
        }
        let answering = kept.rows.iter().skip(index + 1).find(|r| r.session_id == session && r.purpose.is_none());
        let Some(answering) = answering else { return Vec::new() };
        let params = json!({"session_id": root, "index": answering.index, "generation": generation});
        let next: Detail = match self.ask(SESSION_CALL, params).await {
            Ok(next) => next,
            Err(e) => {
                tracing::warn!(error = %e, "the results of a call's tools were not read");
                return Vec::new();
            }
        };
        let mut found: HashMap<String, (String, usize, bool)> = HashMap::new();
        let mut budget = BLOBS_MAX;
        for message in next.request.messages.into_iter().rev() {
            if ids.iter().all(|id| found.contains_key(id)) {
                break;
            }
            let message = self.resolve(root, message, &mut budget).await;
            for block in message.get("content").and_then(Value::as_array).into_iter().flatten() {
                if block.get("type").and_then(Value::as_str) != Some("tool_result") {
                    continue;
                }
                let id = block.get("tool_use_id").and_then(Value::as_str).unwrap_or_default();
                let text = match block.get("content") {
                    Some(Value::String(text)) => text.clone(),
                    Some(Value::Array(parts)) => parts
                        .iter()
                        .filter(|p| p.get("type").and_then(Value::as_str) == Some("text"))
                        .filter_map(|p| p.get("text").and_then(Value::as_str))
                        .collect::<Vec<_>>()
                        .join(" "),
                    _ => String::new(),
                };
                let error = block.get("is_error").and_then(Value::as_bool).unwrap_or(false);
                found.entry(id.to_string()).or_insert((text, 0, error));
            }
        }
        ids.iter()
            .filter_map(|id| {
                let (text, _, error) = found.remove(id)?;
                let chars = text.chars().count();
                Some(ToolResultView::new(id.clone(), clip(&text, RESULT_CLIP), chars, error))
            })
            .collect()
    }
}
