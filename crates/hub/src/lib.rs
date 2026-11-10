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
//! through an `EngineLink` without knowing which carrier is behind it
//! (concept §3.1.1), in the engine interface's words: it asks for a session
//! to be created, read, run, and reads the engine's events by what they are,
//! never by what the engine calls them. Its callers are of two kinds with one
//! set of semantics: clients, and the server side — the kernel and modules
//! (concept §3.2).
//!
//! An instance's engine is trusted as far as its instance is: one in an
//! extension's own process is held to bounds the product's own are not
//! (`links.rs`).
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
pub use links::{EXTENSION_BUFFER_FRAMES, FRAMES_PER_SECOND, FRAME_MAX_BYTES};
pub use rows::{HubCell, HubRow, HUB_TOPICS};
pub use replay::{MAX_BYTES as REPLAY_MAX_BYTES, MAX_FRAMES as REPLAY_MAX_FRAMES};
pub use spaces::{AuditHook, Spaces};
pub use topics::TOPIC_ENDED;

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex as StdMutex, RwLock as StdRwLock, Weak};

use harnix_contract::sync::{lock, read, write};
use harnix_contract::{codes, FrameSink, RpcError, Subject};
use harnix_engine_api::ops::Entry;
use harnix_kernel::instances::{holding, holds, Holding, Side};
use harnix_kernel::{InstanceBook, Listed, TopicSource};
use serde_json::{json, Value};
use tokio::sync::{oneshot, Mutex};

use clients::Clients;
use links::Linked;
use session::SessionState;

/// How many answered permission prompts the hub keeps for `harnix.session.answerer`.
pub(crate) const ANSWERERS_KEPT: usize = 1024;

/// `harnix.spaces`: the same value as `harnix.spaces.list`, pushed when a
/// space is added or removed at runtime. Declared only where something can
/// change the list — with `[apps] enabled` — by [`Hub::declare_spaces_topic`];
/// elsewhere the list is fixed at assembly and there is nothing to watch.
pub const SPACES_TOPIC: &str = "harnix.spaces";

/// The instance every process has: the profile's `[engine]`.
pub use harnix_contract::MAIN_INSTANCE as MAIN;

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
    instances: Arc<dyn InstanceBook>,
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
    /// module reads it (`harnix.session.answerer`) or [`ANSWERERS_KEPT`] newer
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
    /// Where the instances are. The hub is to observe it — every link it builds is handed over —
    /// from before it adds any.
    pub instances: Arc<dyn InstanceBook>,
    pub spaces: Spaces,
}

impl Hub {
    /// The hub, asking `config.instances` about instances. Whoever made both has the registry
    /// observe the hub before it adds any instance.
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
        let told = guard.topics.publish(key, params, value, &|t, v, s| self.within(t, v, s));
        let out: Vec<_> = told.into_iter().map(|(to, frame)| guard.clients.to_many(&to, frame)).collect();
        send_after(guard, out).await;
    }

    /// What the registry knows of a listed instance: whose it is. Asked holding none of the hub's
    /// locks.
    pub(crate) fn slot(&self, instance: &str) -> Option<Listed> {
        debug_assert!(!holds(Side::Hub), "the hub calls the registry holding its own lock");
        self.instances.listed(instance)
    }

    /// Every listed instance.
    pub(crate) fn slots(&self) -> Vec<Listed> {
        debug_assert!(!holds(Side::Hub), "the hub calls the registry holding its own lock");
        self.instances.all()
    }

    /// Build a space's default instance from its template, for its space.
    pub(crate) async fn build_owned(&self, template: &str, name: &str, space: &str) -> Result<(), RpcError> {
        debug_assert!(!holds(Side::Hub), "the hub calls the registry holding its own lock");
        self.instances.ensure_for_space(template, name, space).await
    }

    /// The link an instance was handed over with.
    pub(crate) fn linked(&self, instance: &str) -> Option<Arc<Linked>> {
        read(&self.links).get(instance).cloned()
    }

    /// Which instance a call means: the one it names, or `main`. One the
    /// hub was not handed is refused by name.
    pub(crate) fn instance_of(&self, params: &Value) -> Result<String, RpcError> {
        let name = match params.get("instance") {
            None | Some(Value::Null) => MAIN,
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

    /// The link to an instance, to ask its engine something; refused by name for one the hub was
    /// not handed.
    pub(crate) fn engine(&self, instance: &str) -> Result<Arc<Linked>, RpcError> {
        self.linked(instance).ok_or_else(|| {
            RpcError::new(codes::INSTANCE_NOT_FOUND, format!("there is no instance `{instance}`"))
        })
    }

    /// One engine method of an instance, its answer as the engine gives it: for the assembly's
    /// own reads of the engine (`hello`), which name a row of the hub's.
    pub async fn passthrough(&self, instance: &str, method: &str, params: Value) -> Result<Value, RpcError> {
        self.engine(instance)?.ops().passthrough(method, params).await
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

    /// `harnix.session.answerer {session_id, tool_use_id}`: the device that answered that tool's
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

    /// `harnix.session.entries.append {instance, session_id, event, payload}`: an entry in the session's log,
    /// for people and never for the model (concept §3.2.1). A module writes under its own namespace,
    /// `app.<id>`, so no module can write another's; the kernel names one. A person's words are
    /// `harnix.session.send`, so a device writes none.
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
                    "entries are the application's; a device speaks through harnix.session.send",
                ))
            }
        };
        let event = params
            .get("event")
            .and_then(Value::as_str)
            .filter(|e| !e.is_empty())
            .ok_or_else(|| RpcError::invalid_params("missing event"))?;
        let payload = params.get("payload").cloned().unwrap_or(Value::Null);
        self.engine(&key.0)?.ops().append_entry(&key.1, Entry::new(ns, event, payload)).await
    }

    /// `(instance, session_id)` from a call's params.
    pub(crate) fn key_of(&self, params: &Value) -> Result<SessionKey, RpcError> {
        Ok((self.instance_of(params)?, require_session_id(params)?))
    }
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

/// Now, in ms since the epoch.
pub(crate) fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
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
