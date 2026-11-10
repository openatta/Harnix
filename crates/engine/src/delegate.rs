//! `delegate_scene!`: a scene that is another scene in everything but what it says.
//!
//! Harnix wraps Agix's scenes twice — attended ([`crate::attended`]) and admitting the host's
//! tools ([`crate::admit`]) — and each wrapper changes one or two of `Scene`'s two dozen methods.
//! Written out by hand, the rest were two dozen copies of the same line per wrapper, and a method
//! left off fell back to the trait's default instead of the wrapped scene's, silently.
//!
//! ```ignore
//! delegate_scene!(Attended => 0; [id, name, /* … */];
//!     fn interactive(&self) -> bool { true }
//! );
//! ```
//!
//! The field (`0`, `inner`) is the wrapped `Arc<dyn Scene>`; the list names every method handed
//! to it; the items after it are the wrapper's own.

macro_rules! delegate_scene {
    ($ty:ty => $inner:tt; [$($method:ident),* $(,)?]; $($own:item)*) => {
        impl base::interface::scene::Scene for $ty {
            $( delegate_scene!(@one $inner $method); )*
            $($own)*
        }
    };
    (@one $f:tt id) => { fn id(&self) -> &str { self.$f.id() } };
    (@one $f:tt name) => { fn name(&self) -> &str { self.$f.name() } };
    (@one $f:tt description) => { fn description(&self) -> &str { self.$f.description() } };
    (@one $f:tt build_system_prompt) => {
        fn build_system_prompt(
            &self,
            ctx: &base::interface::scene::ScenePromptContext,
        ) -> Vec<base::interface::prompt::PromptBlock> {
            self.$f.build_system_prompt(ctx)
        }
    };
    (@one $f:tt tools) => { fn tools(&self) -> Vec<String> { self.$f.tools() } };
    (@one $f:tt requires_project) => { fn requires_project(&self) -> bool { self.$f.requires_project() } };
    (@one $f:tt supports_team) => { fn supports_team(&self) -> bool { self.$f.supports_team() } };
    (@one $f:tt extra_tools) => {
        fn extra_tools(&self) -> Vec<std::sync::Arc<dyn base::tool::Tool>> { self.$f.extra_tools() }
    };
    (@one $f:tt token_budget) => {
        fn token_budget(&self) -> base::interface::scene::TokenBudget { self.$f.token_budget() }
    };
    (@one $f:tt build_system_reminder) => {
        fn build_system_reminder(&self, ctx: &base::interface::scene::ReminderContext) -> String {
            self.$f.build_system_reminder(ctx)
        }
    };
    (@one $f:tt build_session_context) => {
        fn build_session_context(&self, ctx: &base::interface::scene::ScenePromptContext) -> String {
            self.$f.build_session_context(ctx)
        }
    };
    (@one $f:tt disallowed_tools) => { fn disallowed_tools(&self) -> Vec<String> { self.$f.disallowed_tools() } };
    (@one $f:tt permits_tool) => { fn permits_tool(&self, name: &str) -> bool { self.$f.permits_tool(name) } };
    (@one $f:tt deferred_tools) => { fn deferred_tools(&self) -> Vec<String> { self.$f.deferred_tools() } };
    (@one $f:tt path_style) => {
        fn path_style(&self) -> base::context::config::PathStyle { self.$f.path_style() }
    };
    (@one $f:tt interactive) => { fn interactive(&self) -> bool { self.$f.interactive() } };
    (@one $f:tt bundled_skills) => { fn bundled_skills(&self) -> Vec<String> { self.$f.bundled_skills() } };
    (@one $f:tt modules) => { fn modules(&self) -> Option<Vec<String>> { self.$f.modules() } };
    (@one $f:tt disabled_modules) => { fn disabled_modules(&self) -> Vec<String> { self.$f.disabled_modules() } };
    (@one $f:tt permits_module) => { fn permits_module(&self, id: &str) -> bool { self.$f.permits_module(id) } };
    (@one $f:tt execution_params) => {
        fn execution_params(&self) -> base::interface::scene::ExecutionParams { self.$f.execution_params() }
    };
    (@one $f:tt auto_name_session) => { fn auto_name_session(&self) -> bool { self.$f.auto_name_session() } };
    (@one $f:tt session_name_prompt) => {
        fn session_name_prompt(&self, first_message: &str) -> Option<String> {
            self.$f.session_name_prompt(first_message)
        }
    };
    (@one $f:tt memory_extraction_prompt) => {
        fn memory_extraction_prompt(&self) -> Option<String> { self.$f.memory_extraction_prompt() }
    };
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use base::context::config::PathStyle;
    use base::interface::prompt::PromptBlock;
    use base::interface::scene::{Scene, ScenePromptContext, TokenBudget};

    use crate::admit::Admitting;
    use crate::attended::Attended;

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
            Vec::new()
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
            vec!["Bash".into()]
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
            false
        }
        fn session_name_prompt(&self, first: &str) -> Option<String> {
            Some(format!("name {first}"))
        }
        fn memory_extraction_prompt(&self) -> Option<String> {
            Some("remember".into())
        }
    }

    fn same(a: &dyn Scene, b: &dyn Scene) {
        assert_eq!((a.id(), a.name(), a.description()), (b.id(), b.name(), b.description()));
        assert_eq!((a.requires_project(), a.supports_team()), (b.requires_project(), b.supports_team()));
        let (ta, tb) = (a.token_budget(), b.token_budget());
        assert_eq!((ta.compact_threshold, ta.compact_keep_recent), (tb.compact_threshold, tb.compact_keep_recent));
        assert_eq!((a.disallowed_tools(), a.deferred_tools()), (b.disallowed_tools(), b.deferred_tools()));
        assert_eq!(a.path_style(), b.path_style());
        assert_eq!((a.bundled_skills(), a.modules(), a.disabled_modules()), (b.bundled_skills(), b.modules(), b.disabled_modules()));
        assert_eq!((a.permits_module("m"), a.permits_module("x")), (b.permits_module("m"), b.permits_module("x")));
        assert_eq!(a.auto_name_session(), b.auto_name_session());
        assert_eq!(a.session_name_prompt("hi"), b.session_name_prompt("hi"));
        assert_eq!(a.memory_extraction_prompt(), b.memory_extraction_prompt());
        assert_eq!(a.execution_params().max_api_calls_per_turn, b.execution_params().max_api_calls_per_turn);
    }

    /// Each wrapper is the scene it wraps in everything it does not say otherwise: a method the
    /// list left off would answer the trait's default here, and differ.
    #[test]
    fn a_wrapped_scene_answers_as_the_scene_it_wraps() {
        let odd: Arc<dyn Scene> = Arc::new(Odd);
        let attended = Attended(odd.clone());
        same(&attended, &Odd);
        assert_eq!(attended.tools(), Odd.tools());
        assert!(attended.interactive() && !Odd.interactive());

        let admitting = Admitting::new(odd.clone(), vec!["app__*".into()]);
        same(&admitting, &Odd);
        assert_eq!(admitting.interactive(), Odd.interactive());
        assert!(admitting.permits_tool("app__x") && !Odd.permits_tool("app__x"));
    }
}
