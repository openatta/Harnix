//! The session customizer an engine is built with: the prompt gates extensions gave.

use std::sync::Arc;

use daemon::{SessionCtx, SessionCustomizer};
use harnix_engine_api::EngineContribution;
use runtime::agent::Builder;

use crate::adapt::gate::AgixGate;

/// The customizer for `contribution`, or none when it has nothing to customize.
pub(crate) fn customizer(contribution: &EngineContribution) -> Option<SessionCustomizer> {
    let gates: Vec<_> = contribution.prompt_gates().to_vec();
    if gates.is_empty() {
        return None;
    }
    Some(Arc::new(move |mut builder: Builder, _cx: &SessionCtx<'_>| {
        for gate in &gates {
            builder = builder.prompt_middleware(Arc::new(AgixGate(gate.clone())));
        }
        builder
    }))
}
