//! Computed topics: a whole value the hub fans out, computed by whoever owns the topic.
//!
//! The hub holds the subscriptions and the last value of each `(topic, params)`; the owner says
//! what the value is, when asked, and that it may have changed. Nothing is computed while nobody
//! watches.

use std::time::Duration;

use serde_json::Value;

use crate::Rpc;

/// What a computed topic's value is.
#[async_trait::async_trait]
pub trait TopicSource: Send + Sync {
    /// The current value for one parameter set.
    async fn compute(&self, params: &Value) -> Rpc<Value>;

    /// Someone subscribed to this parameter set; called before its value is computed for them.
    async fn on_subscribe(&self, _params: &Value) {}

    /// For a value that changes without anything to say so: computed again this often while
    /// anyone watches it.
    fn every(&self) -> Option<Duration> {
        None
    }
}

/// Where a computed topic's owner says its value may have changed. Whoever fans the topic out
/// computes it again, if anyone watches it.
pub trait TopicChanged: Send + Sync {
    fn changed(&self, topic: &str, params: Value);
}
