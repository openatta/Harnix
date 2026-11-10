//! A [`PromptGate`] as Agix's prompt middleware.

use std::sync::Arc;

use base::interface::prompt_middleware::{PromptCall, PromptDecision as AgixDecision, PromptDelivery, PromptMiddleware};
use harnix_engine_api::{Delivery, PromptDecision, PromptFacts, PromptGate};

pub(crate) struct AgixGate(pub(crate) Arc<dyn PromptGate>);

#[async_trait::async_trait]
impl PromptMiddleware for AgixGate {
    async fn inspect(&self, call: &PromptCall<'_>) -> AgixDecision {
        let delivery = match call.delivery {
            PromptDelivery::Turn => Delivery::Turn,
            PromptDelivery::Steer => Delivery::Steer,
        };
        let facts = PromptFacts::new(call.session_id, call.turn_id, call.content, delivery);
        match self.0.inspect(&facts).await {
            PromptDecision::Block(message) => AgixDecision::Block {
                message: (!message.is_empty()).then_some(message),
            },
            _ => AgixDecision::Continue,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::adapt::{agix_source, assert_covers, impl_methods, trait_methods};

    #[test]
    fn every_method_of_agixs_prompt_middleware_is_written_or_left_to_its_default_on_purpose() {
        let agix = trait_methods(&agix_source("crates/core/src/interface/prompt_middleware.rs"), "pub trait PromptMiddleware:");
        let ours = impl_methods(include_str!("gate.rs"), "impl PromptMiddleware for AgixGate");
        // A gate sees the text; Agix's default asks `inspect` with the attachments left out.
        assert_covers(agix, ours, &["inspect"], &["inspect_with_attachments"]);
    }
}
