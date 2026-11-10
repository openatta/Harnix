//! The system space's view of a session's model calls, read back out of the engine's recording.
//!
//! What the model was sent and what came back, call by call, summed per turn, and the request
//! envelope's changes. The engine writes every call to a recording; its reader
//! (`harnix_engine::readers`) parses it, and this answers the methods with what it read.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use harnix_contract::sync::lock;
use harnix_contract::{codes, RpcError};
use harnix_engine::readers::RecordingsDir;
use harnix_engine_api::readers::{ReadError, Recordings};
use serde::Serialize;
use serde_json::Value;

use crate::{require_session_id, Builtin};

impl Builtin {
    /// Where an instance writes a session's recording.
    pub(crate) fn recording_dir(&self, instance: &str, session_id: &str) -> Result<PathBuf, RpcError> {
        Ok(self.recordings_root(instance)?.join(session_id))
    }

    pub(crate) fn recordings_root(&self, instance: &str) -> Result<PathBuf, RpcError> {
        let handle = self.instance(instance).ok_or_else(|| {
            RpcError::new(
                codes::INSTANCE_NOT_FOUND,
                format!("there is no instance `{instance}`"),
            )
        })?;
        Ok(handle.recordings_root)
    }

    /// The reader of an instance's recordings: one per root, kept, so what it read is kept too.
    pub(crate) fn recordings(&self, instance: &str) -> Result<Arc<dyn Recordings>, RpcError> {
        let root = self.recordings_root(instance)?;
        Ok(self.traces.reader(&root))
    }

    /// `harnix.requestHeaders {instance?, session_id}` — the envelope timeline of this
    /// session's current run, oldest first, one entry per change.
    pub(crate) async fn request_headers(&self, params: Value) -> Result<Value, RpcError> {
        let (reader, session) = self.recording_of(&params)?;
        answer(reader.envelopes(&session).await)
    }
    /// `harnix.system.trace {instance?, session_id}` — every model call of this session's current
    /// run, oldest first, each with what went in and what came out, and the tool calls it asked
    /// for with their results (found in the next call's messages). The system space's calls tab.
    pub(crate) async fn trace(&self, params: Value) -> Result<Value, RpcError> {
        let (reader, session) = self.recording_of(&params)?;
        answer(reader.calls(&session).await)
    }

    /// `harnix.system.turns {instance?, session_id}` — the trace summed up per turn: how many calls,
    /// the time to first token, model and tool time, tokens in and out, the cache's share of what
    /// went in (by tokens, so one cold call is not hidden by warm ones), and how much the context
    /// grew since the turn before. The system space's per-turn tab.
    pub(crate) async fn turns(&self, params: Value) -> Result<Value, RpcError> {
        let (reader, session) = self.recording_of(&params)?;
        answer(reader.turns(&session).await)
    }

    /// `harnix.system.trace.call {instance?, session_id, index}` — one call in full: every system
    /// block, the tool catalog, the messages as sent, and the whole answer.
    pub(crate) async fn trace_call(&self, params: Value) -> Result<Value, RpcError> {
        let (reader, session) = self.recording_of(&params)?;
        let index = params.get("index").and_then(Value::as_u64).ok_or_else(|| {
            RpcError::invalid_params("`index` is the call's position in the trace")
        })?;
        answer(reader.call(&session, index as usize).await)
    }

    /// The reader and the session of `{instance?, session_id}`.
    fn recording_of(&self, params: &Value) -> Result<(Arc<dyn Recordings>, String), RpcError> {
        let session_id = require_session_id(params)?;
        let instance = params
            .get("instance")
            .and_then(Value::as_str)
            .unwrap_or(harnix_contract::SYSTEM_INSTANCE);
        Ok((self.recordings(instance)?, session_id))
    }
}

/// A reader's answer as the method's: what it read, or the error a client is told.
pub(crate) fn answer<T: Serialize>(read: Result<T, ReadError>) -> Result<Value, RpcError> {
    let read = read.map_err(read_error)?;
    serde_json::to_value(read).map_err(|e| RpcError::new(codes::INTERNAL_ERROR, e.to_string()))
}

pub(crate) fn read_error(e: ReadError) -> RpcError {
    match e {
        ReadError::Missing(m) => RpcError::missing(m),
        ReadError::Unreadable(m) => RpcError::new(codes::INTERNAL_ERROR, m),
        other => RpcError::new(codes::INTERNAL_ERROR, other.to_string()),
    }
}

/// The recordings readers, by root.
#[derive(Default)]
pub(crate) struct Traces(Mutex<HashMap<PathBuf, Arc<RecordingsDir>>>);

impl Traces {
    fn reader(&self, root: &Path) -> Arc<dyn Recordings> {
        lock(&self.0)
            .entry(root.to_path_buf())
            .or_insert_with(|| Arc::new(RecordingsDir::new(root)))
            .clone()
    }
}
