//! What an extension gives an instance's engine.

use std::any::Any;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::credentials::CredentialSource;
use crate::gate::{PromptDecision, PromptFacts, PromptGate};
use crate::provider::{SessionFacts, ToolHook, ToolProvider};
use crate::scene::SceneLayer;
use crate::tool::Tool;

/// Scenes, tool providers, prompt gates, a credential source and tool names every scene admits:
/// what one extension, or several gathered in order, give an engine.
#[non_exhaustive]
#[derive(Default, Clone)]
pub struct EngineContribution {
    scenes: Vec<SceneLayer>,
    providers: Vec<Arc<dyn ToolProvider>>,
    prompt_gates: Vec<Arc<dyn PromptGate>>,
    credentials: Option<Arc<dyn CredentialSource>>,
    admitted: Vec<String>,
    foreign_hooks: Vec<ForeignHook>,
}

impl EngineContribution {
    pub fn new() -> Self {
        Self::default()
    }

    /// A scene beyond the engine's own, for `session.create` to name.
    pub fn scene(&mut self, layer: SceneLayer) -> &mut Self {
        self.scenes.push(layer);
        self
    }

    /// Tools (and hooks) for every session the instance builds, delegates included.
    pub fn provider(&mut self, provider: Arc<dyn ToolProvider>) -> &mut Self {
        self.providers.push(provider);
        self
    }

    /// A gate on every message to every session the instance builds.
    pub fn prompt_gate(&mut self, gate: Arc<dyn PromptGate>) -> &mut Self {
        self.prompt_gates.push(gate);
        self
    }

    /// Provider keys a provider's settings leave unset. The first source given wins; a second
    /// is ignored with a warning.
    pub fn credentials(&mut self, source: Arc<dyn CredentialSource>) -> &mut Self {
        if self.credentials.is_some() {
            tracing::warn!("a second credential source was offered; keeping the first");
        } else {
            self.credentials = Some(source);
        }
        self
    }

    /// Tool names every scene of the instance lets into its sessions besides its own list
    /// (`prefix*` or exact) — the engine's own scenes and other extensions' included: what a
    /// provider gives every session would otherwise be removed again by a scene with a list.
    pub fn admit_everywhere(&mut self, pattern: &str) -> &mut Self {
        if !self.admitted.iter().any(|p| p == pattern) {
            self.admitted.push(pattern.to_string());
        }
        self
    }

    /// A hook in the engine's own terms, which only the engine adapter can read: what
    /// `harnix::agix`'s `raw_hook` gives (not stable). One the adapter does not recognise is
    /// left out with a warning.
    pub fn foreign_hook(&mut self, hook: Arc<dyn Any + Send + Sync>) -> &mut Self {
        self.foreign_hooks.push(ForeignHook { hook, enabled: None });
        self
    }

    /// Another contribution's, after these.
    pub fn extend(&mut self, other: EngineContribution) {
        self.scenes.extend(other.scenes);
        self.providers.extend(other.providers);
        self.prompt_gates.extend(other.prompt_gates);
        self.foreign_hooks.extend(other.foreign_hooks);
        for pattern in &other.admitted {
            self.admit_everywhere(pattern);
        }
        if let Some(source) = other.credentials {
            self.credentials(source);
        }
    }

    /// The same contribution, with its providers, gates and hooks doing nothing while `enabled`
    /// is false: an extension that fails after its instance was built cannot take its scenes
    /// back out of the engine, but it stops reaching sessions built after.
    pub fn gated(mut self, enabled: Arc<AtomicBool>) -> Self {
        self.providers = self
            .providers
            .into_iter()
            .map(|inner| Arc::new(Gated { inner, enabled: enabled.clone() }) as Arc<dyn ToolProvider>)
            .collect();
        self.prompt_gates = self
            .prompt_gates
            .into_iter()
            .map(|inner| Arc::new(Gated { inner, enabled: enabled.clone() }) as Arc<dyn PromptGate>)
            .collect();
        for hook in &mut self.foreign_hooks {
            hook.enabled = Some(enabled.clone());
        }
        self
    }

    pub fn scenes(&self) -> &[SceneLayer] {
        &self.scenes
    }

    /// The ids of its scenes, as declared (`{base}` unreplaced).
    pub fn scene_ids(&self) -> Vec<String> {
        self.scenes.iter().map(|s| s.id().to_string()).collect()
    }

    pub fn providers(&self) -> &[Arc<dyn ToolProvider>] {
        &self.providers
    }

    pub fn prompt_gates(&self) -> &[Arc<dyn PromptGate>] {
        &self.prompt_gates
    }

    pub fn credential_source(&self) -> Option<&Arc<dyn CredentialSource>> {
        self.credentials.as_ref()
    }

    pub fn admitted(&self) -> &[String] {
        &self.admitted
    }

    pub fn foreign_hooks(&self) -> &[ForeignHook] {
        &self.foreign_hooks
    }
}

impl std::fmt::Debug for EngineContribution {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EngineContribution")
            .field("scenes", &self.scene_ids())
            .field("providers", &self.providers.iter().map(|p| p.name().to_string()).collect::<Vec<_>>())
            .field("prompt_gates", &self.prompt_gates.len())
            .field("credentials", &self.credentials.is_some())
            .field("admitted", &self.admitted)
            .field("foreign_hooks", &self.foreign_hooks.len())
            .finish()
    }
}

/// A hook [`EngineContribution::foreign_hook`] was given, and whether it applies now.
#[non_exhaustive]
#[derive(Clone)]
pub struct ForeignHook {
    hook: Arc<dyn Any + Send + Sync>,
    enabled: Option<Arc<AtomicBool>>,
}

impl ForeignHook {
    pub fn hook(&self) -> &Arc<dyn Any + Send + Sync> {
        &self.hook
    }

    /// False while the contribution it came with is switched off ([`EngineContribution::gated`]).
    pub fn is_enabled(&self) -> bool {
        self.enabled.as_ref().is_none_or(|e| e.load(Ordering::Acquire))
    }
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
    use super::*;
    use crate::scene::{SceneBase, SceneLayer};

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
        async fn before(&self, _: &crate::provider::ToolCallFacts) -> crate::provider::HookDecision {
            crate::provider::HookDecision::Block("never".into())
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
        c.provider(Arc::new(One)).prompt_gate(Arc::new(Refuse)).foreign_hook(Arc::new(7u8));
        let enabled = Arc::new(AtomicBool::new(true));
        let c = c.gated(enabled.clone());
        let facts = SessionFacts::new("s", "/p", "harnix", 0);
        let message = PromptFacts::new("s", "t", "hi", crate::gate::Delivery::Turn);
        assert_eq!(c.providers()[0].hooks(&facts).len(), 1);
        assert_eq!(c.prompt_gates()[0].inspect(&message).await, PromptDecision::Block("no".into()));
        assert!(c.foreign_hooks()[0].is_enabled());
        enabled.store(false, Ordering::Release);
        assert!(c.providers()[0].hooks(&facts).is_empty());
        assert_eq!(c.prompt_gates()[0].inspect(&message).await, PromptDecision::Proceed);
        assert!(!c.foreign_hooks()[0].is_enabled());
        assert_eq!(c.providers()[0].name(), "one");
    }

    #[test]
    fn contributions_add_up_in_order_and_admit_a_name_once() {
        let mut a = EngineContribution::new();
        a.scene(SceneLayer::new("a", SceneBase::builtin("harnix"))).admit_everywhere("app__*");
        let mut b = EngineContribution::new();
        b.scene(SceneLayer::new("b-{base}", SceneBase::named("a"))).admit_everywhere("app__*").admit_everywhere("x");
        a.extend(b);
        assert_eq!(a.scene_ids(), ["a", "b-{base}"]);
        assert_eq!(a.admitted(), ["app__*", "x"]);
    }
}
