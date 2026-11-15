//! What an extension gives an instance's engine.

use std::any::Any;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::gate::PromptGate;
use crate::provider::ToolProvider;
use crate::scene::SceneLayer;

/// Scenes, tool providers, prompt gates and tool names every scene admits:
/// what one extension, or several gathered in order, give an engine.
#[non_exhaustive]
#[derive(Default, Clone)]
pub struct EngineContribution {
    scenes: Vec<SceneLayer>,
    providers: Vec<Arc<dyn ToolProvider>>,
    prompt_gates: Vec<Arc<dyn PromptGate>>,
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

    /// [`EngineContribution::foreign_hook`], applied only while `enabled`: how the host keeps a
    /// hook of an extension that stopped from reaching sessions built after. Not stable, like it.
    pub fn foreign_hook_while(&mut self, hook: Arc<dyn Any + Send + Sync>, enabled: Arc<AtomicBool>) -> &mut Self {
        self.foreign_hooks.push(ForeignHook { hook, enabled: Some(enabled) });
        self
    }

    pub fn scenes(&self) -> &[SceneLayer] {
        &self.scenes
    }

    pub fn providers(&self) -> &[Arc<dyn ToolProvider>] {
        &self.providers
    }

    pub fn prompt_gates(&self) -> &[Arc<dyn PromptGate>] {
        &self.prompt_gates
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
            .field("scenes", &self.scenes.iter().map(|s| s.id().to_string()).collect::<Vec<_>>())
            .field("providers", &self.providers.iter().map(|p| p.name().to_string()).collect::<Vec<_>>())
            .field("prompt_gates", &self.prompt_gates.len())
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

    /// False while the extension it came with is switched off
    /// ([`EngineContribution::foreign_hook_while`]).
    pub fn is_enabled(&self) -> bool {
        self.enabled.as_ref().is_none_or(|e| e.load(Ordering::Acquire))
    }
}
