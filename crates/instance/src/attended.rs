//! Agix's `harnix` scene, attended: what every engine Harnix builds serves under that id.
//!
//! Agix's scene is unattended — a call its permission policy would ask about is refused before
//! anyone is asked. A Harnix session has someone to ask: the person at the interface (every
//! watcher gets the prompt, the first answer wins) or a module that answers prompts itself. So
//! the engine gets this one in its place, which is the built-in scene in everything but
//! [`Scene::interactive`].

use std::sync::Arc;

use base::interface::scene::Scene;

/// The id it is served under: the built-in one's.
pub const ATTENDED_SCENE: &str = "harnix";

/// A base scene that is attended.
pub struct Attended(pub(crate) Arc<dyn Scene>);

impl Attended {
    /// Agix's `harnix` scene, attended.
    pub fn harnix() -> Self {
        Self(Arc::new(scene::harnix::HarnixScene))
    }
}

delegate_scene!(Attended => 0; [
    id, name, description, build_system_prompt, tools, requires_project, supports_team,
    extra_tools, token_budget, build_system_reminder, build_session_context, disallowed_tools,
    permits_tool, deferred_tools, path_style, bundled_skills, modules, disabled_modules,
    permits_module, execution_params, auto_name_session, session_name_prompt,
    memory_extraction_prompt,
];
    fn interactive(&self) -> bool {
        true
    }
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_is_the_built_in_scene_but_attended() {
        let built_in = scene::harnix::HarnixScene;
        let attended = Attended::harnix();
        assert!(!built_in.interactive() && attended.interactive());
        assert_eq!(attended.id(), ATTENDED_SCENE);
        assert_eq!(attended.tools(), built_in.tools());
        assert_eq!(attended.deferred_tools(), built_in.deferred_tools());
        assert_eq!(attended.path_style(), built_in.path_style());
        assert_eq!(attended.bundled_skills(), built_in.bundled_skills());
    }
}
