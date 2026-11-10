//! The scenes an engine serves: what extensions contribute, built over their bases, Agix's
//! `harnix` scene attended, and every one of them admitting the tools the host gives every
//! session.

use std::sync::Arc;

use base::interface::scene::Scene;
use harnix_engine_api::{BaseKind, Interaction, SceneBase, SceneLayer};

use crate::adapt::scene::Layered;

/// The id Agix's `harnix` scene is served under: the built-in one's, attended.
pub const ATTENDED_SCENE: &str = "harnix";

/// Agix's `harnix` scene, attended — what every engine Harnix builds serves under that id.
///
/// Agix's scene is unattended: a call its permission policy would ask about is refused before
/// anyone is asked. A Harnix session has someone to ask — the person at the interface (every
/// watcher gets the prompt, the first answer wins) or an extension that answers prompts itself —
/// so the engine gets this in its place, the built-in scene in everything but who answers.
pub fn attended() -> SceneLayer {
    SceneLayer::new("{base}", SceneBase::builtin(ATTENDED_SCENE)).interactive(true)
}

/// One of Agix's own scenes, as `base` names it.
fn builtin(id: &str, interaction: Option<Interaction>) -> Option<Arc<dyn Scene>> {
    if id == "code" {
        if let Some(interaction) = interaction {
            let interaction = match interaction {
                Interaction::Interactive => scene::code::Interaction::Interactive,
                _ => scene::code::Interaction::Unattended,
            };
            return Some(Arc::new(scene::code::CodeScene::new(interaction)));
        }
    }
    let mut registry = scene::SceneRegistry::new();
    registry.register_builtin();
    registry.resolve(id)
}

/// The scene `base` is, given the scenes built so far.
pub(crate) fn resolve(base: &SceneBase, built: &[Arc<dyn Scene>]) -> anyhow::Result<Arc<dyn Scene>> {
    match base.kind() {
        BaseKind::Builtin { id, args } => builtin(id, args.chosen_interaction())
            .ok_or_else(|| anyhow::anyhow!("`{id}` is not one of the engine's own scenes")),
        BaseKind::Named(id) => built
            .iter()
            .find(|s| s.id() == id)
            .cloned()
            .or_else(|| builtin(id, None))
            .ok_or_else(|| anyhow::anyhow!("no scene `{id}` to build on")),
        BaseKind::Foreign(any) => any
            .downcast_ref::<Arc<dyn Scene>>()
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("the scene `{}` is not one this engine can read", base.id())),
        _ => anyhow::bail!("the scene `{}` has a base this engine does not know", base.id()),
    }
}

/// A layer over the scene its base resolves to, for the engine of `instance`.
pub fn build(layer: &SceneLayer, built: &[Arc<dyn Scene>], instance: &Arc<str>) -> anyhow::Result<Arc<dyn Scene>> {
    Ok(Arc::new(Layered::new(layer, resolve(layer.base(), built)?, instance)))
}

/// What extensions contribute, and the `harnix` scene attended unless one of them contributes
/// one under that id itself. With tools to admit, every scene — Agix's own ones too, served
/// under their own ids — admits them.
pub(crate) fn host_scenes(
    layers: &[SceneLayer],
    admitted: &[String],
    instance: &Arc<str>,
) -> anyhow::Result<Vec<Arc<dyn Scene>>> {
    let mut scenes: Vec<Arc<dyn Scene>> = Vec::new();
    for layer in layers {
        let scene = build(layer, &scenes, instance)?;
        scenes.push(scene);
    }
    if !scenes.iter().any(|s| s.id() == ATTENDED_SCENE) {
        let scene = build(&attended(), &scenes, instance)?;
        scenes.push(scene);
    }
    if admitted.is_empty() {
        return Ok(scenes);
    }
    let mut builtin = scene::SceneRegistry::new();
    builtin.register_builtin();
    for id in builtin.ids() {
        if scenes.iter().any(|s| s.id() == id) {
            continue;
        }
        if let Some(scene) = builtin.resolve(&id) {
            scenes.push(scene);
        }
    }
    let admitting = admitted
        .iter()
        .fold(SceneLayer::new("{base}", SceneBase::named("")), |layer, pattern| layer.admit(pattern));
    Ok(scenes
        .into_iter()
        .map(|s| Arc::new(Layered::new(&admitting, s, instance)) as Arc<dyn Scene>)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_harnix_scene_is_the_built_in_one_but_attended() {
        let built_in = scene::harnix::HarnixScene;
        let attended = build(&attended(), &[], &"main".into()).unwrap();
        assert!(!built_in.interactive() && attended.interactive());
        assert_eq!(attended.id(), ATTENDED_SCENE);
        assert_eq!(attended.tools(), built_in.tools());
        assert_eq!(attended.deferred_tools(), built_in.deferred_tools());
        assert_eq!(attended.path_style(), built_in.path_style());
        assert_eq!(attended.bundled_skills(), built_in.bundled_skills());
    }

    #[test]
    fn a_listed_scene_admits_the_hosts_tools_and_nothing_else_changes() {
        let scenes = host_scenes(&[], &["app__*".into()], &"main".into()).unwrap();
        let harnix = scenes.iter().find(|s| s.id() == ATTENDED_SCENE).unwrap();
        let plain = build(&attended(), &[], &"main".into()).unwrap();
        assert!(!plain.permits_tool("app__acme_notes__search"));
        assert!(harnix.permits_tool("app__acme_notes__search"));
        assert!(harnix.permits_tool("Bash") && harnix.permits_tool("mcp__github__x"));
        assert!(!harnix.permits_tool("plugin__other__x"));
        assert!(harnix.tools().contains(&"app__*".to_string()));
        assert_eq!((harnix.id(), harnix.interactive()), (plain.id(), plain.interactive()));
        let mut ids: Vec<&str> = scenes.iter().map(|s| s.id()).collect();
        ids.sort();
        assert_eq!(ids, ["code", "harnix", "mini"], "Agix's own scenes are served too, admitting");
    }

    #[test]
    fn a_base_is_found_among_the_scenes_before_it_or_the_engines_own() {
        let first = SceneLayer::new("first", SceneBase::builtin("mini"));
        let second = SceneLayer::new("second-{base}", SceneBase::named("first"));
        let scenes = host_scenes(&[first, second], &[], &"main".into()).unwrap();
        assert_eq!(scenes[1].id(), "second-first");
        assert!(host_scenes(&[SceneLayer::new("x", SceneBase::named("nowhere"))], &[], &"main".into()).is_err());
        assert!(host_scenes(&[SceneLayer::new("x", SceneBase::builtin("nowhere"))], &[], &"main".into()).is_err());
    }
}
