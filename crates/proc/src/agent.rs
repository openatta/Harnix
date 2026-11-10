//! The host's agent engine in an extension process: its own, beside the host, never over the wire.
//!
//! This process does not serve an engine of its own yet, so it has no instance to put an agent
//! package or skills in: a package that asks for one is refused at `ctl.place`, by name. What its
//! component offers sessions stays here, for the engine it will have; the product reaches its
//! tools over the wire (`ctl.call_tool`).

use async_trait::async_trait;
use harnix_package::acquire::refused;
use harnix_wasm_port::{Agent, AgentInstall, AgentRef, AgentSwitch, Offer, RpcError, Skills};

pub(crate) struct NoEngine;

fn none(what: &str) -> RpcError {
    refused(format!("this extension process has no agent engine of its own to {what}"))
}

#[async_trait]
impl Agent for NoEngine {
    async fn has_instance(&self, _instance: String) -> Result<bool, RpcError> {
        Ok(false)
    }
    async fn install_agent(&self, _install: AgentInstall) -> Result<serde_json::Value, RpcError> {
        Err(none("install an agent package in"))
    }
    async fn uninstall_agent(&self, _agent: AgentRef) -> Result<(), RpcError> {
        Err(none("uninstall an agent package from"))
    }
    async fn switch_agent(&self, _switch: AgentSwitch) -> Result<(), RpcError> {
        Err(none("switch an agent package in"))
    }
    async fn place_skills(&self, _skills: Skills) -> Result<(), RpcError> {
        Err(none("place skills in"))
    }
    async fn remove_skills(&self, _skills: Skills) -> Result<(), RpcError> {
        Ok(())
    }
    async fn reload_instance(&self, _instance: String) -> Result<(), RpcError> {
        Ok(())
    }
    async fn expose_tools(&self, offer: Offer) -> Result<(), RpcError> {
        tracing::debug!(app = %offer.app, tools = offer.tools.len(), "tools offered to this process's sessions");
        Ok(())
    }
}
