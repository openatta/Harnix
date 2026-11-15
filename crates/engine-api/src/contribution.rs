//! What an extension gives an instance's engine.

use std::sync::Arc;

use crate::gate::PromptGate;
use crate::provider::ToolProvider;
use crate::scene::SceneLayer;

/// Scenes, tool providers and prompt gates: what one extension, or several gathered in order,
/// give an engine.
#[non_exhaustive]
#[derive(Default, Clone)]
pub struct EngineContribution {
    scenes: Vec<SceneLayer>,
    providers: Vec<Arc<dyn ToolProvider>>,
    prompt_gates: Vec<Arc<dyn PromptGate>>,
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

    pub fn scenes(&self) -> &[SceneLayer] {
        &self.scenes
    }

    pub fn providers(&self) -> &[Arc<dyn ToolProvider>] {
        &self.providers
    }

    pub fn prompt_gates(&self) -> &[Arc<dyn PromptGate>] {
        &self.prompt_gates
    }
}

impl std::fmt::Debug for EngineContribution {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EngineContribution")
            .field("scenes", &self.scenes.iter().map(|s| s.id().to_string()).collect::<Vec<_>>())
            .field("providers", &self.providers.iter().map(|p| p.name().to_string()).collect::<Vec<_>>())
            .field("prompt_gates", &self.prompt_gates.len())
            .finish()
    }
}
