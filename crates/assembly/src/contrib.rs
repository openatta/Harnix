//! Native contributions to an in-process instance, gathered from modules.
//!
//! Scenes, a session customizer (native tools, middleware, event sinks), a
//! credential source for provider keys. Several modules may each give a
//! customizer; they are applied in registration order, each handed the
//! builder the previous one returned, so Agix needs no notion of "several".

use std::sync::Arc;

use base::interface::credentials::CredentialSource;
use base::interface::scene::AgentScene;
use daemon::{SessionCtx, SessionCustomizer};
use runtime::agent::Builder;

#[derive(Default, Clone)]
pub struct EngineContrib {
    pub(crate) scenes: Vec<Arc<dyn AgentScene>>,
    pub(crate) customizers: Vec<SessionCustomizer>,
    pub(crate) credentials: Option<Arc<dyn CredentialSource>>,
}

impl EngineContrib {
    pub fn new() -> Self {
        Self::default()
    }

    /// A scene beyond Agix's built-in ones, available to `session.create`.
    pub fn scene(&mut self, scene: Arc<dyn AgentScene>) -> &mut Self {
        self.scenes.push(scene);
        self
    }

    /// The last word on every session engine this instance builds.
    pub fn customize_session(
        &mut self,
        f: impl Fn(Builder, &SessionCtx<'_>) -> Builder + Send + Sync + 'static,
    ) -> &mut Self {
        self.customizers.push(Arc::new(f));
        self
    }

    /// Provider keys a provider entry leaves unset. The first module to give
    /// one wins; a second is ignored with a warning at assembly.
    pub fn credentials(&mut self, source: Arc<dyn CredentialSource>) -> &mut Self {
        if self.credentials.is_some() {
            tracing::warn!("a second credential source was offered; keeping the first");
        } else {
            self.credentials = Some(source);
        }
        self
    }

    /// Another module's contributions, after these.
    pub fn extend(&mut self, other: EngineContrib) {
        self.scenes.extend(other.scenes);
        self.customizers.extend(other.customizers);
        if let Some(source) = other.credentials {
            self.credentials(source);
        }
    }

    /// The same contributions, with every customizer skipped while
    /// `enabled` is false. A module that fails to start after the instance
    /// was built cannot take its scenes back out of the pool, but its
    /// customizer stops being applied to new sessions (design §7.3).
    pub fn gated(mut self, enabled: Arc<std::sync::atomic::AtomicBool>) -> Self {
        self.customizers = self
            .customizers
            .into_iter()
            .map(|f| {
                let enabled = enabled.clone();
                let gated: SessionCustomizer = Arc::new(move |builder: Builder, cx: &SessionCtx<'_>| {
                    if enabled.load(std::sync::atomic::Ordering::Acquire) {
                        f(builder, cx)
                    } else {
                        builder
                    }
                });
                gated
            })
            .collect();
        self
    }

    pub fn scene_ids(&self) -> Vec<String> {
        self.scenes.iter().map(|s| s.id().to_string()).collect()
    }

    pub(crate) fn composed(&self) -> Option<SessionCustomizer> {
        if self.customizers.is_empty() {
            return None;
        }
        let all = self.customizers.clone();
        Some(Arc::new(move |mut builder: Builder, cx: &SessionCtx<'_>| {
            for f in &all {
                builder = f(builder, cx);
            }
            builder
        }))
    }
}

impl std::fmt::Debug for EngineContrib {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EngineContrib")
            .field("scenes", &self.scene_ids())
            .field("customizers", &self.customizers.len())
            .field("credentials", &self.credentials.is_some())
            .finish()
    }
}
