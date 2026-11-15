//! A [`ToolProvider`] as an Agix plugin, its [`ToolHook`]s as tool middleware.

use std::path::PathBuf;
use std::sync::Arc;

use base::interface::tool_middleware::{NextDispatch, ToolAnswer, ToolCall, ToolExec, ToolMiddleware, ToolOutcome};
use harnix_engine_api::{Cancel, HookDecision, ToolCallFacts, ToolHook, ToolProvider};
use runtime::plugin::{Plugin, PluginSession, Registrar};

use super::tool::{AgixTool, Site};

/// Registered for every session an engine builds, delegates included: what the provider gives
/// that session, then.
pub(crate) struct AgixPlugin(Arc<dyn ToolProvider>, Arc<Site>);

impl AgixPlugin {
    /// `provider`'s tools for the sessions of `site`'s engine.
    pub(crate) fn new(provider: Arc<dyn ToolProvider>, site: Arc<Site>) -> Self {
        Self(provider, site)
    }
}

impl Plugin for AgixPlugin {
    fn name(&self) -> &str {
        self.0.name()
    }

    fn register(&self, session: &PluginSession<'_>, r: &mut Registrar) {
        for tool in self.0.tools() {
            r.tool(Arc::new(AgixTool::new(tool, self.1.clone())));
        }
        for hook in self.0.hooks() {
            let at = Session { id: session.session_id.to_string(), depth: session.depth, cwd: session.project_root.to_path_buf() };
            r.tool_middleware(Arc::new(AgixHook(hook, at)));
        }
    }
}

/// A hook around every tool call of a session: asked first, then the call runs, or does not.
struct AgixHook(Arc<dyn ToolHook>, Session);

/// The session a hook is registered for, which a call's facts say.
struct Session {
    id: String,
    depth: u32,
    cwd: PathBuf,
}

impl AgixHook {
    /// What the hook is told of `call`: the session it was registered for, and where it works.
    fn facts(&self, call: &ToolCall, cancel: Cancel) -> ToolCallFacts {
        ToolCallFacts::new(&call.name, call.input.clone(), cancel)
            .with_session(&self.1.id, self.1.depth)
            .with_cwd(self.1.cwd.clone())
    }
}

#[async_trait::async_trait]
impl ToolMiddleware for AgixHook {
    async fn around(&self, call: &ToolCall, exec: &mut ToolExec, next: NextDispatch<'_>) -> ToolOutcome {
        let cancel = Cancel::new();
        let facts = self.facts(call, cancel.clone());
        let decision = {
            let _dropped = cancel.guard();
            let turn = exec.cancel().clone();
            let asked = self.0.before(&facts);
            tokio::pin!(asked);
            tokio::select! {
                biased;
                decision = &mut asked => decision,
                _ = turn.cancelled() => {
                    cancel.cancel();
                    asked.await
                }
            }
        };
        match decision {
            HookDecision::Block(reason) => Ok(ToolAnswer::failure(reason)),
            HookDecision::AddContext(note) => {
                let mut answer = next.run(call, exec).await?;
                answer.text.push_str(&note);
                Ok(answer)
            }
            _ => next.run(call, exec).await,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapt::{agix_source, assert_covers, impl_methods, trait_methods};

    #[test]
    fn every_method_of_agixs_plugin_is_written_or_left_to_its_default_on_purpose() {
        let agix = trait_methods(&agix_source("crates/runtime/src/plugin.rs"), "pub trait Plugin:");
        let ours = impl_methods(include_str!("provider.rs"), "impl Plugin for AgixPlugin");
        // A provider's tools are Rust's: the layer is Agix's default, `Rust`.
        assert_covers(agix, ours, &["name", "register"], &["layer"]);
    }

    #[test]
    fn every_method_of_agixs_tool_middleware_is_written() {
        let agix = trait_methods(&agix_source("crates/core/src/interface/tool_middleware.rs"), "pub trait ToolMiddleware:");
        let ours = impl_methods(include_str!("provider.rs"), "impl ToolMiddleware for AgixHook");
        assert_covers(agix, ours, &["around"], &[]);
    }

    struct Quiet;

    #[async_trait::async_trait]
    impl ToolHook for Quiet {
        async fn before(&self, _: &ToolCallFacts) -> HookDecision {
            HookDecision::Proceed
        }
    }

    #[test]
    fn a_hooks_facts_carry_the_session_depth_and_cwd_it_was_registered_for() {
        let at = Session { id: "s9".into(), depth: 1, cwd: PathBuf::from("/work") };
        let hook = AgixHook(Arc::new(Quiet), at);
        let call = ToolCall { name: "Bash".into(), input: serde_json::json!({"command": "ls"}) };
        let facts = hook.facts(&call, Cancel::new());
        assert_eq!((facts.name(), facts.session_id(), facts.depth()), ("Bash", "s9", 1));
        assert_eq!(facts.cwd(), std::path::Path::new("/work"));
        assert_eq!(facts.input()["command"], "ls");
    }
}
