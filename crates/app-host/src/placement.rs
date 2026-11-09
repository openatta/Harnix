//! An application's agent side, put in place and switched with it, through the agent engine: its
//! agent package installed into each instance its manifest names (`[agent] install_into`), its
//! skills placed in each one's skills directory, and both switched on and off with the
//! application. Where those instances are is the [`Agent`]'s business: the product's own, for an
//! application embedded in its process; the application's own process's, otherwise.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use harnix_app_package::acquire::{internal, refused};
use harnix_app_package::package;
use harnix_app_port::{AgentInstall, AgentRef, AgentSwitch, Agent, RpcError, Skills};
use serde_json::{json, Value};

/// What installing put in place, as the record keeps it (its `parts`).
#[derive(Default)]
pub(crate) struct Placed {
    pub(crate) agent: Option<(String, String, Vec<String>)>,
    pub(crate) skills: BTreeMap<String, Vec<String>>,
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
            "skills": self.skills,
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
        if let Some(skills) = parts.get("skills").and_then(Value::as_object) {
            for (instance, names) in skills {
                placed.skills.insert(instance.clone(), strings(names));
            }
        }
        placed
    }

    /// Take it all back, as far as it goes.
    pub(crate) async fn undo(&self, engine: &dyn Agent, id: &str) {
        for (instance, names) in &self.skills {
            let _ = engine
                .remove_skills(Skills {
                    app: id.to_string(),
                    instance: instance.clone(),
                    source: Default::default(),
                    names: names.clone(),
                })
                .await;
            reload(engine, instance).await;
        }
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

/// An instance re-reads its configuration — its skills directory included.
async fn reload(engine: &dyn Agent, instance: &str) {
    if let Err(e) = engine.reload_instance(instance.to_string()).await {
        tracing::warn!(%instance, error = %e.message, "the instance did not reload");
    }
}

/// Put the package unpacked at `root` in place: its agent package into each instance it names,
/// its skills into each one's skills directory. What was put where is written to `placed` as it is
/// done, so that a failure takes back exactly that.
pub(crate) async fn place(
    engine: &Arc<dyn Agent>,
    root: &Path,
    app_dir: &Path,
    placed: &mut Placed,
) -> Result<(), RpcError> {
    let inspected = package::inspect_dir(root).map_err(refused)?;
    let manifest = &inspected.manifest;
    let id = manifest.app.id.clone();
    let targets = &manifest.agent.install_into;
    for target in targets {
        if !engine.has_instance(target.clone()).await? {
            return Err(refused(format!(
                "`{id}` installs its agent package into `{target}`, and there is no such instance"
            )));
        }
    }
    if let Some(agent) = &inspected.agent {
        let zip = app_dir.join("agent.zip");
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
    if !inspected.skills.is_empty() {
        for target in targets {
            // Recorded before it is done: undoing removes only what is marked as ours.
            placed
                .skills
                .insert(target.clone(), inspected.skills.clone());
            engine
                .place_skills(Skills {
                    app: id.clone(),
                    instance: target.clone(),
                    source: root.join("agent").join("skills"),
                    names: inspected.skills.clone(),
                })
                .await?;
            reload(engine.as_ref(), target).await;
        }
    }
    Ok(())
}

/// The agent package and skills, in the instances the install put them in: switched on (or off)
/// with the application.
pub(crate) async fn switch(
    engine: &dyn Agent,
    id: &str,
    root: &Path,
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
    if let Some(skills) = parts.get("skills").filter(|s| s.is_object()) {
        for (instance, names) in skills.as_object().into_iter().flatten() {
            let skills = Skills {
                app: id.to_string(),
                instance: instance.clone(),
                source: root.join("agent").join("skills"),
                names: strings(names),
            };
            if on {
                engine.place_skills(skills).await?;
            } else {
                let _ = engine.remove_skills(skills).await;
            }
            reload(engine, instance).await;
        }
    }
    Ok(())
}
