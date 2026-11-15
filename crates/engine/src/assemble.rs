//! An engine's configuration from an instance's profile, and an engine served on a stream it was
//! handed instead of a socket it binds.
//!
//! The second is what an extension process does with the engine descriptor it inherits: the
//! product is the only one at the other end, so there is nothing to listen on, and no path for
//! anyone else to connect to.

use std::path::Path;
use std::sync::Arc;

use daemon::rpc::{codes, send_frame, FrameSink, RpcRequest, RpcResponse, Sink};
use daemon::DaemonServer;
use harnix_engine_api::{EngineContribution, EngineProfile};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::unix::OwnedWriteHalf;
use tokio::net::UnixStream;
use tokio::sync::Mutex;

use crate::engine::EngineConfig;

/// The largest request one frame may carry: as the daemon's own transports.
const MAX_FRAME_BYTES: usize = 16 * 1024 * 1024;
/// Requests one connection may have in flight, as the daemon's own transports.
const MAX_IN_FLIGHT: usize = 64;

/// The engine `profile` describes, on `data_root`, with what the host contributes.
pub fn engine_config(instance: &str, profile: &EngineProfile, data_root: &Path, contrib: EngineContribution) -> EngineConfig {
    EngineConfig {
        scene: profile.scene.clone(),
        scenes: profile.scenes.clone(),
        model: profile.model.clone(),
        max_tokens: profile.max_tokens,
        session_cap: profile.session_cap,
        session_idle_timeout_secs: profile.session_idle_timeout_secs,
        permission_prompt_timeout_secs: profile.permission_prompt_timeout_secs,
        data_root: data_root.to_path_buf(),
        cwd: None,
        contrib,
        instance: instance.to_string(),
        chains: Default::default(),
    }
}

/// Newline-delimited JSON on the write half of the stream.
struct LineSink(Mutex<OwnedWriteHalf>);

#[async_trait::async_trait]
impl FrameSink for LineSink {
    async fn send_json(&self, json: String) -> bool {
        let mut buf = json.into_bytes();
        buf.push(b'\n');
        let mut w = self.0.lock().await;
        w.write_all(&buf).await.is_ok() && w.flush().await.is_ok()
    }
}

/// Serve `server` on `stream` as one daemon connection, until the stream closes: the requests
/// the other end sends are dispatched as any connection's are, each on its own task, and what the
/// engine pushes goes back on it. The connection's subscriptions end with it.
pub async fn serve_stream(server: Arc<DaemonServer>, stream: UnixStream) {
    let (read, write) = stream.into_split();
    let sink: Sink = Arc::new(LineSink(Mutex::new(write)));
    let client = server.accept_connection(sink);
    let in_flight = Arc::new(tokio::sync::Semaphore::new(MAX_IN_FLIGHT));
    let mut read = BufReader::new(read);
    let mut line = Vec::new();
    loop {
        line.clear();
        // Bounded as the bytes arrive: a peer that never sends a newline cannot make this buffer
        // until it dies.
        let mut frame = tokio::io::AsyncReadExt::take(&mut read, MAX_FRAME_BYTES as u64 + 1);
        match frame.read_until(b'\n', &mut line).await {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        if line.len() > MAX_FRAME_BYTES {
            tracing::warn!(limit = MAX_FRAME_BYTES, "closing a connection that sent a frame over the size limit");
            break;
        }
        let Ok(req) = serde_json::from_slice::<RpcRequest>(&line) else {
            continue;
        };
        let id = req.id.clone().unwrap_or(serde_json::Value::Null);
        let (server, client) = (server.clone(), client.clone());
        let Ok(permit) = in_flight.clone().try_acquire_owned() else {
            let refusal = RpcResponse::err(
                id,
                codes::TOO_MANY_IN_FLIGHT,
                format!("this connection already has {MAX_IN_FLIGHT} requests in flight"),
            );
            tokio::spawn(async move {
                send_frame(&client, &refusal).await;
            });
            continue;
        };
        tokio::spawn(async move {
            let resp = server.dispatch_public(req, client.clone()).await;
            send_frame(&client, &resp).await;
            drop(permit);
        });
    }
    server.drop_connection(client.id()).await;
}
