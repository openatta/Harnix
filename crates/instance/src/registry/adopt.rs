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

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use harnix_contract::sync::lock;
use harnix_contract::{codes, ChainLink, RpcError};
use harnix_engine::{Connection, DaemonLink, Refusal, Taken, WakeGate};
use harnix_engine_api::link::{EngineControl, EngineIdentity, EngineLink, InstanceCarrier, LinkStatus};
use harnix_engine_api::ops::{Answer, SessionInfo};
use harnix_kernel::{Chain, InstanceObserver, Rpc, TurnChains, TurnWatch};
use serde_json::{json, Value};
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
    /// Its process runs in the host's sandbox. That is then the one boundary of the commands its
    /// engine runs, and the engine's own command sandbox is turned off: macOS refuses one nested
    /// in it. Not in it, the engine keeps its own.
    pub sandboxed: bool,
    pub wake: Arc<dyn Wake>,
    /// The spaces it declared, whose sessions are on this engine: who may use it besides itself.
    /// None, and it is no space's.
    pub spaces: Vec<String>,
}

/// The engine's own setting for its command sandbox. Written to its global tier: a project's
/// settings may not widen the sandbox.
const DISABLE: (&str, &str) = ("sandbox", "dangerously_disable_sandbox");

/// Turn the engine's own command sandbox off in the settings under `data_root` when its process
/// is `sandboxed`, and back on when it is not. `Ok(true)` when they changed.
pub(super) fn command_sandbox(data_root: &Path, sandboxed: bool) -> std::io::Result<bool> {
    let path = data_root.join("settings.json");
    let mut settings = crate::inherit::read(&path)?;
    let (section, key) = DISABLE;
    let now = settings.get(section).and_then(|s| s.get(key)).and_then(Value::as_bool).unwrap_or(false);
    if now == sandboxed {
        return Ok(false);
    }
    if !settings.is_object() {
        settings = json!({});
    }
    if !settings.get(section).is_some_and(Value::is_object) {
        settings[section] = json!({});
    }
    let table = settings[section].as_object_mut().expect("an object");
    if sandboxed {
        table.insert(key.into(), json!(true));
    } else {
        table.remove(key);
    }
    crate::inherit::write_private(&path, &settings)?;
    Ok(true)
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
        Arc::new_cyclic(|me| Self {
            link: DaemonLink::gated(InstanceCarrier::Adopted, instance.clone(), Arc::new(Gate(me.clone()))),
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

    /// The link the hub is handed: the daemon link itself, whose gate wakes the process for a
    /// call and answers its list while it sleeps.
    pub(super) fn link(&self) -> Arc<DaemonLink> {
        self.link.clone()
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
        // Kept by the gate as it answers.
        let _ = self.link.ops().list().await;
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
            Ok(rows) => rows.iter().any(SessionInfo::loaded),
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

/// What the registry keeps of the instance: its status, and stopping it, which lets it go.
pub(super) struct AdoptedControl(pub(super) Arc<Adoption>);

#[async_trait::async_trait]
impl EngineControl for AdoptedControl {
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

/// What the daemon link asks of the adoption before each call.
struct Gate(Weak<Adoption>);

#[async_trait::async_trait]
impl WakeGate for Gate {
    async fn awake(&self) -> Result<(), RpcError> {
        match self.0.upgrade() {
            Some(adoption) => adoption.awake().await,
            None => Ok(()),
        }
    }

    fn asleep_list(&self) -> Option<Vec<SessionInfo>> {
        let adoption = self.0.upgrade()?;
        adoption.asleep().map(|_| lock(&adoption.listed).clone())
    }

    fn listed(&self, rows: &[SessionInfo]) {
        if let Some(adoption) = self.0.upgrade() {
            *lock(&adoption.listed) = rows.to_vec();
        }
    }
}
