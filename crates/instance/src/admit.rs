//! Scenes that admit a host's own tools besides their own list.
//!
//! A scene with a tool list keeps every other tool out of its sessions — plugin tools included,
//! which Agix registers and the scene's policy then removes. A host that contributes tools through
//! a plugin to every session of an instance (Harnix's runtime applications, `app__*`) needs every
//! scene of that instance to let them in. [`Admitting`] is a scene with the same everything as the
//! one it wraps, and a tool policy that also admits the given names (`prefix*` or exact). A scene
//! that lists no tools admits everything not disallowed already, and is left as it is.

use std::sync::Arc;

use base::context::config::PathStyle;
use base::interface::prompt::PromptBlock;
use base::interface::scene::{ExecutionParams, ReminderContext, Scene, ScenePromptContext, TokenBudget};
use base::tool::Tool;

pub struct Admitting {
    inner: Arc<dyn Scene>,
    admitted: Vec<String>,
}

impl Admitting {
    pub fn new(inner: Arc<dyn Scene>, admitted: Vec<String>) -> Self {
        Self { inner, admitted }
    }

    fn admits(&self, name: &str) -> bool {
        self.admitted.iter().any(|entry| match entry.strip_suffix('*') {
            Some(prefix) => name.starts_with(prefix),
            None => entry == name,
        })
    }
}

impl Scene for Admitting {
    fn id(&self) -> &str {
        self.inner.id()
    }
    fn name(&self) -> &str {
        self.inner.name()
    }
    fn description(&self) -> &str {
        self.inner.description()
    }
    fn build_system_prompt(&self, ctx: &ScenePromptContext) -> Vec<PromptBlock> {
        self.inner.build_system_prompt(ctx)
    }
    fn tools(&self) -> Vec<String> {
        let mut tools = self.inner.tools();
        if !tools.is_empty() {
            tools.extend(self.admitted.iter().cloned());
        }
        tools
    }
    fn requires_project(&self) -> bool {
        self.inner.requires_project()
    }
    fn supports_team(&self) -> bool {
        self.inner.supports_team()
    }
    fn extra_tools(&self) -> Vec<Arc<dyn Tool>> {
        self.inner.extra_tools()
    }
    fn token_budget(&self) -> TokenBudget {
        self.inner.token_budget()
    }
    fn build_system_reminder(&self, ctx: &ReminderContext) -> String {
        self.inner.build_system_reminder(ctx)
    }
    fn build_session_context(&self, ctx: &ScenePromptContext) -> String {
        self.inner.build_session_context(ctx)
    }
    fn disallowed_tools(&self) -> Vec<String> {
        self.inner.disallowed_tools()
    }
    fn permits_tool(&self, name: &str) -> bool {
        self.inner.permits_tool(name) || (self.admits(name) && !self.inner.tools().is_empty())
    }
    fn deferred_tools(&self) -> Vec<String> {
        self.inner.deferred_tools()
    }
    fn path_style(&self) -> PathStyle {
        self.inner.path_style()
    }
    fn interactive(&self) -> bool {
        self.inner.interactive()
    }
    fn bundled_skills(&self) -> Vec<String> {
        self.inner.bundled_skills()
    }
    fn modules(&self) -> Option<Vec<String>> {
        self.inner.modules()
    }
    fn disabled_modules(&self) -> Vec<String> {
        self.inner.disabled_modules()
    }
    fn permits_module(&self, id: &str) -> bool {
        self.inner.permits_module(id)
    }
    fn execution_params(&self) -> ExecutionParams {
        self.inner.execution_params()
    }
    fn auto_name_session(&self) -> bool {
        self.inner.auto_name_session()
    }
    fn session_name_prompt(&self, first_message: &str) -> Option<String> {
        self.inner.session_name_prompt(first_message)
    }
    fn memory_extraction_prompt(&self) -> Option<String> {
        self.inner.memory_extraction_prompt()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_listed_scene_admits_the_hosts_tools_and_nothing_else_changes() {
        let inner: Arc<dyn Scene> = Arc::new(crate::attended::Attended::harnix());
        let scene = Admitting::new(inner.clone(), vec!["app__*".into()]);
        assert!(!inner.permits_tool("app__acme_notes__search"));
        assert!(scene.permits_tool("app__acme_notes__search"));
        assert!(scene.permits_tool("Bash") && scene.permits_tool("mcp__github__x"));
        assert!(!scene.permits_tool("plugin__other__x"));
        assert!(scene.tools().contains(&"app__*".to_string()));
        assert_eq!((scene.id(), scene.interactive()), (inner.id(), inner.interactive()));
        assert_eq!(scene.deferred_tools(), inner.deferred_tools());
    }
}
