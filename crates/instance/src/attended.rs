//! Agix's `harnix` scene, attended: what every engine Harnix builds serves under that id.
//!
//! Agix's scene is unattended — a call its permission policy would ask about is refused before
//! anyone is asked. A Harnix session has someone to ask: the person at the interface (every
//! watcher gets the prompt, the first answer wins) or a module that answers prompts itself. So
//! the engine gets this one in its place, which is the built-in scene in everything but
//! [`Scene::interactive`].

use std::sync::Arc;

use base::context::config::PathStyle;
use base::interface::prompt::PromptBlock;
use base::interface::scene::{
    ExecutionParams, ReminderContext, Scene, ScenePromptContext, TokenBudget,
};
use base::tool::Tool;

/// The id it is served under: the built-in one's.
pub const ATTENDED_SCENE: &str = "harnix";

/// A base scene that is attended.
pub struct Attended(Arc<dyn Scene>);

impl Attended {
    /// Agix's `harnix` scene, attended.
    pub fn harnix() -> Self {
        Self(Arc::new(scene::harnix::HarnixScene))
    }
}

impl Scene for Attended {
    fn id(&self) -> &str {
        self.0.id()
    }
    fn name(&self) -> &str {
        self.0.name()
    }
    fn description(&self) -> &str {
        self.0.description()
    }
    fn build_system_prompt(&self, ctx: &ScenePromptContext) -> Vec<PromptBlock> {
        self.0.build_system_prompt(ctx)
    }
    fn tools(&self) -> Vec<String> {
        self.0.tools()
    }
    fn requires_project(&self) -> bool {
        self.0.requires_project()
    }
    fn supports_team(&self) -> bool {
        self.0.supports_team()
    }
    fn extra_tools(&self) -> Vec<Arc<dyn Tool>> {
        self.0.extra_tools()
    }
    fn token_budget(&self) -> TokenBudget {
        self.0.token_budget()
    }
    fn build_system_reminder(&self, ctx: &ReminderContext) -> String {
        self.0.build_system_reminder(ctx)
    }
    fn build_session_context(&self, ctx: &ScenePromptContext) -> String {
        self.0.build_session_context(ctx)
    }
    fn disallowed_tools(&self) -> Vec<String> {
        self.0.disallowed_tools()
    }
    fn permits_tool(&self, name: &str) -> bool {
        self.0.permits_tool(name)
    }
    fn deferred_tools(&self) -> Vec<String> {
        self.0.deferred_tools()
    }
    fn path_style(&self) -> PathStyle {
        self.0.path_style()
    }
    fn interactive(&self) -> bool {
        true
    }
    fn bundled_skills(&self) -> Vec<String> {
        self.0.bundled_skills()
    }
    fn modules(&self) -> Option<Vec<String>> {
        self.0.modules()
    }
    fn disabled_modules(&self) -> Vec<String> {
        self.0.disabled_modules()
    }
    fn permits_module(&self, id: &str) -> bool {
        self.0.permits_module(id)
    }
    fn execution_params(&self) -> ExecutionParams {
        self.0.execution_params()
    }
    fn auto_name_session(&self) -> bool {
        self.0.auto_name_session()
    }
    fn session_name_prompt(&self, first_message: &str) -> Option<String> {
        self.0.session_name_prompt(first_message)
    }
    fn memory_extraction_prompt(&self) -> Option<String> {
        self.0.memory_extraction_prompt()
    }
}

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
