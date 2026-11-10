//! A session's model calls, read back out of the engine's recording.
//!
//! What the model was sent and what came back, call by call, summed per turn, what each Agent of
//! a session has used, and the request envelope's changes. The engine writes every call to a
//! recording; its reader (`harnix_ext::engine::readers`) parses it, and these methods answer with
//! what it read. The system space's inspector tabs are the first reader of them.

use std::sync::Arc;

use harnix_ext::engine::readers::{ReadError, Recordings};
use harnix_ext::{caps, codes, Access, Call, Extension, Manifest, Registrar, Rpc, RpcError, SYSTEM_INSTANCE};
use serde::Serialize;
use serde_json::{json, Value};

/// How many sessions one `usage` call may ask about.
const MAX_SESSIONS: usize = 200;

#[derive(Default)]
pub struct Observability;

#[async_trait::async_trait]
impl Extension for Observability {
    type Config = ();

    fn manifest(&self) -> Manifest {
        Manifest::new("observability", env!("CARGO_PKG_VERSION"))
            .title("Observability")
            .disclosure("读本机的模型调用录像：每次调用的请求与应答、每轮的汇总、会话的用量、请求信封的变化")
            .requires([caps::RECORDINGS_READ])
    }

    fn register(&self, r: &mut Registrar<'_, ()>) {
        r.method("trace")
            .access(Access::device())
            .describe("every model call of a session's current run, oldest first, with its tool calls and their results")
            .raw(|call, params| async move {
                let (reader, session) = recording_of(&call, &params)?;
                answer(reader.calls(&session).await)
            });
        r.method("trace.call")
            .access(Access::device())
            .describe("one model call in full: the system blocks, the tool catalog, the messages as sent, the answer")
            .raw(|call, params| async move {
                let (reader, session) = recording_of(&call, &params)?;
                let index = params
                    .get("index")
                    .and_then(Value::as_u64)
                    .ok_or_else(|| RpcError::invalid_params("`index` is the call's position in the trace"))?;
                answer(reader.call(&session, index as usize).await)
            });
        r.method("turns")
            .access(Access::device())
            .describe("the trace summed up per turn: time to first token, tokens, the cache's share, the context's growth")
            .raw(|call, params| async move {
                let (reader, session) = recording_of(&call, &params)?;
                answer(reader.turns(&session).await)
            });
        r.method("usage")
            .access(Access::device())
            .describe("what each Agent of each session has used")
            .raw(|call, params| async move { usage(&call, &params).await });
        r.method("calls")
            .access(Access::device())
            .describe("the request envelope's timeline of a session's current run, one entry per change")
            .raw(|call, params| async move {
                let (reader, session) = recording_of(&call, &params)?;
                answer(reader.envelopes(&session).await)
            });
    }
}

/// `{instance?, session_ids}` → `{instance, sessions: {<id>: {recording, agents}}}`; `agents` is
/// the session's own figures first, then each sub-agent's with its parent.
async fn usage(call: &Call, params: &Value) -> Rpc<Value> {
    let instance = instance_of(params);
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
    // An empty list asks nothing of the instance, whichever it names.
    let out = if ids.is_empty() {
        serde_json::Map::new()
    } else {
        let read = recordings(call, instance)?.usage(&ids).await.map_err(read_error)?;
        read.into_iter()
            .map(|(id, usage)| (id, serde_json::to_value(usage).unwrap_or(Value::Null)))
            .collect()
    };
    Ok(json!({"instance": instance, "sessions": out}))
}

fn instance_of(params: &Value) -> &str {
    params.get("instance").and_then(Value::as_str).unwrap_or(SYSTEM_INSTANCE)
}

/// The reader of an instance's recordings.
fn recordings(call: &Call, instance: &str) -> Rpc<Arc<dyn Recordings>> {
    call.ext()
        .engine(instance)
        .ok_or_else(|| RpcError::new(codes::INSTANCE_NOT_FOUND, format!("there is no instance `{instance}`")))?
        .recordings()
}

/// The reader and the session of `{instance?, session_id}`.
fn recording_of(call: &Call, params: &Value) -> Rpc<(Arc<dyn Recordings>, String)> {
    let session = params
        .get("session_id")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| RpcError::invalid_params("missing session_id"))?;
    Ok((recordings(call, instance_of(params))?, session))
}

/// A reader's answer as the method's: what it read, or the error a client is told.
fn answer<T: Serialize>(read: Result<T, ReadError>) -> Rpc<Value> {
    let read = read.map_err(read_error)?;
    serde_json::to_value(read).map_err(|e| RpcError::new(codes::INTERNAL_ERROR, e.to_string()))
}

fn read_error(e: ReadError) -> RpcError {
    match e {
        ReadError::Missing(m) => RpcError::missing(m),
        ReadError::Unreadable(m) => RpcError::new(codes::INTERNAL_ERROR, m),
        other => RpcError::new(codes::INTERNAL_ERROR, other.to_string()),
    }
}
