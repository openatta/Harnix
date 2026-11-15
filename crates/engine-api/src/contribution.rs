//! What an extension gives an instance's engine.

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
}

impl std::fmt::Debug for EngineContribution {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EngineContribution")
            .field("scenes", &self.scenes.iter().map(|s| s.id().to_string()).collect::<Vec<_>>())
            .field("providers", &self.providers.iter().map(|p| p.name().to_string()).collect::<Vec<_>>())
            .field("prompt_gates", &self.prompt_gates.len())
            .field("admitted", &self.admitted)
            .finish()
    }
}
