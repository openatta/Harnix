//! The session customizer an engine is built with: the raw hooks extensions gave (in the engine's
//! own terms), then their prompt gates.

use std::sync::Arc;

use daemon::{SessionCtx, SessionCustomizer};
use harnix_engine_api::EngineContribution;
use runtime::agent::Builder;

use crate::adapt::gate::AgixGate;

/// A hook on every session builder the engine makes: the last word on the session, in Agix's
/// terms. What `harnix::agix` calls a raw hook.
pub type RawHook = SessionCustomizer;

/// The customizer for `contribution`, or none when it has nothing to customize.
pub(crate) fn customizer(contribution: &EngineContribution) -> Option<SessionCustomizer> {
    let mut raw = Vec::new();
    for hook in contribution.foreign_hooks() {
        match hook.hook().downcast_ref::<RawHook>() {
            Some(f) => raw.push((hook.clone(), f.clone())),
            None => tracing::warn!("a hook this engine cannot read was contributed; left out"),
        }
    }
    let gates: Vec<_> = contribution.prompt_gates().to_vec();
    if raw.is_empty() && gates.is_empty() {
        return None;
    }
    Some(Arc::new(move |mut builder: Builder, cx: &SessionCtx<'_>| {
        for (hook, f) in &raw {
            if hook.is_enabled() {
                builder = f(builder, cx);
            }
        }
        for gate in &gates {
            builder = builder.prompt_middleware(Arc::new(AgixGate(gate.clone())));
        }
        builder
    }))
}
