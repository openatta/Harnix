//! What the engine has written down, read back: a session's model calls, and its transcript.
//!
//! The engine writes every model call a session makes to a recording, and every message to a
//! transcript. A reader turns those into Harnix's own structures, which serialize to the shape of
//! the system space's methods. Where a part is the engine's own (a system block's role, a tool's
//! definition, a message, a call's usage) it is carried as JSON as the engine wrote it, and
//! changes with the engine.
//!
//! The readers read through the engine's protocol, one set per instance: what an instance
//! records, they read from the instance, so one that is down cannot be read
//! ([`ReadError::Unavailable`]).

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
    /// The instance cannot be reached now (down, restarting); the message says what the link said.
    Unavailable(String),
}

impl std::fmt::Display for ReadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReadError::Missing(m) | ReadError::Unreadable(m) | ReadError::Unavailable(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for ReadError {}

/// One instance's recordings, by session id.
#[async_trait::async_trait]
pub trait Recordings: Send + Sync {
    /// The model calls `session` made in its recording's current run, oldest first: the session's
    /// own, not its sub-agents' (a sub-agent's id reads its own). `index` is the call's place in
    /// the recording of the session's root, so it skips what other sessions of the tree made.
    async fn calls(&self, session: &str) -> Result<Trace, ReadError>;
    /// One call in full, by its `index` in [`Recordings::calls`], with the results its tool calls
    /// got.
    async fn call(&self, session: &str, index: usize) -> Result<CallDetail, ReadError>;
    /// The turn calls summed up per turn.
    async fn turns(&self, session: &str) -> Result<Turns, ReadError>;
    /// What the model was sent besides the messages, once per change, oldest first.
    async fn envelopes(&self, session: &str) -> Result<Envelopes, ReadError>;
    /// What each Agent of each session used this run: the session first, then every sub-agent
    /// under it. Never wakes an instance that sleeps: what was read before, marked `stale`.
    async fn usage(&self, sessions: &[String]) -> Result<BTreeMap<String, SessionUsage>, ReadError>;
}

/// One instance's transcripts.
#[async_trait::async_trait]
pub trait Transcripts: Send + Sync {
    /// A case-insensitive search of the messages of `sessions`, in that order (the caller's: most
    /// recent first), one hit per session, as far as `limits` let it go.
    async fn search(&self, sessions: &[String], query: &str, limits: &SearchLimits) -> Result<SearchResult, ReadError>;
}

/// Which run of the session a reading covers: its recording started over when the process running
/// the session first called the model, so what an earlier process recorded is not in it.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecordingRun {
    /// RFC 3339: when this run's recording began.
    pub created_at: String,
    /// Opaque; equal while it is the same recording.
    pub generation: u64,
    /// This reader saw an earlier run of the same recording go: figures from before are not in these.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub earlier_run: bool,
}

impl RecordingRun {
    pub fn new(created_at: String, generation: u64, earlier_run: bool) -> Self {
        Self { created_at, generation, earlier_run }
    }
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
    /// The run these calls are of; `None` when there is no recording.
    pub run: Option<RecordingRun>,
}

impl Trace {
    pub fn new(recording: bool, calls: Vec<CallSummary>, damaged: Option<usize>) -> Self {
        Self { recording, calls, damaged, run: None }
    }

    pub fn run(mut self, run: Option<RecordingRun>) -> Self {
        self.run = run;
        self
    }
}

/// One call, for a row.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CallSummary {
    /// The call's place in the recording of the session's root.
    pub index: usize,
    pub session_id: String,
    /// Set for a sub-agent's call.
    pub parent_session_id: Option<String>,
    pub agent_type: Option<String>,
    pub turn_id: Option<String>,
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
    /// `ok`, `cancelled`, `error`, `in_flight` or `unfinished`.
    pub status: String,
    /// The engine's: `{kind, message, http_status, retry_after_ms}`.
    pub error: Option<Value>,
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
        session_id: String,
        parent_session_id: Option<String>,
        agent_type: Option<String>,
        turn_id: Option<String>,
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
            session_id,
            parent_session_id,
            agent_type,
            turn_id,
            turn,
            step,
            purpose,
            ts,
            provider,
            model,
            input,
            output,
            tools,
            status: String::new(),
            error: None,
            stop_reason: None,
            usage: None,
            duration_ms: None,
            timing: None,
        }
    }

    pub fn ended(
        mut self,
        status: String,
        error: Option<Value>,
        stop_reason: Option<String>,
        usage: Option<Value>,
        duration_ms: Option<u64>,
    ) -> Self {
        self.status = status;
        self.error = error;
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
    /// The tool catalog's fingerprint: two calls with the same one sent the same tools.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools_digest: Option<String>,
}

impl CallInput {
    pub fn new(system: Vec<BlockSize>, tools: usize, messages: usize) -> Self {
        Self { system, tools, messages, tools_digest: None }
    }

    pub fn with_tools_digest(mut self, digest: Option<String>) -> Self {
        self.tools_digest = digest;
        self
    }
}

/// A system block: which one, and how long.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BlockSize {
    pub source: Option<String>,
    pub chars: usize,
    /// The block's fingerprint: two calls with the same one sent the same block.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub digest: Option<String>,
}

impl BlockSize {
    pub fn new(source: Option<String>, chars: usize) -> Self {
        Self { source, chars, digest: None }
    }

    pub fn with_digest(mut self, digest: Option<String>) -> Self {
        self.digest = digest;
        self
    }
}

/// What a call answered, counted.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CallOutput {
    pub chars: usize,
    pub thinking_chars: usize,
    pub tool_uses: usize,
    /// The start of the answer's text, cut with a `…`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_preview: Option<String>,
}

impl CallOutput {
    pub fn new(chars: usize, thinking_chars: usize, tool_uses: usize) -> Self {
        Self { chars, thinking_chars, tool_uses, text_preview: None }
    }

    pub fn with_text_preview(mut self, preview: Option<String>) -> Self {
        self.text_preview = preview;
        self
    }
}

/// A tool call a call asked for: its input and result are in the call's detail.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolUseSummary {
    pub id: String,
    pub name: String,
    /// The start of its input, as compact JSON, cut with a `…`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_preview: Option<String>,
    /// Whether its result was an error, once the next call has carried the result.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_is_error: Option<bool>,
}

impl ToolUseSummary {
    pub fn new(id: String, name: String) -> Self {
        Self { id, name, input_preview: None, result_is_error: None }
    }

    pub fn with_preview(mut self, input_preview: Option<String>, result_is_error: Option<bool>) -> Self {
        self.input_preview = input_preview;
        self.result_is_error = result_is_error;
        self
    }
}

/// When a call was sent, how long to its first chunk, how long it streamed after it (the engine
/// gives the call's end, not its last chunk), its output rate.
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
    pub session_id: String,
    pub system: Vec<SystemBlock>,
    /// The engine's tool definitions.
    pub tools: Vec<Value>,
    /// The engine's messages, as sent.
    pub messages: Vec<Value>,
    pub model: String,
    pub max_tokens: u32,
    /// The engine's.
    pub thinking_mode: Value,
    /// The engine's; `null` when none was set.
    pub effort: Value,
    pub provider: String,
    pub purpose: Option<String>,
    pub turn: u32,
    pub step: u32,
}

impl CallDetailInput {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        session_id: String,
        system: Vec<SystemBlock>,
        tools: Vec<Value>,
        messages: Vec<Value>,
        model: String,
        max_tokens: u32,
        thinking_mode: Value,
        effort: Value,
        provider: String,
        purpose: Option<String>,
        turn: u32,
        step: u32,
    ) -> Self {
        Self { session_id, system, tools, messages, model, max_tokens, thinking_mode, effort, provider, purpose, turn, step }
    }
}

#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CallDetailOutput {
    pub text: String,
    pub thinking_chars: usize,
    /// `{id, name, input}` each.
    pub tool_uses: Vec<Value>,
    /// What the tool calls got back, as the next call of the same session sent it to the model;
    /// empty when that call is not made yet.
    pub tool_results: Vec<ToolResultView>,
    pub status: String,
    pub error: Option<Value>,
    pub stop_reason: Option<String>,
    pub usage: Option<Value>,
    pub duration_ms: Option<u64>,
}

impl CallDetailOutput {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        text: String,
        thinking_chars: usize,
        tool_uses: Vec<Value>,
        status: String,
        error: Option<Value>,
        stop_reason: Option<String>,
        usage: Option<Value>,
        duration_ms: Option<u64>,
    ) -> Self {
        Self { text, thinking_chars, tool_uses, tool_results: Vec::new(), status, error, stop_reason, usage, duration_ms }
    }

    pub fn results(mut self, results: Vec<ToolResultView>) -> Self {
        self.tool_results = results;
        self
    }
}

/// What one tool call got back, as the next call of the same session sent it to the model.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolResultView {
    pub id: String,
    /// Clipped; `chars` is the whole length.
    pub text: String,
    pub chars: usize,
    pub is_error: bool,
}

impl ToolResultView {
    pub fn new(id: String, text: String, chars: usize, is_error: bool) -> Self {
        Self { id, text, chars, is_error }
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
    pub run: Option<RecordingRun>,
}

impl Turns {
    pub fn new(recording: bool, turns: Vec<TurnSummary>) -> Self {
        Self { recording, turns, run: None }
    }

    pub fn run(mut self, run: Option<RecordingRun>) -> Self {
        self.run = run;
        self
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
    pub run: Option<RecordingRun>,
}

impl Envelopes {
    pub fn new(recording: bool, headers: Vec<Envelope>, calls: u64, auxiliary: u64, damaged: usize) -> Self {
        Self { recording, headers, calls, auxiliary, damaged, run: None }
    }

    pub fn run(mut self, run: Option<RecordingRun>) -> Self {
        self.run = run;
        self
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

/// What one session has used this run, per Agent: its own figures first, then each sub-agent's.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionUsage {
    pub recording: bool,
    pub agents: Vec<AgentUsage>,
    /// The run these figures cover: only the latest, not a total over runs.
    pub run: Option<RecordingRun>,
    /// The instance sleeps: these are the figures read before, not read now.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub stale: bool,
}

impl SessionUsage {
    pub fn new(recording: bool, agents: Vec<AgentUsage>) -> Self {
        Self { recording, agents, run: None, stale: false }
    }

    pub fn run(mut self, run: Option<RecordingRun>) -> Self {
        self.run = run;
        self
    }

    pub fn stale(mut self, stale: bool) -> Self {
        self.stale = stale;
        self
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
    /// RFC 3339.
    pub ts: String,
    /// The message's place in the session's history, when the engine says.
    pub seq: Option<usize>,
}

impl SearchHit {
    pub fn new(session_id: &str, role: &str, snippet: String, ts: String) -> Self {
        Self { session_id: session_id.to_string(), role: role.to_string(), snippet, ts, seq: None }
    }

    pub fn at(mut self, seq: usize) -> Self {
        self.seq = Some(seq);
        self
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
