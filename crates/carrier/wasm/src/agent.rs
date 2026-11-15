//! The host's [`Agent`], over the component's context: a package's agent side, in this process's
//! instances.
//!
//! Its agent package is installed into the instances its manifest names (`engine.plugin.install`
//! and friends, as the component), and its tools are in every session built here ([`crate::WasmTools`] reads what
//! [`Agent::expose_tools`] offered).

use async_trait::async_trait;
use harnix_ext::RpcError;
use harnix_wasm_port::{Agent, AgentInstall, AgentRef, AgentSwitch, Offer};
use serde_json::{json, Value};

use crate::kernel::CarrierKernel;

#[async_trait]
impl Agent for CarrierKernel {
    async fn has_instance(&self, instance: String) -> Result<bool, RpcError> {
        Ok(self.cx()?.engine(&instance).is_some())
    }

    async fn install_agent(&self, a: AgentInstall) -> Result<Value, RpcError> {
        let params = json!({
            "name": a.name,
            "version": a.version,
            "download_url": format!("file://{}", a.archive.display()),
            "instance": a.instance,
        });
        let answer = self.cx()?.call("engine.plugin.install", params).await?;
        Ok(answer.get("disclosure").cloned().unwrap_or(answer))
    }

    async fn uninstall_agent(&self, a: AgentRef) -> Result<(), RpcError> {
        let params = json!({"name": a.name, "version": a.version, "instance": a.instance});
        self.cx()?.call("engine.plugin.uninstall", params).await.map(|_| ())
    }

    async fn switch_agent(&self, s: AgentSwitch) -> Result<(), RpcError> {
        let method = if s.on { "engine.plugin.enable" } else { "engine.plugin.disable" };
        self.cx()?
            .call(method, json!({"name": s.name, "instance": s.instance}))
            .await
            .map(|_| ())
    }

    async fn expose_tools(&self, offer: Offer) -> Result<(), RpcError> {
        self.registry.offer(offer);
        Ok(())
    }
}
