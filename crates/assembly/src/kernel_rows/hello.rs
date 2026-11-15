//! `harnix.hello`: what a client is told once, after the handshake — what the `main` engine is
//! and the models it can run a session on, what scenes and commands exist there, every instance and the protocol it speaks, the ceilings,
//! the shell's part, and every extension (what the shell activates its UI packages by).
//!
//! The assembly's to answer: it is the one layer that knows all of it.

use std::sync::Arc;

use harnix_hub::{Hub, MAIN};
use harnix_instance::InstanceRegistry;
use harnix_kernel::Rpc;
use serde_json::{json, Map, Value};

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
        // Once per connection, and none of the four waits on another.
        let (status, scenes, commands, providers) = tokio::join!(
            self.hub.passthrough(MAIN, "engine.daemon.status", json!({})),
            self.hub.passthrough(MAIN, "engine.scene.list", json!({})),
            self.hub.passthrough(MAIN, "engine.commands.list", json!({})),
            self.hub.passthrough(MAIN, "engine.config.getProvider", json!({})),
        );
        let (status, scenes, commands) = (
            status.unwrap_or(json!({})),
            scenes.unwrap_or(json!({})),
            commands.unwrap_or(json!({})),
        );
        let facts = self.instances.facts(MAIN).map(|f| f.engine).unwrap_or_default();
        Ok(json!({
            "protocol_version": harnix_contract::PROTOCOL_VERSION,
            "contrib_api_version": harnix_contract::CONTRIB_API_VERSION,
            // The two parts that change with the engine — a session event's payload, an `engine.*`
            // answer — are its shape, not this protocol's: which shape is this version.
            "engine_protocol": harnix_engine::AGIX_PROTOCOL,
            "engine": {
                "model": facts.model,
                "active_scenes": facts.active_scenes,
                "scene": facts.scene,
                "data_root": facts.root_label(),
                "has_credentials": facts.has_credentials,
                "providers": model_choices(providers.ok()),
                "status": status,
            },
            "instances": self.instances.identities(),
            "permission_prompt_timeout_secs": self.prompt_timeout_secs,
            "scenes": scenes.get("scenes").cloned().unwrap_or(json!([])),
            "commands": commands.get("commands").cloned().unwrap_or(json!([])),
            // What a client fills in when it names none: the instance, and the space (none until
            // a product names one).
            "defaults": { "instance": MAIN, "space": self.hub.default_space() },
            // Every extension and whether it is enabled: the shell activates the UI packages
            // compiled into it by this, before its first draw. What changes later, and the
            // interface modules to load, are `harnix.ui`'s.
            "extensions": self.ui.extensions(),
            "shell": self.shell,
            "limits": self.limits,
        }))
    }
}

/// What `engine.config.getProvider` answers, cut to what a client offers to switch a session to:
/// each provider's default model and the others it allows, and which provider is the default —
/// no key, no address. `null` when it could not be read: a client then does not say that no model
/// is set up.
fn model_choices(answer: Option<Value>) -> Value {
    let Some(answer) = answer else {
        return Value::Null;
    };
    let providers: Map<String, Value> = answer["providers"]
        .as_object()
        .into_iter()
        .flatten()
        .map(|(id, p)| {
            let models = p.get("models").filter(|m| m.is_array()).cloned().unwrap_or(json!([]));
            (id.clone(), json!({"default_model": p["default_model"].clone(), "models": models}))
        })
        .collect();
    json!({"providers": providers, "default_provider": answer["default_provider"].clone()})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_client_is_told_the_models_and_nothing_of_how_they_are_reached() {
        let answer = json!({
            "default_provider": "acme",
            "providers": {
                "acme": {"api_type": "openai_compatible", "base_url": "https://acme.invalid/v1", "api_key": "sk-…", "default_model": "m-1", "models": ["m-2"]},
                "bare": {"api_type": "anthropic"},
            },
        });
        assert_eq!(
            model_choices(Some(answer)),
            json!({
                "default_provider": "acme",
                "providers": {"acme": {"default_model": "m-1", "models": ["m-2"]}, "bare": {"default_model": null, "models": []}},
            })
        );
        assert_eq!(model_choices(None), Value::Null);
    }
}
