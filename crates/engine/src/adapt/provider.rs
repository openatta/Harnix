//! A [`ToolProvider`] as an Agix plugin, its [`ToolHook`]s as tool middleware.

use std::sync::Arc;

use base::interface::tool_middleware::{NextDispatch, ToolAnswer, ToolCall, ToolExec, ToolMiddleware, ToolOutcome};
use harnix_engine_api::{Cancel, HookDecision, SessionFacts, ToolCallFacts, ToolHook, ToolProvider};
use runtime::plugin::{Plugin, PluginSession, Registrar};

use super::tool::AgixTool;

/// Registered for every session an engine builds, delegates included: what the provider gives
/// that session, then.
pub(crate) struct AgixPlugin(Arc<dyn ToolProvider>, Arc<str>);

impl AgixPlugin {
    /// `provider`'s tools for the sessions of `instance`'s engine.
    pub(crate) fn new(provider: Arc<dyn ToolProvider>, instance: Arc<str>) -> Self {
        Self(provider, instance)
    }
}

impl Plugin for AgixPlugin {
    fn name(&self) -> &str {
        self.0.name()
    }

    fn register(&self, session: &PluginSession<'_>, r: &mut Registrar) {
        let facts = SessionFacts::new(session.session_id, session.project_root, session.scene, session.depth);
        for tool in self.0.tools(&facts) {
            r.tool(Arc::new(AgixTool::new(tool, self.1.clone())));
        }
        for hook in self.0.hooks(&facts) {
            r.tool_middleware(Arc::new(AgixHook(hook)));
        }
    }
}

/// A hook around every tool call of a session: asked first, then the call runs, or does not.
struct AgixHook(Arc<dyn ToolHook>);

#[async_trait::async_trait]
impl ToolMiddleware for AgixHook {
    async fn around(&self, call: &ToolCall, exec: &mut ToolExec, next: NextDispatch<'_>) -> ToolOutcome {
        let cancel = Cancel::new();
        let facts = ToolCallFacts::new(&call.name, call.input.clone(), cancel.clone());
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
}
