//! An Agix daemon in another process, behind [`EngineLink`] (design §5.7).
//!
//! Newline-delimited JSON-RPC over a unix socket, or loopback TCP with the
//! daemon's token. The link either attaches to a daemon somebody else runs or
//! spawns one — `harnix daemon`, this same binary, so the versions agree by
//! construction — and keeps it running.
//!
//! A lost connection is not a lost instance. The link goes `Degraded`, keeps
//! trying, and once it is back writes [`crate::link::RECONNECTED`] into the
//! event stream, so the hub knows every subscription it had is gone and
//! resyncs. Turns on the far side keep running meanwhile (Agix §5.6).
//!
//! The daemon trusts an authenticated connection completely and has no
//! per-method authorization (Agix §5.1), so a daemon serves one Harnix: a
//! spawned one is reachable only through a socket in a directory only this
//! user can open; an attached one is the operator's to keep exclusive.

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};

use harnix_contract::RpcError;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{mpsc, oneshot, watch, Mutex};
use tokio_util::sync::CancellationToken;

use crate::link::{self, Carrier, EngineLink, EventReceiver, LinkStatus};

#[derive(Debug, Clone)]
pub enum Endpoint {
    Unix(PathBuf),
    /// Loopback TCP. The token is the daemon's `AGIX_DAEMON_TOKEN`, held by
    /// Harnix and never given to a client.
    Tcp { addr: String, token: String },
}

/// How to start the daemon this link supervises.
#[derive(Debug, Clone)]
pub struct SpawnSpec {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
}

#[derive(Debug, Clone)]
pub struct DaemonSpec {
    pub endpoint: Endpoint,
    /// `None` attaches to a daemon somebody else runs.
    pub spawn: Option<SpawnSpec>,
}

/// How long a spawned daemon has to start listening.
const START_TIMEOUT: Duration = Duration::from_secs(60);
/// A daemon that keeps exiting less than this long after the link reached
/// it, this many times in a row, is not restarted again: something about it is wrong, and a restart loop
/// would hide what.
const QUICK_EXIT: Duration = Duration::from_secs(5);
const QUICK_EXITS_ALLOWED: u32 = 5;
const MAX_BACKOFF: Duration = Duration::from_secs(5);

type Writer = Box<dyn AsyncWrite + Send + Unpin>;
type Pending = HashMap<u64, oneshot::Sender<Result<Value, RpcError>>>;

pub struct DaemonLink {
    spec: DaemonSpec,
    writer: Mutex<Option<Writer>>,
    pending: StdMutex<Pending>,
    next_id: AtomicU64,
    events_tx: mpsc::Sender<Value>,
    events_rx: StdMutex<Option<EventReceiver>>,
    status: watch::Sender<LinkStatus>,
    identity: StdMutex<Value>,
    child: Mutex<Option<Child>>,
    pid: AtomicU32,
    cancel: CancellationToken,
}

impl DaemonLink {
    /// Start connecting (and spawning) in the background. The link is
    /// `Starting` until the daemon answers.
    pub fn start(spec: DaemonSpec) -> Arc<Self> {
        let (events_tx, events_rx) = mpsc::channel(link::EVENT_CAPACITY);
        let (status, _) = watch::channel(LinkStatus::Starting);
        let this = Arc::new(Self {
            spec,
            writer: Mutex::new(None),
            pending: StdMutex::new(HashMap::new()),
            next_id: AtomicU64::new(1),
            events_tx,
            events_rx: StdMutex::new(Some(events_rx)),
            status,
            identity: StdMutex::new(Value::Null),
            child: Mutex::new(None),
            pid: AtomicU32::new(0),
            cancel: CancellationToken::new(),
        });
        tokio::spawn(this.clone().supervise());
        this
    }

    async fn supervise(self: Arc<Self>) {
        let mut connected_before = false;
        let mut backoff = Duration::from_millis(200);
        let mut quick_exits = 0u32;
        loop {
            if self.cancel.is_cancelled() {
                return;
            }
            if self.spec.spawn.is_some() {
                match self.ensure_child().await {
                    Ok(true) => {
                        quick_exits += 1;
                        if quick_exits >= QUICK_EXITS_ALLOWED {
                            self.down(format!(
                                "the daemon exited within {}s of starting {quick_exits} times in a row",
                                QUICK_EXIT.as_secs()
                            ));
                            return;
                        }
                    }
                    Ok(false) => {}
                    Err(e) => {
                        self.degrade(format!("could not start the daemon: {e:#}"));
                        self.pause(&mut backoff).await;
                        continue;
                    }
                }
            }
            let reader = match self.connect().await {
                Ok(reader) => reader,
                Err(e) => {
                    self.degrade(format!("cannot connect: {e:#}"));
                    self.pause(&mut backoff).await;
                    continue;
                }
            };
            let lost = CancellationToken::new();
            let reading = tokio::spawn(self.clone().read(reader, lost.clone()));
            match self.handshake().await {
                Ok(()) => {}
                Err(Refusal::Fatal(reason)) => {
                    self.down(reason);
                    self.disconnect().await;
                    reading.abort();
                    self.stop_child().await;
                    return;
                }
                Err(Refusal::Retry(reason)) => {
                    self.disconnect().await;
                    reading.abort();
                    self.degrade(reason);
                    self.pause(&mut backoff).await;
                    continue;
                }
            }
            if connected_before {
                let _ = self.events_tx.send(link::reconnected_frame()).await;
            }
            connected_before = true;
            backoff = Duration::from_millis(200);
            let started = Instant::now();
            self.status.send_replace(LinkStatus::Ready);
            tracing::info!(endpoint = ?self.spec.endpoint, "daemon link ready");

            tokio::select! {
                _ = lost.cancelled() => {}
                _ = self.cancel.cancelled() => {
                    reading.abort();
                    return;
                }
            }
            self.disconnect().await;
            if started.elapsed() >= QUICK_EXIT {
                quick_exits = 0;
            }
            self.degrade("the connection closed".into());
            tracing::warn!(endpoint = ?self.spec.endpoint, "daemon link lost; reconnecting");
        }
    }

    async fn pause(&self, backoff: &mut Duration) {
        tokio::select! {
            _ = tokio::time::sleep(*backoff) => {}
            _ = self.cancel.cancelled() => {}
        }
        *backoff = (*backoff * 2).min(MAX_BACKOFF);
    }

    fn degrade(&self, reason: String) {
        let since_ms = match &*self.status.borrow() {
            LinkStatus::Degraded { since_ms, .. } => *since_ms,
            _ => link::now_ms(),
        };
        self.status
            .send_replace(LinkStatus::Degraded { since_ms, reason });
    }

    fn down(&self, reason: String) {
        tracing::error!(endpoint = ?self.spec.endpoint, %reason, "daemon link down");
        self.status.send_replace(LinkStatus::Down { reason });
    }

    /// Make sure the child runs. True when one had exited and this started
    /// it again.
    async fn ensure_child(&self) -> anyhow::Result<bool> {
        let Some(spawn) = &self.spec.spawn else {
            return Ok(false);
        };
        let mut guard = self.child.lock().await;
        let mut restarted = false;
        if let Some(child) = guard.as_mut() {
            match child.try_wait()? {
                None => return Ok(false),
                Some(status) => {
                    tracing::warn!(%status, "the spawned daemon exited; starting it again");
                    restarted = true;
                }
            }
        }
        if let Endpoint::Unix(path) = &self.spec.endpoint {
            let _ = std::fs::remove_file(path);
        }
        let started = Instant::now();
        let mut command = Command::new(&spawn.program);
        command
            .args(&spawn.args)
            .envs(spawn.env.iter().cloned())
            // The child exits when this pipe closes — which is also what
            // happens when this process is killed outright. Nothing else
            // reliably outlives a SIGKILL to tell it.
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .kill_on_drop(true);
        let child = command.spawn()?;
        tracing::info!(pid = child.id(), program = %spawn.program.display(), "spawned a daemon");
        self.pid.store(child.id().unwrap_or(0), Ordering::Relaxed);
        *guard = Some(child);
        drop(guard);
        self.wait_listening(started).await?;
        Ok(restarted)
    }

    async fn wait_listening(&self, started: Instant) -> anyhow::Result<()> {
        loop {
            if let Some(child) = self.child.lock().await.as_mut() {
                if let Some(status) = child.try_wait()? {
                    anyhow::bail!("it exited before listening ({status})");
                }
            }
            let ready = match &self.spec.endpoint {
                Endpoint::Unix(path) => tokio::net::UnixStream::connect(path).await.is_ok(),
                Endpoint::Tcp { addr, .. } => tokio::net::TcpStream::connect(addr).await.is_ok(),
            };
            if ready {
                return Ok(());
            }
            if started.elapsed() > START_TIMEOUT {
                anyhow::bail!("it did not listen within {}s", START_TIMEOUT.as_secs());
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    async fn connect(&self) -> anyhow::Result<Box<dyn tokio::io::AsyncRead + Send + Unpin>> {
        let (reader, writer): (Box<dyn tokio::io::AsyncRead + Send + Unpin>, Writer) =
            match &self.spec.endpoint {
                Endpoint::Unix(path) => {
                    let (r, w) = tokio::net::UnixStream::connect(path).await?.into_split();
                    (Box::new(r), Box::new(w))
                }
                Endpoint::Tcp { addr, .. } => {
                    let (r, w) = tokio::net::TcpStream::connect(addr).await?.into_split();
                    (Box::new(r), Box::new(w))
                }
            };
        *self.writer.lock().await = Some(writer);
        Ok(reader)
    }

    async fn handshake(&self) -> Result<(), Refusal> {
        if let Endpoint::Tcp { token, .. } = &self.spec.endpoint {
            self.raw_call("daemon.auth", json!({"token": token}))
                .await
                .map_err(|e| Refusal::Fatal(format!("the daemon refused the token: {}", e.message)))?;
        }
        let identity = self
            .raw_call("daemon.ping", json!({}))
            .await
            .map_err(|e| Refusal::Retry(format!("daemon.ping failed: {}", e.message)))?;
        let theirs = identity
            .get("protocol_version")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let ours = link::expected_protocol();
        if theirs != ours {
            return Err(Refusal::Fatal(format!(
                "the daemon speaks protocol version {theirs} and this Harnix speaks {ours}; \
                 run a daemon built from the same Agix"
            )));
        }
        *self.identity.lock().unwrap_or_else(|e| e.into_inner()) = json!({
            "version": identity.get("version"),
            "protocol_version": theirs,
            "capabilities": identity.get("capabilities"),
        });
        self.raw_call("daemon.subscribeEvents", json!({}))
            .await
            .map_err(|e| Refusal::Retry(format!("daemon.subscribeEvents failed: {}", e.message)))?;
        Ok(())
    }

    /// Route every line: replies to their callers, everything else to the
    /// event stream, in the order read. A reply is therefore never handed
    /// over before the events the daemon wrote ahead of it.
    async fn read(
        self: Arc<Self>,
        reader: Box<dyn tokio::io::AsyncRead + Send + Unpin>,
        lost: CancellationToken,
    ) {
        let mut lines = BufReader::new(reader).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let Ok(frame) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            let id = frame.get("id").and_then(Value::as_u64);
            match id {
                Some(id) if frame.get("method").is_none() => {
                    let waiter = self
                        .pending
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .remove(&id);
                    if let Some(waiter) = waiter {
                        let _ = waiter.send(reply(frame));
                    }
                }
                _ => {
                    if self.events_tx.send(frame).await.is_err() {
                        break;
                    }
                }
            }
        }
        lost.cancel();
    }

    async fn disconnect(&self) {
        *self.writer.lock().await = None;
        let waiting: Vec<_> = self
            .pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .drain()
            .collect();
        let status = self.status.borrow().clone();
        for (_, waiter) in waiting {
            let _ = waiter.send(Err(link::unavailable(&status)));
        }
    }

    async fn raw_call(&self, method: &str, params: Value) -> Result<Value, RpcError> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(id, tx);
        let mut line = serde_json::to_string(
            &json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}),
        )
        .map_err(|e| RpcError::invalid_params(e.to_string()))?;
        line.push('\n');
        let written = {
            let mut writer = self.writer.lock().await;
            match writer.as_mut() {
                Some(w) => w.write_all(line.as_bytes()).await.is_ok() && w.flush().await.is_ok(),
                None => false,
            }
        };
        if !written {
            self.pending
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(&id);
            return Err(link::unavailable(&self.status.borrow()));
        }
        rx.await
            .unwrap_or_else(|_| Err(link::unavailable(&self.status.borrow())))
    }

    async fn stop_child(&self) {
        let Some(mut child) = self.child.lock().await.take() else {
            return;
        };
        // Closing stdin asks it to finish; the kill is for one that does not.
        drop(child.stdin.take());
        if tokio::time::timeout(Duration::from_secs(5), child.wait())
            .await
            .is_err()
        {
            let _ = child.kill().await;
        }
        if let Endpoint::Unix(path) = &self.spec.endpoint {
            let _ = std::fs::remove_file(path);
        }
    }
}

enum Refusal {
    /// Trying again cannot help: wrong protocol, wrong token.
    Fatal(String),
    Retry(String),
}

fn reply(frame: Value) -> Result<Value, RpcError> {
    if let Some(error) = frame.get("error") {
        return Err(RpcError {
            code: error.get("code").and_then(Value::as_i64).unwrap_or(-32603) as i32,
            message: error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            data: error.get("data").cloned(),
        });
    }
    Ok(frame.get("result").cloned().unwrap_or(Value::Null))
}

#[async_trait::async_trait]
impl EngineLink for DaemonLink {
    async fn call(&self, method: &str, params: Value) -> Result<Value, RpcError> {
        let status = self.status.borrow().clone();
        if !status.is_ready() {
            return Err(link::unavailable(&status));
        }
        self.raw_call(method, params).await
    }

    fn take_events(&self) -> Option<EventReceiver> {
        self.events_rx
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
    }

    /// Replies and events come off one socket in the order the daemon wrote
    /// them, and the reader hands events over before it resolves a later
    /// reply. A mark queued after a reply arrived is therefore behind that
    /// call's events already.
    async fn mark(&self, id: u64) {
        let _ = self.events_tx.send(link::barrier_frame(id)).await;
    }

    fn status(&self) -> watch::Receiver<LinkStatus> {
        self.status.subscribe()
    }

    fn identity(&self) -> Value {
        self.identity
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    fn carrier(&self) -> Carrier {
        if self.spec.spawn.is_some() {
            Carrier::Spawn
        } else {
            Carrier::Attach
        }
    }

    fn pid(&self) -> Option<u32> {
        match self.pid.load(Ordering::Relaxed) {
            0 => None,
            pid => Some(pid),
        }
    }

    async fn shutdown(&self) {
        self.cancel.cancel();
        self.disconnect().await;
        self.stop_child().await;
        self.status.send_replace(LinkStatus::Down {
            reason: "stopped".into(),
        });
    }
}
