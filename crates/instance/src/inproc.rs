//! An engine in this process, behind [`EngineLink`].
//!
//! Calls go straight to the engine's dispatch — no socket, no id mapping.
//! Frames the engine pushes reach the link through a connection the engine
//! minted for it, which has the same outbound queue every daemon connection
//! has: a link whose reader falls behind loses a session's subscription and
//! is told so, exactly as over a socket.

use std::sync::{Arc, Mutex};

use daemon::rpc::{Client, FrameSink, RpcRequest, RpcResponse, Sink};
use harnix_contract::RpcError;
use serde_json::{json, Value};
use tokio::sync::{mpsc, watch};

use crate::engine::Engine;
use crate::link::{self, Carrier, EngineLink, EventReceiver, LinkStatus};

pub struct InProcessLink {
    engine: Engine,
    client: Arc<Client>,
    events: Mutex<Option<EventReceiver>>,
    status: watch::Sender<LinkStatus>,
}

impl InProcessLink {
    /// Connect to `engine` and subscribe to its host events, before anyone
    /// else can reach it.
    pub async fn start(engine: Engine) -> Arc<Self> {
        let (tx, rx) = mpsc::channel(link::EVENT_CAPACITY);
        let client = engine.server.accept_connection(Arc::new(ChannelSink(tx)) as Sink);
        let (status, _) = watch::channel(LinkStatus::Ready);
        let this = Arc::new(Self {
            engine,
            client,
            events: Mutex::new(Some(rx)),
            status,
        });
        if let Err(e) = this.call("daemon.subscribeEvents", json!({})).await {
            tracing::warn!(error = %e.message, "host events not subscribed");
        }
        this
    }

    pub fn engine(&self) -> &Engine {
        &self.engine
    }
}

#[async_trait::async_trait]
impl EngineLink for InProcessLink {
    async fn call(&self, method: &str, params: Value) -> Result<Value, RpcError> {
        let req: RpcRequest = serde_json::from_value(json!({
            "jsonrpc": "2.0", "method": method, "params": params, "id": 0,
        }))
        .map_err(|e| RpcError::invalid_params(e.to_string()))?;
        tracing::debug!(method, "engine call →");
        let resp = self.engine.server.dispatch_public(req, self.client.clone()).await;
        tracing::debug!(method, ok = resp.error.is_none(), "engine call ←");
        split(resp)
    }

    fn take_events(&self) -> Option<EventReceiver> {
        self.events.lock().unwrap_or_else(|e| e.into_inner()).take()
    }

    /// Through the engine's own queue for this connection, which is first in,
    /// first out: once the mark has reached the sink, so has everything the
    /// engine queued before it.
    async fn mark(&self, id: u64) {
        let frame = serde_json::to_string(&link::barrier_frame(id)).unwrap_or_default();
        let _ = self.client.send_json(frame).await;
    }

    fn status(&self) -> watch::Receiver<LinkStatus> {
        self.status.subscribe()
    }

    fn identity(&self) -> Value {
        daemon::discovery::identity()
    }

    fn carrier(&self) -> Carrier {
        Carrier::InProcess
    }

    async fn shutdown(&self) {
        self.engine.server.drop_connection(self.client.id()).await;
        self.engine.shutdown().await;
        self.status.send_replace(LinkStatus::Down {
            reason: "stopped".into(),
        });
    }
}

/// The link's end of the engine connection. Replies come back from dispatch
/// directly, so anything carrying an id here is not ours.
struct ChannelSink(mpsc::Sender<Value>);

#[async_trait::async_trait]
impl FrameSink for ChannelSink {
    async fn send_json(&self, json: String) -> bool {
        let Ok(frame) = serde_json::from_str::<Value>(&json) else {
            return true;
        };
        if frame.get("id").is_some() {
            return true;
        }
        self.0.send(frame).await.is_ok()
    }
}

pub(crate) fn split(resp: RpcResponse) -> Result<Value, RpcError> {
    if let Some(error) = resp.error {
        return Err(RpcError {
            code: error.code,
            message: error.message,
            data: error.data,
        });
    }
    Ok(resp.result.unwrap_or(Value::Null))
}
