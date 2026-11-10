//! `harnix.system.usage`: what each Agent of a session has used, as the engine's recordings
//! reader sums it (`harnix_engine::readers`): per Agent, its calls and turns, the models it used,
//! its tokens and the time the model took. The reader keeps what it folded, and reads on from where
//! it stopped as a running session's recording grows.
//!
//! Like the trace, it covers the session's current run (see `recordings.rs`).

use harnix_contract::RpcError;
use serde_json::{json, Value};

use crate::Builtin;

/// How many sessions one call may ask about.
const MAX_SESSIONS: usize = 200;

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
        // An empty list asks nothing of the instance, whichever it names.
        let out = if ids.is_empty() {
            serde_json::Map::new()
        } else {
            let reader = self.recordings(instance)?;
            let read = reader.usage(&ids).await.map_err(crate::recordings::read_error)?;
            read.into_iter()
                .map(|(id, usage)| (id, serde_json::to_value(usage).unwrap_or(Value::Null)))
                .collect()
        };
        Ok(json!({"instance": instance, "sessions": out}))
    }
}
