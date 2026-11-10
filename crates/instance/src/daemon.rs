//! A daemon in another process, kept connected — and running, when this process spawned it
//! (concept §3.1.1).
//!
//! The link is the engine's ([`harnix_engine::DaemonLink`]): it speaks to whatever connection it
//! is given. This keeps giving it one. It spawns `harnix daemon` — this same binary, so the
//! versions agree by construction — and starts it again when it exits; or it attaches to a daemon
//! somebody else runs. A lost connection is not a lost instance: the link goes `Degraded` and this
//! connects again, backing off; a daemon that keeps exiting soon after it was reached is not
//! started again, and one that refuses what it was started with is not either.
//!
//! A spawned daemon is reachable only through a socket in a directory only this user can open;
//! an attached one is the operator's to keep exclusive.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use harnix_contract::RpcError;
use harnix_engine::{Connection, DaemonLink, Endpoint, Recorder, Refusal, ReplayLink};
use harnix_engine_api::link::{EngineControl, EngineIdentity, EngineLink, InstanceCarrier, LinkStatus};
use harnix_supervise::{Child, ChildSpec, RestartPolicy, Restarts};
use tokio::sync::{watch, Mutex};

/// How to start the daemon this link supervises.
#[derive(Debug, Clone)]
pub struct SpawnSpec {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    /// The instance's lock on its data root, when the daemon takes one (`--instance`). A daemon
    /// that exits before listening while somebody else holds it is not started again: the
    /// instance is down, saying who holds it (ipdrun-08).
    pub lock: Option<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct DaemonSpec {
    pub endpoint: Endpoint,
    /// `None` attaches to a daemon somebody else runs.
    pub spawn: Option<SpawnSpec>,
}

/// What a spawned daemon is asked for beyond its command line, and what its link records with.
/// The default is what a link has always done, recording nothing.
#[derive(Default)]
#[non_exhaustive]
pub struct SpawnOptions {
    /// The daemon leads a session of its own, and whatever is still in that session once the
    /// daemon has gone — stopped, killed, crashed, or about to be started again — is ended
    /// (ipdrun-07). The daemon then no longer gets the terminal signals sent to this process's
    /// group; its exit follows stdin closing, as it always did.
    pub reap_on_stop: bool,
    /// Its part of the shared MCP servers (`servers.rs`): reloaded through this link, its
    /// credential renewed when the link starts the daemon again and revoked when it goes for good.
    pub(crate) servers: Option<Arc<crate::servers::Configured>>,
    /// What the sessions it creates and resumes are recorded with.
    pub(crate) recorder: Option<Recorder>,
}

impl SpawnOptions {
    pub fn reap_on_stop(mut self) -> Self {
        self.reap_on_stop = true;
        self
    }
}

/// How long a spawned daemon has to start listening.
const START_TIMEOUT: Duration = Duration::from_secs(60);

/// A daemon's link, and what keeps it connected: the instance's control half. The hub is handed
/// the link itself ([`Supervised::link`]).
pub struct Supervised {
    spec: DaemonSpec,
    link: Arc<DaemonLink>,
    /// The link, recording: what the hub's calls go through.
    replay: Option<Arc<ReplayLink>>,
    child: Mutex<Option<Child>>,
    pid: AtomicU32,
    reap: bool,
    servers: Option<Arc<crate::servers::Configured>>,
}

impl Supervised {
    /// Start connecting (and spawning) in the background. The link is `Starting` until the daemon
    /// answers.
    pub fn start(spec: DaemonSpec) -> Arc<Self> {
        Self::start_with(spec, SpawnOptions::default())
    }

    /// [`Supervised::start`], with what [`SpawnOptions`] asks of a spawned daemon. An attached one
    /// ignores what is about spawning.
    pub fn start_with(spec: DaemonSpec, options: SpawnOptions) -> Arc<Self> {
        let carrier = if spec.spawn.is_some() { InstanceCarrier::Spawn } else { InstanceCarrier::Attach };
        let link = DaemonLink::new(carrier, label(&spec.endpoint));
        Self::launch(spec, options, link)
    }

    fn launch(spec: DaemonSpec, options: SpawnOptions, link: Arc<DaemonLink>) -> Arc<Self> {
        let reap = options.reap_on_stop && spec.spawn.is_some();
        let servers = options.servers.filter(|_| spec.spawn.is_some());
        let replay = options.recorder.map(|recorder| ReplayLink::daemon(link.clone(), recorder));
        let this = Arc::new(Self {
            spec,
            link,
            replay,
            child: Mutex::new(None),
            pid: AtomicU32::new(0),
            reap,
            servers,
        });
        if let Some(servers) = &this.servers {
            servers.attach(&this.link);
        }
        tokio::spawn(this.clone().supervise());
        this
    }

    /// The link the hub is handed: through its recorder, when it has one.
    pub(crate) fn link(&self) -> Arc<dyn EngineLink> {
        match &self.replay {
            Some(replay) => replay.clone(),
            None => self.link.clone(),
        }
    }

    async fn supervise(self: Arc<Self>) {
        let stopping = self.link.stopping();
        // A daemon that keeps exiting soon after the link reached it is not started again: something
        // about it is wrong, and a restart loop would hide what.
        let policy = RestartPolicy::daemon();
        let mut restarts = policy.restarts();
        loop {
            if stopping.is_cancelled() {
                return;
            }
            if self.spec.spawn.is_some() {
                match self.ensure_child().await {
                    Ok(true) => {
                        if restarts.quick_exit() {
                            self.down(format!(
                                "the daemon exited within {}s of starting {} times in a row",
                                policy.quick.as_secs(),
                                policy.give_up_after
                            ));
                            return;
                        }
                    }
                    Ok(false) => {}
                    Err(e) => {
                        if let Some(refused) = e.downcast_ref::<Refused>() {
                            self.down(refused.0.clone());
                            self.stop_child().await;
                            return;
                        }
                        let held = self
                            .spec
                            .spawn
                            .as_ref()
                            .and_then(|s| s.lock.as_deref())
                            .and_then(crate::data_lock::holder);
                        if let Some(held) = held {
                            self.down(format!("{held}; the daemon gave up waiting for it ({e:#})"));
                            self.stop_child().await;
                            return;
                        }
                        self.link.degrade(format!("could not start the daemon: {e:#}"));
                        self.pause(&mut restarts).await;
                        continue;
                    }
                }
            }
            let connection = match Connection::open(&self.spec.endpoint).await {
                Ok(connection) => connection,
                Err(e) => {
                    self.link.degrade(format!("cannot connect: {e:#}"));
                    self.pause(&mut restarts).await;
                    continue;
                }
            };
            let taken = match self.link.take(connection, self.spec.endpoint.token()).await {
                Ok(taken) => taken,
                Err(Refusal::Fatal(reason)) => {
                    self.down(reason);
                    self.stop_child().await;
                    return;
                }
                Err(Refusal::Retry(reason)) => {
                    self.link.degrade(reason);
                    self.pause(&mut restarts).await;
                    continue;
                }
            };
            restarts.reset_backoff();
            let started = Instant::now();
            tokio::select! {
                // A shutdown closes the connection too: it is the shutdown, not a loss.
                biased;
                _ = stopping.cancelled() => return,
                _ = taken.lost() => {}
            }
            self.link.disconnect().await;
            restarts.ran_for(started.elapsed());
            self.link.degrade("the connection closed".into());
            tracing::warn!(daemon = %label(&self.spec.endpoint), "daemon link lost; reconnecting");
        }
    }

    async fn pause(&self, restarts: &mut Restarts) {
        let stopping = self.link.stopping();
        tokio::select! {
            _ = tokio::time::sleep(restarts.next_backoff()) => {}
            _ = stopping.cancelled() => {}
        }
    }

    fn down(&self, reason: String) {
        self.link.down(reason);
        self.gone();
    }

    /// The daemon is not coming back on this link: what it presented to this process's servers
    /// admits nothing any more.
    fn gone(&self) {
        if let Some(servers) = &self.servers {
            servers.revoke();
        }
    }

    /// Make sure the child runs. True when one had exited and this started it again.
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
                    child.reap().await;
                    // What the exited process presented to this process's servers admits nothing
                    // now; the one about to start reads a credential of its own.
                    if let Some(servers) = &self.servers {
                        servers.renew();
                    }
                }
            }
        }
        if let Endpoint::Unix(path) = &self.spec.endpoint {
            let _ = std::fs::remove_file(path);
            let _ = std::fs::remove_file(crate::serve::refusal_path(path));
        }
        let started = Instant::now();
        // The child exits when its stdin closes — which is also what happens when this process
        // is killed outright. Nothing else reliably outlives a SIGKILL to tell it.
        let mut child_spec = ChildSpec::new(&spawn.program).args(&spawn.args).envs(spawn.env.iter().cloned());
        if self.reap {
            child_spec = child_spec.own_session();
        }
        let child = child_spec.spawn()?;
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
                    if status.code() == Some(crate::serve::REFUSED_EXIT) {
                        let why = match &self.spec.endpoint {
                            Endpoint::Unix(path) => {
                                std::fs::read_to_string(crate::serve::refusal_path(path)).unwrap_or_default()
                            }
                            Endpoint::Tcp { .. } => String::new(),
                        };
                        let why = if why.trim().is_empty() { "no reason given".to_string() } else { why.trim().to_string() };
                        return Err(Refused(format!("the daemon refused to start ({status}): {why}")).into());
                    }
                    anyhow::bail!("it exited before listening ({status})");
                }
            }
            if self.spec.endpoint.listening().await {
                return Ok(());
            }
            if started.elapsed() > START_TIMEOUT {
                anyhow::bail!("it did not listen within {}s", START_TIMEOUT.as_secs());
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    async fn stop_child(&self) {
        let Some(child) = self.child.lock().await.take() else {
            return;
        };
        // Closing stdin asks it to finish; the kill is for one that does not.
        child.stop(Duration::from_secs(5)).await;
        if let Endpoint::Unix(path) = &self.spec.endpoint {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// Where the daemon is, for the log: never the token a TCP endpoint carries.
fn label(endpoint: &Endpoint) -> String {
    match endpoint {
        Endpoint::Unix(path) => path.display().to_string(),
        Endpoint::Tcp { addr, .. } => addr.clone(),
    }
}

/// A daemon that exited with [`crate::serve::REFUSED_EXIT`]: starting it again would only refuse again.
#[derive(Debug)]
struct Refused(String);

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Refused {}

#[async_trait::async_trait]
impl EngineControl for Supervised {
    fn status(&self) -> watch::Receiver<LinkStatus> {
        self.link.status()
    }

    fn identity(&self) -> EngineIdentity {
        self.link.identity()
    }

    fn carrier(&self) -> InstanceCarrier {
        self.link.carrier()
    }

    fn pid(&self) -> Option<u32> {
        match self.pid.load(Ordering::Relaxed) {
            0 => None,
            pid => Some(pid),
        }
    }

    fn reaps_on_stop(&self) -> bool {
        self.reap
    }

    async fn reload_settings(&self) -> Result<(), RpcError> {
        self.link.reload_settings().await
    }

    async fn shutdown(&self) {
        self.link.shutdown().await;
        self.gone();
        self.stop_child().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use harnix_contract::sync::lock;
    use harnix_engine_api::EngineEvent;
    use serde_json::{json, Value};
    use std::sync::Mutex as StdMutex;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    /// A daemon that exits with the refusal status before listening is not started again: the
    /// instance is down, with the reason the daemon wrote (ipdrun-10).
    #[tokio::test]
    async fn a_daemon_that_refuses_what_it_was_started_with_takes_the_instance_down() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("d.sock");
        let why = crate::serve::refusal_path(&socket);
        let link = Supervised::start(DaemonSpec {
            endpoint: Endpoint::Unix(socket),
            spawn: Some(SpawnSpec {
                program: "/bin/sh".into(),
                args: vec![
                    "-c".into(),
                    format!(
                        "printf 'this binary declares no template `t`' > '{}'; exit {}",
                        why.display(),
                        crate::serve::REFUSED_EXIT
                    ),
                ],
                env: Vec::new(),
                lock: None,
            }),
        });
        let mut status = link.status();
        let down = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if let LinkStatus::Down { reason } = &*status.borrow_and_update() {
                    return reason.clone();
                }
                status.changed().await.unwrap();
            }
        })
        .await
        .expect("the instance never went down");
        assert!(down.contains("declares no template `t`"), "{down}");
        assert!(down.contains("refused to start"), "{down}");
        link.shutdown().await;
    }

    use tokio::net::{TcpListener, UnixListener};

    /// One connection to a daemon of the test's own: it answers the handshake, then hands every
    /// other request to the test.
    struct Fake<S> {
        lines: tokio::io::Lines<BufReader<tokio::io::ReadHalf<S>>>,
        writer: tokio::io::WriteHalf<S>,
    }

    impl<S: tokio::io::AsyncRead + tokio::io::AsyncWrite> Fake<S> {
        async fn accept(stream: S) -> Self {
            let (r, w) = tokio::io::split(stream);
            let mut fake = Fake { lines: BufReader::new(r).lines(), writer: w };
            loop {
                let (id, method) = fake.request().await.expect("the handshake");
                let result = match method.as_str() {
                    "daemon.auth" | "daemon.subscribeEvents" => json!({}),
                    "daemon.ping" => json!({"version": "test", "protocol_version": harnix_engine::protocol_version()}),
                    other => panic!("`{other}` before the handshake was over"),
                };
                fake.answer(id, result).await;
                if method == "daemon.subscribeEvents" {
                    return fake;
                }
            }
        }

        /// The next request: its id and method. `None` when the link hung up.
        async fn request(&mut self) -> Option<(u64, String)> {
            let line = self.lines.next_line().await.ok()??;
            let v: Value = serde_json::from_str(&line).unwrap();
            Some((v["id"].as_u64().unwrap(), v["method"].as_str().unwrap().to_string()))
        }

        async fn answer(&mut self, id: u64, result: Value) {
            let mut line = json!({"jsonrpc": "2.0", "id": id, "result": result}).to_string();
            line.push('\n');
            self.writer.write_all(line.as_bytes()).await.unwrap();
        }
    }

    fn attach(socket: &std::path::Path) -> Arc<Supervised> {
        Supervised::start(DaemonSpec { endpoint: Endpoint::Unix(socket.to_path_buf()), spawn: None })
    }

    async fn until(link: &Supervised, what: &str, pred: impl Fn(&LinkStatus) -> bool) -> LinkStatus {
        let mut status = link.status();
        tokio::time::timeout(Duration::from_secs(20), async {
            loop {
                let now = status.borrow_and_update().clone();
                if pred(&now) {
                    return now;
                }
                status.changed().await.unwrap();
            }
        })
        .await
        .unwrap_or_else(|_| panic!("never {what}: {:?}", link.status().borrow()))
    }

    /// The daemon goes away and comes back: the link reconnects by itself, and writes
    /// `harnix.reconnected` into the event stream, so the hub knows its subscriptions are gone.
    #[tokio::test]
    async fn a_lost_daemon_is_found_again_and_the_hub_is_told() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("d.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        let link = attach(&socket);
        let mut events = link.link().take_events().unwrap();

        let first = Fake::accept(listener.accept().await.unwrap().0).await;
        until(&link, "ready", LinkStatus::is_ready).await;
        drop(first);
        until(&link, "degraded", |s| matches!(s, LinkStatus::Degraded { .. })).await;

        let _second = Fake::accept(listener.accept().await.unwrap().0).await;
        until(&link, "ready again", LinkStatus::is_ready).await;
        let event = tokio::time::timeout(Duration::from_secs(5), events.recv()).await.unwrap().unwrap();
        assert!(matches!(event, EngineEvent::Reconnected), "{event:?}");
        link.shutdown().await;
    }

    /// A spawned daemon that keeps exiting soon after the link reached it is started again only
    /// so many times; then the instance is down, saying why, instead of looping.
    #[tokio::test]
    async fn a_daemon_that_keeps_exiting_quickly_is_not_started_again_and_again() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let accepted = Arc::new(AtomicU32::new(0));
        let serving = {
            let accepted = accepted.clone();
            tokio::spawn(async move {
                loop {
                    let (stream, _) = listener.accept().await.unwrap();
                    accepted.fetch_add(1, Ordering::SeqCst);
                    tokio::spawn(async move {
                        // Readiness probes connect and go: only a link's connection says anything.
                        let mut peek = [0u8; 1];
                        if stream.peek(&mut peek).await.unwrap_or(0) == 0 {
                            return;
                        }
                        let fake = Fake::accept(stream).await;
                        // The daemon "exits" a moment after the link reached it.
                        tokio::time::sleep(Duration::from_millis(100)).await;
                        drop(fake);
                    });
                }
            })
        };
        let link = Supervised::start(DaemonSpec {
            endpoint: Endpoint::Tcp { addr, token: "t".into() },
            spawn: Some(SpawnSpec {
                program: "/bin/sh".into(),
                args: vec!["-c".into(), "exit 0".into()],
                env: Vec::new(),
                lock: None,
            }),
        });
        let down = until(&link, "down", |s| matches!(s, LinkStatus::Down { .. })).await;
        let LinkStatus::Down { reason } = down else { unreachable!() };
        assert!(reason.contains("5 times in a row"), "{reason}");
        tokio::time::sleep(Duration::from_millis(500)).await;
        let after = accepted.load(Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(500)).await;
        assert_eq!(accepted.load(Ordering::SeqCst), after, "still being started after it was down");
        serving.abort();
        link.shutdown().await;
    }

    /// Each process the link starts presents a credential of its own to this process's servers:
    /// one that exited is revoked as the next is started with a new one, and the last is revoked
    /// once the link gives up.
    #[tokio::test]
    async fn a_daemon_started_again_reads_a_new_credential_and_the_old_one_is_revoked() {
        #[derive(Default)]
        struct Kept(StdMutex<Vec<String>>, AtomicU32);
        impl crate::ServerTokens for Kept {
            fn issue(&self) -> String {
                let token = format!("t{}", self.1.fetch_add(1, Ordering::SeqCst));
                lock(&self.0).push(token.clone());
                token
            }
            fn revoke(&self, token: &str) {
                lock(&self.0).retain(|t| t != token);
            }
        }

        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("root");
        let seen = dir.path().join("seen");
        let kept = Arc::new(Kept::default());
        let shared = Arc::new(crate::SharedServers::default());
        shared.issue_with(kept.clone());
        shared.set([("app-x".to_string(), json!({"type": "remote", "url": "x"}))].into_iter().collect());

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let serving = tokio::spawn(async move {
            loop {
                let (stream, _) = listener.accept().await.unwrap();
                tokio::spawn(async move {
                    let mut peek = [0u8; 1];
                    if stream.peek(&mut peek).await.unwrap_or(0) == 0 {
                        return;
                    }
                    let fake = Fake::accept(stream).await;
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    drop(fake);
                });
            }
        });
        let options = SpawnOptions { servers: Some(shared.configure("a", &root)), ..Default::default() };
        // The daemon notes which credential its settings carry, and exits.
        let script = format!(
            "grep -o 'Bearer t[0-9]*' '{}' >> '{}'; exit 0",
            root.join("settings.json").display(),
            seen.display()
        );
        let link = Supervised::start_with(
            DaemonSpec {
                endpoint: Endpoint::Tcp { addr, token: "t".into() },
                spawn: Some(SpawnSpec {
                    program: "/bin/sh".into(),
                    args: vec!["-c".into(), script],
                    env: Vec::new(),
                    lock: None,
                }),
            },
            options,
        );
        until(&link, "down", |s| matches!(s, LinkStatus::Down { .. })).await;
        let seen = std::fs::read_to_string(&seen).unwrap();
        let read: Vec<&str> = seen.lines().collect();
        assert!(read.len() >= 2, "{read:?}");
        for (n, line) in read.iter().enumerate() {
            assert_eq!(*line, format!("Bearer t{n}"), "each start reads a credential of its own: {read:?}");
        }
        assert!(
            lock(&kept.0).is_empty(),
            "a credential still admits after the link gave up"
        );
        serving.abort();
        link.shutdown().await;
    }

    /// Shutting down while the connection is closing ends `Down`, never `Degraded`: the loss is
    /// the shutdown's own, and nothing reconnects afterwards.
    #[tokio::test]
    async fn a_link_shut_down_while_its_connection_closes_stays_down() {
        for _ in 0..10 {
            let dir = tempfile::tempdir().unwrap();
            let socket = dir.path().join("d.sock");
            let listener = UnixListener::bind(&socket).unwrap();
            let link = attach(&socket);
            let fake = Fake::accept(listener.accept().await.unwrap().0).await;
            until(&link, "ready", LinkStatus::is_ready).await;
            let (_, ()) = tokio::join!(link.shutdown(), async { drop(fake) });
            tokio::time::sleep(Duration::from_millis(50)).await;
            assert_eq!(*link.status().borrow(), LinkStatus::Down { reason: "stopped".into() });
            let reconnect = tokio::time::timeout(Duration::from_millis(300), listener.accept()).await;
            assert!(reconnect.is_err(), "a stopped link connected again");
        }
    }
}
