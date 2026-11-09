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
mod instances;
mod lifecycle;
mod replay;
mod session;
mod spaces;
mod topics;
mod turns;
mod views;

pub use clients::ClientId;
pub use instances::{Registry, Slot};
pub use replay::{MAX_BYTES as REPLAY_MAX_BYTES, MAX_FRAMES as REPLAY_MAX_FRAMES};
pub use spaces::{AuditHook, Spaces};
pub use topics::TOPIC_ENDED;

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex as StdMutex, RwLock as StdRwLock};

use harnix_contract::sync::{lock, read, write};
use harnix_contract::{codes, FrameSink, Gate, RpcError, Subject};
use serde_json::{json, Value};
use tokio::sync::{oneshot, Mutex, RwLock};

use clients::Clients;
use session::SessionState;

/// Engine methods a client may send through untouched.
///
/// An allow-list, not a deny-list: the engine trusts whoever reaches its
/// dispatch, and in this build that is the hub. What is not named here is not
/// reachable — see `harnix_authz`, which is where the decision is actually made.
pub const PASSTHROUGH: &[&str] = &[
    "session.create",
    "session.list",
    "session.get",
    "session.history",
    "session.entries",
    "session.fork",
    "session.resume",
    "session.respondToPrompt",
    "scene.list",
    "scene.describe",
    "scene.activate",
    "commands.list",
    "mcp.status",
    "plugin.list",
    "daemon.status",
    "daemon.ping",
    "daemon.doctor",
];

/// How many answered permission prompts the hub keeps for `harnix.prompts.answerer`.
const ANSWERERS_KEPT: usize = 1024;

/// The hub's own methods. Named here so `harnix_authz` can open them and the
/// generated method catalog can list them without tracing dispatch.
pub const HUB_METHODS: &[&str] = &[
    "harnix.hello",
    "harnix.sessions",
    "harnix.attach",
    "harnix.detach",
    "harnix.send",
    "harnix.queue.remove",
    "harnix.queue.edit",
    "harnix.subscribe",
    "harnix.unsubscribe",
    "harnix.instances.list",
    "harnix.instances.templates",
    "harnix.instances.sessions",
    "harnix.instances.create",
    "harnix.instances.ensure",
    "harnix.instances.stop",
    "harnix.instances.restart",
    "harnix.instances.wait",
    "harnix.sessions.tasks",
    "harnix.entries.append",
    "harnix.sessions.move",
    "harnix.prompts.answerer",
    "harnix.spaces.list",
    "harnix.process",
];

/// The hub's own topics: the instance tree as whole values (concept §3.2).
/// Each with the parameters it is subscribed by.
pub const HUB_TOPICS: &[(&str, &[&str])] = &[
    ("harnix.instances", &[]),
    ("harnix.instances.sessions", &["instance"]),
    ("harnix.sessions.tasks", &["instance", "session_id"]),
    ("harnix.sessions.status", &[]),
    ("harnix.process", &[]),
];

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
    /// In replay, which recording each session was created to play. The
    /// engine keeps none across a restart of its own (Agix: `recorder` is not
    /// persisted), so a session resumed in a daemon that came back would
    /// otherwise call the model for real.
    recordings: HashMap<SessionKey, String>,
}

pub struct Hub {
    /// Where recordings are played back from, when this process was started
    /// in replay. `None` is the ordinary case: record, never play.
    replay_from: Option<std::path::PathBuf>,
    /// Replay fails a call whose request differs from the recorded one (ipdrun-45).
    replay_strict: std::sync::atomic::AtomicBool,
    /// Whether the `harnix.process` topic's clock has been started (at its first subscriber).
    process_ticking: std::sync::atomic::AtomicBool,
    instances: StdRwLock<BTreeMap<String, Arc<Slot>>>,
    registry: Mutex<Registry>,
    inner: Arc<Mutex<Inner>>,
    next_client: AtomicU64,
    barriers: StdMutex<HashMap<u64, oneshot::Sender<()>>>,
    next_barrier: AtomicU64,
    /// Topic values being recomputed, so a burst of changes costs one
    /// computation rather than one each, and an older one never lands last.
    refreshing: StdMutex<views::Refreshes>,
    /// Interface modules from installed packages, told to a client at hello.
    /// Held here rather than resolved per call because it changes when a
    /// package is installed, not when someone asks.
    ui_contributions: RwLock<Value>,
    /// What the shell is told about this product at hello: its brand, which built-in spaces it
    /// keeps, whether runtime interface modules load. Set once at assembly.
    shell: StdRwLock<Value>,
    /// Who answered a permission prompt, by `(session, tool_use_id)`, until a
    /// module reads it (`harnix.prompts.answerer`) or [`ANSWERERS_KEPT`] newer
    /// answers push it out: what the tool's own audit names as the person who
    /// confirmed.
    answerers: StdMutex<VecDeque<((String, String), String)>>,
    /// The process's own facts for `harnix.process`: versions, when it started.
    process: StdRwLock<Value>,
    started: std::time::Instant,
    /// What each connected client said about itself (device, topology, user agent, since), for
    /// `harnix.process`. A tap or another in-process client has none and is not listed.
    described: StdMutex<BTreeMap<ClientId, Value>>,
    /// The ceilings a client has to respect, assembled once by the layer that
    /// knows all three. Reported rather than restated: `hello` used to build
    /// this object by hand and left `max_upload_bytes` out of it entirely.
    limits: Value,
    /// How long a permission ask waits for an answer before it is denied
    /// (R-12): a client shows the countdown.
    prompt_timeout_secs: u64,
    spaces: StdMutex<Spaces>,
    audit: StdRwLock<Option<AuditHook>>,
}

pub struct HubConfig {
    pub replay_from: Option<std::path::PathBuf>,
    pub limits: harnix_contract::Limits,
    pub prompt_timeout_secs: u64,
    pub registry: Registry,
    pub spaces: Spaces,
}

impl Hub {
    pub fn new(config: HubConfig) -> Arc<Self> {
        let mut inner = Inner::default();
        for (name, params) in HUB_TOPICS {
            inner
                .topics
                .declare(name, params.iter().map(|p| p.to_string()).collect());
        }
        Arc::new(Self {
            replay_from: config.replay_from,
            replay_strict: std::sync::atomic::AtomicBool::new(false),
            process_ticking: std::sync::atomic::AtomicBool::new(false),
            instances: StdRwLock::new(BTreeMap::new()),
            registry: Mutex::new(config.registry),
            inner: Arc::new(Mutex::new(inner)),
            next_client: AtomicU64::new(1),
            barriers: StdMutex::new(HashMap::new()),
            next_barrier: AtomicU64::new(1),
            refreshing: StdMutex::new(views::Refreshes::default()),
            ui_contributions: RwLock::new(json!([])),
            shell: StdRwLock::new(json!({})),
            answerers: StdMutex::new(VecDeque::new()),
            process: StdRwLock::new(json!({})),
            started: std::time::Instant::now(),
            described: StdMutex::new(BTreeMap::new()),
            limits: serde_json::to_value(config.limits).unwrap_or(json!({})),
            prompt_timeout_secs: config.prompt_timeout_secs,
            spaces: StdMutex::new(config.spaces),
            audit: StdRwLock::new(None),
        })
    }

    pub(crate) fn replay_from(&self) -> Option<&std::path::Path> {
        self.replay_from.as_deref()
    }

    /// In replay, fail a call whose request differs from the recorded one rather than report it
    /// (ipdrun-45). Off by default; meaningless outside replay. Set once, at startup, by whoever
    /// started the process in replay — the same operator's decision as replay itself.
    pub fn set_replay_strict(&self, strict: bool) {
        self.replay_strict
            .store(strict, std::sync::atomic::Ordering::Relaxed);
    }

    /// The recorder options a new or resumed session gets (see [`with_recording`]).
    pub(crate) fn recording(&self, params: Value) -> Value {
        with_recording(
            params,
            self.replay_from(),
            self.replay_strict.load(std::sync::atomic::Ordering::Relaxed),
        )
    }

    /// Replace what a client will be told the packages contribute.
    ///
    /// Called at assembly and again after an install: a package that needed a
    /// reload to appear is a package nobody will believe installed.
    pub async fn set_ui_contributions(&self, contributions: Value) {
        *self.ui_contributions.write().await = contributions;
    }

    /// Declare [`SPACES_TOPIC`]. Its value is computed when it is subscribed
    /// to and recomputed when a space is added or removed.
    pub async fn declare_spaces_topic(&self) {
        self.inner.lock().await.topics.declare(SPACES_TOPIC, Vec::new());
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
        mut guard: tokio::sync::MutexGuard<'_, Inner>,
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

    pub(crate) fn slot(&self, instance: &str) -> Option<Arc<Slot>> {
        read(&self.instances).get(instance).cloned()
    }

    pub(crate) fn slots(&self) -> Vec<Arc<Slot>> {
        read(&self.instances).values().cloned().collect()
    }

    /// Which instance a call means: the one it names, or `system`. One that
    /// does not exist is refused by name.
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
        if self.slot(name).is_none() {
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
        let Some(slot) = self.slot(instance) else {
            return Err(RpcError::new(
                codes::INSTANCE_NOT_FOUND,
                format!("there is no instance `{instance}`"),
            ));
        };
        slot.link.call(method, params).await
    }

    /// Return once every frame the instance sent before this call has been
    /// routed — not merely received.
    ///
    /// `run_turn`'s reply can overtake the turn's last events, so without
    /// this a turn could be settled while `turn_complete` was still on its
    /// way, and watchers would see the settlement first (IPDrun N-105).
    pub(crate) async fn barrier(&self, instance: &str) {
        let Some(slot) = self.slot(instance) else {
            return;
        };
        let id = self
            .next_barrier
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        lock(&self.barriers).insert(id, tx);
        slot.link.mark(id).await;
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

    /// The shell's part of hello: brand, built-in spaces, runtime modules.
    pub fn set_shell(&self, shell: Value) {
        *write(&self.shell) = shell;
    }

    /// `harnix.process` as a topic: its memory, threads and connections change without an event to
    /// say so, so it is computed again every few seconds — only while someone watches it.
    fn tick_process(self: &Arc<Self>) {
        if self.process_ticking.swap(true, std::sync::atomic::Ordering::SeqCst) {
            return;
        }
        let hub = Arc::downgrade(self);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(PROCESS_EVERY).await;
                let Some(hub) = hub.upgrade() else { return };
                hub.refresh("harnix.process", json!({}));
            }
        });
    }

    /// The process's own facts for `harnix.process`.
    pub fn set_process(&self, process: Value) {
        *write(&self.process) = process;
    }

    /// Versions, uptime, memory and who is connected: the system space's strip.
    fn process(&self) -> Value {
        let mut out = read(&self.process).clone();
        let connections: Vec<Value> = lock(&self.described)
            .iter()
            .map(|(id, d)| {
                let mut d = d.clone();
                d["client"] = json!(id.0);
                d
            })
            .collect();
        if let Some(obj) = out.as_object_mut() {
            obj.insert(
                "uptime_secs".into(),
                json!(self.started.elapsed().as_secs()),
            );
            obj.insert("peak_rss_bytes".into(), json!(peak_rss_bytes()));
            obj.insert("threads".into(), json!(os_threads()));
            obj.insert("connections".into(), json!(connections));
        }
        out
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

    pub async fn shutdown(&self) {
        for slot in self.slots() {
            slot.stop().await;
        }
    }

    /// What a client is told once, after the handshake: what the `system`
    /// engine is, what scenes and commands exist there, every instance and
    /// the protocol it speaks, and the ceilings.
    async fn hello(&self) -> Result<Value, RpcError> {
        // Once per connection, and none of the three waits on another.
        let (status, scenes, commands) = tokio::join!(
            self.call(SYSTEM, "daemon.status", json!({})),
            self.call(SYSTEM, "scene.list", json!({})),
            self.call(SYSTEM, "commands.list", json!({})),
        );
        let (status, scenes, commands) = (
            status.unwrap_or(json!({})),
            scenes.unwrap_or(json!({})),
            commands.unwrap_or(json!({})),
        );
        let system = self.slot(SYSTEM);
        let facts = system.as_ref().map(|s| s.facts.clone()).unwrap_or_default();
        let instances: Vec<Value> = self
            .slots()
            .iter()
            .map(|slot| {
                let identity = slot.link.identity();
                json!({
                    "id": slot.id,
                    "carrier": slot.link.carrier(),
                    "status": *slot.link.status().borrow(),
                    "version": identity.get("version"),
                    "protocol_version": identity.get("protocol_version"),
                })
            })
            .collect();
        Ok(json!({
            "protocol_version": harnix_contract::PROTOCOL_VERSION,
            "contrib_api_version": harnix_contract::CONTRIB_API_VERSION,
            "engine": {
                "model": facts.model,
                "active_scenes": facts.active_scenes,
                "scene": facts.scene,
                "data_root": facts.root_label(),
                "has_credentials": facts.has_credentials,
                "status": status,
            },
            "instances": instances,
            "permission_prompt_timeout_secs": self.prompt_timeout_secs,
            "scenes": scenes.get("scenes").cloned().unwrap_or(json!([])),
            "commands": commands.get("commands").cloned().unwrap_or(json!([])),
            // Interface modules the installed packages contribute, flat and
            // in load order. An array, not a map by point: the client imports
            // each one and the module decides what it registers, so grouping
            // by point here would be a shape nothing reads.
            "contributions": self.ui_contributions.read().await.clone(),
            "shell": read(&self.shell).clone(),
            "limits": self.limits.clone(),
        }))
    }

    /// Dispatch one already-authorized call.
    ///
    /// The hub's own semantics first, then the engine. Module methods never
    /// reach here: the application answers those above the hub, and their
    /// names cannot collide with these — the authorization table refuses a
    /// name already taken.
    async fn dispatch(
        self: &Arc<Self>,
        subject: &Subject,
        method: &str,
        params: Value,
    ) -> Result<Value, RpcError> {
        if spaces::CHECKED.contains(&method) {
            self.check_space(subject, method, &params)?;
        }
        match method {
            "harnix.hello" => return self.hello().await,
            "harnix.process" => return Ok(self.process()),
            "harnix.subscribe" => {
                if params.get("topic").and_then(Value::as_str) == Some("harnix.process") {
                    self.tick_process();
                }
                return self.subscribe(subject, params).await;
            }
            "harnix.unsubscribe" => return self.unsubscribe(subject, params).await,
            "harnix.sessions" if params.get("space").is_some() => {
                return self.space_sessions(&self.claimed(&params)?).await;
            }
            "harnix.sessions" | "harnix.instances.sessions" => {
                let instance = self.instance_of(&params)?;
                return self.sessions_view(&instance).await;
            }
            "harnix.spaces.list" => return Ok(self.spaces_view()),
            "harnix.instances.list" => return Ok(self.instances_view()),
            "harnix.instances.templates" => return Ok(self.templates_view(subject, &params).await),
            "harnix.instances.create" => return self.create_instance(subject, params, false).await,
            "harnix.instances.ensure" => return self.create_instance(subject, params, true).await,
            "harnix.instances.stop" => return self.stop_instance(subject, params).await,
            "harnix.instances.restart" => return self.restart_instance(subject, params).await,
            "harnix.instances.wait" => return self.wait_instance(params).await,
            "harnix.instances.inheritProviders" => return self.inherit_providers().await,
            "harnix.sessions.tasks" => {
                let key = self.key_of(&params)?;
                return self.tasks_view(&key).await;
            }
            "harnix.entries.append" => return self.append_entry(subject, params).await,
            "harnix.sessions.move" => return self.move_session(subject, params).await,
            "harnix.prompts.answerer" => return self.answerer(subject, &params),
            "harnix.attach" => return self.attach(subject, params).await,
            "harnix.detach" => return self.detach(subject, params).await,
            "harnix.send" => return self.send(subject, params).await,
            "harnix.queue.remove" => return self.queue_remove(params).await,
            "harnix.queue.edit" => return self.queue_edit(params).await,
            "session.interrupt" => return self.interrupt(params).await,
            "session.close" | "session.delete" => {
                return self.close_or_delete(method, params).await
            }
            "session.create" => return self.create(subject, params).await,
            "session.fork" => {
                let key = self.key_of(&params)?;
                let space = params
                    .get("space")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .unwrap_or_else(|| self.space_of(&key));
                let forked = self.call(&key.0, method, without_instance(params)).await?;
                if let Some(sid) = forked.get("session_id").and_then(Value::as_str) {
                    let new = (key.0.clone(), sid.to_string());
                    self.bind_space(&new, &space);
                    self.note_starter(&new, subject);
                }
                self.refresh("harnix.instances.sessions", json!({"instance": key.0}));
                return Ok(forked);
            }
            _ => {}
        }
        // A device reaches the engine only through the passthrough list. A
        // module or the kernel reaches what the authorizer let through — a
        // grant (the settings module writing a provider) is decided there,
        // not re-decided here.
        let device = matches!(subject, Subject::Device { .. });
        let mut params = params;
        if method == "session.respondToPrompt" {
            // The engine does not take who answered; the hub keeps it for the tool's audit.
            let tool_use = params.as_object_mut().and_then(|o| o.remove("tool_use_id"));
            if let (Subject::Device { id }, Some(Value::String(tool_use)), Some(session)) =
                (subject, tool_use, params.get("session_id").and_then(Value::as_str))
            {
                let mut kept = lock(&self.answerers);
                if kept.len() == ANSWERERS_KEPT {
                    kept.pop_front();
                }
                kept.push_back(((session.to_string(), tool_use), format!("device:{id}")));
            }
        }
        if PASSTHROUGH.contains(&method) || (!device && is_engine_method(method)) {
            let instance = self.instance_of(&params)?;
            let session_of_call = params.get("session_id").and_then(Value::as_str).map(str::to_string);
            let prompt = (method == "session.respondToPrompt")
                .then(|| {
                    let field = |k: &str| params.get(k).and_then(Value::as_str).map(str::to_string);
                    field("session_id").zip(field("prompt_id"))
                })
                .flatten();
            let answer = self
                .call(&instance, method, without_instance(params))
                .await?;
            if matches!(method, "session.goal" | "session.setGoal" | "session.updateGoal") {
                if let (Some(goal), Some(session)) = (answer.get("goal"), session_of_call.as_ref()) {
                    self.goal_heard(&(instance.clone(), session.clone()), goal.clone()).await;
                }
            }
            if let Some((session, prompt_id)) = prompt {
                self.answered(&(instance.clone(), session), |p| {
                    p.get("prompt_id").and_then(Value::as_str) == Some(prompt_id.as_str())
                })
                .await;
            }
            // What the session list shows changed; the engine sends no event for it.
            if RELISTED.contains(&method) {
                self.refresh("harnix.instances.sessions", json!({"instance": instance}));
            }
            return Ok(answer);
        }
        Err(RpcError::not_found(format!(
            "`{method}` is not a method of this process"
        )))
    }

    /// `harnix.prompts.answerer {session_id, tool_use_id}`: the device that answered that tool's
    /// permission prompt, once (it is forgotten on reading); `null` when no device answered it
    /// (the session's permission mode let it run) or the answer was pushed out. For the
    /// application's audit; a device cannot ask (helm-01).
    fn answerer(&self, subject: &Subject, params: &Value) -> Result<Value, RpcError> {
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
    async fn append_entry(&self, subject: &Subject, params: Value) -> Result<Value, RpcError> {
        let key = self.key_of(&params)?;
        let ns = match subject {
            Subject::Module { id } => format!("app.{id}"),
            Subject::Kernel => params
                .get("ns")
                .and_then(Value::as_str)
                .map(str::to_string)
                .ok_or_else(|| RpcError::invalid_params("the kernel names an entry's ns"))?,
            Subject::Device { .. } => {
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

/// The hub is the thing behind the gate. Authorization decides; this answers.
///
/// A separate type rather than `impl Gate for Hub`, because every method here
/// needs the `Arc` — a turn outlives the call that started it, which is the
/// whole point of the hub owning turns.
pub struct HubGate(pub Arc<Hub>);

#[async_trait::async_trait]
impl Gate for HubGate {
    async fn call(
        &self,
        subject: &Subject,
        method: &str,
        params: Value,
    ) -> Result<Value, RpcError> {
        self.0.dispatch(subject, method, params).await
    }
}

/// Engine methods after which an instance's session list reads differently.
const RELISTED: &[&str] = &[
    "session.rename",
    "session.clear",
    "session.compact",
    "session.dropMessages",
    "session.editMessage",
];

/// Whether a name is in one of the engine's namespaces, so a module's grant
/// can reach it. `harnix.*` and `app.*` never are.
fn is_engine_method(method: &str) -> bool {
    !method.starts_with("harnix.") && !method.starts_with("app.") && method.contains('.')
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
    guard: tokio::sync::MutexGuard<'_, Inner>,
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
        let inner = Arc::new(Mutex::new(Inner::default()));
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
