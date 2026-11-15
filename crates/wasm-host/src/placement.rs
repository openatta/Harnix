//! A package's agent side, put in place and switched with it, through the agent engine: its
//! agent package installed into each instance its manifest names (`[agent] install_into`), and
//! switched on and off with the package. Where those instances are is the [`Agent`]'s business:
//! the product's own, for an package embedded in its process; the package's own process's,
//! otherwise. Its skills are not placed in an instance: they are a directory of the package's
//! (`docs/harnix-ext-package.md`), listed to the sessions of the spaces that see them.

use std::path::Path;
use std::sync::Arc;

use harnix_package::acquire::{internal, no_such_instance, refused};
use harnix_package::package;
use harnix_wasm_port::{AgentInstall, AgentRef, AgentSwitch, Agent, RpcError};
use serde_json::{json, Value};

/// What installing put in place, as the record keeps it (its `parts`).
#[derive(Default)]
pub(crate) struct Placed {
    pub(crate) agent: Option<(String, String, Vec<String>)>,
    /// The engine's disclosure of the agent package, for the installer to show.
    pub(crate) disclosure: Value,
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array()
        .map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect())
        .unwrap_or_default()
}

impl Placed {
    pub(crate) fn parts(&self) -> Value {
        json!({
            "agent": self.agent.as_ref().map(|(package, version, instances)| json!({
                "package": package, "version": version, "instances": instances,
            })),
        })
    }

    /// What a record's `parts` say was put in place.
    pub(crate) fn of(parts: &Value) -> Self {
        let mut placed = Placed::default();
        if let Some(agent) = parts.get("agent").filter(|a| !a.is_null()) {
            placed.agent = Some((
                agent["package"].as_str().unwrap_or_default().to_string(),
                agent["version"].as_str().unwrap_or_default().to_string(),
                strings(&agent["instances"]),
            ));
        }
        placed
    }

    /// Take it all back, as far as it goes.
    pub(crate) async fn undo(&self, engine: &dyn Agent, id: &str) {
        if let Some((package, version, instances)) = &self.agent {
            for instance in instances {
                let agent = AgentRef {
                    app: id.to_string(),
                    name: package.clone(),
                    version: version.clone(),
                    instance: instance.clone(),
                };
                if let Err(e) = engine.uninstall_agent(agent).await {
                    tracing::warn!(app = %id, %instance, error = %e.message, "the agent package was not uninstalled");
                }
            }
        }
    }
}

/// Put the package unpacked at `root` in place: its agent package into each instance it goes
/// into. What was put where is written to `placed` as it is done, so that a failure takes back
/// exactly that.
///
/// Which instances, the carrier says (`instances`): given, they are the agent engine's own, and
/// the manifest's `install_into` — the product's instances, which get only the package's tools —
/// is not looked for here. Not given, each instance `install_into` names must be the agent
/// engine's.
pub(crate) async fn place(
    engine: &Arc<dyn Agent>,
    root: &Path,
    instances: Option<&[String]>,
    placed: &mut Placed,
) -> Result<(), RpcError> {
    let inspected = package::inspect_dir(root).map_err(refused)?;
    let manifest = &inspected.manifest;
    let id = manifest.app.id.clone();
    let targets: &[String] = match instances {
        Some(instances) => instances,
        None => {
            for target in &manifest.agent.install_into {
                if !engine.has_instance(target.clone()).await? {
                    return Err(no_such_instance(&id, target));
                }
            }
            &manifest.agent.install_into
        }
    };
    // Into no instance, there is nothing to pack: a package's directory may not even be writable
    // where it runs.
    if let Some(agent) = inspected.agent.as_ref().filter(|_| !targets.is_empty()) {
        // In the temporary directory, not beside the package: an extension process may write only
        // its run directory, where its `TMPDIR` is.
        // Unique per placement: two in one process, of the same package, run side by side.
        let zip = tempfile::Builder::new()
            .prefix(&format!("{id}-"))
            .suffix("-agent.zip")
            .tempfile()
            .and_then(|f| f.into_temp_path().keep().map_err(|e| e.error))
            .map_err(|e| internal(e.to_string()))?;
        package::zip_dir(&root.join("agent"), &zip).map_err(internal)?;
        let mut instances = Vec::new();
        let mut failure = None;
        let mut disclosed = Value::Null;
        for target in targets {
            let install = AgentInstall {
                app: id.clone(),
                name: agent.name.clone(),
                version: agent.version.clone(),
                archive: zip.clone(),
                instance: target.clone(),
            };
            match engine.install_agent(install).await {
                Ok(disclosure) => {
                    // What the engine says the package puts in front of the model: the installer
                    // shows it.
                    if disclosed.is_null() {
                        disclosed = disclosure;
                    }
                    instances.push(target.clone());
                }
                Err(e) => {
                    failure = Some(refused(format!(
                        "`{id}`'s agent package did not install into `{target}`: {}",
                        e.message
                    )));
                    break;
                }
            }
        }
        let _ = std::fs::remove_file(&zip);
        placed.agent = Some((agent.name.clone(), agent.version.clone(), instances));
        placed.disclosure = disclosed;
        if let Some(e) = failure {
            return Err(e);
        }
    }
    Ok(())
}

/// The agent package, in the instances the install put it in: switched on (or off) with the
/// package.
pub(crate) async fn switch(
    engine: &dyn Agent,
    id: &str,
    parts: &Value,
    on: bool,
) -> Result<(), RpcError> {
    if let Some(agent) = parts.get("agent").filter(|a| !a.is_null()) {
        let name = agent["package"].as_str().unwrap_or_default();
        for instance in strings(&agent["instances"]) {
            let switch = AgentSwitch {
                app: id.to_string(),
                name: name.to_string(),
                instance: instance.clone(),
                on,
            };
            if let Err(e) = engine.switch_agent(switch).await {
                if on {
                    return Err(refused(format!(
                        "`{id}`'s agent package could not be switched on in `{instance}`: {}",
                        e.message
                    )));
                }
                tracing::warn!(app = %id, %instance, error = %e.message, "the agent package was not switched off");
            }
        }
    }
    Ok(())
}
