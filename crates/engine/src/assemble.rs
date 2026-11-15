//! An engine's configuration from an instance's profile, an engine served on a stream it was
//! handed instead of a socket it binds, and an engine in this process reached that way.
//!
//! A stream is what an extension process serves its engine to the product on, the engine
//! descriptor it inherits: the product is the only one at the other end, so there is nothing to
//! listen on, and no path for anyone else to connect to. It is also how an engine in this process
//! is reached ([`InProcessEngine`]): one end of an in-memory stream, behind the same
//! [`DaemonLink`] a daemon in another process is behind, so there is one link and one ordering.

use std::path::Path;
use std::sync::{Arc, Mutex as StdMutex};

use daemon::rpc::{codes, send_frame, FrameSink, RpcRequest, RpcResponse, Sink};
use daemon::DaemonServer;
use harnix_contract::sync::lock;
use harnix_contract::RpcError;
use harnix_engine_api::link::{EngineControl, EngineIdentity, InstanceCarrier, LinkStatus};
use harnix_engine_api::{EngineContribution, EngineProfile};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader, WriteHalf};
use tokio::sync::{watch, Mutex};
use tokio::task::JoinHandle;

use crate::daemon_link::{Connection, DaemonLink};
use crate::engine::{Engine, EngineConfig};

/// The largest request one frame may carry: as the daemon's own transports.
const MAX_FRAME_BYTES: usize = 16 * 1024 * 1024;
/// Requests one connection may have in flight, as the daemon's own transports.
const MAX_IN_FLIGHT: usize = 64;
/// What one direction of an in-process engine's stream holds before its writer waits for the
/// reader: the pipe's, not a frame's, limit — a larger frame goes through in pieces.
const DUPLEX_BYTES: usize = 256 * 1024;

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
struct LineSink<W>(Mutex<W>);

#[async_trait::async_trait]
impl<W: AsyncWrite + Send + Unpin + 'static> FrameSink for LineSink<W> {
    async fn send_json(&self, json: String) -> bool {
        let mut buf = json.into_bytes();
        buf.push(b'\n');
        let mut w = self.0.lock().await;
        w.write_all(&buf).await.is_ok() && w.flush().await.is_ok()
    }
}

/// Serve `server` on `stream` as one daemon connection, until the other end closes it: the
/// requests it sends are dispatched as any connection's are, each on its own task, and their
/// replies go back on the stream through the connection's one queue, in order with what the engine
/// pushes. No token is asked for: whoever holds the other end was handed it. The connection's
/// subscriptions end with it.
pub fn serve_stream<S>(server: Arc<DaemonServer>, stream: S) -> JoinHandle<()>
where
    S: AsyncRead + AsyncWrite + Send + 'static,
{
    let (read, write) = tokio::io::split(stream);
    tokio::spawn(serve_halves(server, read, write))
}

async fn serve_halves<R, S>(server: Arc<DaemonServer>, read: R, write: WriteHalf<S>)
where
    R: AsyncRead + Send + Unpin,
    S: AsyncRead + AsyncWrite + Send + 'static,
{
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

/// An engine built in this process, behind a [`DaemonLink`] on one end of an in-memory stream it
/// serves on the other ([`serve_stream`]). What the registry stops: the link, then the engine.
///
/// The link is the instance's link as it is for a daemon — the same handshake, the same reader
/// routing replies to their callers and everything else to the event stream in the order read,
/// the same mark. Unlike a daemon's, the connection is never found again: the engine is in this
/// process, and only it going away closes the stream, so a lost connection is a link that is down.
pub struct InProcessEngine {
    engine: Engine,
    link: Arc<DaemonLink>,
    serving: StdMutex<Option<JoinHandle<()>>>,
}

impl InProcessEngine {
    /// Serve `engine` on a fresh stream and connect a link to it; `label` names the instance in
    /// the log. The link is `Ready`, and subscribed to the engine's events, once this returns.
    pub async fn start(engine: Engine, label: impl Into<String>) -> anyhow::Result<Arc<Self>> {
        let (ours, theirs) = tokio::io::duplex(DUPLEX_BYTES);
        let serving = serve_stream(engine.server.clone(), theirs);
        let link = DaemonLink::new(InstanceCarrier::InProcess, label);
        let (read, write) = tokio::io::split(ours);
        let taken = match link.take(Connection::new(read, write), None).await {
            Ok(taken) => taken,
            Err(refusal) => {
                serving.abort();
                engine.shutdown().await;
                anyhow::bail!("the engine in this process refused its own link: {refusal:?}");
            }
        };
        // Holding `taken` keeps the reader going; letting it go, once the link is shut down, ends
        // the reader and with it this end of the stream.
        let watched = link.clone();
        tokio::spawn(async move {
            let stopping = watched.stopping();
            tokio::select! {
                _ = taken.lost() => {
                    watched.disconnect().await;
                    watched.down("the engine in this process closed its connection".into());
                }
                _ = stopping.cancelled() => {}
            }
        });
        Ok(Arc::new(Self { engine, link, serving: StdMutex::new(Some(serving)) }))
    }

    /// The link the hub is handed, as a daemon's would be.
    pub fn link(&self) -> &Arc<DaemonLink> {
        &self.link
    }
}

#[async_trait::async_trait]
impl EngineControl for InProcessEngine {
    fn status(&self) -> watch::Receiver<LinkStatus> {
        self.link.status()
    }

    fn identity(&self) -> EngineIdentity {
        self.link.identity()
    }

    fn carrier(&self) -> InstanceCarrier {
        InstanceCarrier::InProcess
    }

    async fn reload_settings(&self) -> Result<(), RpcError> {
        self.link.reload_settings().await
    }

    /// The link first: what is still sending through it — a hub's queued turn — is told the
    /// instance is unavailable rather than starting a session the engine then waits out. Its end
    /// of the stream closing ends the connection, and its subscriptions, before the engine stops.
    async fn shutdown(&self) {
        self.link.shutdown().await;
        let serving = lock(&self.serving).take();
        if let Some(serving) = serving {
            let _ = serving.await;
        }
        self.engine.shutdown().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use harnix_engine_api::{EngineEvent, EngineLink};
    use serde_json::json;

    async fn engine(dir: &Path) -> Engine {
        crate::build(EngineConfig {
            scene: "harnix".into(),
            scenes: Vec::new(),
            model: "test-model".into(),
            max_tokens: 1000,
            session_cap: 4,
            session_idle_timeout_secs: 60,
            permission_prompt_timeout_secs: 60,
            data_root: dir.to_path_buf(),
            cwd: Some(dir.to_path_buf()),
            contrib: Default::default(),
            instance: "main".into(),
            chains: Default::default(),
        })
        .await
        .unwrap()
    }

    /// What the build's constants say is what a running engine says in `daemon.ping`: the
    /// manifest `build.rs` reads and the engine this links are the same Agix.
    #[tokio::test]
    async fn the_engine_says_what_the_build_constants_say() {
        let dir = tempfile::tempdir().unwrap();
        let here = InProcessEngine::start(engine(dir.path()).await, "main").await.unwrap();
        let identity = here.identity();
        assert_eq!(identity.version.as_deref(), Some(crate::AGIX_VERSION));
        assert_eq!(identity.protocol_version, Some(crate::AGIX_PROTOCOL));
        assert_eq!(here.carrier(), InstanceCarrier::InProcess);
        here.shutdown().await;
    }

    /// A call's reply goes to its caller and never into the event stream, and a mark made after
    /// it comes out behind the events the engine wrote before the reply: one ordered stream.
    /// Shut down, the link is down and calls through it are refused.
    #[tokio::test]
    async fn replies_go_to_their_callers_and_marks_come_after_them() {
        let dir = tempfile::tempdir().unwrap();
        let here = InProcessEngine::start(engine(dir.path()).await, "main").await.unwrap();
        let link = here.link().clone();
        let mut events = link.take_events().unwrap();
        let created = link.ops().passthrough("session.create", json!({})).await.unwrap();
        assert!(created.get("session_id").is_some(), "{created}");
        link.mark(7).await;
        let mut seen = Vec::new();
        loop {
            match tokio::time::timeout(std::time::Duration::from_secs(10), events.recv()).await.unwrap().unwrap() {
                EngineEvent::Barrier { id } => {
                    assert_eq!(id, 7);
                    break;
                }
                other => seen.push(other),
            }
        }
        assert!(
            seen.iter().any(|e| matches!(e, EngineEvent::Daemon { .. })),
            "the session's creation was told before the mark: {seen:?}"
        );
        here.shutdown().await;
        assert!(matches!(*here.status().borrow(), LinkStatus::Down { .. }));
        assert!(link.ops().passthrough("daemon.status", json!({})).await.is_err());
    }
}
