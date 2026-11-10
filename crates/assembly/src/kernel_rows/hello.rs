//! `harnix.hello`: what a client is told once, after the handshake — what the `system` engine is,
//! what scenes and commands exist there, every instance and the protocol it speaks, the ceilings,
//! the shell's part, and every extension (what the shell activates its UI packages by).
//!
//! The assembly's to answer: it is the one layer that knows all of it.

use std::sync::Arc;

use harnix_hub::{Hub, SYSTEM};
use harnix_instance::InstanceRegistry;
use harnix_kernel::Rpc;
use serde_json::{json, Value};

use super::UiModules;

pub(crate) struct Hello {
    pub hub: Arc<Hub>,
    pub instances: Arc<InstanceRegistry>,
    /// The ceilings a client has to respect, assembled once by the layer that knows all three.
    /// Reported rather than restated: `hello` used to build this object by hand and left
    /// `max_upload_bytes` out of it entirely.
    pub limits: Value,
    /// How long a permission ask waits for an answer before it is denied (R-12): a client shows
    /// the countdown.
    pub prompt_timeout_secs: u64,
    /// What the shell is told about this product: its brand, which built-in spaces it keeps,
    /// whether runtime interface modules load.
    pub shell: Value,
    pub ui: Arc<UiModules>,
}

impl Hello {
    pub async fn answer(&self) -> Rpc<Value> {
        // Once per connection, and none of the three waits on another.
        let (status, scenes, commands) = tokio::join!(
            self.hub.passthrough(SYSTEM, "engine.daemon.status", json!({})),
            self.hub.passthrough(SYSTEM, "engine.scene.list", json!({})),
            self.hub.passthrough(SYSTEM, "engine.commands.list", json!({})),
        );
        let (status, scenes, commands) = (
            status.unwrap_or(json!({})),
            scenes.unwrap_or(json!({})),
            commands.unwrap_or(json!({})),
        );
        let facts = self.instances.facts(SYSTEM).map(|f| f.engine).unwrap_or_default();
        Ok(json!({
            "protocol_version": harnix_contract::PROTOCOL_VERSION,
            "contrib_api_version": harnix_contract::CONTRIB_API_VERSION,
            // The two parts that change with the engine — a session event's payload, an `engine.*`
            // answer — are its shape, not this protocol's: which shape is this version.
            "engine_protocol": harnix_engine::protocol_version(),
            "engine": {
                "model": facts.model,
                "active_scenes": facts.active_scenes,
                "scene": facts.scene,
                "data_root": facts.root_label(),
                "has_credentials": facts.has_credentials,
                "status": status,
            },
            "instances": self.instances.identities(),
            "permission_prompt_timeout_secs": self.prompt_timeout_secs,
            "scenes": scenes.get("scenes").cloned().unwrap_or(json!([])),
            "commands": commands.get("commands").cloned().unwrap_or(json!([])),
            // What a client fills in when it names none: the instance, and the space (none until
            // a product names one).
            "defaults": { "instance": SYSTEM, "space": self.hub.default_space() },
            // Every extension and whether it is enabled: the shell activates the UI packages
            // compiled into it by this, before its first draw. What changes later, and the
            // interface modules to load, are `harnix.ui`'s.
            "extensions": self.ui.extensions(),
            "shell": self.shell,
            "limits": self.limits,
        }))
    }
}
