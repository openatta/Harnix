//! The hub, made from the instances and the modules' declarations: what it reports at hello,
//! the spaces it serves, the topics it carries.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use harnix_hub::{Hub, HubConfig};
use harnix_instance::link::now_ms;
use harnix_instance::{Owner, Profile};
use serde_json::{json, Value};

use super::instances::Instances;
use super::wiring::AppsWiring;
use crate::cli::Cli;
use crate::modules::Record;
use crate::paths::Roots;

/// The hub, with its instances added and its topics declared.
pub(super) struct Assembled {
    pub hub: Arc<Hub>,
    /// The system instance's history: where audit entries are filed.
    pub history: Option<Arc<dyn history::store::HistoryStore>>,
    /// The brand's icon on disk, resolved against the profile: what the transport serves at
    /// [`harnix_transport::BRAND_ICON_PATH`].
    pub brand_icon: Option<PathBuf>,
}

pub(super) async fn assemble(
    cli: &Cli,
    profile: &Profile,
    roots: &Roots,
    app: (&str, &str),
    records: &[Record],
    instances: Instances,
    apps: &AppsWiring,
) -> anyhow::Result<Assembled> {
    let Instances {
        registry,
        system,
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
    let spaces = harnix_hub::Spaces::load(
        roots.state.join("spaces.json"),
        declared_spaces(records),
        profile.shell.keeps(harnix_contract::SYSTEM_SPACE),
    )?;
    let hub = Hub::new(HubConfig {
        replay_from: cli.replay_dir.clone(),
        limits,
        prompt_timeout_secs: profile.engine.permission_prompt_timeout_secs,
        registry,
        spaces,
    });
    hub.set_replay_strict(cli.replay_strict);
    let brand_icon = brand_icon(profile, cli.profile.as_deref());
    hub.set_shell(shell(profile, roots, brand_icon.is_some()));
    let (name, version) = app;
    let mut process = json!({
        "app": { "name": name, "version": version },
        "harnix": crate::HARNIX_VERSION,
        "agix": daemon::discovery::identity().get("version").cloned().unwrap_or(json!(null)),
        "started_ms": now_ms(),
    });
    apps.process(&mut process);
    hub.set_process(process);
    let started = now_ms();
    hub.add_instance(harnix_hub::SYSTEM, system, None, BTreeMap::new(), Owner::Kernel, started)
        .await;
    for (id, built) in fixed {
        hub.add_instance(&id, built, None, BTreeMap::new(), Owner::Kernel, started)
            .await;
    }
    hub.restore().await;
    if profile.apps.enabled {
        hub.declare_spaces_topic().await;
    }
    apps.hub(&hub).await;
    for record in records.iter().filter(|r| r.is_enabled()) {
        for topic in &record.topics {
            hub.declare_topic(&topic.name, topic.params.clone()).await;
        }
    }
    Ok(Assembled {
        hub,
        history,
        brand_icon,
    })
}

/// The spaces the enabled modules are the backend of. A space id taken twice keeps the first.
fn declared_spaces(records: &[Record]) -> Vec<harnix_module::SpaceDecl> {
    let mut declared: Vec<harnix_module::SpaceDecl> = Vec::new();
    for record in records.iter().filter(|r| r.is_enabled()) {
        for space in &record.spaces {
            let reserved = [
                harnix_contract::SESSIONS_SPACE,
                harnix_contract::PROJECTS_SPACE,
                harnix_contract::SYSTEM_SPACE,
            ];
            if reserved.contains(&space.id.as_str()) || declared.iter().any(|d| d.id == space.id) {
                tracing::warn!(module = %record.manifest.id, space = %space.id, "a space with this id already exists; ignored");
                continue;
            }
            declared.push(space.clone());
        }
    }
    declared
}

/// The brand's icon, relative to the profile it was written in.
fn brand_icon(profile: &Profile, profile_path: Option<&Path>) -> Option<PathBuf> {
    profile.brand.icon.as_ref().map(|icon| match profile_path {
        Some(path) => path.parent().unwrap_or(Path::new(".")).join(icon),
        None => icon.clone(),
    })
}

/// What the shell is told at hello: the brand, which built-in spaces this product keeps,
/// whether installed packages' interface modules load (concept §5.8), and whether it starts new
/// sessions at all (`new_session`, ipdrun-46).
fn shell(profile: &Profile, roots: &Roots, has_icon: bool) -> Value {
    json!({
        "spaces": profile.shell.spaces,
        "brand": {
            "name": profile.brand.name,
            "icon": has_icon.then_some(harnix_transport::BRAND_ICON_PATH),
            "product_id": profile.brand.product_id.as_deref().unwrap_or("harnix"),
        },
        "runtime_modules": profile.ui.runtime_modules,
        "new_session": profile.shell.new_session,
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
