//! What the engine answers `session.calls`, `session.call`, `session.callBlob` and `session.search`
//! with, as far as it is used here: the protocol's shapes, read leniently — a field it adds is
//! ignored, one it leaves out is the default.

use harnix_contract::{codes, RpcError};
use harnix_engine_link::ops::codes as engine_codes;
use harnix_engine_api::readers::ReadError;
use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize, Default)]
#[serde(default)]
pub(super) struct CallList {
    pub recording: Option<RunInfo>,
    pub calls: Vec<Row>,
    pub next: usize,
    pub total: usize,
    pub damaged: usize,
}

#[derive(Deserialize, Default, Clone, PartialEq, Debug)]
#[serde(default)]
pub(super) struct RunInfo {
    pub created_at: String,
    pub generation: u64,
}

#[derive(Deserialize, Default, Clone)]
#[serde(default)]
pub(super) struct Row {
    pub index: usize,
    pub session_id: String,
    pub parent_session_id: Option<String>,
    pub agent_type: Option<String>,
    pub turn_id: Option<String>,
    pub turn: u32,
    pub step: u32,
    pub purpose: Option<String>,
    pub ts: String,
    pub provider: String,
    pub model: String,
    pub input: RowInput,
    pub output: RowOutput,
    pub status: String,
    pub stop_reason: Option<String>,
    pub usage: Option<Value>,
    pub timing: RowTiming,
    pub error: Option<Value>,
}

#[derive(Deserialize, Default, Clone)]
#[serde(default)]
pub(super) struct RowInput {
    pub system: Vec<BlockSizeRow>,
    pub tools: Option<usize>,
    pub messages: usize,
}

#[derive(Deserialize, Default, Clone, PartialEq, Debug)]
#[serde(default)]
pub(super) struct BlockSizeRow {
    pub source: Option<String>,
    pub chars: Option<usize>,
}

#[derive(Deserialize, Default, Clone)]
#[serde(default)]
pub(super) struct RowOutput {
    pub chars: usize,
    pub thinking_chars: usize,
    pub tool_uses: Vec<ToolRef>,
}

#[derive(Deserialize, Default, Clone)]
#[serde(default)]
pub(super) struct ToolRef {
    pub id: String,
    pub name: String,
}

#[derive(Deserialize, Default, Clone)]
#[serde(default)]
pub(super) struct RowTiming {
    pub sent_at: String,
    pub ttft_ms: Option<u64>,
    pub duration_ms: Option<u64>,
}

/// `session.call`: the parts are kept as JSON, for [`super::trace`] to resolve references in.
#[derive(Deserialize, Default)]
#[serde(default)]
pub(super) struct Detail {
    pub request: Request,
    pub response: Response,
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub(super) struct Request {
    pub session_id: String,
    pub turn: u32,
    pub step: u32,
    pub purpose: Option<String>,
    pub provider: String,
    pub model: String,
    pub max_tokens: u32,
    pub thinking_mode: Value,
    pub effort: Value,
    pub system: Vec<Value>,
    pub tools: Value,
    pub messages: Vec<Value>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub(super) struct Response {
    pub status: String,
    pub text: String,
    pub thinking_chars: usize,
    pub tool_uses: Vec<Value>,
    pub stop_reason: Option<String>,
    pub usage: Option<Value>,
    pub duration_ms: Option<u64>,
    pub error: Option<Value>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub(super) struct Blob {
    pub value: Value,
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub(super) struct SearchAnswer {
    pub hits: Vec<HitRow>,
    pub scanned: usize,
    pub truncated: bool,
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub(super) struct HitRow {
    pub session_id: String,
    pub seq: usize,
    pub origin: String,
    pub ts: String,
    pub snippet: String,
}

/// What a read through the link failed with, as a reader says it.
pub(super) fn read_error(e: RpcError) -> ReadError {
    match e.code {
        codes::INSTANCE_UNAVAILABLE => ReadError::Unavailable(e.message),
        engine_codes::SESSION_NOT_FOUND | codes::INVALID_PARAMS => ReadError::Missing(e.message),
        _ => ReadError::Unreadable(e.message),
    }
}

/// The generation the recording has now, when the call was refused because the one it named is
/// gone.
pub(super) fn restarted(e: &RpcError) -> Option<u64> {
    if e.code != codes::INVALID_PARAMS {
        return None;
    }
    e.data.as_ref()?.get("generation")?.as_u64()
}

/// Reads `value` as `T`, or says it does not read.
pub(super) fn parse<T: serde::de::DeserializeOwned>(value: Value, what: &str) -> Result<T, ReadError> {
    serde_json::from_value(value).map_err(|e| ReadError::Unreadable(format!("the engine's answer to {what} does not read: {e}")))
}
