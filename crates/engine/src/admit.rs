//! Scenes that admit a host's own tools besides their own list.
//!
//! A scene with a tool list keeps every other tool out of its sessions — plugin tools included,
//! which Agix registers and the scene's policy then removes. A host that contributes tools through
//! a plugin to every session of an instance (Harnix's runtime applications, `app__*`) needs every
//! scene of that instance to let them in. [`Admitting`] is a scene with the same everything as the
//! one it wraps, and a tool policy that also admits the given names (`prefix*` or exact). A scene
//! that lists no tools admits everything not disallowed already, and is left as it is.

use std::sync::Arc;

use base::interface::scene::Scene;

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

delegate_scene!(Admitting => inner; [
    id, name, description, build_system_prompt, requires_project, supports_team, extra_tools,
    token_budget, build_system_reminder, build_session_context, disallowed_tools, deferred_tools,
    path_style, interactive, bundled_skills, modules, disabled_modules, permits_module,
    execution_params, auto_name_session, session_name_prompt, memory_extraction_prompt,
];
    fn tools(&self) -> Vec<String> {
        let mut tools = self.inner.tools();
        if !tools.is_empty() {
            tools.extend(self.admitted.iter().cloned());
        }
        tools
    }
    fn permits_tool(&self, name: &str) -> bool {
        self.inner.permits_tool(name) || (self.admits(name) && !self.inner.tools().is_empty())
    }
);

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
