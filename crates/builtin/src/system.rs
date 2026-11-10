//! The system space's backend: looking into and acting on any instance's sessions, and stopping,
//! restarting and building any instance.
//!
//! Every method here is one call made as the `system` module, with the capabilities the
//! application granted it, through the same admission point a device goes through — so each is
//! decided by the table and on the audit trail as a child of the device's call, and none is a
//! private channel. What Agix does not offer is not offered here either (concept §5.5).

use harnix_contract::RpcError;
use serde_json::{json, Value};

use crate::{require_session_id, Builtin};

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
            .unwrap_or("sessions");
        if space == "system" {
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
