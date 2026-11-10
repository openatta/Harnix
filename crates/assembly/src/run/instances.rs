//! Step 4: the instances — `main`, with the native contributions of the extensions still enabled;
//! the profile's `[[instances]]`; and the registry the ledger's are built from later.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use harnix_router::RouteTable;
use harnix_engine_api::InstanceCarrier;
use harnix_instance::{
    Built, DownLink, EngineConfig, Factory, Facts, HistoryStore, Ledger, Owner, Recording, RegistryConfig,
};
use harnix_ext::Credentials;

use crate::host::{lifecycle, Record};
use crate::paths::Roots;
use crate::profile::Profile;

/// What the instance registry is made with: its templates and ledger, and the instances that
/// are there from the start.
pub(super) struct Instances {
    pub registry: RegistryConfig,
    /// The main instance, built in this process from the profile's `[engine]`.
    pub main: Built,
    /// The main instance's history: where audit entries are filed.
    pub history: Option<Arc<dyn HistoryStore>>,
    /// The profile's `[[instances]]`, each built or down with its reason.
    pub fixed: Vec<(String, Built)>,
}

pub(super) async fn build(
    profile: &Profile,
    roots: &Roots,
    records: &mut [Record],
    table: &mut RouteTable,
    credentials: &Arc<dyn Credentials>,
    engines: &Arc<harnix_authz::EngineCredentials>,
    recording: Arc<Recording>,
    grants: Arc<dyn harnix_kernel::GrantBook>,
) -> anyhow::Result<Instances> {
    let mut registry = registry(profile, roots, records, table, credentials, engines, recording, grants)?;

    let engine_profile = &profile.instances.engine;
    let engine = harnix_instance::build_engine(EngineConfig {
        scene: engine_profile.scene.clone(),
        scenes: engine_profile.scenes.clone(),
        model: engine_profile.model.clone(),
        max_tokens: engine_profile.max_tokens,
        session_cap: engine_profile.session_cap,
        session_idle_timeout_secs: engine_profile.session_idle_timeout_secs,
        permission_prompt_timeout_secs: engine_profile.permission_prompt_timeout_secs,
        data_root: roots.engine.clone(),
        cwd: None,
        instance: harnix_contract::MAIN_INSTANCE.to_string(),
        chains: registry.factory.chains(),
        contrib: lifecycle::main_contribution(records),
    })
    .await?;
    tracing::info!(
        scenes = ?engine.active_scenes,
        model = %engine.model,
        "engine ready (in-process)"
    );
    let history = engine.history.clone();
    let main = registry.factory.in_process(harnix_hub::MAIN, engine).await;

    let fixed = fixed(profile, &mut registry).await?;
    Ok(Instances {
        registry,
        main,
        history,
        fixed,
    })
}

/// The factory every instance is built with and the templates it builds from: the profile's,
/// then each extension's in order. An extension whose template takes an id already taken is
/// disabled.
fn registry(
    profile: &Profile,
    roots: &Roots,
    records: &mut [Record],
    table: &mut RouteTable,
    credentials: &Arc<dyn Credentials>,
    engines: &Arc<harnix_authz::EngineCredentials>,
    recording: Arc<Recording>,
    grants: Arc<dyn harnix_kernel::GrantBook>,
) -> anyhow::Result<RegistryConfig> {
    let resolve = {
        let credentials = credentials.clone();
        Arc::new(move |name: &str| credentials.get(name).ok().map(|s| s.expose().to_string()))
    };
    let mut factory = Factory::new(resolve)?;
    // A template that inherits providers inherits the main instance's.
    factory.providers_from = Some(roots.engine.clone());
    factory.recording = recording;
    // What the extensions give every engine built in this process.
    let (providers, admitted) = lifecycle::everywhere(records);
    factory.providers.extend(providers);
    factory.admitted.extend(admitted);
    // A daemon reaches the packages' tools over HTTP, presenting a credential of its own.
    factory.servers.issue_with(Arc::new(EngineTokens(engines.clone())));
    let mut registry = RegistryConfig {
        templates: BTreeMap::new(),
        ledger: Ledger::load(roots.state.join("kernel").join("instances.json"))?,
        factory,
        instances_dir: roots.state.join("instances"),
        disabled: records
            .iter()
            .filter(|r| !r.is_enabled())
            .map(|r| r.manifest.id().to_string())
            .collect(),
        fixed: BTreeMap::new(),
        grants,
    };
    for decl in &profile.instances.instance_templates {
        let template = decl.template()?;
        registry
            .add_template(template, Owner::Kernel)
            .map_err(anyhow::Error::msg)?;
    }
    // A disabled extension's templates are registered too, so what was built
    // from them is listed as down because of that extension rather than as
    // built from a template that does not exist.
    for record in records.iter_mut() {
        let templates = record.descriptor.as_ref().map(|d| d.templates().to_vec()).unwrap_or_default();
        for template in templates {
            if let Err(reason) =
                registry.add_template(template, Owner::Module(record.manifest.id().to_string()))
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
async fn fixed(profile: &Profile, registry: &mut RegistryConfig) -> anyhow::Result<Vec<(String, Built)>> {
    let mut fixed = Vec::new();
    for decl in &profile.instances.instances {
        check_id(&decl.id)?;
        let root = instance_root(&registry.instances_dir, decl.data_root.as_deref(), &decl.id);
        let built = match decl.carrier() {
            Ok(carrier) => {
                registry
                    .fixed
                    .insert(decl.id.clone(), (carrier.clone(), root.clone()));
                match registry.factory.build(&decl.id, &carrier, root).await {
                    Ok(built) => built,
                    Err(e) => down(harnix_instance::templates::carrier_of(&carrier), format!("{e:#}")),
                }
            }
            Err(e) => down(InstanceCarrier::InProcess, format!("{e:#}")),
        };
        fixed.push((decl.id.clone(), built));
    }
    Ok(fixed)
}

/// An `[[instances]]` id: never `main`, and only what an instance id may be.
fn check_id(id: &str) -> anyhow::Result<()> {
    if id == harnix_hub::MAIN {
        anyhow::bail!("[[instances]] cannot be called `main`: that is the profile's [engine]");
    }
    harnix_instance::templates::check_value("[[instances]] id", id).map_err(anyhow::Error::msg)
}

/// Where an `[[instances]]` entry keeps its data: its `data_root`, or its id, under the
/// instances directory.
fn instance_root(instances_dir: &Path, data_root: Option<&str>, id: &str) -> std::path::PathBuf {
    instances_dir.join(data_root.unwrap_or(id))
}

fn down(carrier: InstanceCarrier, reason: String) -> Built {
    tracing::error!(%reason, "instance not built");
    Built::new(DownLink::new(carrier, reason), Facts::default())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `main` is the profile's `[engine]`, never an `[[instances]]` entry; an id is checked
    /// before anything is built from it.
    #[test]
    fn an_instance_cannot_be_called_system() {
        let e = check_id(harnix_hub::MAIN).unwrap_err();
        assert!(e.to_string().contains("cannot be called `main`"), "{e}");
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

/// What issues and revokes the credential each daemon this process spawns presents at a package's
/// tool endpoint, as its instance's engine.
struct EngineTokens(Arc<harnix_authz::EngineCredentials>);

impl harnix_instance::ServerTokens for EngineTokens {
    fn issue(&self, instance: &str) -> String {
        self.0.issue_engine(instance)
    }

    fn revoke(&self, token: &str) {
        self.0.revoke(token)
    }
}
