//! The hub: the holder of sessions.
//!
//! One invariant, and everything else follows from it:
//!
//! > **The hub subscribes before any client does.**
//!
//! From that come four things, none of which would justify a layer alone and
//! all of which together do (concept §3.2):
//!
//! 1. **Refreshing loses nothing.** Half a turn is in the buffer, and does
//!    not depend on the engine re-sending it.
//! 2. **A turn does not hang off a connection.** `run_turn` is started here,
//!    so closing a tab, switching device or losing the network does not end
//!    it; the final result is broadcast to every watcher.
//! 3. **Several devices agree.** One session can be watched from several
//!    clients; any of them may answer a permission ask, first answer wins.
//! 4. **Queueing is product behavior, not an error.** The engine allows one
//!    turn per session at a time. That constraint should not reach the user
//!    unchanged, so a send during a turn is queued here and sent when the
//!    turn settles.
//!
//! A session is `(instance, session)`, and the hub reaches each instance
//! through an [`EngineLink`] without knowing which carrier is behind it
//! (concept §3.1.1). Its callers are of two kinds with one set of semantics:
//! clients, and the server side — the kernel and modules (concept §3.2).
//!
//! This layer knows nothing about HTTP, TLS, connections or topology. It
//! speaks to `dyn FrameSink`, which is "a downstream that can take frames",
//! and that is all it ever learns about the other side.

mod clients;
mod engine_rows;
mod lifecycle;
mod links;
mod replay;
mod rows;
mod session;
mod spaces;
mod topics;
mod turns;
mod views;

pub use clients::ClientId;
pub use engine_rows::{ENGINE_REFUSALS, HUB_OWNED};
pub use rows::{HubCell, HubRow, HUB_TOPICS};
pub use replay::{MAX_BYTES as REPLAY_MAX_BYTES, MAX_FRAMES as REPLAY_MAX_FRAMES};
pub use spaces::{AuditHook, Spaces};
pub use topics::TOPIC_ENDED;

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex as StdMutex, RwLock as StdRwLock, Weak};

use harnix_contract::sync::{lock, read, write};
use harnix_contract::{codes, FrameSink, RpcError, Subject};
use harnix_instance::{InstanceFacts, InstanceRegistry};
use harnix_kernel::instances::{holding, holds, Holding, Side};
use harnix_kernel::TopicSource;
use serde_json::{json, Value};
use tokio::sync::{oneshot, Mutex};

use clients::Clients;
use links::Linked;
use session::SessionState;

/// How many answered permission prompts the hub keeps for `harnix.prompts.answerer`.
pub(crate) const ANSWERERS_KEPT: usize = 1024;

/// `harnix.spaces`: the same value as `harnix.spaces.list`, pushed when a
/// space is added or removed at runtime. Declared only where something can
/// change the list — with `[apps] enabled` — by [`Hub::declare_spaces_topic`];
/// elsewhere the list is fixed at assembly and there is nothing to watch.
pub const SPACES_TOPIC: &str = "harnix.spaces";

/// The instance every process has: the profile's `[engine]`.
pub use harnix_contract::SYSTEM_INSTANCE as SYSTEM;

/// A session's full name. Two instances may each have a session `s1`; they
/// are different sessions.
pub(crate) type SessionKey = (String, String);

#[derive(Default)]
struct Inner {
    clients: Clients,
    /// Sends what was decided under this lock after it is released, in the order decided
    /// ([`send_after`]).
    dispatch: clients::Dispatch,
    sessions: HashMap<SessionKey, SessionState>,
    topics: topics::Topics,
}

/// The hub's lock, marked held while it is, so that a call into the instance registry can assert
/// it is made holding none (`harnix_kernel::instances`).
#[derive(Default)]
pub(crate) struct InnerLock(Mutex<Inner>);

pub(crate) struct Locked<'a> {
    guard: tokio::sync::MutexGuard<'a, Inner>,
    _held: Holding,
}

impl InnerLock {
    pub(crate) async fn lock(&self) -> Locked<'_> {
        let guard = self.0.lock().await;
        Locked { guard, _held: holding(Side::Hub) }
    }
}

impl std::ops::Deref for Locked<'_> {
    type Target = Inner;
    fn deref(&self) -> &Inner {
        &self.guard
    }
}

impl std::ops::DerefMut for Locked<'_> {
    fn deref_mut(&mut self) -> &mut Inner {
        &mut self.guard
    }
}

pub struct Hub {
    /// The hub itself, for what it starts from a callback that is handed only `&self`.
    this: Weak<Hub>,
    /// Where instances are built, listed and stopped; the hub asks it, holding none of its locks,
    /// what an instance is and to build a space's default.
    instances: Arc<InstanceRegistry>,
    /// The links the registry handed over, by instance: what the hub routes and calls through.
    links: StdRwLock<BTreeMap<String, Arc<Linked>>>,
    /// What computes each computed topic: the hub's own and those other parts of the kernel serve
    /// through it.
    sources: StdRwLock<HashMap<String, Arc<dyn TopicSource>>>,
    /// The topic values recomputed on a clock ([`TopicSource::every`]), once their first
    /// subscriber came.
    ticking: StdMutex<HashSet<views::Marker>>,
    inner: Arc<InnerLock>,
    next_client: AtomicU64,
    barriers: StdMutex<HashMap<u64, oneshot::Sender<()>>>,
    next_barrier: AtomicU64,
    /// Topic values being recomputed, so a burst of changes costs one
    /// computation rather than one each, and an older one never lands last.
    refreshing: StdMutex<views::Refreshes>,
    /// Who answered a permission prompt, by `(session, tool_use_id)`, until a
    /// module reads it (`harnix.prompts.answerer`) or [`ANSWERERS_KEPT`] newer
    /// answers push it out: what the tool's own audit names as the person who
    /// confirmed.
    answerers: StdMutex<VecDeque<((String, String), String)>>,
    /// What each connected client said about itself (device, topology, user agent, since): who
    /// is connected, for the process's own view. A tap or another in-process client has none and
    /// is not listed.
    described: StdMutex<BTreeMap<ClientId, Value>>,
    spaces: StdMutex<Spaces>,
    audit: StdRwLock<Option<AuditHook>>,
}

pub struct HubConfig {
    /// Where the instances are: the hub observes it, and is handed every link it builds.
    pub instances: Arc<InstanceRegistry>,
    pub spaces: Spaces,
}

impl Hub {
    /// The hub, observing `config.instances`: every instance the registry adds from now on is
    /// handed to it.
    pub fn new(config: HubConfig) -> Arc<Self> {
        let hub = Arc::new_cyclic(|this: &Weak<Hub>| {
            let mut inner = Inner::default();
            let mut sources: HashMap<String, Arc<dyn TopicSource>> = HashMap::new();
            for (name, params) in HUB_TOPICS {
                inner
                    .topics
                    .declare(name, params.iter().map(|p| p.to_string()).collect());
                sources.insert(name.to_string(), views::own(this, name));
            }
            Self {
                this: this.clone(),
                instances: config.instances.clone(),
                links: StdRwLock::new(BTreeMap::new()),
                sources: StdRwLock::new(sources),
                ticking: StdMutex::new(HashSet::new()),
                inner: Arc::new(InnerLock(Mutex::new(inner))),
                next_client: AtomicU64::new(1),
                barriers: StdMutex::new(HashMap::new()),
                next_barrier: AtomicU64::new(1),
                refreshing: StdMutex::new(views::Refreshes::default()),
                answerers: StdMutex::new(VecDeque::new()),
                described: StdMutex::new(BTreeMap::new()),
                spaces: StdMutex::new(config.spaces),
                audit: StdRwLock::new(None),
            }
        });
        config.instances.observe(hub.clone());
        hub
    }

    /// The hub, as an `Arc`, from a callback handed `&self`. Always there: whoever calls it
    /// holds one.
    pub(crate) fn arc(&self) -> Arc<Hub> {
        self.this.upgrade().expect("the hub is alive while it is called")
    }

    /// Declare [`SPACES_TOPIC`]. Its value is computed when it is subscribed
    /// to and recomputed when a space is added or removed.
    pub async fn declare_spaces_topic(&self) {
        self.serve_topic(SPACES_TOPIC, Vec::new(), views::own(&self.this, SPACES_TOPIC)).await;
    }

    /// Declare a host topic, at assembly. Subscribing to one that was never
    /// declared is refused.
    pub async fn declare_topic(&self, name: &str, params: Vec<String>) {
        self.inner.lock().await.topics.declare(name, params);
    }

    /// Publish the whole current value of a topic for one parameter set,
    /// and push it to that set's subscribers.
    pub async fn publish(&self, topic: &str, params: Value, value: Value) {
        let guard = self.inner.lock().await;
        self.publish_locked(guard, topic, params, value).await;
    }

    /// [`Hub::publish`], with the hub's lock already taken.
    pub(crate) async fn publish_locked(
        &self,
        mut guard: Locked<'_>,
        topic: &str,
        params: Value,
        value: Value,
    ) {
        let key = match guard.topics.key(topic, &params) {
            Ok(key) => key,
            Err(e) => {
                tracing::warn!(topic, error = %e.message, "not published");
                return;
            }
        };
        let (to, frame) = guard.topics.publish(key, params, value);
        let out = guard.clients.to_many(&to, frame);
        send_after(guard, [out]).await;
    }

    /// What the registry knows of a listed instance: its owner, its engine's facts. Asked
    /// holding none of the hub's locks.
    pub(crate) fn slot(&self, instance: &str) -> Option<InstanceFacts> {
        debug_assert!(!holds(Side::Hub), "the hub calls the registry holding its own lock");
        self.instances.facts(instance)
    }

    /// Every listed instance.
    pub(crate) fn slots(&self) -> Vec<InstanceFacts> {
        debug_assert!(!holds(Side::Hub), "the hub calls the registry holding its own lock");
        self.instances.list()
    }

    /// Build a space's default instance from its template, for its space.
    pub(crate) async fn build_owned(&self, template: &str, name: &str, owner: harnix_instance::Owner) -> Result<Value, RpcError> {
        debug_assert!(!holds(Side::Hub), "the hub calls the registry holding its own lock");
        self.instances.ensure_owned(template, name, owner).await
    }

    /// The link an instance was handed over with.
    pub(crate) fn linked(&self, instance: &str) -> Option<Arc<Linked>> {
        read(&self.links).get(instance).cloned()
    }

    /// Which instance a call means: the one it names, or `system`. One the
    /// hub was not handed is refused by name.
    pub(crate) fn instance_of(&self, params: &Value) -> Result<String, RpcError> {
        let name = match params.get("instance") {
            None | Some(Value::Null) => SYSTEM,
            Some(Value::String(name)) => name.as_str(),
            Some(other) => {
                return Err(RpcError::invalid_params(format!(
                    "`instance` is a name, not {other}"
                )))
            }
        };
        if self.linked(name).is_none() {
            return Err(RpcError::new(
                codes::INSTANCE_NOT_FOUND,
                format!("there is no instance `{name}`"),
            ));
        }
        Ok(name.to_string())
    }

    /// One JSON-RPC call into an instance.
    pub async fn call(
        &self,
        instance: &str,
        method: &str,
        params: Value,
    ) -> Result<Value, RpcError> {
        let Some(linked) = self.linked(instance) else {
            return Err(RpcError::new(
                codes::INSTANCE_NOT_FOUND,
                format!("there is no instance `{instance}`"),
            ));
        };
        linked.link.call(method, params).await
    }

    /// Return once every frame the instance sent before this call has been
    /// routed — not merely received.
    ///
    /// `run_turn`'s reply can overtake the turn's last events, so without
    /// this a turn could be settled while `turn_complete` was still on its
    /// way, and watchers would see the settlement first.
    pub(crate) async fn barrier(&self, instance: &str) {
        let Some(linked) = self.linked(instance) else {
            return;
        };
        let id = self
            .next_barrier
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        lock(&self.barriers).insert(id, tx);
        linked.link.mark(id).await;
        // Bounded, because a link that went down between the reply and the
        // mark will never route it; settling late beats never settling.
        let _ = tokio::time::timeout(std::time::Duration::from_secs(10), rx).await;
        lock(&self.barriers).remove(&id);
    }

    pub(crate) fn barrier_reached(&self, id: u64) {
        if let Some(tx) = lock(&self.barriers).remove(&id) {
            let _ = tx.send(());
        }
    }

    /// Register a client and get the id its frames are addressed by.
    pub async fn add_client(&self, sink: Arc<dyn FrameSink>) -> ClientId {
        let id = ClientId(
            self.next_client
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed),
        );
        self.inner.lock().await.clients.insert(id, sink);
        id
    }

    /// What a client is, as the transport saw it at handshake: shown in the system space.
    pub fn describe_client(&self, id: ClientId, description: Value) {
        lock(&self.described).insert(id, description);
    }

    /// Who is connected, each as the transport described it, with its client id.
    pub fn connections(&self) -> Vec<Value> {
        lock(&self.described)
            .iter()
            .map(|(id, d)| {
                let mut d = d.clone();
                d["client"] = json!(id.0);
                d
            })
            .collect()
    }

    pub async fn remove_client(&self, id: ClientId) {
        lock(&self.described).remove(&id);
        let mut guard = self.inner.lock().await;
        guard.clients.remove(id);
        guard.topics.forget(id);
        for state in guard.sessions.values_mut() {
            state.watchers.remove(&id);
        }
    }

    /// Stop routing every instance. The registry stops the instances themselves.
    pub async fn shutdown(&self) {
        let linked: Vec<Arc<Linked>> = write(&self.links).values().cloned().collect();
        for linked in linked {
            linked.stop();
        }
    }

    /// `harnix.prompts.answerer {session_id, tool_use_id}`: the device that answered that tool's
    /// permission prompt, once (it is forgotten on reading); `null` when no device answered it
    /// (the session's permission mode let it run) or the answer was pushed out. For the
    /// application's audit; a device cannot ask (helm-01).
    pub(crate) fn answerer(&self, subject: &Subject, params: &Value) -> Result<Value, RpcError> {
        if matches!(subject, Subject::Device { .. }) {
            return Err(RpcError::new(codes::REFUSED, "answerers are for the application's audit"));
        }
        let field = |k: &str| {
            params
                .get(k)
                .and_then(Value::as_str)
                .map(str::to_string)
                .ok_or_else(|| RpcError::invalid_params(format!("missing {k}")))
        };
        let key = (field("session_id")?, field("tool_use_id")?);
        let mut kept = lock(&self.answerers);
        let who = kept
            .iter()
            .position(|(k, _)| *k == key)
            .and_then(|i| kept.remove(i))
            .map(|(_, who)| who);
        Ok(json!({ "answerer": who }))
    }

    /// `harnix.entries.append {instance, session_id, event, payload}`: an entry in the session's log,
    /// for people and never for the model (concept §3.2.1). A module writes under its own namespace,
    /// `app.<id>`, so no module can write another's; the kernel names one. A person's words are
    /// `harnix.send`, so a device writes none.
    pub(crate) async fn append_entry(&self, subject: &Subject, params: Value) -> Result<Value, RpcError> {
        let key = self.key_of(&params)?;
        let ns = match subject {
            Subject::Extension { id } => format!("app.{id}"),
            Subject::Kernel => params
                .get("ns")
                .and_then(Value::as_str)
                .map(str::to_string)
                .ok_or_else(|| RpcError::invalid_params("the kernel names an entry's ns"))?,
            _ => {
                return Err(RpcError::new(
                    codes::REFUSED,
                    "entries are the application's; a device speaks through harnix.send",
                ))
            }
        };
        let event = params
            .get("event")
            .and_then(Value::as_str)
            .filter(|e| !e.is_empty())
            .ok_or_else(|| RpcError::invalid_params("missing event"))?;
        let payload = params.get("payload").cloned().unwrap_or(Value::Null);
        self.call(
            &key.0,
            "session.appendEntry",
            json!({"session_id": key.1, "ns": ns, "event": event, "payload": payload}),
        )
        .await
    }

    /// `(instance, session_id)` from a call's params.
    pub(crate) fn key_of(&self, params: &Value) -> Result<SessionKey, RpcError> {
        Ok((self.instance_of(params)?, require_session_id(params)?))
    }
}

/// The space a call says it comes from; the session space when it says none.
pub(crate) fn claimed_space(params: &Value) -> String {
    params
        .get("space")
        .and_then(Value::as_str)
        .unwrap_or(harnix_contract::SESSIONS_SPACE)
        .to_string()
}

/// The transport registers its clients here.
#[async_trait::async_trait]
impl harnix_kernel::ClientRegistry for Hub {
    async fn add_client(&self, sink: Arc<dyn FrameSink>) -> ClientId {
        Hub::add_client(self, sink).await
    }

    fn describe_client(&self, id: ClientId, description: Value) {
        Hub::describe_client(self, id, description)
    }

    async fn remove_client(&self, id: ClientId) {
        Hub::remove_client(self, id).await
    }
}

/// The engine has no notion of instances or spaces; the names are Harnix's.
pub(crate) fn without_instance(mut params: Value) -> Value {
    if let Some(obj) = params.as_object_mut() {
        obj.remove("instance");
        obj.remove("space");
    }
    params
}

pub(crate) fn notification(method: &str, params: Value) -> String {
    serde_json::to_string(&json!({"jsonrpc": "2.0", "method": method, "params": params}))
        .unwrap_or_else(|_| String::from("{}"))
}

pub(crate) fn require_session_id(params: &Value) -> Result<String, RpcError> {
    params
        .get("session_id")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| RpcError::invalid_params("missing session_id"))
}

/// Recording is on by default, and it is the hub that turns it on because it
/// owns every way a runnable session comes into being — `session.create` and
/// the `session.resume` behind an attach.
///
/// Without a recording there is no answer to "what did the model actually
/// see", and that question is not optional (concept §6.4). No `dir` is named: the
/// engine resolves the root from settings and assembly resolves the same way
/// for reading, so there is one answer rather than two.
///
/// # Which mode is not a client's decision
///
/// A client cannot ask for `replay`, or for a different root: the key is
/// overwritten, not defaulted. Replay is a **process-level** decision, made
/// by whoever started this one — the same kind of decision as which engine
/// root to serve, and for the same reason.
///
/// When the operator has made it, and only then, a client may name *which*
/// recording to play. That is not a loosening: outside replay the name means
/// nothing, and inside it, naming the fixture is the entire point.
pub(crate) fn with_recording(mut params: Value, replay_from: Option<&std::path::Path>, strict: bool) -> Value {
    let named = params
        .get("options")
        .and_then(|o| o.get("recorder"))
        .and_then(|r| r.get("name"))
        .and_then(Value::as_str)
        .map(str::to_string);

    let recorder = match replay_from {
        Some(dir) => json!({
            "mode": "replay",
            "name": named,
            "dir": dir.display().to_string(),
            // Divergences are reported, not fatal, unless the operator
            // asked for strict replay (`--replay-strict`, ipdrun-45).
            //
            // Strict replay compares the live request against the recorded
            // one. The prompt carries a date and absolute paths: the engine
            // now dates a replaying session by its recording (Agix ipdrun-06),
            // but paths are the application's to keep stable — fixed roots, or
            // workspace-relative tool paths. Until an application has done
            // that, strictness would only be a test that fails on another
            // machine, so it stays the operator's choice.
            "strict": strict,
        }),
        None => json!({"mode": "record"}),
    };
    let options = params
        .as_object_mut()
        .map(|obj| obj.entry("options").or_insert_with(|| json!({})));
    if let Some(Value::Object(options)) = options {
        options.insert("recorder".into(), recorder);
    }
    params
}

pub(crate) fn new_id(prefix: &str) -> String {
    format!("{prefix}-{}", uuid::Uuid::new_v4().simple())
}

/// Release the hub's lock, then send what was decided under it; return once it is sent.
///
/// Handed to the dispatcher while the lock is still held, so frames leave in the order their
/// seqs were given, whichever task decided them. The lock is not held while they are sent: a
/// downstream that takes its time holds up the frames behind it and nothing else of the hub
/// (and `FrameSink::deliver` must return at once in any case).
async fn send_after(
    guard: Locked<'_>,
    out: impl IntoIterator<Item = clients::Outgoing>,
) {
    let sent = guard.dispatch.submit(out.into_iter().collect());
    drop(guard);
    let _ = sent.await;
}

pub(crate) fn is_code(result: &Result<Value, RpcError>, code: i32) -> bool {
    matches!(result, Err(e) if e.code == code)
}

/// The most memory this process has held, in bytes; `None` where the platform does not say.
///
/// The peak rather than the current figure: it is what `getrusage` gives on every Unix without
/// a dependency, and for "is this process growing" it is the more useful of the two.
fn peak_rss_bytes() -> Option<u64> {
    #[cfg(unix)]
    {
        let mut usage = std::mem::MaybeUninit::<libc::rusage>::zeroed();
        // SAFETY: `getrusage` writes the whole struct for RUSAGE_SELF and touches nothing else.
        let rc = unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) };
        if rc != 0 {
            return None;
        }
        let max = unsafe { usage.assume_init() }.ru_maxrss.max(0) as u64;
        // macOS reports bytes, Linux and the BSDs kilobytes.
        Some(if cfg!(target_os = "macos") {
            max
        } else {
            max * 1024
        })
    }
    #[cfg(not(unix))]
    {
        None
    }
}

/// How many threads the operating system counts in this process — the runtime's workers, the
/// blocking pool, and whatever the engine and its tools started.
fn os_threads() -> Option<u64> {
    #[cfg(target_os = "macos")]
    {
        let mut info = std::mem::MaybeUninit::<libc::proc_taskinfo>::zeroed();
        let size = std::mem::size_of::<libc::proc_taskinfo>() as libc::c_int;
        // SAFETY: `proc_pidinfo` writes at most `size` bytes into `info` and reports how many.
        let wrote = unsafe {
            libc::proc_pidinfo(
                libc::getpid(),
                libc::PROC_PIDTASKINFO,
                0,
                info.as_mut_ptr().cast(),
                size,
            )
        };
        if wrote != size {
            return None;
        }
        Some(unsafe { info.assume_init() }.pti_threadnum.max(0) as u64)
    }
    #[cfg(target_os = "linux")]
    {
        std::fs::read_to_string("/proc/self/status")
            .ok()?
            .lines()
            .find_map(|line| line.strip_prefix("Threads:"))
            .and_then(|n| n.trim().parse().ok())
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        None
    }
}

/// How often the `harnix.process` topic is computed again while watched.
const PROCESS_EVERY: std::time::Duration = std::time::Duration::from_secs(5);

/// Now, in ms since the epoch.
pub(crate) fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

#[cfg(test)]
mod recording_tests {
    use super::*;

    #[test]
    fn the_process_counts_its_threads() {
        if cfg!(any(target_os = "macos", target_os = "linux")) {
            let _worker = std::thread::spawn(|| std::thread::sleep(std::time::Duration::from_millis(200)));
            assert!(os_threads().is_some_and(|n| n >= 2));
        }
    }

    #[test]
    fn replay_is_lenient_unless_the_operator_asked_for_strict() {
        let dir = std::path::Path::new("/rec");
        let named = json!({"options": {"recorder": {"name": "fixture", "mode": "record", "strict": true}}});
        let lenient = with_recording(named.clone(), Some(dir), false);
        assert_eq!(
            lenient["options"]["recorder"],
            json!({"mode": "replay", "name": "fixture", "dir": "/rec", "strict": false}),
            "a client's own mode and strictness are overwritten"
        );
        let strict = with_recording(named.clone(), Some(dir), true);
        assert_eq!(strict["options"]["recorder"]["strict"], true);
        assert_eq!(
            with_recording(named, None, true)["options"]["recorder"],
            json!({"mode": "record"}),
            "outside replay strictness means nothing"
        );
    }
}

#[cfg(test)]
mod sending_tests {
    use super::*;
    use harnix_contract::Frame;
    use std::time::Duration;

    /// Takes every frame, in order; holds the first up until `release` is notified.
    struct Slow {
        seen: StdMutex<Vec<String>>,
        release: tokio::sync::Notify,
    }

    #[async_trait::async_trait]
    impl FrameSink for Slow {
        async fn deliver(&self, frame: Frame) -> bool {
            if lock(&self.seen).is_empty() {
                self.release.notified().await;
            }
            lock(&self.seen).push(frame.payload.to_string());
            true
        }
    }

    /// A downstream that takes its time holds up the frames behind it, not the hub's state: the
    /// lock is free while it waits, and what it is sent arrives in the order it was decided.
    #[tokio::test]
    async fn a_slow_downstream_does_not_hold_the_hub() {
        let inner = Arc::new(InnerLock::default());
        let slow = Arc::new(Slow { seen: StdMutex::new(Vec::new()), release: tokio::sync::Notify::new() });
        inner.lock().await.clients.insert(ClientId(1), slow.clone());

        let mut sending = Vec::new();
        for n in 0..3 {
            let inner = inner.clone();
            sending.push(tokio::spawn(async move {
                let guard = inner.lock().await;
                let out = guard.clients.to_many(&[ClientId(1)], Frame::host(format!("f{n}")));
                send_after(guard, [out]).await;
            }));
            // Each decided before the next: the order the frames must arrive in.
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let free = tokio::time::timeout(Duration::from_secs(1), inner.lock()).await;
        assert!(free.is_ok(), "the hub's lock was held while a downstream waited");
        drop(free);
        assert!(lock(&slow.seen).is_empty());
        slow.release.notify_one();
        for s in sending {
            s.await.unwrap();
        }
        assert_eq!(*lock(&slow.seen), ["f0", "f1", "f2"]);
    }
}
