//! Clients: what the transport registers with the hub, and the handle frames are addressed by.

use std::sync::Arc;

use harnix_contract::FrameSink;

/// One client of the hub: a connection's downstream, or an in-process watcher.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ClientId(pub u64);

/// What the transport needs of the hub: to register a client, describe it, and forget it. A
/// trait so the transport names no hub, and its topologies can be exercised against something
/// smaller than one.
#[async_trait::async_trait]
pub trait ClientRegistry: Send + Sync {
    async fn add_client(&self, sink: Arc<dyn FrameSink>) -> ClientId;
    fn describe_client(&self, id: ClientId, description: serde_json::Value);
    async fn remove_client(&self, id: ClientId);
}
