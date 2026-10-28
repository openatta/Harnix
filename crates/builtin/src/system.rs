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
        "harnix.system.tasks.output",
        "session.taskOutput",
        &["task_id", "tail_bytes"],
    ),
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
        if name == "harnix.system.trace" {
            return self.trace(params).await;
        }
        if name == "harnix.system.trace.call" {
            return self.trace_call(params).await;
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

impl Builtin {
    /// `harnix.sessions.setModel { instance?, session_id, model, provider?, space? }`: the engine's
    /// `session.setModel` on a session of the space the caller claims (the session space when it
    /// claims none), checked the way the hub checks a session call. Never across spaces — that is
    /// the system space's, through `harnix.system.model` (ipdrun-12).
    pub(crate) async fn session_set_model(&self, params: Value) -> Result<Value, RpcError> {
        let session_id = require_session_id(&params)?;
        let model = crate::require_str(&params, "model")?;
        let instance = params
            .get("instance")
            .cloned()
            .unwrap_or(json!(harnix_contract::SYSTEM_INSTANCE));
        let space = params
            .get("space")
            .and_then(Value::as_str)
            .unwrap_or(harnix_contract::SESSIONS_SPACE);
        if space == harnix_contract::SYSTEM_SPACE {
            return Err(RpcError::new(
                harnix_contract::codes::SPACE_MISMATCH,
                "harnix.sessions.setModel acts within one space; across spaces it is harnix.system.model",
            ));
        }
        // The hub refuses a session of another space here, with SPACE_MISMATCH and where it is.
        self.call(
            "model",
            "session.get",
            json!({"instance": instance, "session_id": session_id, "space": space}),
        )
        .await?;
        let mut out = json!({"instance": instance, "session_id": session_id, "model": model});
        if let Some(provider) = params.get("provider").filter(|p| !p.is_null()) {
            out["provider"] = provider.clone();
        }
        self.call("model", "session.setModel", out).await
    }
}
