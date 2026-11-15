//! An engine in this process, behind [`EngineLink`].
//!
//! Calls go straight to the engine's dispatch — no socket, no id mapping. Frames the engine pushes
//! reach the link through a connection the engine minted for it, which has the same outbound
//! queue every daemon connection has: a link whose reader falls behind loses a session's
//! subscription and is told so, exactly as over a socket.

use std::sync::{Arc, Mutex};

use daemon::rpc::{Client, FrameSink, RpcRequest, RpcResponse, Sink};
use daemon::DaemonServer;
use harnix_contract::sync::lock;
use harnix_contract::RpcError;
use harnix_engine_api::link::{EngineControl, EngineIdentity, EngineLink, EventReceiver, InstanceCarrier, LinkStatus, EVENT_CAPACITY};
use harnix_engine_api::ops::EngineOps;
use harnix_engine_api::EngineEvent;
use serde_json::{json, Value};
use tokio::sync::{mpsc, watch};

use crate::engine::Engine;
use crate::translate::{self, AgixOps, Rpc};

/// The method of the mark [`EngineLink::mark`] writes into the engine's queue for this link.
const BARRIER: &str = "harnix.barrier";

pub struct InProcessLink {
    engine: Engine,
    rpc: Arc<Dispatch>,
    ops: AgixOps,
    events: Mutex<Option<EventReceiver>>,
    status: watch::Sender<LinkStatus>,
    /// What the engine said in `daemon.ping` when the link was made.
    identity: EngineIdentity,
}

/// The engine's dispatch, as this link's connection.
struct Dispatch {
    server: Arc<DaemonServer>,
    client: Arc<Client>,
    status: watch::Receiver<LinkStatus>,
}

impl InProcessLink {
    /// Connect to `engine` and subscribe to its host events, before anyone else can reach it.
    pub async fn start(engine: Engine) -> Arc<Self> {
        let (tx, rx) = mpsc::channel(EVENT_CAPACITY);
        let client = engine.server.accept_connection(Arc::new(ChannelSink(tx)) as Sink);
        let (status, ready) = watch::channel(LinkStatus::Ready);
        let rpc = Arc::new(Dispatch { server: engine.server.clone(), client, status: ready });
        let identity = match rpc.call(translate::DAEMON_PING, json!({})).await {
            Ok(ping) => EngineIdentity::new(
                ping.get("version").and_then(Value::as_str).map(str::to_string),
                ping.get("protocol_version").and_then(Value::as_u64),
                ping.get("capabilities").cloned().unwrap_or(Value::Null),
            ),
            Err(e) => {
                tracing::warn!(error = %e.message, "the engine did not say what it is");
                EngineIdentity::default()
            }
        };
        let this = Arc::new(Self {
            engine,
            ops: AgixOps(rpc.clone()),
            rpc,
            events: Mutex::new(Some(rx)),
            status,
            identity,
        });
        if let Err(e) = this.rpc.call(translate::DAEMON_SUBSCRIBE_EVENTS, json!({})).await {
            tracing::warn!(error = %e.message, "host events not subscribed");
        }
        this
    }

    pub fn engine(&self) -> &Engine {
        &self.engine
    }

}

impl translate::Raw for InProcessLink {
    fn rpc(&self) -> Arc<dyn Rpc> {
        self.rpc.clone()
    }

    fn events(&self) -> Option<EventReceiver> {
        lock(&self.events).take()
    }
}

#[async_trait::async_trait]
impl Rpc for Dispatch {
    async fn call(&self, method: &str, params: Value) -> Result<Value, RpcError> {
        // Stopping or stopped: a call now would start work the engine is being asked to end.
        let status = self.status.borrow().clone();
        if !status.is_ready() {
            return Err(status.unavailable());
        }
        let req: RpcRequest = serde_json::from_value(json!({
            "jsonrpc": "2.0", "method": method, "params": params, "id": 0,
        }))
        .map_err(|e| RpcError::invalid_params(e.to_string()))?;
        tracing::debug!(method, "engine call →");
        let resp = self.server.dispatch_public(req, self.client.clone()).await;
        tracing::debug!(method, ok = resp.error.is_none(), "engine call ←");
        split(resp)
    }
}

#[async_trait::async_trait]
impl EngineControl for InProcessLink {
    fn status(&self) -> watch::Receiver<LinkStatus> {
        self.status.subscribe()
    }

    fn identity(&self) -> EngineIdentity {
        self.identity.clone()
    }

    fn carrier(&self) -> InstanceCarrier {
        InstanceCarrier::InProcess
    }

    async fn reload_settings(&self) -> Result<(), RpcError> {
        self.rpc.call(translate::CONFIG_RELOAD, json!({})).await.map(|_| ())
    }

    async fn shutdown(&self) {
        // Down first: what is still sending through this link — a hub's queued turn — is told the
        // instance is unavailable rather than starting a session the engine then waits out.
        self.status.send_replace(LinkStatus::Down { reason: "stopped".into() });
        self.engine.server.drop_connection(self.rpc.client.id()).await;
        self.engine.shutdown().await;
    }
}

#[async_trait::async_trait]
impl EngineLink for InProcessLink {
    fn ops(&self) -> &dyn EngineOps {
        &self.ops
    }

    fn take_events(&self) -> Option<EventReceiver> {
        translate::Raw::events(self)
    }

    /// Through the engine's own queue for this connection, which is first in, first out: once the
    /// mark has reached the sink, so has everything the engine queued before it.
    async fn mark(&self, id: u64) {
        let frame = json!({"jsonrpc": "2.0", "method": BARRIER, "params": {"id": id}});
        let _ = self.rpc.client.send_json(frame.to_string()).await;
    }
}

/// The link's end of the engine connection. Replies come back from dispatch directly, so anything
/// carrying an id here is not ours.
struct ChannelSink(mpsc::Sender<EngineEvent>);

#[async_trait::async_trait]
impl FrameSink for ChannelSink {
    async fn send_json(&self, json: String) -> bool {
        let Ok(frame) = serde_json::from_str::<Value>(&json) else {
            return true;
        };
        if frame.get("id").is_some() {
            return true;
        }
        if frame.get("method").and_then(Value::as_str) == Some(BARRIER) {
            let id = frame["params"]["id"].as_u64().unwrap_or(0);
            return self.0.send(EngineEvent::Barrier { id }).await.is_ok();
        }
        match translate::event(frame) {
            Some(event) => self.0.send(event).await.is_ok(),
            None => true,
        }
    }
}

fn split(resp: RpcResponse) -> Result<Value, RpcError> {
    if let Some(error) = resp.error {
        return Err(RpcError {
            code: error.code,
            message: error.message,
            data: error.data,
        });
    }
    Ok(resp.result.unwrap_or(Value::Null))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What the build's constants say is what a running engine says in `daemon.ping`: the
    /// manifest `build.rs` reads and the engine this links are the same Agix.
    #[tokio::test]
    async fn the_engine_says_what_the_build_constants_say() {
        let dir = tempfile::tempdir().unwrap();
        let engine = crate::build(crate::EngineConfig {
            scene: "harnix".into(),
            scenes: Vec::new(),
            model: "test-model".into(),
            max_tokens: 1000,
            session_cap: 4,
            session_idle_timeout_secs: 60,
            permission_prompt_timeout_secs: 60,
            data_root: dir.path().to_path_buf(),
            cwd: Some(dir.path().to_path_buf()),
            contrib: Default::default(),
            instance: "main".into(),
            chains: Default::default(),
        })
        .await
        .unwrap();
        let link = InProcessLink::start(engine).await;
        let identity = link.identity();
        assert_eq!(identity.version.as_deref(), Some(crate::AGIX_VERSION));
        assert_eq!(identity.protocol_version, Some(crate::AGIX_PROTOCOL));
        link.shutdown().await;
    }
}
