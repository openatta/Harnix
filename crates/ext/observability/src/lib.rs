//! A session's model calls, read back out of the engine's recording.
//!
//! What the model was sent and what came back, call by call, summed per turn, what each Agent of
//! a session has used, and the request envelope's changes. The engine writes every call to a
//! recording; its reader (`harnix_ext::engine::readers`) reads it through the engine's protocol, and
//! these methods answer with what it read. The conversation's trajectory view and the right column's
//! usage tab read them for the open session, the system space's inspector for any session.
//!
//! A session's rows are a device's for the sessions of the space it claims (`RowScope::Session`):
//! before a recording is read, the session is asked of the engine as the caller, and the hub judges
//! the claim as it does for every read of a session (system-space.md §10). `usage` names many
//! sessions at once and stays an open row, as the system space's console reads it.

use std::sync::Arc;

use harnix_ext::engine::readers::{ReadError, Recordings};
use harnix_ext::{caps, codes, Access, Call, Extension, Manifest, Registrar, Rpc, RpcError, RowScope, MAIN_INSTANCE};
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
        // A person's rows about one session: for the sessions of the space the call claims.
        let session = || Access::device().scoped(RowScope::Session);
        r.method("trace")
            .access(session())
            .describe("every model call of a session's current run, oldest first, with the tool calls each asked for")
            .raw(|call, params| async move {
                let (reader, session) = recording_of(&call, &params).await?;
                answer(reader.calls(&session).await)
            });
        r.method("trace.call")
            .access(session())
            .describe("one model call in full: the system blocks, the tool catalog, the messages as sent, the answer, and what its tool calls got back")
            .raw(|call, params| async move {
                let (reader, session) = recording_of(&call, &params).await?;
                let index = params
                    .get("index")
                    .and_then(Value::as_u64)
                    .ok_or_else(|| RpcError::invalid_params("`index` is the call's position in the trace"))?;
                answer(reader.call(&session, index as usize).await)
            });
        r.method("turns")
            .access(session())
            .describe("the trace summed up per turn: time to first token, tokens, the cache's share, the context's growth")
            .raw(|call, params| async move {
                let (reader, session) = recording_of(&call, &params).await?;
                answer(reader.turns(&session).await)
            });
        r.method("usage")
            .access(Access::device())
            .describe("what each Agent of each session has used this run (the latest, not a total); a sleeping instance is not woken, and what it read before is marked stale")
            .raw(|call, params| async move { usage(&call, &params).await });
        r.method("calls")
            .access(session())
            .describe("the request envelope's timeline of a session's current run, one entry per change")
            .raw(|call, params| async move {
                let (reader, session) = recording_of(&call, &params).await?;
                answer(reader.envelopes(&session).await)
            });
    }
}

/// `{instance?, session_ids}` → `{instance, sessions: {<id>: {recording, agents, run, stale?}}}`;
/// `agents` is the session's own figures first, then each sub-agent's with its parent.
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
    params.get("instance").and_then(Value::as_str).unwrap_or(MAIN_INSTANCE)
}

/// The reader of an instance's recordings.
fn recordings(call: &Call, instance: &str) -> Rpc<Arc<dyn Recordings>> {
    call.ext()
        .engine(instance)
        .ok_or_else(|| RpcError::new(codes::INSTANCE_NOT_FOUND, format!("there is no instance `{instance}`")))?
        .recordings()
}

/// The session a read is about, as the engine is asked for it: `{session_id, instance?, space?}`
/// and nothing else the caller sent — the claim goes with it, for the hub to judge.
fn session_target(params: &Value) -> Rpc<Value> {
    let session = params
        .get("session_id")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| RpcError::invalid_params("missing session_id"))?;
    let mut target = json!({"session_id": session});
    for key in ["instance", "space"] {
        if let Some(value) = params.get(key).filter(|v| v.is_string()) {
            target[key] = value.clone();
        }
    }
    Ok(target)
}

/// The reader and the session of `{instance?, session_id, space?}`, once the engine has answered
/// for the session as the caller: a session the caller may not see is refused before its recording
/// is touched, by the hub's own words.
async fn recording_of(call: &Call, params: &Value) -> Rpc<(Arc<dyn Recordings>, String)> {
    let target = session_target(params)?;
    call.call("engine.session.get", target.clone()).await?;
    let session = target["session_id"].as_str().unwrap_or_default().to_string();
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
        ReadError::Unavailable(m) => RpcError::new(codes::INSTANCE_UNAVAILABLE, m),
        other => RpcError::new(codes::INTERNAL_ERROR, other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use harnix_ext::{ErasedExtension, ExtId};

    /// The rows about one session are a device's, narrowed to the sessions of the space it claims;
    /// `usage`, which names many sessions, is a device's and open.
    #[test]
    fn a_sessions_rows_are_a_devices_for_the_sessions_of_its_space() {
        let id = ExtId::native("observability").unwrap();
        let described = ErasedExtension::register(&Observability, &id, &Value::Null).unwrap();
        let access = |name: &str| {
            described
                .methods()
                .iter()
                .find(|m| m.name() == name)
                .unwrap_or_else(|| panic!("no method {name}"))
                .access()
                .clone()
        };
        for name in ["observability.trace", "observability.trace.call", "observability.turns", "observability.calls"] {
            let row = access(name);
            assert!(row.opens_devices(), "{name} is a device's");
            assert!(!row.opens_every_extension(), "{name} is not an extension's");
            assert_eq!(row.scope(), RowScope::Session, "{name} is about one session");
        }
        assert!(access("observability.usage").opens_devices());
        assert_eq!(access("observability.usage").scope(), RowScope::Open);
    }

    /// What the engine is asked about the session: the session, the instance and the space the
    /// caller claimed, nothing else it sent.
    #[test]
    fn the_session_is_asked_of_the_engine_with_the_callers_claim() {
        let target = session_target(&json!({"session_id": "S1", "instance": "side", "space": "acme", "index": 3})).unwrap();
        assert_eq!(target, json!({"session_id": "S1", "instance": "side", "space": "acme"}));
        assert_eq!(session_target(&json!({"session_id": "S1", "space": null})).unwrap(), json!({"session_id": "S1"}));
        assert!(session_target(&json!({"instance": "side"})).is_err());
        assert!(session_target(&json!({"session_id": ""})).is_err());
    }
}
