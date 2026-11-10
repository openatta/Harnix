//! Instances, from the server side: built from templates, as the extension.

use std::collections::BTreeMap;
use std::time::Duration;

use serde_json::{json, Value};

use crate::context::ExtensionContext;
use crate::Rpc;

/// An extension's instances. It builds from its own templates (`<id>.<template>`) and the
/// profile's; stopping one another extension built takes `instances.admin`.
pub struct Instances<'a> {
    cx: &'a ExtensionContext,
}

impl<'a> Instances<'a> {
    pub(crate) fn new(cx: &'a ExtensionContext) -> Self {
        Self { cx }
    }

    /// Build an instance from a template.
    pub async fn create(&self, template: &str, name: &str, params: BTreeMap<String, String>) -> Rpc<Value> {
        self.cx
            .call("harnix.instances.create", json!({"template": template, "name": name, "params": params}))
            .await
    }

    /// The instance by this name, built from this template if it is not there yet: what an
    /// extension calls at start.
    pub async fn ensure(&self, template: &str, name: &str, params: BTreeMap<String, String>) -> Rpc<Value> {
        self.cx
            .call("harnix.instances.ensure", json!({"template": template, "name": name, "params": params}))
            .await
    }

    /// [`Instances::ensure`], answering once the instance is ready: a spawned daemon is
    /// `starting` for a moment after it is built, and a session call in that moment is refused.
    /// One that goes down is `INSTANCE_UNAVAILABLE` at once, with why; so is running out of
    /// `timeout`.
    pub async fn ensure_ready(
        &self,
        template: &str,
        name: &str,
        params: BTreeMap<String, String>,
        timeout: Duration,
    ) -> Rpc<Value> {
        self.cx
            .call(
                "harnix.instances.ensure",
                json!({
                    "template": template, "name": name, "params": params,
                    "wait": "ready", "timeout_ms": timeout.as_millis() as u64,
                }),
            )
            .await
    }

    /// Wait until an instance is ready — one being restarted, or reconnecting — and answer with
    /// it. The same errors as [`Instances::ensure_ready`].
    pub async fn wait_ready(&self, instance: &str, timeout: Duration) -> Rpc<Value> {
        self.cx
            .call("harnix.instances.wait", json!({"instance": instance, "timeout_ms": timeout.as_millis() as u64}))
            .await
    }

    /// Stop an instance and forget it. Its data stays.
    pub async fn stop(&self, instance: &str) -> Rpc<Value> {
        self.cx.call("harnix.instances.stop", json!({"instance": instance})).await
    }
}
