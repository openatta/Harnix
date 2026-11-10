//! The instance registry and the hub, made from the instances and the extensions' declarations:
//! the instances the hub is handed, the spaces it serves, the topics it carries, and what a
//! client is told at hello.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use harnix_hub::{Hub, HubConfig};
use harnix_instance::link::now_ms;
use harnix_instance::{HistoryStore, InstanceRegistry, Owner, INSTANCES_TOPIC};
use serde_json::{json, Value};

use super::instances::Instances;
use crate::cli::Cli;
use crate::gate::Behind;
use crate::host::Record;
use crate::kernel_rows::{Hello, UiModules, UiTopic, UI};
use crate::paths::Roots;
use crate::process::Process;
use crate::profile::Profile;

/// The hub, with its instances added and its topics declared.
pub(super) struct Assembled {
    pub hub: Arc<Hub>,
    pub instances: Arc<InstanceRegistry>,
    /// The interface modules a client is told of at hello, as packages and applications change.
    pub ui: Arc<UiModules>,
    /// The main instance's history: where audit entries are filed.
    pub history: Option<Arc<dyn HistoryStore>>,
    /// The brand's icon on disk, resolved against the profile: what the transport serves at
    /// [`harnix_transport::BRAND_ICON_PATH`].
    pub brand_icon: Option<PathBuf>,
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn assemble(
    cli: &Cli,
    profile: &Profile,
    roots: &Roots,
    app: (&str, &str),
    records: &[Record],
    instances: Instances,
    behind: &Behind,
) -> anyhow::Result<Assembled> {
    let Instances {
        registry,
        main,
        history,
        fixed,
    } = instances;
    if let Some(dir) = &cli.replay_dir {
        tracing::warn!(dir = %dir.display(), strict = cli.replay_strict, "replaying recordings; no model will be called");
    }
    // The three ceilings, assembled where all three are known. The hub
    // reports them at hello and the transport enforces the frame one; both
    // read this rather than restating a number.
    let limits = harnix_contract::Limits {
        max_frame_bytes: harnix_contract::MAX_FRAME_BYTES,
        max_upload_bytes: harnix_contract::MAX_UPLOAD_BYTES,
        replay_max_frames: harnix_hub::REPLAY_MAX_FRAMES,
        max_in_flight: harnix_contract::MAX_IN_FLIGHT,
    };
    // A switched-off extension's spaces are nobody's to claim: without `system` nothing reaches
    // across spaces.
    let withheld = records
        .iter()
        .filter(|r| !r.is_enabled())
        .filter_map(|r| r.descriptor.as_ref())
        .flat_map(|d| d.spaces().iter().map(|s| s.id.clone()))
        .collect();
    let declared = declared_spaces(records, profile.extensions.default_space.as_deref())?;
    let spaces = harnix_hub::Spaces::load(roots.state.join("kernel").join("spaces.json"), declared.clone(), withheld)?;
    let turns = registry.factory.turns.clone();
    let registry = InstanceRegistry::new(registry);
    let hub = Hub::new(HubConfig {
        instances: registry.clone(),
        spaces,
        turns,
    });
    // Before any instance is added: the hub is handed every link the registry builds.
    registry.observe(hub.clone());
    // The rows have been in the route table since registration; from here they answer.
    let _ = behind.instances.set(registry.clone());
    hub.serve_topic(INSTANCES_TOPIC, Vec::new(), InstanceRegistry::topic(&behind.instances)).await;
    let brand_icon = brand_icon(profile, cli.profile.as_deref());
    let ui = Arc::new(UiModules::new(behind.diagnostics.clone()));
    ui.notify(hub.clone());
    hub.serve_topic(UI, Vec::new(), Arc::new(UiTopic(ui.clone()))).await;
    let _ = behind.hello.set(Arc::new(Hello {
        hub: hub.clone(),
        instances: registry.clone(),
        limits: serde_json::to_value(limits).unwrap_or(json!({})),
        prompt_timeout_secs: profile.instances.engine.permission_prompt_timeout_secs,
        shell: shell(profile, roots, brand_icon.is_some()),
        ui: ui.clone(),
    }));
    let (name, version) = app;
    let process = json!({
        "app": { "name": name, "version": version },
        "harnix": crate::HARNIX_VERSION,
        "agix": harnix_engine::identity().version,
        "started_ms": now_ms(),
    });
    let process = Arc::new(Process::new(process, hub.clone()));
    let _ = behind.process.set(process);
    registry.add(harnix_hub::MAIN, main, Owner::Kernel).await;
    for (id, built) in fixed {
        registry.add(&id, built, Owner::Kernel).await;
    }
    registry.restore().await;
    hub.declare_spaces_topic().await;
    for d in records.iter().filter(|r| r.is_enabled()).filter_map(|r| r.descriptor.as_ref()) {
        for topic in d.topics() {
            if topic.is_computed() {
                hub.serve_topic(topic.name(), topic.params().to_vec(), Arc::new(topic.clone())).await;
            } else {
                hub.declare_topic(topic.name(), topic.params().to_vec()).await;
            }
        }
    }
    Ok(Assembled {
        hub,
        instances: registry,
        ui,
        history,
        brand_icon,
    })
}

/// The spaces the enabled extensions are the backend of. A space id taken twice keeps the first.
/// `default_space` (`[extensions] default_space`) names the default in place of the one an
/// extension declared.
fn declared_spaces(
    records: &[Record],
    default_space: Option<&str>,
) -> anyhow::Result<Vec<harnix_contract::SpaceDecl>> {
    let mut declared: Vec<harnix_contract::SpaceDecl> = Vec::new();
    for record in records.iter().filter(|r| r.is_enabled()) {
        let Some(d) = &record.descriptor else { continue };
        for space in d.spaces() {
            if declared.iter().any(|d| d.id == space.id) {
                tracing::warn!(extension = %record.manifest.id(), space = %space.id, "a space with this id already exists; ignored");
                continue;
            }
            declared.push(space.clone());
        }
    }
    if let Some(id) = default_space {
        anyhow::ensure!(
            declared.iter().any(|d| d.id == id),
            "[extensions] default_space = {id:?} is not a space any enabled extension here declares"
        );
        for decl in &mut declared {
            decl.default_space = decl.id == id;
        }
    }
    Ok(declared)
}

/// The brand's icon, relative to the profile it was written in.
fn brand_icon(profile: &Profile, profile_path: Option<&Path>) -> Option<PathBuf> {
    profile.brand.icon.as_ref().map(|icon| match profile_path {
        Some(path) => path.parent().unwrap_or(Path::new(".")).join(icon),
        None => icon.clone(),
    })
}

/// What the shell is told at hello: the brand, whether installed packages' interface modules
/// load (concept §5.8), whether it starts new sessions at all (`new_session`, ipdrun-46), and
/// whether the composer switches a session's model (`model_switch`).
fn shell(profile: &Profile, roots: &Roots, has_icon: bool) -> Value {
    json!({
        "brand": {
            "name": profile.brand.name,
            "icon": has_icon.then_some(harnix_transport::BRAND_ICON_PATH),
            "product_id": profile.brand.product_id.as_deref().unwrap_or("harnix"),
        },
        "runtime_modules": profile.ui.runtime_modules,
        "new_session": profile.shell.new_session,
        "model_switch": profile.shell.model_switch,
        "narrow_details": profile.shell.narrow_details.as_str(),
        "state_dir": roots.state.display().to_string(),
        "projects_dir": roots.projects.display().to_string(),
        // Whose process this is: the avatar's name. One process, one person (concept §10).
        "user": std::env::var("USER").or_else(|_| std::env::var("USERNAME")).ok(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The brand's icon is found beside the profile that names it, not beside the process.
    #[test]
    fn the_brand_icon_is_resolved_against_the_profile() {
        let mut profile = Profile::default();
        assert_eq!(brand_icon(&profile, Some(Path::new("/p/profile.toml"))), None);
        profile.brand.icon = Some(PathBuf::from("icon.svg"));
        assert_eq!(
            brand_icon(&profile, Some(Path::new("/p/profile.toml"))),
            Some(PathBuf::from("/p/icon.svg"))
        );
        assert_eq!(brand_icon(&profile, None), Some(PathBuf::from("icon.svg")));
    }

    /// The shell is told where the icon is served, not where it is on disk, and only when
    /// there is one.
    #[test]
    fn the_shell_is_told_the_icon_s_path_only_when_there_is_one() {
        let roots = Roots {
            engine: "/e".into(),
            projects: "/p".into(),
            state: "/s".into(),
        };
        let profile = Profile::default();
        assert_eq!(shell(&profile, &roots, false)["brand"]["icon"], Value::Null);
        assert_eq!(shell(&profile, &roots, true)["brand"]["icon"], harnix_transport::BRAND_ICON_PATH);
        assert_eq!(shell(&profile, &roots, false)["brand"]["product_id"], "harnix");
        assert_eq!(shell(&profile, &roots, false)["state_dir"], "/s");
    }
}
