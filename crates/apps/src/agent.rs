//! The host's [`Agent`], over this process's Agix: an embedded application's agent side.
//!
//! Its agent package is installed into the instances its manifest names (`plugin.install` and
//! friends), its skills are copied into each one's skills directory, marked as its own, and its
//! tools are in every session built here ([`crate::AppsBridge`] reads what
//! [`Agent::expose_tools`] offered). Reached, like the kernel, through the `apps` module's context.

use std::path::PathBuf;

use async_trait::async_trait;
use harnix_app_package::acquire::{internal, refused};
use harnix_app_package::package;
use harnix_app_port::{Agent, AgentInstall, AgentRef, AgentSwitch, Offer, Skills};
use harnix_contract::RpcError;
use harnix_module::ModuleContext;
use serde_json::{json, Value};

use crate::platform::KernelPlatform;

/// What marks a skill directory as an application's, inside an instance's skills directory.
const SKILL_MARK: &str = ".harnix-app";

impl KernelPlatform {
    fn skills_dir(cx: &ModuleContext, instance: &str) -> Result<PathBuf, RpcError> {
        cx.instance(instance)
            .map(|h| h.data_root.join("skills"))
            .ok_or_else(|| refused(format!("there is no instance `{instance}`")))
    }
}

#[async_trait]
impl Agent for KernelPlatform {

    async fn has_instance(&self, instance: String) -> Result<bool, RpcError> {
        Ok(self.cx()?.instance(&instance).is_some())
    }

    async fn install_agent(&self, a: AgentInstall) -> Result<Value, RpcError> {
        let params = json!({
            "name": a.name,
            "version": a.version,
            "download_url": format!("file://{}", a.archive.display()),
            "instance": a.instance,
        });
        let answer = self.cx()?.call("plugin.install", params).await?;
        Ok(answer.get("disclosure").cloned().unwrap_or(answer))
    }

    async fn uninstall_agent(&self, a: AgentRef) -> Result<(), RpcError> {
        let params = json!({"name": a.name, "version": a.version, "instance": a.instance});
        self.cx()?.call("plugin.uninstall", params).await.map(|_| ())
    }

    async fn switch_agent(&self, s: AgentSwitch) -> Result<(), RpcError> {
        let method = if s.on { "plugin.enable" } else { "plugin.disable" };
        self.cx()?
            .call(method, json!({"name": s.name, "instance": s.instance}))
            .await
            .map(|_| ())
    }

    /// Copy `<source>/<name>/` into the instance's skills directory, marked as the application's.
    /// A directory of that name that is not the application's is somebody else's skill, and
    /// refuses it.
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

    /// Remove the skills marked as the application's, and nothing else.
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
                    tracing::warn!(app = %s.app, skill = %name, error = %e, "a skill was not removed");
                }
            }
        }
        Ok(())
    }

    async fn reload_instance(&self, instance: String) -> Result<(), RpcError> {
        self.cx()?
            .call("config.reload", json!({"instance": instance}))
            .await
            .map(|_| ())
    }

    async fn expose_tools(&self, offer: Offer) -> Result<(), RpcError> {
        self.registry.offer(offer);
        Ok(())
    }
}
