//! What the host does with engine contributions, which an extension never needs to: gather
//! several into one, and switch one off with the extension that gave it.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use harnix_engine_api::{
    EngineContribution, PromptDecision, PromptFacts, PromptGate, SessionFacts, Tool, ToolHook, ToolProvider,
};

/// `from`'s, after what `into` has.
pub(crate) fn merge(into: &mut EngineContribution, from: &EngineContribution) {
    for layer in from.scenes() {
        into.scene(layer.clone());
    }
    for provider in from.providers() {
        into.provider(provider.clone());
    }
    for gate in from.prompt_gates() {
        into.prompt_gate(gate.clone());
    }
    for pattern in from.admitted() {
        into.admit_everywhere(pattern);
    }
}

/// The same contribution, with its providers, gates and hooks doing nothing while `enabled` is
/// false: an extension that fails after its instance was built cannot take its scenes back out of
/// the engine, but it stops reaching sessions built after.
pub(crate) fn gated(c: &EngineContribution, enabled: Arc<AtomicBool>) -> EngineContribution {
    let mut out = EngineContribution::new();
    for layer in c.scenes() {
        out.scene(layer.clone());
    }
    for inner in c.providers() {
        out.provider(Arc::new(Gated { inner: inner.clone(), enabled: enabled.clone() }));
    }
    for inner in c.prompt_gates() {
        out.prompt_gate(Arc::new(Gated { inner: inner.clone(), enabled: enabled.clone() }));
    }
    for pattern in c.admitted() {
        out.admit_everywhere(pattern);
    }
    out
}

/// The ids of a contribution's scenes, as declared (`{base}` unreplaced).
pub(crate) fn scene_ids(c: &EngineContribution) -> Vec<String> {
    c.scenes().iter().map(|s| s.id().to_string()).collect()
}

struct Gated<T: ?Sized> {
    inner: Arc<T>,
    enabled: Arc<AtomicBool>,
}

impl<T: ?Sized> Gated<T> {
    fn on(&self) -> bool {
        self.enabled.load(Ordering::Acquire)
    }
}

impl ToolProvider for Gated<dyn ToolProvider> {
    fn name(&self) -> &str {
        self.inner.name()
    }

    fn tools(&self, session: &SessionFacts) -> Vec<Arc<dyn Tool>> {
        if self.on() { self.inner.tools(session) } else { Vec::new() }
    }

    fn hooks(&self, session: &SessionFacts) -> Vec<Arc<dyn ToolHook>> {
        if self.on() { self.inner.hooks(session) } else { Vec::new() }
    }
}

#[async_trait::async_trait]
impl PromptGate for Gated<dyn PromptGate> {
    async fn inspect(&self, message: &PromptFacts) -> PromptDecision {
        if self.on() { self.inner.inspect(message).await } else { PromptDecision::Proceed }
    }
}

#[cfg(test)]
mod tests {
    use harnix_engine_api::{Delivery, HookDecision, SceneBase, SceneLayer, ToolCallFacts};

    use super::*;

    struct One;

    impl ToolProvider for One {
        fn name(&self) -> &str {
            "one"
        }
        fn tools(&self, _: &SessionFacts) -> Vec<Arc<dyn Tool>> {
            vec![]
        }
        fn hooks(&self, _: &SessionFacts) -> Vec<Arc<dyn ToolHook>> {
            vec![Arc::new(Never)]
        }
    }

    struct Never;

    #[async_trait::async_trait]
    impl ToolHook for Never {
        async fn before(&self, _: &ToolCallFacts) -> HookDecision {
            HookDecision::Block("never".into())
        }
    }

    struct Refuse;

    #[async_trait::async_trait]
    impl PromptGate for Refuse {
        async fn inspect(&self, _: &PromptFacts) -> PromptDecision {
            PromptDecision::Block("no".into())
        }
    }

    #[tokio::test]
    async fn a_gated_contribution_does_nothing_while_it_is_off() {
        let mut c = EngineContribution::new();
        c.provider(Arc::new(One)).prompt_gate(Arc::new(Refuse));
        let enabled = Arc::new(AtomicBool::new(true));
        let c = gated(&c, enabled.clone());
        let facts = SessionFacts::new("s", "/p", "harnix", 0);
        let message = PromptFacts::new("s", "t", "hi", Delivery::Turn);
        assert_eq!(c.providers()[0].hooks(&facts).len(), 1);
        assert_eq!(c.prompt_gates()[0].inspect(&message).await, PromptDecision::Block("no".into()));
        enabled.store(false, Ordering::Release);
        assert!(c.providers()[0].hooks(&facts).is_empty());
        assert_eq!(c.prompt_gates()[0].inspect(&message).await, PromptDecision::Proceed);
        assert_eq!(c.providers()[0].name(), "one");
    }

    #[test]
    fn contributions_add_up_in_order_and_admit_a_name_once() {
        let mut a = EngineContribution::new();
        a.scene(SceneLayer::new("a", SceneBase::builtin("harnix"))).admit_everywhere("ext__*");
        let mut b = EngineContribution::new();
        b.scene(SceneLayer::new("b-{base}", SceneBase::named("a"))).admit_everywhere("ext__*").admit_everywhere("x");
        merge(&mut a, &b);
        assert_eq!(scene_ids(&a), ["a", "b-{base}"]);
        assert_eq!(a.admitted(), ["ext__*", "x"]);
    }
}
