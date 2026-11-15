//! Agix itself, for what Harnix's engine types do not say. **Not stable**: these are Agix's own
//! types and change with Agix, outside every promise Harnix makes about its interfaces.
//!
//! [`RawScenes::raw`] makes a scene base of an Agix scene; [`RawHooks::raw_hook`] gives an
//! engine a session customizer.

use std::sync::Arc;

pub use ::base;
pub use ::daemon::SessionCtx;
pub use ::runtime::agent::Builder;
pub use ::runtime::plugin;
pub use ::scene;

pub use crate::hooks::RawHook;

use harnix_engine_api::{EngineContribution, SceneBase};

/// An Agix scene as the base of a layer: `SceneBase::raw(scene)`.
pub trait RawScenes {
    fn raw(scene: Arc<dyn base::interface::scene::Scene>) -> SceneBase;
}

impl RawScenes for SceneBase {
    fn raw(scene: Arc<dyn base::interface::scene::Scene>) -> SceneBase {
        let id = scene.id().to_string();
        SceneBase::foreign(&id, Arc::new(scene))
    }
}

/// A raw session customizer for an engine: `contribution.raw_hook(hook)`.
pub trait RawHooks {
    fn raw_hook(&mut self, hook: RawHook) -> &mut Self;
}

impl RawHooks for EngineContribution {
    fn raw_hook(&mut self, hook: RawHook) -> &mut Self {
        self.foreign_hook(Arc::new(hook))
    }
}
