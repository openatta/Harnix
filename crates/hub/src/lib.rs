//! The hub: the holder of sessions.
//!
//! One invariant, and everything else follows from it:
//!
//! > **The hub subscribes before any client does.**
//!
//! From that come four things, none of which would justify a layer alone and
//! all of which together do (§3.2):
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
//! (design §5.3). Its callers are of two kinds with one set of semantics:
//! clients, and the server side — the kernel and modules (design §6.1).
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

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex as StdMutex, RwLock as StdRwLock};

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

/// The hub's own methods. Named here so `harnix_authz` can open them and the
/// generated method catalog can list them without tracing dispatch.
pub const HUB_METHODS: &[&str] = &[
    "harnix.hello",
    "harnix.sessions",
    "harnix.attach",
    "harnix.detach",
    "harnix.send",
    "harnix.queue.remove",
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
    "harnix.spaces.list",
    "harnix.process",
];

/// The hub's own topics: the instance tree as whole values (design §6.3).
/// Each with the parameters it is subscribed by.
pub const HUB_TOPICS: &[(&str, &[&str])] = &[
    ("harnix.instances", &[]),
    ("harnix.instances.sessions", &["instance"]),
    ("harnix.sessions.tasks", &["instance", "session_id"]),
    ("harnix.sessions.status", &[]),
];

/// The instance every process has: the profile's `[engine]`.
pub use harnix_contract::SYSTEM_INSTANCE as SYSTEM;

/// A session's full name. Two instances may each have a session `s1`; they
/// are different sessions (design §5.6).
pub(crate) type SessionKey = (String, String);

#[derive(Default)]
struct Inner {
    clients: Clients,
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
    instances: StdRwLock<BTreeMap<String, Arc<Slot>>>,
    registry: Mutex<Registry>,
    inner: Arc<Mutex<Inner>>,
    next_client: AtomicU64,
    barriers: StdMutex<HashMap<u64, oneshot::Sender<()>>>,
    next_barrier: AtomicU64,
    /// Topic values waiting to be recomputed, so a burst of changes costs one
    /// computation rather than one each.
    refreshing: StdMutex<HashSet<(String, String)>>,
    /// Interface modules from installed packages, told to a client at hello.
    /// Held here rather than resolved per call because it changes when a
    /// package is installed, not when someone asks.
    ui_contributions: RwLock<Value>,
    /// What the shell is told about this product at hello: its brand, which built-in spaces it
    /// keeps, whether runtime interface modules load. Set once at assembly.
    shell: StdRwLock<Value>,
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
            instances: StdRwLock::new(BTreeMap::new()),
            registry: Mutex::new(config.registry),
            inner: Arc::new(Mutex::new(inner)),
            next_client: AtomicU64::new(1),
            barriers: StdMutex::new(HashMap::new()),
            next_barrier: AtomicU64::new(1),
            refreshing: StdMutex::new(HashSet::new()),
            ui_contributions: RwLock::new(json!([])),
            shell: StdRwLock::new(json!({})),
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

    /// Replace what a client will be told the packages contribute.
    ///
    /// Called at assembly and again after an install: a package that needed a
    /// reload to appear is a package nobody will believe installed.
    pub async fn set_ui_contributions(&self, contributions: Value) {
        *self.ui_contributions.write().await = contributions;
    }

    /// Declare a host topic, at assembly. Subscribing to one that was never
    /// declared is refused.
    pub async fn declare_topic(&self, name: &str, params: Vec<String>) {
        self.inner.lock().await.topics.declare(name, params);
    }

    /// Publish the whole current value of a topic for one parameter set,
    /// and push it to that set's subscribers.
    pub async fn publish(&self, topic: &str, params: Value, value: Value) {
        let mut guard = self.inner.lock().await;
        let key = match guard.topics.key(topic, &params) {
            Ok(key) => key,
            Err(e) => {
                tracing::warn!(topic, error = %e.message, "not published");
                return;
            }
        };
        let (to, frame) = guard.topics.publish(key, params, value);
        guard.clients.send_many(&to, &frame).await;
    }

    pub(crate) fn slot(&self, instance: &str) -> Option<Arc<Slot>> {
        self.instances
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(instance)
            .cloned()
    }

    pub(crate) fn slots(&self) -> Vec<Arc<Slot>> {
        self.instances
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .values()
            .cloned()
            .collect()
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
        self.barriers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(id, tx);
        slot.link.mark(id).await;
        // Bounded, because a link that went down between the reply and the
        // mark will never route it; settling late beats never settling.
        let _ = tokio::time::timeout(std::time::Duration::from_secs(10), rx).await;
        self.barriers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&id);
    }

    pub(crate) fn barrier_reached(&self, id: u64) {
        if let Some(tx) = self
            .barriers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&id)
        {
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
        self.described
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(id, description);
    }

    /// The shell's part of hello: brand, built-in spaces, runtime modules.
    pub fn set_shell(&self, shell: Value) {
        *self.shell.write().unwrap_or_else(|e| e.into_inner()) = shell;
    }

    /// The process's own facts for `harnix.process`.
    pub fn set_process(&self, process: Value) {
        *self.process.write().unwrap_or_else(|e| e.into_inner()) = process;
    }

    /// Versions, uptime, memory and who is connected: the system space's strip.
    fn process(&self) -> Value {
        let mut out = self
            .process
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let connections: Vec<Value> = self
            .described
            .lock()
            .unwrap_or_else(|e| e.into_inner())
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
            obj.insert("connections".into(), json!(connections));
        }
        out
    }

    pub async fn remove_client(&self, id: ClientId) {
        self.described
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&id);
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
        let status = self
            .call(SYSTEM, "daemon.status", json!({}))
            .await
            .unwrap_or(json!({}));
        let scenes = self
            .call(SYSTEM, "scene.list", json!({}))
            .await
            .unwrap_or(json!({}));
        let commands = self
            .call(SYSTEM, "commands.list", json!({}))
            .await
            .unwrap_or(json!({}));
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
                "data_root": facts.data_root.display().to_string(),
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
            "shell": self.shell.read().unwrap_or_else(|e| e.into_inner()).clone(),
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
            "harnix.subscribe" => return self.subscribe(subject, params).await,
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
            "harnix.sessions.tasks" => {
                let key = self.key_of(&params)?;
                return self.tasks_view(&key).await;
            }
            "harnix.entries.append" => return self.append_entry(subject, params).await,
            "harnix.attach" => return self.attach(subject, params).await,
            "harnix.detach" => return self.detach(subject, params).await,
            "harnix.send" => return self.send(subject, params).await,
            "harnix.queue.remove" => return self.queue_remove(params).await,
            "session.interrupt" => return self.interrupt(params).await,
            "session.close" | "session.delete" => {
                return self.close_or_delete(method, params).await
            }
            "session.create" => return self.create(params).await,
            "session.fork" => {
                let key = self.key_of(&params)?;
                let space = params
                    .get("space")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .unwrap_or_else(|| self.space_of(&key));
                let forked = self.call(&key.0, method, without_instance(params)).await?;
                if let Some(sid) = forked.get("session_id").and_then(Value::as_str) {
                    self.bind_space(&(key.0.clone(), sid.to_string()), &space);
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
        if PASSTHROUGH.contains(&method) || (!device && is_engine_method(method)) {
            let instance = self.instance_of(&params)?;
            let answer = self
                .call(&instance, method, without_instance(params))
                .await?;
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

    /// `harnix.entries.append {instance, session_id, event, payload}`: an entry in the session's log,
    /// for people and never for the model (design §7.6). A module writes under its own namespace,
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
/// see", and that question is not optional (§6.4). No `dir` is named: the
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
pub(crate) fn with_recording(mut params: Value, replay_from: Option<&std::path::Path>) -> Value {
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
            // Divergences are reported, not fatal — and that is a concession
            // to a fact, not a preference.
            //
            // Strict replay compares the live request against the recorded
            // one. The assembled prompt carries **today's date** and absolute
            // paths, so a strict fixture stops matching the morning after it
            // was recorded, and again on any machine with a different
            // checkout path. What replay is for here is making an agent's
            // *behaviour* deterministic — the tool calls, the permission
            // asks, the sub-agents — and that works regardless.
            //
            // Making strict possible needs the engine's `environment` seam
            // (the one that owns "the date in the prompt") to be settable by
            // a host. That is filed; until then, strictness would only ever
            // be a test that fails at midnight.
            "strict": false,
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
