//! The system space's backend (design §9): looking into and acting on any instance's sessions.
//!
//! Every method here is one engine call made as the `system` module, through the same admission
//! point a device goes through — so each is decided by the table and on the audit trail, and none
//! is a private channel. What Agix does not offer is not offered here either (§9.3).

use harnix_contract::RpcError;
use serde_json::{json, Value};

use crate::{require_session_id, Builtin};

/// What each `harnix.system.*` method asks the engine, and which parameters it passes on.
const ENGINE: &[(&str, &str, &[&str])] = &[
    ("harnix.system.context", "session.inspectContext", &[]),
    ("harnix.system.tasks.stop", "session.stopTask", &["task_id"]),
    (
        "harnix.system.messages.drop",
        "session.dropMessages",
        &["at", "total"],
    ),
    (
        "harnix.system.messages.edit",
        "session.editMessage",
        &["at", "text", "total"],
    ),
    ("harnix.system.compact", "session.compact", &[]),
    ("harnix.system.clear", "session.clear", &[]),
    (
        "harnix.system.model",
        "session.setModel",
        &["model", "provider"],
    ),
    ("harnix.system.rename", "session.rename", &["name"]),
];

impl Builtin {
    pub(crate) async fn system_call(&self, name: &str, params: Value) -> Result<Value, RpcError> {
        if name == "harnix.system.calls" {
            return self.request_headers(params).await;
        }
        let Some((_, method, keys)) = ENGINE.iter().find(|(n, _, _)| *n == name) else {
            return Err(RpcError::not_found(format!(
                "`{name}` is not a system method"
            )));
        };
        let mut out = json!({
            "instance": params.get("instance").cloned().unwrap_or(json!(harnix_contract::SYSTEM_INSTANCE)),
            "session_id": require_session_id(&params)?,
        });
        for key in *keys {
            if let Some(value) = params.get(*key) {
                out[*key] = value.clone();
            }
        }
        self.call("system", method, out).await
    }
}
