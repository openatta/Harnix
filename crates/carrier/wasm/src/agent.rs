//! The host's [`Agent`], over the component's context: a package's agent side, in this process's
//! instances.
//!
//! Its agent package is installed into the instances its manifest names (`engine.plugin.install`
//! and friends, as the component), its skills are copied into each one's skills directory, marked
//! as its own, and its tools are in every session built here ([`crate::WasmTools`] reads what
//! [`Agent::expose_tools`] offered).

use std::path::PathBuf;

use async_trait::async_trait;
use harnix_ext::RpcError;
use harnix_ext::ExtensionContext;
use harnix_package::acquire::{internal, refused};
use harnix_package::package;
use harnix_wasm_port::{Agent, AgentInstall, AgentRef, AgentSwitch, Offer, Skills};
use serde_json::{json, Value};

use crate::kernel::CarrierKernel;

/// What marks a skill directory as a package's, inside an instance's skills directory.
const SKILL_MARK: &str = ".harnix-app";

impl CarrierKernel {
    fn skills_dir(cx: &ExtensionContext, instance: &str) -> Result<PathBuf, RpcError> {
        let engine = cx.engine(instance).ok_or_else(|| refused(format!("there is no instance `{instance}`")))?;
        Ok(engine.skills()?.path().to_path_buf())
    }
}

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

    /// Copy `<source>/<name>/` into the instance's skills directory, marked as the package's. A
    /// directory of that name that is not the package's is somebody else's skill, and refuses it.
    async fn place_skills(&self, s: Skills) -> Result<(), RpcError> {
        let (id, instance) = (&s.app, &s.instance);
        let dir = Self::skills_dir(&self.cx()?, instance)?;
        for name in &s.names {
            let target = dir.join(name);
            if target.exists()
                && std::fs::read_to_string(target.join(SKILL_MARK))
                    .ok()
                    .as_deref()
                    != Some(id.as_str())
            {
                return Err(refused(format!(
                    "the instance `{instance}` has a skill `{name}` of its own; `{id}` would replace it"
                )));
            }
            let _ = std::fs::remove_dir_all(&target);
            let source = s.source.join(name);
            let mut files = Vec::new();
            package::walk(&source, &source, &mut files)
                .map_err(|e| internal(format!("{}: {e}", source.display())))?;
            for (relative, path) in files {
                let to = target.join(&relative);
                if let Some(parent) = to.parent() {
                    std::fs::create_dir_all(parent)
                        .map_err(|e| internal(format!("{}: {e}", parent.display())))?;
                }
                std::fs::copy(&path, &to).map_err(|e| internal(format!("{}: {e}", to.display())))?;
            }
            std::fs::write(target.join(SKILL_MARK), id).map_err(|e| internal(e.to_string()))?;
        }
        Ok(())
    }

    /// Remove the skills marked as the package's, and nothing else.
    async fn remove_skills(&self, s: Skills) -> Result<(), RpcError> {
        let Ok(dir) = Self::skills_dir(&self.cx()?, &s.instance) else {
            return Ok(());
        };
        for name in &s.names {
            let target = dir.join(name);
            if std::fs::read_to_string(target.join(SKILL_MARK))
                .ok()
                .as_deref()
                == Some(s.app.as_str())
            {
                if let Err(e) = std::fs::remove_dir_all(&target) {
                    tracing::warn!(package = %s.app, skill = %name, error = %e, "a skill was not removed");
                }
            }
        }
        Ok(())
    }

    async fn reload_instance(&self, instance: String) -> Result<(), RpcError> {
        self.cx()?
            .call("engine.config.reload", json!({"instance": instance}))
            .await
            .map(|_| ())
    }

    async fn expose_tools(&self, offer: Offer) -> Result<(), RpcError> {
        self.registry.offer(offer);
        Ok(())
    }
}
