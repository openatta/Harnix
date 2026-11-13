//! Adopting an extension process's own engine: the instance `proc.<id>` (concept §3.1.1).
//!
//! An extension process with an engine of its own serves it on the stream it inherited; the
//! product's end of that stream comes to the carrier with every start of the process, and the
//! carrier hands it here ([`Adoption::attach`]). The instance is one [`DaemonLink`] for as long as
//! the extension is adopted, whichever run of its process is up: listed, and handed to the hub
//! once, as an extension's engine (`InstanceTrust::Extension`) — the hub holds it to what the
//! product's own engines are not.
//!
//! Between runs the link is degraded, and the next run's stream reconnects it: the hub resyncs the
//! sessions anyone watches, which come back from the engine's own store. A process let go while
//! idle ([`Adoption::hibernate`]) is different: the instance is listed hibernating, its session
//! list is answered from what was read before it went, and the first call that needs the engine
//! wakes the process ([`Wake`]) and waits for it. Reading lists wakes nothing: the system space
//! opening does not start an extension's process.
//!
//! What the carrier is told back ([`EngineNote`]): each turn the hub starts and settles on the
//! engine, with the chain it runs on behalf of, and an engine that kept flooding.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use harnix_contract::sync::lock;
use harnix_contract::{codes, ChainLink, RpcError};
use harnix_engine::{Connection, DaemonLink, Refusal, Taken};
use harnix_engine_api::link::{EngineControl, EngineIdentity, EngineLink, EventReceiver, InstanceCarrier, LinkStatus};
use harnix_engine_api::ops::{Answer, EngineOps, Entry, Said, SessionCreated, SessionInfo, Subscribed, Tasks, TurnInput};
use harnix_kernel::{Chain, InstanceObserver, Rpc, TurnChains, TurnWatch};
use serde_json::Value;
use tokio::sync::{mpsc, watch};

use crate::link::now_ms;

/// How long a call that needs a hibernating engine waits for its process to be up again.
pub const WAKE_WITHIN: Duration = Duration::from_secs(60);

/// The instance an extension's own engine is adopted as.
pub fn instance_of(ext: &str) -> String {
    format!("proc.{ext}")
}

/// What wakes an extension process let go while idle: the carrier's, which starts it again and
/// answers once it is ready — its engine attached again included.
#[async_trait::async_trait]
pub trait Wake: Send + Sync {
    async fn wake(&self) -> Rpc<()>;
}

/// How an extension's engine is adopted.
pub struct Adopting {
    pub ext: harnix_contract::ExtId,
    /// The engine's data root, in the extension's run directory (`run/engine/`): written before
    /// its process starts, read by its engine.
    pub data_root: PathBuf,
    /// `main`'s providers are written into its settings before it starts, and again when they
    /// change.
    pub inherit_providers: bool,
    pub wake: Arc<dyn Wake>,
    /// The spaces it declared, whose sessions are on this engine: who may use it besides itself.
    /// None, and it is no space's.
    pub spaces: Vec<String>,
}

/// What the carrier of an adopted engine is told.
#[derive(Debug, Clone, PartialEq)]
pub enum EngineNote {
    /// A turn started in one of its sessions, or the running one is now on behalf of more: the
    /// whole chain.
    TurnStarted { session_id: String, chain: Vec<ChainLink> },
    TurnSettled { session_id: String },
    /// It kept sending more than an extension's engine may: the hub dropped it, and its process
    /// is the carrier's to end.
    Flooded,
}

/// One extension's adopted engine: what the carrier keeps, and what the registry lists.
pub struct Adoption {
    instance: String,
    ext: harnix_contract::ExtId,
    /// Its extension's spaces ([`Adopting::spaces`]).
    spaces: Vec<String>,
    link: Arc<DaemonLink>,
    pub(super) inherits: bool,
    wake: Arc<dyn Wake>,
    /// The hub, which says whether one of its sessions is in use.
    observer: Arc<dyn InstanceObserver<dyn EngineLink>>,
    turns: Arc<TurnChains>,
    /// The run whose stream the link has now.
    taken: Mutex<Option<Taken>>,
    /// Since when its process has been let go while idle.
    asleep: watch::Sender<Option<u64>>,
    /// Its sessions, as last listed: the answer while it sleeps.
    listed: Mutex<Vec<SessionInfo>>,
    /// One wake at a time; the rest wait for it.
    waking: tokio::sync::Mutex<()>,
    notes: mpsc::UnboundedSender<EngineNote>,
    notes_rx: Mutex<Option<mpsc::UnboundedReceiver<EngineNote>>>,
    /// The carrier was told it floods, this run.
    flooded: AtomicBool,
}

impl Adoption {
    pub(super) fn new(
        instance: String,
        spec: &Adopting,
        observer: Arc<dyn InstanceObserver<dyn EngineLink>>,
        turns: Arc<TurnChains>,
    ) -> Arc<Self> {
        let (notes, notes_rx) = mpsc::unbounded_channel();
        Arc::new(Self {
            link: DaemonLink::new(InstanceCarrier::Adopted, instance.clone()),
            instance,
            ext: spec.ext.clone(),
            spaces: spec.spaces.clone(),
            inherits: spec.inherit_providers,
            wake: spec.wake.clone(),
            observer,
            turns,
            taken: Mutex::new(None),
            asleep: watch::channel(None).0,
            listed: Mutex::new(Vec::new()),
            waking: tokio::sync::Mutex::new(()),
            notes,
            notes_rx: Mutex::new(Some(notes_rx)),
            flooded: AtomicBool::new(false),
        })
    }

    pub fn instance(&self) -> &str {
        &self.instance
    }

    /// Whose engine it is, and the spaces its sessions are.
    pub(super) fn whose(&self) -> (&harnix_contract::ExtId, &[String]) {
        (&self.ext, &self.spaces)
    }

    /// What it tells its carrier; the first caller gets it.
    pub fn notes(&self) -> Option<mpsc::UnboundedReceiver<EngineNote>> {
        lock(&self.notes_rx).take()
    }

    /// A start of its process: the product's end of its engine's stream. Up once its engine
    /// answers as one of this build's; refused otherwise, and the stream closed.
    pub async fn attach(&self, stream: std::os::unix::net::UnixStream) -> Rpc<()> {
        let unavailable = |why: String| RpcError::new(codes::INSTANCE_UNAVAILABLE, format!("`{}`: {why}", self.instance));
        stream.set_nonblocking(true).map_err(|e| unavailable(format!("its engine's stream: {e}")))?;
        let stream = tokio::net::UnixStream::from_std(stream).map_err(|e| unavailable(format!("its engine's stream: {e}")))?;
        match self.link.take(Connection::unix(stream), None).await {
            Ok(taken) => {
                *lock(&self.taken) = Some(taken);
                self.flooded.store(false, Ordering::Relaxed);
                self.asleep.send_replace(None);
                Ok(())
            }
            Err(Refusal::Fatal(why) | Refusal::Retry(why)) => Err(unavailable(why)),
        }
    }

    /// Its process is gone, and is started again: the instance is degraded until the next run
    /// attaches; a call meanwhile is told so.
    pub async fn detach(&self, why: &str) {
        self.link.degrade(why.to_string());
        lock(&self.taken).take();
        self.link.disconnect().await;
    }

    /// Its process is let go while idle: its sessions as they list now kept to answer with, and
    /// the instance listed hibernating until a call that needs its engine wakes it.
    pub async fn hibernate(&self) {
        if let Ok(rows) = self.link.ops().list().await {
            *lock(&self.listed) = rows;
        }
        self.asleep.send_replace(Some(now_ms()));
        lock(&self.taken).take();
        self.link.disconnect().await;
    }

    /// Since when it hibernates, while it does.
    pub fn asleep(&self) -> Option<u64> {
        *self.asleep.borrow()
    }

    /// Whether anything is using its engine: a session in its memory, or one the hub has a turn,
    /// a watcher, a task or a question of. Asked of a process that is up; one asleep is not.
    pub async fn engaged(&self) -> bool {
        if self.asleep().is_some() {
            return false;
        }
        if self.observer.engaged(&self.instance).await {
            return true;
        }
        match self.link.ops().list().await {
            Ok(rows) => {
                let loaded = rows.iter().any(SessionInfo::loaded);
                *lock(&self.listed) = rows;
                loaded
            }
            // What it cannot say is not a sign that nobody uses it.
            Err(_) => true,
        }
    }

    /// The chain of the turn running in one of its sessions; none when none runs.
    pub fn turn_chain(&self, session: &str) -> Vec<ChainLink> {
        self.turns.chain(&self.instance, session).links().to_vec()
    }

    /// It sent more than it may windows in a row: told to the carrier once a run.
    pub(super) fn flooding(&self) {
        if !self.flooded.swap(true, Ordering::Relaxed) {
            let _ = self.notes.send(EngineNote::Flooded);
        }
    }

    /// Let go of it for good: the link shut, its turns no longer watched.
    pub(super) async fn release(&self) {
        self.turns.unwatch(&self.instance);
        lock(&self.taken).take();
        self.link.shutdown().await;
    }

    /// Up, waking its process first when it sleeps: within [`WAKE_WITHIN`], or the call is told
    /// the instance is unavailable.
    async fn awake(&self) -> Answer<()> {
        if self.asleep().is_none() {
            return Ok(());
        }
        let _one = self.waking.lock().await;
        if self.asleep().is_none() {
            return Ok(());
        }
        tracing::info!(instance = %self.instance, "waking an extension's engine for a call");
        match tokio::time::timeout(WAKE_WITHIN, self.wake.wake()).await {
            Ok(Ok(())) if self.asleep().is_none() => Ok(()),
            Ok(Ok(())) => Err(LinkStatus::Hibernating { since_ms: 0 }.unavailable()),
            Ok(Err(e)) => Err(RpcError::new(codes::INSTANCE_UNAVAILABLE, format!("`{}` did not wake: {}", self.instance, e.message))),
            Err(_) => Err(RpcError::new(
                codes::INSTANCE_UNAVAILABLE,
                format!("`{}` did not wake within {}s", self.instance, WAKE_WITHIN.as_secs()),
            )),
        }
    }
}

/// The hub's turns on this engine, for its carrier.
impl TurnWatch for Adoption {
    fn started(&self, session: &str, chain: &Chain) {
        let _ = self.notes.send(EngineNote::TurnStarted { session_id: session.to_string(), chain: chain.links().to_vec() });
    }

    fn settled(&self, session: &str) {
        let _ = self.notes.send(EngineNote::TurnSettled { session_id: session.to_string() });
    }
}

/// The link the hub is handed: the daemon link, behind what wakes the process for a call and
/// answers its list while it sleeps.
pub(super) struct AdoptedLink(pub(super) Arc<Adoption>);

#[async_trait::async_trait]
impl EngineControl for AdoptedLink {
    fn status(&self) -> watch::Receiver<LinkStatus> {
        self.0.link.status()
    }

    fn identity(&self) -> EngineIdentity {
        self.0.link.identity()
    }

    fn carrier(&self) -> InstanceCarrier {
        InstanceCarrier::Adopted
    }

    /// One asleep reads its settings as it wakes: not woken for it.
    async fn reload_settings(&self) -> Result<(), RpcError> {
        if self.0.asleep().is_some() {
            return Ok(());
        }
        self.0.link.reload_settings().await
    }

    async fn shutdown(&self) {
        self.0.release().await;
    }
}

#[async_trait::async_trait]
impl EngineLink for AdoptedLink {
    fn ops(&self) -> &dyn EngineOps {
        self
    }

    fn take_events(&self) -> Option<EventReceiver> {
        self.0.link.take_events()
    }

    async fn mark(&self, id: u64) {
        self.0.link.mark(id).await
    }
}

/// Every call wakes a sleeping engine but the list, which is answered from what it listed last.
#[async_trait::async_trait]
impl EngineOps for AdoptedLink {
    async fn create(&self, params: Value) -> Answer<SessionCreated> {
        self.0.awake().await?;
        self.0.link.ops().create(params).await
    }
    async fn get(&self, session: &str) -> Answer<SessionInfo> {
        self.0.awake().await?;
        self.0.link.ops().get(session).await
    }
    async fn list(&self) -> Answer<Vec<SessionInfo>> {
        if self.0.asleep().is_some() {
            return Ok(lock(&self.0.listed).clone());
        }
        let rows = self.0.link.ops().list().await?;
        *lock(&self.0.listed) = rows.clone();
        Ok(rows)
    }
    async fn subscribe(&self, session: &str) -> Answer<Option<Subscribed>> {
        self.0.awake().await?;
        self.0.link.ops().subscribe(session).await
    }
    async fn resume(&self, session: &str, from: &SessionInfo) -> Answer<()> {
        self.0.awake().await?;
        self.0.link.ops().resume(session, from).await
    }
    async fn history_len(&self, session: &str) -> Answer<u64> {
        self.0.awake().await?;
        self.0.link.ops().history_len(session).await
    }
    async fn transcript(&self, session: &str, limit: usize) -> Answer<Vec<Said>> {
        self.0.awake().await?;
        self.0.link.ops().transcript(session, limit).await
    }
    async fn tasks(&self, session: &str) -> Answer<Tasks> {
        self.0.awake().await?;
        self.0.link.ops().tasks(session).await
    }
    async fn run_turn(&self, session: &str, input: TurnInput) -> Answer<Value> {
        self.0.awake().await?;
        self.0.link.ops().run_turn(session, input).await
    }
    async fn interrupt(&self, session: &str) -> Answer<Value> {
        self.0.awake().await?;
        self.0.link.ops().interrupt(session).await
    }
    async fn rename(&self, session: &str, title: &str) -> Answer<Value> {
        self.0.awake().await?;
        self.0.link.ops().rename(session, title).await
    }
    async fn set_model(&self, session: &str, model: &str, provider: Option<&str>) -> Answer<Value> {
        self.0.awake().await?;
        self.0.link.ops().set_model(session, model, provider).await
    }
    async fn close(&self, params: Value, delete: bool) -> Answer<Value> {
        self.0.awake().await?;
        self.0.link.ops().close(params, delete).await
    }
    async fn fork(&self, params: Value) -> Answer<SessionCreated> {
        self.0.awake().await?;
        self.0.link.ops().fork(params).await
    }
    async fn append_entry(&self, session: &str, entry: Entry) -> Answer<Value> {
        self.0.awake().await?;
        self.0.link.ops().append_entry(session, entry).await
    }
    async fn respond(&self, params: Value) -> Answer<Value> {
        self.0.awake().await?;
        self.0.link.ops().respond(params).await
    }
    async fn passthrough(&self, method: &str, params: Value) -> Answer<Value> {
        self.0.awake().await?;
        self.0.link.ops().passthrough(method, params).await
    }
}
