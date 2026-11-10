//! `harnix.hello`: what a client is told once, after the handshake — what the `system` engine is,
//! what scenes and commands exist there, every instance and the protocol it speaks, the ceilings,
//! the shell's part, and the interface modules the packages and applications contribute.
//!
//! The assembly's to answer: it is the one layer that knows all of it.

use std::sync::{Arc, Mutex};

use harnix_contract::sync::lock;
use harnix_hub::{Hub, SYSTEM};
use harnix_instance::InstanceRegistry;
use harnix_kernel::Rpc;
use serde_json::{json, Value};

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
            self.hub.call(SYSTEM, "daemon.status", json!({})),
            self.hub.call(SYSTEM, "scene.list", json!({})),
            self.hub.call(SYSTEM, "commands.list", json!({})),
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
            // Interface modules the installed packages contribute, flat and
            // in load order. An array, not a map by point: the client imports
            // each one and the module decides what it registers, so grouping
            // by point here would be a shape nothing reads.
            "contributions": self.ui.all(),
            "shell": self.shell,
            "limits": self.limits,
        }))
    }
}

/// The interface modules the page is told at hello: the installed packages', and the running
/// applications'. Two sources that change apart, told as one list. Replaced at assembly and
/// again after an install: a package that needed a reload to appear is a package nobody will
/// believe installed.
#[derive(Default)]
pub(crate) struct UiModules {
    lists: Mutex<(Vec<Value>, Vec<Value>)>,
}

impl UiModules {
    pub fn set_packages(&self, modules: Vec<Value>) {
        lock(&self.lists).0 = modules;
    }

    #[cfg_attr(not(feature = "apps"), allow(dead_code))]
    pub fn set_apps(&self, modules: Vec<Value>) {
        lock(&self.lists).1 = modules;
    }

    fn all(&self) -> Value {
        let lists = lock(&self.lists);
        json!(lists.0.iter().chain(&lists.1).cloned().collect::<Vec<_>>())
    }
}
