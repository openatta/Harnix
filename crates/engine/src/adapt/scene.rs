//! A [`SceneLayer`] as an Agix scene: the one place each of `Scene`'s methods is handed to a base.
//!
//! Every wrapper Harnix puts around a scene — attended, admitting the host's tools, flow's three —
//! is a layer, and this is the only `impl Scene` that forwards. The test at the bottom reads the
//! names of `Scene`'s methods out of Agix's source and compares them with the ones written here:
//! a method Agix adds is red here, rather than silently answered by the trait's default.

use std::sync::Arc;

use base::context::config::PathStyle;
use base::interface::prompt::PromptBlock;
use base::interface::scene::{ExecutionParams, ReminderContext, Scene, ScenePromptContext, TokenBudget};
use harnix_engine_api::scene::SessionContext;
use harnix_engine_api::{SceneFacts, SceneLayer};

use super::tool::{AgixTool, Site};

/// A layer, built over the scene its base resolved to.
pub(crate) struct Layered {
    base: Arc<dyn Scene>,
    id: String,
    name: String,
    description: String,
    prompts: Vec<PromptBlock>,
    tools: Vec<Arc<dyn base::tool::Tool>>,
    tool_names: Vec<String>,
    admitted: Vec<String>,
    undisallowed: Vec<String>,
    interactive: Option<bool>,
    requires_project: Option<bool>,
    auto_name: Option<bool>,
    name_prompt: Option<Option<String>>,
    session_context: Option<SessionContext>,
}

impl Layered {
    /// `layer` over `base`, its tools called by sessions of `site`'s engine.
    pub(crate) fn new(layer: &SceneLayer, base: Arc<dyn Scene>, site: &Arc<Site>) -> Self {
        let p = layer.parts();
        let fill = |pattern: Option<&str>, own: &str| match pattern {
            Some(pattern) => pattern.replace("{base}", own),
            None => own.to_string(),
        };
        Self {
            id: fill(Some(p.id), base.id()),
            name: fill(p.name, base.name()),
            description: fill(p.description, base.description()),
            prompts: p
                .prompts
                .iter()
                .map(|t| if t.is_cached() { PromptBlock::system_cached(t.text()) } else { PromptBlock::system(t.text()) })
                .collect(),
            tools: p.tools.iter().map(|t| Arc::new(AgixTool::new(t.clone(), site.clone())) as Arc<dyn base::tool::Tool>).collect(),
            tool_names: p.tools.iter().map(|t| t.spec().name().to_string()).collect(),
            admitted: p.admitted.to_vec(),
            undisallowed: p.undisallowed.to_vec(),
            interactive: p.interactive,
            requires_project: p.requires_project,
            auto_name: p.auto_name,
            name_prompt: p.name_prompt.map(|o| o.map(str::to_string)),
            session_context: p.session_context.cloned(),
            base,
        }
    }

    fn admits(&self, name: &str) -> bool {
        self.admitted.iter().any(|entry| match entry.strip_suffix('*') {
            Some(prefix) => name.starts_with(prefix),
            None => entry == name,
        })
    }
}

impl Scene for Layered {
    fn id(&self) -> &str {
        &self.id
    }
    fn name(&self) -> &str {
        &self.name
    }
    fn description(&self) -> &str {
        &self.description
    }
    fn build_system_prompt(&self, ctx: &ScenePromptContext) -> Vec<PromptBlock> {
        let mut blocks = self.base.build_system_prompt(ctx);
        blocks.extend(self.prompts.iter().cloned());
        blocks
    }
    /// The base's list, then the layer's tools and the names it admits; an empty list (every
    /// tool) stays empty.
    fn tools(&self) -> Vec<String> {
        let mut tools = self.base.tools();
        if !tools.is_empty() {
            tools.extend(self.tool_names.iter().cloned());
            tools.extend(self.admitted.iter().cloned());
        }
        tools
    }
    fn requires_project(&self) -> bool {
        self.requires_project.unwrap_or_else(|| self.base.requires_project())
    }
    fn supports_team(&self) -> bool {
        self.base.supports_team()
    }
    fn extra_tools(&self) -> Vec<Arc<dyn base::tool::Tool>> {
        let mut tools = self.base.extra_tools();
        tools.extend(self.tools.iter().cloned());
        tools
    }
    fn token_budget(&self) -> TokenBudget {
        self.base.token_budget()
    }
    fn build_system_reminder(&self, ctx: &ReminderContext) -> String {
        self.base.build_system_reminder(ctx)
    }
    fn build_session_context(&self, ctx: &ScenePromptContext) -> String {
        let base = self.base.build_session_context(ctx);
        let Some(f) = &self.session_context else {
            return base;
        };
        let facts = SceneFacts::new(&ctx.cwd, &ctx.os, &ctx.shell, &ctx.date, &ctx.model_name);
        let own = f(&facts);
        match (base.is_empty(), own.is_empty()) {
            (_, true) => base,
            (true, false) => own,
            (false, false) => format!("{base}\n\n{own}"),
        }
    }
    fn disallowed_tools(&self) -> Vec<String> {
        self.base
            .disallowed_tools()
            .into_iter()
            .filter(|t| !self.undisallowed.contains(t))
            .collect()
    }
    /// The layer's own tools and what it took off the disallowed list always; what it admits
    /// where the base has a list; otherwise the base's answer.
    fn permits_tool(&self, name: &str) -> bool {
        self.tool_names.iter().any(|t| t == name)
            || self.undisallowed.iter().any(|t| t == name)
            || self.base.permits_tool(name)
            || (self.admits(name) && !self.base.tools().is_empty())
    }
    fn deferred_tools(&self) -> Vec<String> {
        self.base.deferred_tools()
    }
    fn path_style(&self) -> PathStyle {
        self.base.path_style()
    }
    fn interactive(&self) -> bool {
        self.interactive.unwrap_or_else(|| self.base.interactive())
    }
    fn bundled_skills(&self) -> Vec<String> {
        self.base.bundled_skills()
    }
    fn modules(&self) -> Option<Vec<String>> {
        self.base.modules()
    }
    fn disabled_modules(&self) -> Vec<String> {
        self.base.disabled_modules()
    }
    fn permits_module(&self, id: &str) -> bool {
        self.base.permits_module(id)
    }
    fn execution_params(&self) -> ExecutionParams {
        self.base.execution_params()
    }
    fn auto_name_session(&self) -> bool {
        self.auto_name.unwrap_or_else(|| self.base.auto_name_session())
    }
    fn session_name_prompt(&self, first_message: &str) -> Option<String> {
        match &self.name_prompt {
            Some(own) => own.as_ref().map(|p| p.replace("{message}", first_message)),
            None => self.base.session_name_prompt(first_message),
        }
    }
    fn memory_extraction_prompt(&self) -> Option<String> {
        self.base.memory_extraction_prompt()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use base::interface::prompt::PromptBlock;
    use harnix_engine_api::{PromptText, SceneBase};

    #[test]
    fn every_method_of_agixs_scene_is_written_here() {
        let agix = crate::adapt::trait_methods(&crate::adapt::agix_source("crates/core/src/interface/scene.rs"), "pub trait Scene:");
        assert!(agix.len() > 20, "read {} methods of Scene: {agix:?}", agix.len());
        let ours = crate::adapt::impl_methods(include_str!("scene.rs"), "impl Scene for Layered");
        let written: Vec<&str> = agix.iter().map(String::as_str).collect();
        crate::adapt::assert_covers(agix.clone(), ours, &written, &[]);
    }

    /// A scene that answers nothing the trait's defaults would.
    struct Odd;

    impl Scene for Odd {
        fn id(&self) -> &str {
            "odd"
        }
        fn name(&self) -> &str {
            "Odd"
        }
        fn description(&self) -> &str {
            "unlike the defaults"
        }
        fn build_system_prompt(&self, _ctx: &ScenePromptContext) -> Vec<PromptBlock> {
            vec![PromptBlock::system("odd's")]
        }
        fn tools(&self) -> Vec<String> {
            vec!["Read".into()]
        }
        fn token_budget(&self) -> TokenBudget {
            TokenBudget { compact_threshold: 7, compact_keep_recent: 3 }
        }
        fn requires_project(&self) -> bool {
            true
        }
        fn supports_team(&self) -> bool {
            true
        }
        fn disallowed_tools(&self) -> Vec<String> {
            vec!["Bash".into(), "StructuredOutput".into()]
        }
        fn deferred_tools(&self) -> Vec<String> {
            vec!["Grep".into()]
        }
        fn path_style(&self) -> PathStyle {
            PathStyle::WorkspaceRelative
        }
        fn bundled_skills(&self) -> Vec<String> {
            vec!["skill".into()]
        }
        fn modules(&self) -> Option<Vec<String>> {
            Some(vec!["m".into()])
        }
        fn disabled_modules(&self) -> Vec<String> {
            vec!["d".into()]
        }
        fn permits_module(&self, id: &str) -> bool {
            id == "m"
        }
        fn auto_name_session(&self) -> bool {
            true
        }
        fn session_name_prompt(&self, first: &str) -> Option<String> {
            Some(format!("name {first}"))
        }
        fn memory_extraction_prompt(&self) -> Option<String> {
            Some("remember".into())
        }
    }

    fn odd() -> Arc<dyn Scene> {
        Arc::new(Odd)
    }

    /// A layer that says nothing is its base in everything: a method left to the trait's default
    /// would answer differently here.
    #[test]
    fn a_layer_that_says_nothing_is_its_base() {
        let s = Layered::new(&SceneLayer::new("{base}", SceneBase::builtin("odd")), odd(), &Site::bare("main"));
        let (a, b): (&dyn Scene, &dyn Scene) = (&s, &Odd);
        assert_eq!((a.id(), a.name(), a.description()), (b.id(), b.name(), b.description()));
        assert_eq!(a.build_system_prompt(&ctx()).len(), 1);
        assert_eq!((a.tools(), a.requires_project(), a.supports_team()), (b.tools(), b.requires_project(), b.supports_team()));
        let (ta, tb) = (a.token_budget(), b.token_budget());
        assert_eq!((ta.compact_threshold, ta.compact_keep_recent), (tb.compact_threshold, tb.compact_keep_recent));
        assert_eq!((a.disallowed_tools(), a.deferred_tools()), (b.disallowed_tools(), b.deferred_tools()));
        assert_eq!((a.path_style(), a.interactive()), (b.path_style(), b.interactive()));
        assert_eq!((a.bundled_skills(), a.modules(), a.disabled_modules()), (b.bundled_skills(), b.modules(), b.disabled_modules()));
        assert_eq!((a.permits_module("m"), a.permits_module("x")), (b.permits_module("m"), b.permits_module("x")));
        assert_eq!((a.auto_name_session(), a.session_name_prompt("hi")), (b.auto_name_session(), b.session_name_prompt("hi")));
        assert_eq!(a.memory_extraction_prompt(), b.memory_extraction_prompt());
        assert_eq!(a.execution_params().max_api_calls_per_turn, b.execution_params().max_api_calls_per_turn);
        for name in ["Read", "Bash", "Edit", "StructuredOutput"] {
            assert_eq!(a.permits_tool(name), b.permits_tool(name), "{name}");
        }
    }

    #[test]
    fn a_layer_changes_what_it_says_and_names_itself_after_its_base() {
        let layer = SceneLayer::new("structured-{base}", SceneBase::builtin("odd"))
            .name("{base} (structured)")
            .prompt(PromptText::cached("more"))
            .admit("StructuredOutput")
            .undisallow("StructuredOutput")
            .interactive(true)
            .requires_project(false)
            .auto_name(false)
            .name_prompt(None);
        let s = Layered::new(&layer, odd(), &Site::bare("main"));
        assert_eq!((s.id(), s.name(), s.description()), ("structured-odd", "Odd (structured)", "unlike the defaults"));
        let blocks = s.build_system_prompt(&ctx());
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[1].content, "more");
        assert_eq!(s.tools(), ["Read", "StructuredOutput"]);
        assert_eq!(s.disallowed_tools(), ["Bash"]);
        assert!(s.permits_tool("StructuredOutput") && !s.permits_tool("Bash"));
        assert!(s.interactive() && !s.requires_project() && !s.auto_name_session());
        assert_eq!(s.session_name_prompt("hi"), None);
    }

    #[test]
    fn what_a_layer_admits_needs_a_list_to_matter() {
        struct Open;
        impl Scene for Open {
            fn id(&self) -> &str {
                "open"
            }
            fn name(&self) -> &str {
                "Open"
            }
            fn description(&self) -> &str {
                "every tool"
            }
            fn build_system_prompt(&self, _ctx: &ScenePromptContext) -> Vec<PromptBlock> {
                Vec::new()
            }
            fn tools(&self) -> Vec<String> {
                Vec::new()
            }
            fn token_budget(&self) -> TokenBudget {
                TokenBudget { compact_threshold: 0, compact_keep_recent: 0 }
            }
        }
        let s = Layered::new(&SceneLayer::new("{base}", SceneBase::builtin("open")).admit("ext__*"), Arc::new(Open), &Site::bare("main"));
        assert!(s.tools().is_empty(), "an open list stays open");
        assert!(s.permits_tool("ext__acme__x") && s.permits_tool("Bash"));
        let listed = Layered::new(&SceneLayer::new("{base}", SceneBase::builtin("odd")).admit("ext__*"), odd(), &Site::bare("main"));
        assert!(listed.permits_tool("ext__acme__x") && !Odd.permits_tool("ext__acme__x"));
        assert!(!listed.permits_tool("plugin__other__x"));
    }

    fn ctx() -> ScenePromptContext<'static> {
        use std::borrow::Cow;
        ScenePromptContext {
            cwd: Cow::Borrowed("/p"),
            os: Cow::Borrowed("macos"),
            shell: Cow::Borrowed("/bin/zsh"),
            home_dir: Cow::Borrowed("/h"),
            date: Cow::Borrowed("2026-11-09"),
            model_name: Cow::Borrowed("m"),
            skills_text: None,
            mcp_instructions: None,
            session_memory: None,
            is_git: false,
            git_branch: None,
            is_worktree: false,
            git_status: None,
            language: None,
            scratchpad_dir: None,
            output_style_content: None,
            available_tools: None,
            tool_results_ever_cleared: false,
        }
    }
}
