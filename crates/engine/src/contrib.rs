//! What an engine is built with: the extensions' contributions, gathered, and what the host
//! adds to them.

use std::sync::Arc;

use harnix_engine_api::{EngineContribution, ToolProvider};

/// Everything native an in-process engine gets: the contributions of every extension that gives
/// it one, in order, and the host's own providers (a runtime application's tools).
#[derive(Default, Clone, Debug)]
pub struct EngineContrib {
    all: EngineContribution,
}

impl EngineContrib {
    pub fn new() -> Self {
        Self::default()
    }

    /// Another contribution's, after these.
    pub fn extend(&mut self, other: EngineContribution) -> &mut Self {
        self.all.extend(other);
        self
    }

    /// A provider of the host's, for every session of the engine: the name the factory and the
    /// wiring call it by until they speak of providers.
    pub fn plugin(&mut self, provider: Arc<dyn ToolProvider>) -> &mut Self {
        self.all.provider(provider);
        self
    }

    /// [`EngineContribution::admit_everywhere`], under the name the wiring calls it by.
    pub fn admit_tools(&mut self, pattern: impl Into<String>) -> &mut Self {
        self.all.admit_everywhere(&pattern.into());
        self
    }

    pub fn contribution(&self) -> &EngineContribution {
        &self.all
    }
}

impl From<EngineContribution> for EngineContrib {
    fn from(all: EngineContribution) -> Self {
        Self { all }
    }
}

impl std::ops::Deref for EngineContrib {
    type Target = EngineContribution;
    fn deref(&self) -> &EngineContribution {
        &self.all
    }
}

impl std::ops::DerefMut for EngineContrib {
    fn deref_mut(&mut self) -> &mut EngineContribution {
        &mut self.all
    }
}
