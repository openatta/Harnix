//! A Harnix [`Tool`] as an Agix tool, a call's cancellation, and its chain.
//!
//! Agix cancels a tool call by dropping its future; the call's [`ToolCx::cancel`] is fired then
//! too (a guard held across the call), and when the turn is interrupted while the call runs —
//! the call is then waited for, so a tool that has something to stop gets to stop it.
//!
//! A call carries the chain of the turn its session is running ([`ToolCx::chain`]), read once as
//! it starts from the engine's [`ChainSource`].

use std::borrow::Cow;
use std::sync::Arc;

use base::error::ToolError as AgixError;
use base::interface::tool::{
    PermissionDecision, ProgressSender, PromptContext, Tool as AgixToolTrait, ToolContext, ToolResult,
    ToolResultContent,
};
use harnix_engine_api::{Cancel, ChainSource, NoChains, Permission, Progress, SessionRef, Tool, ToolCx, ToolError};
use serde_json::Value;

/// The engine a tool is called from: the instance its calls name their session by, and where a
/// call's chain comes from.
pub(crate) struct Site {
    pub(crate) instance: Arc<str>,
    chains: Arc<dyn ChainSource>,
}

impl Site {
    pub(crate) fn new(instance: &str, chains: Arc<dyn ChainSource>) -> Arc<Self> {
        Arc::new(Self { instance: instance.into(), chains })
    }

    /// An engine of `instance` whose turns no call waits on.
    #[cfg(test)]
    pub(crate) fn bare(instance: &str) -> Arc<Self> {
        Self::new(instance, Arc::new(NoChains))
    }
}

impl Default for Site {
    fn default() -> Self {
        Self { instance: "".into(), chains: Arc::new(NoChains) }
    }
}

/// A tool, and the engine that calls it.
pub(crate) struct AgixTool(Arc<dyn Tool>, Arc<Site>);

impl AgixTool {
    pub(crate) fn new(tool: Arc<dyn Tool>, site: Arc<Site>) -> Self {
        Self(tool, site)
    }
}

#[async_trait::async_trait]
impl AgixToolTrait for AgixTool {
    fn name(&self) -> &str {
        self.0.spec().name()
    }
    fn description(&self) -> &str {
        self.0.spec().description()
    }
    fn input_schema(&self) -> Value {
        self.0.spec().input_schema().clone()
    }
    fn source(&self) -> Cow<'_, str> {
        Cow::Borrowed(self.0.spec().origin().unwrap_or("builtin"))
    }
    async fn prompt(&self, _: &PromptContext) -> String {
        let spec = self.0.spec();
        match spec.documentation() {
            Some(doc) => format!("{}\n\n{doc}", spec.description()),
            None => spec.description().to_string(),
        }
    }
    fn is_read_only(&self, _: &Value) -> bool {
        self.0.spec().is_read_only()
    }
    fn is_concurrency_safe(&self, _: &Value) -> bool {
        self.0.spec().is_concurrency_safe()
    }
    fn is_destructive(&self, _: &Value) -> bool {
        self.0.spec().is_destructive()
    }
    async fn check_permissions(&self, input: &Value, _: &ToolContext) -> PermissionDecision {
        match self.0.permission(input) {
            Permission::Allow => PermissionDecision::allow(),
            Permission::Ask(question) => PermissionDecision::ask(question),
            Permission::Deny(reason) => PermissionDecision::deny(reason),
            _ => PermissionDecision::ask(format!("Run the tool `{}`?", self.0.spec().name())),
        }
    }
    async fn call(&self, input: Value, ctx: ToolContext, progress: ProgressSender) -> Result<ToolResult, AgixError> {
        // The engine's own id for the call; a context that has none still has the progress's.
        let tool_use_id = if ctx.tool_use_id.is_empty() {
            progress.tool_use_id().to_string()
        } else {
            ctx.tool_use_id.clone()
        };
        let cancel = Cancel::new();
        let _dropped = cancel.guard();
        let chain = self.1.chains.chain(&SessionRef::new(&*self.1.instance, ctx.session_id.as_str()));
        let cx = ToolCx::new(&ctx.session_id, &tool_use_id)
            .with_instance(&self.1.instance)
            .with_chain(chain)
            .with_project(ctx.cwd.clone())
            .with_depth(ctx.agent_depth)
            .with_cancel(cancel.clone())
            .with_progress(Progress::new(move |text| progress.send(text)));
        let call = self.0.call(cx, input);
        tokio::pin!(call);
        let out = tokio::select! {
            biased;
            out = &mut call => out,
            _ = ctx.cancel.cancelled() => {
                cancel.cancel();
                call.await
            }
        };
        match out {
            Ok(output) => Ok(ToolResult {
                content: ToolResultContent::Text(output.content().to_string()),
                is_error: output.is_error(),
                structured_content: output.structured().cloned(),
                ..ToolResult::default()
            }),
            Err(ToolError::Cancelled) => Err(AgixError::Cancelled),
            Err(ToolError::NotFound(what)) => Err(AgixError::NotFound(what)),
            Err(ToolError::Failed(why)) => Err(AgixError::Execution(anyhow::anyhow!(why))),
            Err(other) => Err(AgixError::Execution(anyhow::anyhow!(other.to_string()))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use harnix_engine_api::{ToolOutput, ToolSpec};
    use serde_json::json;

    /// What the adapter writes, and what it leaves to Agix's default on purpose: the spec has
    /// nothing to say to the rest.
    const WRITTEN: &[&str] = &[
        "name", "description", "input_schema", "source", "prompt", "is_read_only", "is_concurrency_safe",
        "is_destructive", "check_permissions", "call",
    ];
    const DEFAULTED: &[&str] = &[
        "full_input_schema", "input_schema_for", "prompt_fragment", "self_allow_overrides_mode", "detailed_prompt",
        "is_enabled", "callable_from_code", "code_result", "needs_dispatch", "strict", "is_deferred", "is_dynamic",
        "is_direct", "short_description", "permission_match_content", "permission_content_kind", "affected_paths",
        "interrupt_behavior", "validate_input",
    ];

    #[test]
    fn every_method_of_agixs_tool_is_written_or_left_to_its_default_on_purpose() {
        let agix = crate::adapt::trait_methods(&crate::adapt::agix_source("crates/core/src/interface/tool.rs"), "pub trait Tool:");
        let ours = crate::adapt::impl_methods(include_str!("tool.rs"), "impl AgixToolTrait for AgixTool");
        crate::adapt::assert_covers(agix, ours, WRITTEN, DEFAULTED);
    }

    struct Echo(ToolSpec);

    #[async_trait::async_trait]
    impl Tool for Echo {
        fn spec(&self) -> &ToolSpec {
            &self.0
        }
        fn permission(&self, _: &Value) -> Permission {
            Permission::Allow
        }
        async fn call(&self, cx: ToolCx, input: Value) -> Result<ToolOutput, ToolError> {
            if input["wait"].as_bool() == Some(true) {
                cx.cancel().cancelled().await;
                return Err(ToolError::Cancelled);
            }
            Ok(ToolOutput::text(format!("{} {}", cx.session_id(), input["text"])).with_structured(Some(json!(1))))
        }
    }

    #[tokio::test]
    async fn a_tool_is_told_what_agix_tells_and_answers_in_agixs_terms() {
        let spec = ToolSpec::new("echo", "says it back", json!({"type": "object"})).doc("at length").source("app:x").read_only();
        let tool = AgixTool::new(Arc::new(Echo(spec)), Site::bare("main"));
        assert_eq!((tool.name(), tool.description(), tool.source().as_ref()), ("echo", "says it back", "app:x"));
        assert_eq!(tool.prompt(&PromptContext::default()).await, "says it back\n\nat length");
        assert!(tool.is_read_only(&json!({})) && !tool.is_concurrency_safe(&json!({})));
        let mut ctx = ToolContext::for_test("/p".into());
        ctx.session_id = "s1".into();
        let out = tool.call(json!({"text": "hi"}), ctx.clone(), ProgressSender::noop("t1")).await.unwrap();
        assert!(matches!(out.content, ToolResultContent::Text(ref t) if t == "s1 \"hi\""));
        assert_eq!(out.structured_content, Some(json!(1)));

        let cancel = ctx.cancel.clone();
        let waiting = tool.call(json!({"wait": true}), ctx, ProgressSender::noop("t2"));
        cancel.cancel();
        assert!(matches!(waiting.await, Err(AgixError::Cancelled)));
    }

    /// A tool that says the chain it was called with, and fires when its call is let go of.
    struct Chained(ToolSpec, std::sync::Mutex<Option<Cancel>>);

    #[async_trait::async_trait]
    impl Tool for Chained {
        fn spec(&self) -> &ToolSpec {
            &self.0
        }
        fn permission(&self, _: &Value) -> Permission {
            Permission::Allow
        }
        async fn call(&self, cx: ToolCx, _: Value) -> Result<ToolOutput, ToolError> {
            *self.1.lock().unwrap() = Some(cx.cancel().clone());
            let chain: Vec<String> = cx.chain().iter().map(|l| format!("{}@{}", l.ext(), l.call())).collect();
            if cx.chain().is_empty() {
                std::future::pending::<()>().await;
            }
            Ok(ToolOutput::text(chain.join(",")))
        }
    }

    struct OneTurn;

    impl ChainSource for OneTurn {
        fn chain(&self, session: &SessionRef) -> Vec<harnix_engine_api::ChainLink> {
            match (session.instance.as_str(), session.session_id.as_str()) {
                ("main", "s1") => vec![harnix_engine_api::ChainLink::new("acme.notes", "7")],
                _ => Vec::new(),
            }
        }
    }

    #[tokio::test]
    async fn a_call_carries_its_turns_chain_and_a_dropped_one_is_cancelled() {
        let tool = Arc::new(Chained(ToolSpec::new("c", "d", json!({"type": "object"})), Default::default()));
        let adapted = AgixTool::new(tool.clone(), Site::new("main", Arc::new(OneTurn)));
        let mut ctx = ToolContext::for_test("/p".into());
        ctx.session_id = "s1".into();
        let out = adapted.call(json!({}), ctx.clone(), ProgressSender::noop("t1")).await.unwrap();
        assert!(matches!(out.content, ToolResultContent::Text(ref t) if t == "acme.notes@7"));

        // Another session's turn is on behalf of nothing; Agix drops the call, and the tool's
        // cancellation fires with it.
        ctx.session_id = "s2".into();
        let call = adapted.call(json!({}), ctx, ProgressSender::noop("t2"));
        let _ = tokio::time::timeout(std::time::Duration::from_millis(50), call).await;
        assert!(tool.1.lock().unwrap().as_ref().unwrap().is_cancelled(), "dropped and not cancelled");
    }
}
