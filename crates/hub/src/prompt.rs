//! `harnix.session.setPrompt`: a session's own system-prompt text, kept behind its scene layer's.
//!
//! The engine keeps one text a session adds at the end of its prompt. A session created in a scene
//! layer has the layer's there already, so a caller does not set it straight on the engine
//! (`engine.session.setPrompt` is the hub's): the hub keeps the caller's own text in its ledger and
//! sends the engine the layer's text and the caller's, one after the other. The text replaces the
//! settings' `prompt_append` for that session — the engine's rule for a session's own text.

use std::sync::Arc;

use harnix_contract::sync::lock;
use harnix_contract::RpcError;
use serde_json::{json, Value};

use crate::Hub;

/// What a call says of one of the two texts.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Say {
    /// Not given: left as it is.
    Keep,
    /// `null`: cleared.
    Clear,
    Set(String),
}

impl Say {
    fn read(params: &Value, key: &str) -> Result<Say, RpcError> {
        match params.get(key) {
            None => Ok(Say::Keep),
            Some(Value::Null) => Ok(Say::Clear),
            Some(Value::String(text)) => Ok(Say::Set(text.clone())),
            Some(other) => Err(RpcError::invalid_params(format!("`{key}` is a string or null, not {other}"))),
        }
    }

    /// What the engine is sent for it: `None` not to send it.
    fn sent(self) -> Option<Option<String>> {
        match self {
            Say::Keep => None,
            Say::Clear => Some(None),
            Say::Set(text) => Some(Some(text)),
        }
    }
}

/// The layer's text and the caller's, one after the other; `None` when neither has any.
fn joined(layer: Option<&str>, own: Option<&str>) -> Option<String> {
    let parts: Vec<&str> = [layer, own].into_iter().flatten().filter(|t| !t.trim().is_empty()).collect();
    (!parts.is_empty()).then(|| parts.join("\n\n"))
}

impl Hub {
    /// `harnix.session.setPrompt {instance?, session_id, space?, append?, override?}`: `append` is
    /// the caller's own text, put after the scene layer's; `override` replaces the whole prompt.
    /// Each: a string sets it, `null` clears it, left out leaves it. A session with a turn or a
    /// task running is refused as the engine refuses it (`SESSION_BUSY`), and nothing changes.
    pub(crate) async fn set_prompt(self: &Arc<Self>, params: Value) -> Result<Value, RpcError> {
        let key = self.key_of(&params)?;
        let (append, override_text) = (Say::read(&params, "append")?, Say::read(&params, "override")?);
        if append == Say::Keep && override_text == Say::Keep {
            return Err(RpcError::invalid_params("give `append`, `override`, or both"));
        }
        let linked = self.engine(&key.0)?;
        // One at a time, so the ledger and the engine change in the same order.
        let _one = self.prompting.lock().await;
        let (layer_id, kept) = {
            let spaces = lock(&self.spaces);
            (spaces.layer_of(&key).map(str::to_string), spaces.prompt_of(&key).map(str::to_string))
        };
        let layers = linked.link.scene_layers();
        let layer_text = layer_id.as_deref().and_then(|id| layers.get(id)).and_then(|l| l.prompt_append());
        let own = match &append {
            Say::Keep => kept,
            Say::Clear => None,
            Say::Set(text) => Some(text.clone()).filter(|t| !t.trim().is_empty()),
        };
        // Cleared and with no layer text: back to the settings'.
        let sent_append = match append {
            Say::Keep => None,
            _ => Some(joined(layer_text.as_deref(), own.as_deref())),
        };
        let changed = sent_append.is_some();
        let mut answer = linked.ops().set_prompt(&key.1, sent_append, override_text.sent()).await?;
        if changed {
            lock(&self.spaces).set_prompt(&key, own.clone());
        }
        if let Some(obj) = answer.as_object_mut() {
            // The caller is told of its own text; the layer's is beside it.
            obj.insert("append".into(), json!(own));
            obj.insert("scene_append".into(), json!(layer_text));
        }
        Ok(answer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_engine_is_sent_the_layers_text_then_the_callers() {
        assert_eq!(joined(Some("layer"), Some("mine")).as_deref(), Some("layer\n\nmine"));
        assert_eq!(joined(None, Some("mine")).as_deref(), Some("mine"));
        assert_eq!(joined(Some("layer"), None).as_deref(), Some("layer"));
        assert_eq!(joined(None, Some("  ")), None);
        assert_eq!(Say::read(&json!({"append": null}), "append").unwrap(), Say::Clear);
        assert_eq!(Say::read(&json!({}), "append").unwrap(), Say::Keep);
        assert!(Say::read(&json!({"append": 1}), "append").is_err());
    }
}
