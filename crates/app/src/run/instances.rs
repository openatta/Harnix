//! Step 4: the instances — `system`, with the native contributions of the modules still enabled;
//! the profile's `[[instances]]`; and the registry the ledger's are built from later.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use harnix_authz::MethodTable;
use harnix_hub::Registry;
use harnix_instance::link::{DownLink, EngineLink};
use harnix_instance::{Built, EngineConfig, Factory, Facts, InProcessLink, Ledger, Owner, Profile};
use harnix_module::Credentials;

use super::wiring::AppsWiring;
use crate::modules::{self, Record};
use crate::paths::Roots;

/// What the hub is made with: the registry templates build from, and the instances that are
/// there from the start.
pub(super) struct Instances {
    pub registry: Registry,
    /// The system instance, built in this process from the profile's `[engine]`.
    pub system: Built,
    /// The system instance's history: where audit entries are filed.
    pub history: Option<Arc<dyn history::store::HistoryStore>>,
    /// The profile's `[[instances]]`, each built or down with its reason.
    pub fixed: Vec<(String, Built)>,
}

pub(super) async fn build(
    profile: &Profile,
    roots: &Roots,
    records: &mut [Record],
    table: &mut MethodTable,
    credentials: &Arc<dyn Credentials>,
    apps: &mut AppsWiring,
) -> anyhow::Result<Instances> {
    let mut registry = registry(profile, roots, records, table, credentials, apps)?;

    let engine = harnix_instance::build_engine(EngineConfig {
        scene: profile.engine.scene.clone(),
        scenes: profile.engine.scenes.clone(),
        model: profile.engine.model.clone(),
        max_tokens: profile.engine.max_tokens,
        session_cap: profile.engine.session_cap,
        session_idle_timeout_secs: profile.engine.session_idle_timeout_secs,
        permission_prompt_timeout_secs: profile.engine.permission_prompt_timeout_secs,
        data_root: roots.engine.clone(),
        cwd: None,
        contrib: {
            let mut contrib = modules::engine_contrib(records);
            apps.engine(&mut contrib);
            contrib
        },
    })
    .await?;
    tracing::info!(
        scenes = ?engine.active_scenes,
        model = %engine.model,
        "engine ready (in-process)"
    );
    let facts = Facts::from(&engine);
    let history = engine.history.clone();
    let link: Arc<dyn EngineLink> = InProcessLink::start(engine).await;

    let fixed = fixed(profile, &mut registry).await?;
    Ok(Instances {
        registry,
        system: Built { link, facts },
        history,
        fixed,
    })
}

/// The factory every instance is built with and the templates it builds from: the profile's,
/// then each module's in order. A module whose template takes an id already taken is disabled.
fn registry(
    profile: &Profile,
    roots: &Roots,
    records: &mut [Record],
    table: &mut MethodTable,
    credentials: &Arc<dyn Credentials>,
    apps: &mut AppsWiring,
) -> anyhow::Result<Registry> {
    let resolve = {
        let credentials = credentials.clone();
        Arc::new(move |name: &str| credentials.get(name).ok().map(|s| s.expose().to_string()))
    };
    let mut factory = Factory::new(resolve)?;
    // A template that inherits providers inherits the system instance's.
    factory.providers_from = Some(roots.engine.clone());
    apps.factory(&mut factory);
    let mut registry = Registry {
        templates: BTreeMap::new(),
        ledger: Ledger::load(roots.state.join("instances.json"))?,
        factory,
        instances_dir: roots.state.join("instances"),
        disabled: records
            .iter()
            .filter(|r| !r.is_enabled())
            .map(|r| r.manifest.id.clone())
            .collect(),
        fixed: BTreeMap::new(),
    };
    for decl in &profile.instance_templates {
        let template = decl.template()?;
        registry
            .add_template(template, Owner::Kernel)
            .map_err(anyhow::Error::msg)?;
    }
    // A disabled module's templates are registered too, so what was built
    // from them is listed as down because of that module rather than as
    // built from a template that does not exist.
    for record in records.iter_mut() {
        for template in record.templates.clone() {
            if let Err(reason) =
                registry.add_template(template, Owner::Module(record.manifest.id.clone()))
            {
                if record.is_enabled() {
                    record.disable("registration", reason, table);
                }
                break;
            }
        }
    }
    Ok(registry)
}

/// The profile's other instances. One that cannot be built is listed
/// down with the reason, and the rest start: an instance is not the
/// process.
async fn fixed(profile: &Profile, registry: &mut Registry) -> anyhow::Result<Vec<(String, Built)>> {
    let mut fixed = Vec::new();
    for decl in &profile.instances {
        check_id(&decl.id)?;
        let root = instance_root(&registry.instances_dir, decl.data_root.as_deref(), &decl.id);
        let built = match decl.carrier() {
            Ok(carrier) => {
                registry
                    .fixed
                    .insert(decl.id.clone(), (carrier.clone(), root.clone()));
                match registry.factory.build(&decl.id, &carrier, root).await {
                    Ok(built) => built,
                    Err(e) => down(carrier.carrier(), format!("{e:#}")),
                }
            }
            Err(e) => down(harnix_instance::Carrier::InProcess, format!("{e:#}")),
        };
        fixed.push((decl.id.clone(), built));
    }
    Ok(fixed)
}

/// An `[[instances]]` id: never `system`, and only what an instance id may be.
fn check_id(id: &str) -> anyhow::Result<()> {
    if id == harnix_hub::SYSTEM {
        anyhow::bail!("[[instances]] cannot be called `system`: that is the profile's [engine]");
    }
    harnix_instance::templates::check_value("[[instances]] id", id).map_err(anyhow::Error::msg)
}

/// Where an `[[instances]]` entry keeps its data: its `data_root`, or its id, under the
/// instances directory.
fn instance_root(instances_dir: &Path, data_root: Option<&str>, id: &str) -> std::path::PathBuf {
    instances_dir.join(data_root.unwrap_or(id))
}

fn down(carrier: harnix_instance::Carrier, reason: String) -> Built {
    tracing::error!(%reason, "instance not built");
    Built {
        link: DownLink::new(carrier, reason),
        facts: Facts::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `system` is the profile's `[engine]`, never an `[[instances]]` entry; an id is checked
    /// before anything is built from it.
    #[test]
    fn an_instance_cannot_be_called_system() {
        let e = check_id(harnix_hub::SYSTEM).unwrap_err();
        assert!(e.to_string().contains("cannot be called `system`"), "{e}");
        assert!(check_id("worker").is_ok());
        assert!(check_id("../up").is_err());
    }

    #[test]
    fn an_instance_keeps_its_data_under_its_root_or_its_id() {
        let dir = Path::new("/state/instances");
        assert_eq!(instance_root(dir, None, "worker"), dir.join("worker"));
        assert_eq!(instance_root(dir, Some("shared"), "worker"), dir.join("shared"));
    }
}
