//! What the engine has written down, read back: a session's model calls, and its transcript.
//!
//! The engine writes every model call a session makes to a recording, and every message to a
//! transcript. A reader turns those into Harnix's own structures, which serialize to the shape of
//! the system space's methods. Where a part is the engine's own (a system block's role, a tool's
//! definition, a message, a call's usage) it is carried as JSON as the engine wrote it, and
//! changes with the engine.
//!
//! The readers read files; their implementations do it off the async runtime.

use std::collections::BTreeMap;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Why a reader has no answer.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadError {
    /// What was asked about is not there (a call past the last one, a session that never called
    /// the model); the message says what.
    Missing(String),
    /// It is there and cannot be read; the message says why.
    Unreadable(String),
}

impl std::fmt::Display for ReadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReadError::Missing(m) | ReadError::Unreadable(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for ReadError {}

/// One instance's recordings, by session id.
#[async_trait::async_trait]
pub trait Recordings: Send + Sync {
    /// Every model call of the session's current run, oldest first.
    async fn calls(&self, session: &str) -> Result<Trace, ReadError>;
    /// One call in full.
    async fn call(&self, session: &str, index: usize) -> Result<CallDetail, ReadError>;
    /// The turn calls summed up per turn.
    async fn turns(&self, session: &str) -> Result<Turns, ReadError>;
    /// What the model was sent besides the messages, once per change, oldest first.
    async fn envelopes(&self, session: &str) -> Result<Envelopes, ReadError>;
    /// What each Agent of each session (sub-agents too) has used.
    async fn usage(&self, sessions: &[String]) -> Result<BTreeMap<String, SessionUsage>, ReadError>;
}

/// One instance's transcripts.
#[async_trait::async_trait]
pub trait Transcripts: Send + Sync {
    /// A case-insensitive search of the messages of `sessions`, in that order (the caller's: most
    /// recent first), one hit per session, as far as `limits` let it go.
    async fn search(&self, sessions: &[String], query: &str, limits: &SearchLimits) -> SearchResult;
}

/// A session's calls.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Trace {
    /// False: the session has not called the model in this run.
    pub recording: bool,
    pub calls: Vec<CallSummary>,
    /// Lines of the recording that could not be read: non-zero, it is read in part.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub damaged: Option<usize>,
}

impl Trace {
    pub fn new(recording: bool, calls: Vec<CallSummary>, damaged: Option<usize>) -> Self {
        Self { recording, calls, damaged }
    }
}

/// One call, for a row.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CallSummary {
    pub index: usize,
    pub turn: u32,
    pub step: u32,
    /// Set for a call beside the turns: a compaction, a memory extraction, a title.
    pub purpose: Option<String>,
    pub ts: u64,
    pub provider: String,
    pub model: String,
    pub input: CallInput,
    pub output: CallOutput,
    pub tools: Vec<ToolUseSummary>,
    /// The engine's.
    pub outcome: Value,
    pub stop_reason: Option<String>,
    /// The engine's.
    pub usage: Option<Value>,
    pub duration_ms: Option<u64>,
    pub timing: Option<Timing>,
}

impl CallSummary {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        index: usize,
        turn: u32,
        step: u32,
        purpose: Option<String>,
        ts: u64,
        provider: String,
        model: String,
        input: CallInput,
        output: CallOutput,
        tools: Vec<ToolUseSummary>,
    ) -> Self {
        Self {
            index,
            turn,
            step,
            purpose,
            ts,
            provider,
            model,
            input,
            output,
            tools,
            outcome: Value::Null,
            stop_reason: None,
            usage: None,
            duration_ms: None,
            timing: None,
        }
    }

    pub fn ended(mut self, outcome: Value, stop_reason: Option<String>, usage: Option<Value>, duration_ms: Option<u64>) -> Self {
        self.outcome = outcome;
        self.stop_reason = stop_reason;
        self.usage = usage;
        self.duration_ms = duration_ms;
        self
    }

    pub fn timed(mut self, timing: Option<Timing>) -> Self {
        self.timing = timing;
        self
    }
}

/// What a call was sent, counted.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CallInput {
    pub system: Vec<BlockSize>,
    pub tools: usize,
    pub messages: usize,
}

impl CallInput {
    pub fn new(system: Vec<BlockSize>, tools: usize, messages: usize) -> Self {
        Self { system, tools, messages }
    }
}

/// A system block: which one, and how long.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BlockSize {
    pub source: Option<String>,
    pub chars: usize,
}

impl BlockSize {
    pub fn new(source: Option<String>, chars: usize) -> Self {
        Self { source, chars }
    }
}

/// What a call answered, clipped for a row.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CallOutput {
    pub text: String,
    pub chars: usize,
    pub thinking_chars: usize,
    pub tool_uses: usize,
}

impl CallOutput {
    pub fn new(text: String, chars: usize, thinking_chars: usize, tool_uses: usize) -> Self {
        Self { text, chars, thinking_chars, tool_uses }
    }
}

/// A tool call a call asked for, and its result as the next turn call sent it back.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolUseSummary {
    pub id: String,
    pub name: Value,
    /// Its input as JSON, clipped.
    pub input: String,
    /// The first line of its result, clipped.
    pub result: Option<String>,
    pub result_chars: Option<usize>,
    pub is_error: Option<bool>,
}

impl ToolUseSummary {
    pub fn new(id: String, name: Value, input: String, result: Option<(String, usize, bool)>) -> Self {
        let (result, result_chars, is_error) = match result {
            Some((r, n, e)) => (Some(r), Some(n), Some(e)),
            None => (None, None, None),
        };
        Self { id, name, input, result, result_chars, is_error }
    }
}

/// When a call was sent, how long to its first chunk, how long it streamed, its output rate.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Timing {
    pub sent_at: u64,
    pub ttft_ms: Option<u64>,
    pub generate_ms: Option<u64>,
    pub tokens_per_s: Option<f64>,
}

impl Timing {
    pub fn new(sent_at: u64, ttft_ms: Option<u64>, generate_ms: Option<u64>, tokens_per_s: Option<f64>) -> Self {
        Self { sent_at, ttft_ms, generate_ms, tokens_per_s }
    }
}

/// One call in full.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CallDetail {
    pub index: usize,
    pub input: CallDetailInput,
    pub output: CallDetailOutput,
}

impl CallDetail {
    pub fn new(index: usize, input: CallDetailInput, output: CallDetailOutput) -> Self {
        Self { index, input, output }
    }
}

#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CallDetailInput {
    pub system: Vec<SystemBlock>,
    /// The engine's tool definitions.
    pub tools: Vec<Value>,
    /// The engine's messages, as sent.
    pub messages: Vec<Value>,
    pub model: String,
    pub max_tokens: u32,
    /// The engine's.
    pub thinking_mode: Value,
    pub provider: String,
    pub purpose: Option<String>,
    pub turn: u32,
    pub step: u32,
}

impl CallDetailInput {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        system: Vec<SystemBlock>,
        tools: Vec<Value>,
        messages: Vec<Value>,
        model: String,
        max_tokens: u32,
        thinking_mode: Value,
        provider: String,
        purpose: Option<String>,
        turn: u32,
        step: u32,
    ) -> Self {
        Self { system, tools, messages, model, max_tokens, thinking_mode, provider, purpose, turn, step }
    }
}

#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CallDetailOutput {
    pub text: String,
    pub thinking_chars: usize,
    /// `{id, name, input}` each.
    pub tool_uses: Vec<Value>,
    pub outcome: Value,
    pub stop_reason: Option<String>,
    pub usage: Option<Value>,
    pub duration_ms: Option<u64>,
}

impl CallDetailOutput {
    pub fn new(
        text: String,
        thinking_chars: usize,
        tool_uses: Vec<Value>,
        outcome: Value,
        stop_reason: Option<String>,
        usage: Option<Value>,
        duration_ms: Option<u64>,
    ) -> Self {
        Self { text, thinking_chars, tool_uses, outcome, stop_reason, usage, duration_ms }
    }
}

/// A system block as sent: `text` and `cache` rather than the engine's `content` and
/// `cache_strategy`, `source` (which block) beside `origin` (who put it there).
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SystemBlock {
    pub role: Value,
    pub cache: Value,
    pub source: Value,
    pub origin: Value,
    pub text: String,
}

impl SystemBlock {
    pub fn new(role: Value, cache: Value, source: Value, origin: Value, text: String) -> Self {
        Self { role, cache, source, origin, text }
    }
}

/// A session's turns.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Turns {
    pub recording: bool,
    pub turns: Vec<TurnSummary>,
}

impl Turns {
    pub fn new(recording: bool, turns: Vec<TurnSummary>) -> Self {
        Self { recording, turns }
    }
}

/// One turn's calls, summed: auxiliary calls are counted beside it, not inside its figures.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TurnSummary {
    pub turn: u32,
    pub calls: Vec<usize>,
    pub auxiliary: u64,
    pub started_at: Option<u64>,
    pub duration_ms: Option<u64>,
    pub ttft_ms: Option<u64>,
    pub model_ms: u64,
    pub tool_ms: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    /// The cache's share of what went in, by tokens.
    pub cache_hit: Option<f64>,
    pub tokens_per_s: Option<f64>,
    pub context_tokens: Option<u64>,
    /// How much the context grew since the turn before.
    pub context_delta: Option<i64>,
}

impl TurnSummary {
    pub fn new(turn: u32) -> Self {
        Self {
            turn,
            calls: Vec::new(),
            auxiliary: 0,
            started_at: None,
            duration_ms: None,
            ttft_ms: None,
            model_ms: 0,
            tool_ms: 0,
            input_tokens: 0,
            output_tokens: 0,
            cache_hit: None,
            tokens_per_s: None,
            context_tokens: None,
            context_delta: None,
        }
    }
}

/// A session's envelopes.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Envelopes {
    pub recording: bool,
    pub headers: Vec<Envelope>,
    /// Turn calls, all of them.
    pub calls: u64,
    pub auxiliary: u64,
    pub damaged: usize,
}

impl Envelopes {
    pub fn new(recording: bool, headers: Vec<Envelope>, calls: u64, auxiliary: u64, damaged: usize) -> Self {
        Self { recording, headers, calls, auxiliary, damaged }
    }
}

/// What a call was sent besides the messages.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Envelope {
    /// `initial`, `changed`, or `detail`.
    pub reason: String,
    pub turn: u32,
    pub step: u32,
    pub ts: u64,
    pub provider: String,
    pub model: String,
    pub max_tokens: u32,
    pub thinking_mode: Value,
    pub system: Vec<SystemBlock>,
    pub tools: Vec<Value>,
}

impl Envelope {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        reason: &str,
        turn: u32,
        step: u32,
        ts: u64,
        provider: String,
        model: String,
        max_tokens: u32,
        thinking_mode: Value,
        system: Vec<SystemBlock>,
        tools: Vec<Value>,
    ) -> Self {
        Self { reason: reason.to_string(), turn, step, ts, provider, model, max_tokens, thinking_mode, system, tools }
    }
}

/// What one session has used, per Agent: its own figures first, then each sub-agent's.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionUsage {
    pub recording: bool,
    pub agents: Vec<AgentUsage>,
}

impl SessionUsage {
    pub fn new(recording: bool, agents: Vec<AgentUsage>) -> Self {
        Self { recording, agents }
    }
}

#[non_exhaustive]
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AgentUsage {
    pub session_id: String,
    pub parent_session_id: Option<String>,
    pub calls: u64,
    pub auxiliary: u64,
    pub turns: usize,
    pub models: Vec<String>,
    pub input_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    pub output_tokens: u64,
    pub model_ms: u64,
    pub errors: u64,
    pub first_at: Option<u64>,
    pub last_at: Option<u64>,
}

impl AgentUsage {
    pub fn new(session_id: &str) -> Self {
        Self { session_id: session_id.to_string(), ..Self::default() }
    }
}

/// How far a search goes: sessions looked at, hits, a clock, and how long a snippet is.
#[non_exhaustive]
#[derive(Debug, Clone)]
pub struct SearchLimits {
    pub max_sessions: usize,
    pub max_hits: usize,
    pub snippet: usize,
    pub deadline: Duration,
}

impl Default for SearchLimits {
    fn default() -> Self {
        Self { max_sessions: 200, max_hits: 30, snippet: 160, deadline: Duration::from_secs(3) }
    }
}

impl SearchLimits {
    /// At most `n` hits (at least one, and never more than the default allows).
    pub fn hits(mut self, n: usize) -> Self {
        self.max_hits = n.clamp(1, self.max_hits);
        self
    }
}

#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchHit {
    pub session_id: String,
    pub role: String,
    pub snippet: String,
    pub ts: String,
}

impl SearchHit {
    pub fn new(session_id: &str, role: &str, snippet: String, ts: String) -> Self {
        Self { session_id: session_id.to_string(), role: role.to_string(), snippet, ts }
    }
}

#[non_exhaustive]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchResult {
    pub hits: Vec<SearchHit>,
    pub scanned: usize,
    /// A bound cut the search short: the caller says only recent sessions were searched.
    pub truncated: bool,
}

impl SearchResult {
    pub fn new(hits: Vec<SearchHit>, scanned: usize, truncated: bool) -> Self {
        Self { hits, scanned, truncated }
    }
}
