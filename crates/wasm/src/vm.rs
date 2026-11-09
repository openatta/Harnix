//! One application's VM: its linked component, its resident backend, and the
//! short-lived instances its tool calls and events run in.
//!
//! # Two kinds of instance
//!
//! The **resident** backend is one `Store` that lives as long as the VM. The
//! application's state is in it, so calls into it are serialized (a store is
//! not re-entrant) behind an async mutex: methods from [`AppVm::handle`], and
//! topic values and timer firings from a queue drained by one worker task, in
//! order. Every **tool call and event** gets a store of its own, built from the
//! same linked component and dropped when the call returns — concurrent, and
//! unable to damage anything that outlives it.
//!
//! # Deadlines
//!
//! The engine's epoch makes a running guest yield every tick, which turns an
//! otherwise uninterruptible loop into a future the host can stop polling.
//! Timeouts and cancellations are both "stop polling, drop the store", so
//! there is one path for both. A transient store is dropped anyway; the
//! resident one is *replaced*: code stopped halfway cannot be trusted to have
//! left its own state consistent (and a trap leaves a component instance
//! unusable in any case), so a fresh instance is started and `init` runs again.
//! Its in-memory state is lost and the health snapshot says so (`recycles`).
//! A backend call takes no cancellation for the same reason: a client that
//! goes away must not be able to wipe an application's state.
//!
//! # What an instance holds on the host
//!
//! Subscriptions and timers belong to the resident instance that made them.
//! The VM records them, and when that instance ends — replaced, rebuilt,
//! stopped, disabled — undoes them, so the host never has to know which
//! instance is current.

use crate::bindings::exports::harnix::app::{backend, events, tools};
use crate::bindings::harnix::app::types as wit;
use crate::engine::{wasm_err, AppComponent, WasmEngine};
use crate::health::{Counters, HealthSnapshot, VmState, FAULT_LIMIT};
use crate::spec::{AppSpec, Resolved};
use crate::state::{AppState, MemoryAccount, Role};
use crate::{ProgressSink, Services};
use anyhow::{anyhow, Context};
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::task::AbortHandle;
use tokio_util::sync::CancellationToken;
use wasmtime::component::{HasSelf, InstancePre, Linker};
use wasmtime::Store;

/// Timers one backend instance may have armed at once.
pub const MAX_TIMERS: usize = 64;
/// Hub subscriptions one backend instance may hold at once.
pub const MAX_SUBSCRIPTIONS: usize = 64;
/// Topic values and timer firings waiting for the backend. Past this the
/// backend is not keeping up, and dropping (and counting) beats growing
/// without bound.
const DELIVERY_QUEUE: usize = 256;
/// Longer than this is a timer that will not survive a restart anyway.
const MAX_TIMER_DELAY: Duration = Duration::from_secs(30 * 24 * 3600);
/// JSON-RPC's "internal error", for a backend answer that is not JSON.
const INTERNAL_ERROR: i32 = -32603;
/// How long a backend has to run without crashing for a crash before it to stop counting: past
/// this, the next crash is a first one again and is rebuilt, rather than disabling the
/// application. A backend that crashes once a week is unlucky; one that crashes twice in a minute
/// is broken. [`AppVm::set_healthy_period`] changes it.
pub const HEALTHY_PERIOD: Duration = Duration::from_secs(10 * 60);

static NEXT_VM: AtomicU64 = AtomicU64::new(1);

/// The `harnix:app` world a component implements.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum World {
    /// Tools and events only.
    Tools,
    /// A resident backend, plus tools and events.
    Backend,
}

/// One tool a component exports.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ToolDef {
    pub name: String,
    pub description: String,
    pub doc: Option<String>,
    /// JSON Schema, as the component wrote it.
    pub input_schema: String,
    pub read_only: bool,
    pub concurrency_safe: bool,
}

/// A tool's result. `is_error` is the tool answering that it failed; a trap
/// is a [`CallFailure`] instead.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ToolOutput {
    pub content: String,
    pub structured: Option<Value>,
    pub is_error: bool,
}

/// What a component answers to a lifecycle event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "decision", content = "text", rename_all = "snake_case")]
pub enum HookDecision {
    Proceed,
    Block(String),
    AddContext(String),
}

/// A method's failure as the application reports it, for its caller.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AppError {
    pub code: i32,
    pub message: String,
    pub data: Option<Value>,
}

/// Why a call produced no answer from the component.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallFailure {
    /// The call outlived the application's `timeout_ms`.
    TimedOut,
    /// The caller withdrew (a cancelled turn, a closing session).
    Cancelled,
    /// The component trapped, ran out of memory, or could not be run.
    Faulted(String),
    /// The call was not made: the application is stopped or disabled, its
    /// component does not export what was asked for, or the call would have
    /// deadlocked. Never counted against the application.
    Unavailable(String),
}

impl std::fmt::Display for CallFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TimedOut => write!(f, "the application's call exceeded its timeout"),
            Self::Cancelled => write!(f, "the application's call was cancelled"),
            Self::Faulted(e) => write!(f, "the application failed: {e}"),
            Self::Unavailable(e) => write!(f, "the application cannot take this call: {e}"),
        }
    }
}

impl std::error::Error for CallFailure {}

/// One application's VM. Cheap to clone; every clone is the same VM.
///
/// Must be started, used and dropped inside a tokio runtime: the backend's
/// delivery worker and its timers are tasks.
#[derive(Clone)]
pub struct AppVm {
    inner: Arc<Inner>,
}

/// An [`AppVm`] that does not keep it alive: for what the host hands out that would otherwise
/// hold the VM in a cycle through its own services (a topic forwarder, a method's handler).
#[derive(Clone)]
pub struct WeakAppVm {
    inner: Weak<Inner>,
}

impl WeakAppVm {
    /// The VM, if anything still holds it.
    pub fn upgrade(&self) -> Option<AppVm> {
        self.inner.upgrade().map(|inner| AppVm { inner })
    }
}

struct Inner {
    shared: Arc<Shared>,
    pre: InstancePre<AppState>,
    world: World,
    tools: Option<tools::GuestIndices>,
    events: Option<events::GuestIndices>,
    backend: Option<backend::GuestIndices>,
    timeout: Duration,
    resident: tokio::sync::Mutex<Option<Resident>>,
    /// The resident instance was let go while idle ([`AppVm::hibernate`]): the next call into
    /// the backend starts one again.
    hibernating: std::sync::atomic::AtomicBool,
}

struct Resident {
    store: Store<AppState>,
    guest: backend::Guest,
}

/// The parts every store of the application reaches: what host functions
/// need, and what outlives any one instance.
pub(crate) struct Shared {
    vm_id: u64,
    pub app: String,
    pub caps: Resolved,
    /// `host.config()`, and what the backend's `init` is handed.
    pub config_json: String,
    pub services: Services,
    pub memory: Arc<MemoryAccount>,
    engine: WasmEngine,
    counters: Mutex<Counters>,
    state: Mutex<VmState>,
    subscriptions: Mutex<Vec<(String, Value)>>,
    timers: Mutex<HashMap<String, (u64, AbortHandle)>>,
    timer_generation: AtomicU64,
    deliveries: mpsc::Sender<Delivery>,
    /// `${data}`, when the application was granted it: measured into its storage.
    data_dir: Option<PathBuf>,
    /// What the files under it took when last measured.
    data_bytes: AtomicU64,
    /// [`HEALTHY_PERIOD`], in milliseconds.
    healthy_ms: AtomicU64,
    /// Deliveries queued for the backend or being delivered: a backend with any is not idle.
    pending: AtomicU64,
}

enum Delivery {
    Topic { topic: String, value_json: String },
    Timer(String),
}

/// Which kind of short-lived call is being judged: a tool's faults count against that tool, an
/// event's and a listing's against nothing (`health.rs`).
#[derive(Clone, Copy)]
enum Transient<'a> {
    Tool(&'a str),
    Event(&'a str),
    List,
}

impl Transient<'_> {
    fn what(&self) -> &str {
        match self {
            Self::Tool(name) | Self::Event(name) => name,
            Self::List => "list-tools",
        }
    }
}

/// Why the resident instance is being replaced, for the counter it goes on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Replaced {
    /// A call was abandoned at its deadline.
    Recycle,
    /// It crashed.
    Rebuild,
    /// Its files crossed the storage quota (`storage.rs`).
    Storage,
}

/// How a call into the component ended, before it is judged.
enum Abort {
    TimedOut,
    Cancelled,
    Failed(wasmtime::Error),
}

tokio::task_local! {
    /// The VMs whose resident backend the current task is inside, innermost
    /// last. A backend waiting on `host.call` still holds its instance, so a
    /// call that routes back into it would wait for itself forever; this is
    /// how that call is recognized and refused instead.
    static HOLDING: Vec<u64>;
}

fn holds(vm_id: u64) -> bool {
    HOLDING
        .try_with(|chain| chain.contains(&vm_id))
        .unwrap_or(false)
}

async fn holding<F: Future>(vm_id: u64, f: F) -> F::Output {
    let mut chain = HOLDING.try_with(Clone::clone).unwrap_or_default();
    chain.push(vm_id);
    HOLDING.scope(chain, f).await
}

/// Run `call` against a deadline and, for calls that take one, a cancellation.
async fn with_deadline<T>(
    call: impl Future<Output = wasmtime::Result<T>>,
    timeout: Duration,
    cancel: Option<&CancellationToken>,
) -> Result<T, Abort> {
    let cancelled = async {
        match cancel {
            Some(token) => token.cancelled().await,
            None => std::future::pending().await,
        }
    };
    tokio::select! {
        biased;
        _ = cancelled => Err(Abort::Cancelled),
        result = call => result.map_err(Abort::Failed),
        _ = tokio::time::sleep(timeout) => Err(Abort::TimedOut),
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl AppVm {
    /// Link `component` for the application `spec` describes and, for a
    /// backend, start its resident instance (`init` with `spec.config`).
    ///
    /// Fails when a capability cannot be resolved, the component imports
    /// something this host does not provide, or the backend does not start.
    pub async fn start(
        engine: &WasmEngine,
        component: &AppComponent,
        spec: AppSpec,
        services: Services,
    ) -> anyhow::Result<AppVm> {
        let app = spec.id.clone();
        let caps = Resolved::resolve(&spec.capabilities, &spec.dirs)
            .with_context(|| format!("resolving the capabilities of `{app}`"))?;

        let mut linker: Linker<AppState> = Linker::new(engine.inner());
        wasmtime_wasi::p2::add_to_linker_async(&mut linker)
            .map_err(wasm_err)
            .context("adding WASI to the linker")?;
        crate::bindings::BackendAppEvents::add_to_linker::<_, HasSelf<AppState>>(&mut linker, |s| s)
            .map_err(wasm_err)
            .context("adding the harnix:app host interface to the linker")?;
        let pre = linker
            .instantiate_pre(&component.component)
            .map_err(wasm_err)
            .with_context(|| format!("linking `{app}`"))?;

        let world = component.world();
        let backend = match world {
            World::Backend => Some(
                backend::GuestIndices::new(&pre)
                    .map_err(wasm_err)
                    .with_context(|| format!("`{app}` does not match harnix:app/backend"))?,
            ),
            World::Tools => None,
        };
        let (deliveries, queue) = mpsc::channel(DELIVERY_QUEUE);
        let data_dir = spec
            .capabilities
            .fs
            .iter()
            .any(|g| g.path.starts_with("${data}"))
            .then(|| spec.dirs.data.clone());
        let shared = Arc::new(Shared {
            vm_id: NEXT_VM.fetch_add(1, Ordering::Relaxed),
            app: app.clone(),
            caps,
            config_json: spec.config.to_string(),
            services,
            memory: Arc::new(MemoryAccount::new(spec.limits.memory_bytes)),
            engine: engine.clone(),
            counters: Mutex::new(Counters::default()),
            state: Mutex::new(VmState::Running),
            subscriptions: Mutex::new(Vec::new()),
            timers: Mutex::new(HashMap::new()),
            timer_generation: AtomicU64::new(0),
            deliveries,
            data_dir,
            data_bytes: AtomicU64::new(0),
            healthy_ms: AtomicU64::new(HEALTHY_PERIOD.as_millis() as u64),
            pending: AtomicU64::new(0),
        });
        shared.measure().await;
        let inner = Arc::new(Inner {
            shared,
            tools: tools::GuestIndices::new(&pre).ok(),
            events: events::GuestIndices::new(&pre).ok(),
            pre,
            world,
            backend,
            timeout: spec.limits.timeout,
            resident: tokio::sync::Mutex::new(None),
            hibernating: std::sync::atomic::AtomicBool::new(false),
        });
        if world == World::Backend {
            let resident = inner
                .instantiate_resident()
                .await
                .map_err(|f| anyhow!("starting the backend of `{app}`: {f}"))?;
            *inner.resident.lock().await = Some(resident);
            tokio::spawn(drain(Arc::downgrade(&inner), queue));
        }
        Ok(AppVm { inner })
    }

    pub fn app(&self) -> &str {
        &self.inner.shared.app
    }

    pub fn downgrade(&self) -> WeakAppVm {
        WeakAppVm {
            inner: Arc::downgrade(&self.inner),
        }
    }

    pub fn world(&self) -> World {
        self.inner.world
    }

    /// The tools the component exports. This is the authority, not the
    /// manifest: it is what will actually run.
    pub async fn list_tools(
        &self,
        cancel: &CancellationToken,
    ) -> Result<Vec<ToolDef>, CallFailure> {
        let inner = &self.inner;
        let idx = inner.export(&inner.tools, "tools")?;
        inner.shared.admit()?;
        let mut store = inner.store(Role::Transient, None, None)?;
        let mut started = false;
        let call = async {
            let instance = inner.pre.instantiate_async(&mut store).await?;
            let guest = idx.load(&mut store, &instance)?;
            started = true;
            guest.call_list_tools(&mut store).await
        };
        let outcome = with_deadline(call, inner.timeout, Some(cancel)).await;
        let refused = store.data().limiter.refused();
        drop(store);
        let tools = inner
            .settle_transient(outcome, started, refused, Transient::List)
            .await?;
        Ok(tools.into_iter().map(ToolDef::from).collect())
    }

    /// Run one tool in an instance of its own. `input` is the tool's
    /// arguments; `project` is the calling session's project, which the
    /// manifest's `${project}` grants are placed under (read-only) for this
    /// call alone; `progress` receives the component's `host.progress`.
    pub async fn call_tool(
        &self,
        name: &str,
        input: &Value,
        call_id: &str,
        project: Option<&Path>,
        progress: Option<Arc<dyn ProgressSink>>,
        cancel: &CancellationToken,
    ) -> Result<ToolOutput, CallFailure> {
        let inner = &self.inner;
        let idx = inner.export(&inner.tools, "tools")?;
        inner.shared.admit()?;
        inner.shared.tool_admitted(name)?;
        let input = input.to_string();
        let mut store = inner.store(Role::Transient, progress, project)?;
        let mut started = false;
        let call = async {
            let instance = inner.pre.instantiate_async(&mut store).await?;
            let guest = idx.load(&mut store, &instance)?;
            started = true;
            guest
                .call_call_tool(&mut store, name, &input, call_id)
                .await
        };
        let outcome = with_deadline(call, inner.timeout, Some(cancel)).await;
        let refused = store.data().limiter.refused();
        drop(store);
        let out = inner
            .settle_transient(outcome, started, refused, Transient::Tool(name))
            .await?;
        Ok(ToolOutput {
            structured: out.structured.and_then(|s| match serde_json::from_str(&s) {
                Ok(v) => Some(v),
                Err(e) => {
                    tracing::warn!(app = %inner.shared.app, tool = name, error = %e,
                        "a tool's structured output is not JSON; dropped");
                    None
                }
            }),
            content: out.content,
            is_error: out.is_error,
        })
    }

    /// Offer the component a lifecycle event. A component that does not
    /// export `events` answers `Proceed`.
    pub async fn on_event(
        &self,
        event: &str,
        payload: &Value,
        cancel: &CancellationToken,
    ) -> Result<HookDecision, CallFailure> {
        let inner = &self.inner;
        let Some(idx) = &inner.events else {
            return Ok(HookDecision::Proceed);
        };
        inner.shared.admit()?;
        let payload = payload.to_string();
        let mut store = inner.store(Role::Transient, None, None)?;
        let mut started = false;
        let call = async {
            let instance = inner.pre.instantiate_async(&mut store).await?;
            let guest = idx.load(&mut store, &instance)?;
            started = true;
            guest.call_on_event(&mut store, event, &payload).await
        };
        let outcome = with_deadline(call, inner.timeout, Some(cancel)).await;
        let refused = store.data().limiter.refused();
        drop(store);
        Ok(inner
            .settle_transient(outcome, started, refused, Transient::Event(event))
            .await?
            .into())
    }

    /// Answer the method `app.<id>.<method>` with the resident backend.
    /// `caller` describes who called; the hub has already admitted the call.
    ///
    /// The outer `Result` is whether the backend answered; the inner one is
    /// its answer. Calls queue behind each other and behind deliveries, for at
    /// most the timeout: a call that cannot get the backend by then is answered
    /// `busy` ([`CallFailure::Unavailable`]) rather than waiting behind every
    /// call queued before it.
    pub async fn handle(
        &self,
        method: &str,
        params: &Value,
        caller: &Value,
    ) -> Result<Result<Value, AppError>, CallFailure> {
        let inner = &self.inner;
        inner.export(&inner.backend, "backend")?;
        if holds(inner.shared.vm_id) {
            return Err(CallFailure::Unavailable(format!(
                "`{method}` would re-enter the backend of `{}`, which is waiting on this very call",
                inner.shared.app
            )));
        }
        inner.shared.admit()?;
        let (params, caller) = (params.to_string(), caller.to_string());
        let mut guard = match tokio::time::timeout(inner.timeout, inner.resident.lock()).await {
            Ok(guard) => guard,
            Err(_) => {
                // Stopped or set aside while it waited: that is the answer, not that it is busy.
                inner.shared.admit()?;
                lock(&inner.shared.counters).queue_timeouts += 1;
                return Err(CallFailure::Unavailable(format!(
                    "busy: the backend of `{}` did not come free within its timeout ({} ms) for `{method}`",
                    inner.shared.app,
                    inner.timeout.as_millis()
                )));
            }
        };
        inner.shared.admit()?;
        inner.wake(&mut guard).await?;
        let Some(resident) = guard.as_mut() else {
            return Err(CallFailure::Unavailable(
                "the backend is not running".into(),
            ));
        };
        let call = resident
            .guest
            .call_handle(&mut resident.store, method, &params, &caller);
        let outcome = holding(inner.shared.vm_id, with_deadline(call, inner.timeout, None)).await;
        let answer = inner.settle_resident(&mut guard, outcome, method).await?;
        Ok(match answer {
            Ok(json) => serde_json::from_str(&json).map_err(|e| AppError {
                code: INTERNAL_ERROR,
                message: format!("the backend's answer to `{method}` is not JSON: {e}"),
                data: None,
            }),
            Err(e) => Err(e.into()),
        })
    }

    /// Hand the backend a value on a topic it subscribed to. Queued, in
    /// order, behind whatever the backend is doing; returns at once.
    pub fn deliver_topic(&self, topic: &str, value: &Value) {
        if self.inner.world == World::Backend {
            self.inner.shared.enqueue(Delivery::Topic {
                topic: topic.to_string(),
                value_json: value.to_string(),
            });
        }
    }

    /// Fire the backend's timer `id` now. The VM's own timers come through
    /// here; a host may too (to resume a timer it persisted, say).
    pub fn fire_timer(&self, id: &str) {
        if self.inner.world == World::Backend {
            self.inner.shared.enqueue(Delivery::Timer(id.to_string()));
        }
    }

    /// Stop the application: the backend's `stop`, bounded by the timeout;
    /// then its instance, subscriptions and timers are released. Every later
    /// call is refused.
    pub async fn stop(&self) {
        let inner = &self.inner;
        {
            let mut state = lock(&inner.shared.state);
            if *state == VmState::Running {
                *state = VmState::Stopped;
            }
        }
        let mut guard = inner.resident.lock().await;
        if let Some(mut resident) = guard.take() {
            let call = resident.guest.call_stop(&mut resident.store);
            match holding(inner.shared.vm_id, with_deadline(call, inner.timeout, None)).await {
                Ok(()) => {}
                Err(Abort::Failed(e)) => {
                    tracing::warn!(app = %inner.shared.app, error = %format!("{e:#}"), "backend stop failed")
                }
                Err(_) => tracing::warn!(app = %inner.shared.app, "backend stop timed out"),
            }
        }
        inner.shared.release_bindings().await;
    }

    /// How long the backend must run without crashing before an earlier crash stops counting
    /// against it ([`HEALTHY_PERIOD`] unless set).
    pub fn set_healthy_period(&self, period: Duration) {
        self.inner
            .shared
            .healthy_ms
            .store(period.as_millis() as u64, Ordering::Relaxed);
    }

    pub fn health(&self) -> HealthSnapshot {
        let shared = &self.inner.shared;
        let c = lock(&shared.counters);
        HealthSnapshot {
            app: shared.app.clone(),
            world: self.inner.world,
            state: lock(&shared.state).clone(),
            calls: c.calls,
            faults: c.faults,
            timeouts: c.timeouts,
            consecutive_faults: c.streaks.values().copied().max().unwrap_or(0),
            tools_aside: c.aside.clone(),
            rebuilds: c.rebuilds,
            recycles: c.recycles,
            storage_recycles: c.storage_recycles,
            queue_timeouts: c.queue_timeouts,
            dropped_deliveries: c.dropped_deliveries,
            last_error: c.last_error.clone(),
            memory_bytes: shared.memory.used(),
            memory_peak_bytes: shared.memory.peak(),
            memory_limit_bytes: shared.memory.limit,
            storage_bytes: shared.storage_used(),
            storage_files_bytes: shared.data_bytes.load(Ordering::Relaxed),
            storage_limit_bytes: shared.caps.storage_bytes,
            subscriptions: lock(&shared.subscriptions).len(),
            timers: lock(&shared.timers).len(),
            hibernating: self.hibernating(),
            wakes: c.wakes,
        }
    }

    /// Whether the resident backend was let go while idle and not started again yet.
    pub fn hibernating(&self) -> bool {
        self.inner.hibernating.load(Ordering::Acquire)
    }

    /// Let the resident backend go while nothing holds it — no subscription, no timer, no delivery
    /// queued or running, no call into it (which holds it, so this waits its turn behind one) —
    /// after its `stop`, bounded by the timeout. Its tools and events still run, each in an
    /// instance of its own; the next call into the backend starts it again, `init` and all, with
    /// what it kept in its storage and none of what it held in memory. Not a fault and not a
    /// recycle. Answers whether it hibernated.
    pub async fn hibernate(&self) -> bool {
        let inner = &self.inner;
        if inner.world != World::Backend {
            return false;
        }
        let mut guard = inner.resident.lock().await;
        let shared = &inner.shared;
        let held = !lock(&shared.subscriptions).is_empty()
            || !lock(&shared.timers).is_empty()
            || shared.pending.load(Ordering::Acquire) > 0;
        if shared.admit().is_err() || held {
            return false;
        }
        let Some(mut resident) = guard.take() else {
            return false;
        };
        let call = resident.guest.call_stop(&mut resident.store);
        match holding(shared.vm_id, with_deadline(call, inner.timeout, None)).await {
            Ok(()) => {}
            Err(Abort::Failed(e)) => {
                tracing::warn!(app = %shared.app, error = %format!("{e:#}"), "backend stop failed on hibernating")
            }
            Err(_) => tracing::warn!(app = %shared.app, "backend stop timed out on hibernating"),
        }
        drop(resident);
        shared.release_bindings().await;
        inner.hibernating.store(true, Ordering::Release);
        tracing::info!(app = %shared.app, "backend hibernating");
        true
    }
}

/// The backend's delivery worker: one task per VM, so topic values and
/// timers reach the backend in the order they came. Holds the VM weakly; it
/// ends when the VM is dropped and the queue closes.
async fn drain(vm: Weak<Inner>, mut queue: mpsc::Receiver<Delivery>) {
    while let Some(delivery) = queue.recv().await {
        let Some(inner) = vm.upgrade() else { break };
        inner.deliver(delivery).await;
    }
}

impl Inner {
    fn export<'a, T>(&self, idx: &'a Option<T>, interface: &str) -> Result<&'a T, CallFailure> {
        idx.as_ref().ok_or_else(|| {
            CallFailure::Unavailable(format!(
                "`{}` does not export harnix:app/{interface}",
                self.shared.app
            ))
        })
    }

    fn store(
        &self,
        role: Role,
        progress: Option<Arc<dyn ProgressSink>>,
        project: Option<&Path>,
    ) -> Result<Store<AppState>, CallFailure> {
        let state = AppState::new(self.shared.clone(), role, progress, project)
            .map_err(|e| CallFailure::Faulted(format!("{e:#}")))?;
        let mut store = Store::new(self.shared.engine.inner(), state);
        store.limiter(|s| &mut s.limiter);
        // Yield at every epoch tick rather than trap: what a missed deadline
        // means is the host's to decide, and it decides by dropping the
        // future.
        store.set_epoch_deadline(1);
        store.epoch_deadline_async_yield_and_update(1);
        Ok(store)
    }

    /// A fresh resident instance with `init` run on it.
    async fn instantiate_resident(&self) -> Result<Resident, CallFailure> {
        let idx = self.export(&self.backend, "backend")?;
        let mut store = self.store(Role::Backend, None, None)?;
        let call = async {
            let instance = self.pre.instantiate_async(&mut store).await?;
            let guest = idx.load(&mut store, &instance)?;
            let init = guest
                .call_init(&mut store, &self.shared.config_json)
                .await?;
            Ok((guest, init))
        };
        let outcome = holding(self.shared.vm_id, with_deadline(call, self.timeout, None)).await;
        let failure = match outcome {
            Ok((guest, Ok(()))) => return Ok(Resident { store, guest }),
            Ok((_, Err(why))) => CallFailure::Faulted(format!("`init` refused: {why}")),
            Err(Abort::Failed(e)) => {
                CallFailure::Faulted(self.describe(&e, store.data().limiter.refused()))
            }
            Err(Abort::TimedOut) => CallFailure::TimedOut,
            Err(Abort::Cancelled) => CallFailure::Cancelled,
        };
        // Whatever `init` managed to subscribe or arm before it failed.
        drop(store);
        self.shared.release_bindings().await;
        Err(failure)
    }

    /// A hibernating backend, started again for the call holding `guard`: `init` runs. One that
    /// will not start sets the application aside. Calls waiting behind this one find it started.
    async fn wake(
        &self,
        guard: &mut tokio::sync::MutexGuard<'_, Option<Resident>>,
    ) -> Result<(), CallFailure> {
        if guard.is_some() || !self.hibernating.load(Ordering::Acquire) {
            return Ok(());
        }
        match self.instantiate_resident().await {
            Ok(resident) => {
                **guard = Some(resident);
                self.hibernating.store(false, Ordering::Release);
                lock(&self.shared.counters).wakes += 1;
                tracing::info!(app = %self.shared.app, "backend woken");
                Ok(())
            }
            Err(f) => {
                self.shared
                    .disable(format!("the backend did not start again after hibernating: {f}"));
                Err(f)
            }
        }
    }

    /// A trap's message, or the memory limit when that is what caused it.
    fn describe(&self, e: &wasmtime::Error, refused: bool) -> String {
        if refused {
            format!(
                "memory limit of {} MiB reached ({e:#})",
                self.shared.memory.limit / (1024 * 1024)
            )
        } else {
            format!("{e:#}")
        }
    }

    /// Judge a transient call. `started` is whether instantiation finished,
    /// so the guest's own code was running; `refused` whether the store was
    /// denied memory. Its store is gone: what it wrote is measured now.
    async fn settle_transient<T>(
        &self,
        outcome: Result<T, Abort>,
        started: bool,
        refused: bool,
        call: Transient<'_>,
    ) -> Result<T, CallFailure> {
        let shared = &self.shared;
        shared.measure().await;
        let what = call.what();
        // Could not even be instantiated because the application's other
        // instances hold its memory: the component did nothing wrong.
        if let (Err(Abort::Failed(_)), false, true) = (&outcome, started, refused) {
            return Err(CallFailure::Unavailable(format!(
                "the memory of `{}` ({} MiB) is in use by its other instances",
                shared.app,
                shared.memory.limit / (1024 * 1024)
            )));
        }
        let mut c = lock(&shared.counters);
        c.calls += 1;
        match outcome {
            Ok(v) => {
                if let Transient::Tool(name) = call {
                    c.streaks.remove(name);
                }
                Ok(v)
            }
            Err(Abort::Cancelled) => Err(CallFailure::Cancelled),
            Err(Abort::TimedOut) => {
                c.timeouts += 1;
                c.last_error = Some(format!("`{what}` exceeded its timeout"));
                Err(CallFailure::TimedOut)
            }
            Err(Abort::Failed(e)) => {
                let message = self.describe(&e, refused);
                c.faults += 1;
                c.last_error = Some(format!("`{what}`: {message}"));
                // Only a tool's faults are held against anything, and only against that tool.
                if let Transient::Tool(name) = call {
                    let streak = c.streaks.entry(name.to_string()).or_default();
                    *streak += 1;
                    if *streak >= FAULT_LIMIT {
                        let reason = format!("{FAULT_LIMIT} consecutive faults, the last: {message}");
                        tracing::error!(app = %shared.app, tool = name, %reason, "tool set aside");
                        c.aside.insert(name.to_string(), reason);
                    }
                }
                Err(CallFailure::Faulted(message))
            }
        }
    }

    /// Judge a call into the resident instance, replacing the instance when
    /// it can no longer be trusted — or when its files crossed the storage
    /// quota, so that the instance that takes over has `/data` as the quota
    /// now says.
    async fn settle_resident<T>(
        &self,
        guard: &mut tokio::sync::MutexGuard<'_, Option<Resident>>,
        outcome: Result<T, Abort>,
        what: &str,
    ) -> Result<T, CallFailure> {
        let shared = &self.shared;
        shared.measure().await;
        let refused = guard
            .as_ref()
            .is_some_and(|r| r.store.data().limiter.refused());
        let healthy = Duration::from_millis(shared.healthy_ms.load(Ordering::Relaxed));
        // `Err(None)`: abandoned at its deadline; `Err(Some(..))`: a crash, and how many in a row.
        let judged = {
            let mut c = lock(&shared.counters);
            c.calls += 1;
            match outcome {
                Ok(v) => Ok(v),
                Err(Abort::Cancelled) => return Err(CallFailure::Cancelled),
                Err(Abort::TimedOut) => {
                    c.timeouts += 1;
                    c.last_error = Some(format!(
                        "`{what}` exceeded its timeout; the backend was restarted"
                    ));
                    Err(None)
                }
                Err(Abort::Failed(e)) => {
                    let message = self.describe(&e, refused);
                    // A crash long enough ago is forgiven: this one is a first one again.
                    if c.last_crash.is_some_and(|at| at.elapsed() >= healthy) {
                        c.crashes = 0;
                    }
                    c.last_crash = Some(std::time::Instant::now());
                    c.faults += 1;
                    c.crashes += 1;
                    c.last_error = Some(format!("`{what}`: {message}"));
                    Err(Some((message, c.crashes)))
                }
            }
        };
        match judged {
            Ok(v) => {
                if self.storage_moved(guard) {
                    tracing::warn!(app = %shared.app, full = shared.storage_full(),
                        "the backend's files crossed its storage quota; replacing its instance");
                    self.replace_resident(guard, Replaced::Storage).await;
                }
                Ok(v)
            }
            Err(None) => {
                self.replace_resident(guard, Replaced::Recycle).await;
                Err(CallFailure::TimedOut)
            }
            Err(Some((message, 1))) => {
                self.replace_resident(guard, Replaced::Rebuild).await;
                Err(CallFailure::Faulted(message))
            }
            Err(Some((message, _))) => {
                shared.disable(format!(
                    "the backend crashed again after being rebuilt, in `{what}`: {message}"
                ));
                **guard = None;
                shared.release_bindings().await;
                Err(CallFailure::Faulted(message))
            }
        }
    }

    /// Whether the resident instance's `/data` no longer matches the quota: writable while the
    /// application is over it, or read-only while it has room again.
    fn storage_moved(&self, guard: &tokio::sync::MutexGuard<'_, Option<Resident>>) -> bool {
        let full = self.shared.storage_full();
        guard
            .as_ref()
            .and_then(|r| r.store.data().over_quota)
            .is_some_and(|was| was != full)
    }

    /// Drop the resident instance and start a fresh one. `why` decides
    /// which counter it goes on; a fresh instance that will not start
    /// disables the application.
    async fn replace_resident(
        &self,
        guard: &mut tokio::sync::MutexGuard<'_, Option<Resident>>,
        why: Replaced,
    ) {
        **guard = None;
        self.shared.release_bindings().await;
        // Stopping (or set aside) meanwhile: nothing is to run in it any more.
        if self.shared.admit().is_err() {
            return;
        }
        match self.instantiate_resident().await {
            Ok(resident) => {
                **guard = Some(resident);
                let mut c = lock(&self.shared.counters);
                match why {
                    Replaced::Rebuild => c.rebuilds += 1,
                    Replaced::Recycle => c.recycles += 1,
                    Replaced::Storage => c.storage_recycles += 1,
                }
                tracing::warn!(app = %self.shared.app, ?why, "backend instance replaced");
            }
            Err(f) => self
                .shared
                .disable(format!("the backend could not be restarted: {f}")),
        }
    }

    async fn deliver(&self, delivery: Delivery) {
        // However it ends: queued, then delivered or dropped.
        struct Delivered<'a>(&'a AtomicU64);
        impl Drop for Delivered<'_> {
            fn drop(&mut self) {
                self.0.fetch_sub(1, Ordering::AcqRel);
            }
        }
        let _delivered = Delivered(&self.shared.pending);
        if self.shared.admit().is_err() {
            return;
        }
        let mut guard = self.resident.lock().await;
        if self.shared.admit().is_err() {
            return;
        }
        if self.wake(&mut guard).await.is_err() {
            return;
        }
        let Some(resident) = guard.as_mut() else {
            return;
        };
        let vm_id = self.shared.vm_id;
        let (what, outcome) = match &delivery {
            Delivery::Topic { topic, value_json } => {
                let call = resident
                    .guest
                    .call_on_topic(&mut resident.store, topic, value_json);
                (
                    format!("on-topic {topic}"),
                    holding(vm_id, with_deadline(call, self.timeout, None)).await,
                )
            }
            Delivery::Timer(id) => {
                let call = resident.guest.call_on_timer(&mut resident.store, id);
                (
                    format!("on-timer {id}"),
                    holding(vm_id, with_deadline(call, self.timeout, None)).await,
                )
            }
        };
        let _ = self.settle_resident(&mut guard, outcome, &what).await;
    }
}

impl Shared {
    pub(crate) fn admit(&self) -> Result<(), CallFailure> {
        match &*lock(&self.state) {
            VmState::Running => Ok(()),
            VmState::Stopped => Err(CallFailure::Unavailable(format!(
                "`{}` is stopped",
                self.app
            ))),
            VmState::Disabled { reason } => Err(CallFailure::Unavailable(format!(
                "`{}` is disabled: {reason}",
                self.app
            ))),
        }
    }

    /// Bytes the application keeps: its key-value store, and its files under `${data}` as last
    /// measured.
    pub(crate) fn storage_used(&self) -> u64 {
        self.services
            .kv
            .used_bytes()
            .saturating_add(self.data_bytes.load(Ordering::Relaxed))
    }

    /// Measure the files under `${data}` again: after anything that could have written them, on a
    /// thread that may block. A directory past the walk's limit counts as the whole quota.
    async fn measure(&self) {
        let Some(dir) = self.data_dir.clone() else {
            return;
        };
        let measured = tokio::task::spawn_blocking(move || crate::storage::dir_bytes(&dir))
            .await
            .unwrap_or(None);
        let bytes = measured.unwrap_or_else(|| {
            tracing::warn!(app = %self.app, "its data directory has more entries than are counted; \
                taken as over its quota");
            self.caps.storage_bytes
        });
        self.data_bytes.store(bytes, Ordering::Relaxed);
    }

    /// Whether the tool `name` may be called: not one set aside for faulting.
    fn tool_admitted(&self, name: &str) -> Result<(), CallFailure> {
        match lock(&self.counters).aside.get(name) {
            None => Ok(()),
            Some(reason) => Err(CallFailure::Unavailable(format!(
                "the tool `{name}` of `{}` is set aside: {reason}",
                self.app
            ))),
        }
    }

    /// The key-value store's share of the quota: what the files leave of it.
    pub(crate) fn kv_quota(&self) -> u64 {
        self.caps
            .storage_bytes
            .saturating_sub(self.data_bytes.load(Ordering::Relaxed))
    }

    /// Over the quota: new instances get `/data` read-only until it has room again.
    pub(crate) fn storage_full(&self) -> bool {
        self.data_dir.is_some() && self.storage_used() >= self.caps.storage_bytes
    }

    /// Set the application aside. A VM already stopped stays stopped: what failed while it was
    /// stopping is not a reason to report it broken.
    fn disable(&self, reason: String) {
        let mut state = lock(&self.state);
        if *state == VmState::Running {
            tracing::error!(app = %self.app, %reason, "application disabled");
            *state = VmState::Disabled { reason };
        }
    }

    fn enqueue(&self, delivery: Delivery) {
        self.pending.fetch_add(1, Ordering::AcqRel);
        if let Err(e) = self.deliveries.try_send(delivery) {
            self.pending.fetch_sub(1, Ordering::AcqRel);
            if let mpsc::error::TrySendError::Full(_) = e {
                lock(&self.counters).dropped_deliveries += 1;
                tracing::warn!(app = %self.app, "backend delivery queue full; dropped");
            }
        }
    }

    /// `host.subscribe`, after the role check.
    pub(crate) async fn subscribe(&self, topic: &str, params: Value) -> Result<(), String> {
        {
            let subs = lock(&self.subscriptions);
            if subs.iter().any(|(t, p)| t == topic && *p == params) {
                return Ok(());
            }
            if subs.len() >= MAX_SUBSCRIPTIONS {
                return Err(format!(
                    "this backend already holds {MAX_SUBSCRIPTIONS} subscriptions"
                ));
            }
        }
        self.services
            .host
            .subscribe(&self.app, topic, &params)
            .await?;
        lock(&self.subscriptions).push((topic.to_string(), params));
        Ok(())
    }

    pub(crate) async fn unsubscribe(&self, topic: &str, params: Value) {
        let removed = {
            let mut subs = lock(&self.subscriptions);
            let before = subs.len();
            subs.retain(|(t, p)| !(t == topic && *p == params));
            subs.len() != before
        };
        if removed {
            self.services
                .host
                .unsubscribe(&self.app, topic, &params)
                .await;
        }
    }

    /// `host.set-timer`, after the role check.
    pub(crate) fn arm_timer(self: &Arc<Self>, id: String, after: Duration) -> Result<(), String> {
        let after = after.min(MAX_TIMER_DELAY);
        let mut timers = lock(&self.timers);
        if !timers.contains_key(&id) && timers.len() >= MAX_TIMERS {
            return Err(format!(
                "this backend already has {MAX_TIMERS} timers armed"
            ));
        }
        let generation = self.timer_generation.fetch_add(1, Ordering::Relaxed);
        let shared = Arc::downgrade(self);
        let fired = id.clone();
        let task = tokio::spawn(async move {
            tokio::time::sleep(after).await;
            if let Some(shared) = shared.upgrade() {
                shared.fire(&fired, generation);
            }
        });
        if let Some((_, previous)) = timers.insert(id, (generation, task.abort_handle())) {
            previous.abort();
        }
        Ok(())
    }

    pub(crate) fn cancel_timer(&self, id: &str) {
        if let Some((_, task)) = lock(&self.timers).remove(id) {
            task.abort();
        }
    }

    fn fire(&self, id: &str, generation: u64) {
        {
            let mut timers = lock(&self.timers);
            // Re-armed since this one was set: the newer arming wins.
            if timers.get(id).map(|(g, _)| *g) != Some(generation) {
                return;
            }
            timers.remove(id);
        }
        self.enqueue(Delivery::Timer(id.to_string()));
    }

    /// Undo everything the resident instance held on the host.
    async fn release_bindings(&self) {
        for (_, (_, task)) in lock(&self.timers).drain() {
            task.abort();
        }
        let subs = std::mem::take(&mut *lock(&self.subscriptions));
        for (topic, params) in subs {
            self.services
                .host
                .unsubscribe(&self.app, &topic, &params)
                .await;
        }
    }
}

impl Drop for Shared {
    /// A VM dropped without [`AppVm::stop`] still gives back what it held.
    fn drop(&mut self) {
        for (_, (_, task)) in lock(&self.timers).drain() {
            task.abort();
        }
        let subs = std::mem::take(&mut *lock(&self.subscriptions));
        if subs.is_empty() {
            return;
        }
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            let (host, app) = (self.services.host.clone(), self.app.clone());
            runtime.spawn(async move {
                for (topic, params) in subs {
                    host.unsubscribe(&app, &topic, &params).await;
                }
            });
        }
    }
}

impl From<wit::ToolDef> for ToolDef {
    fn from(t: wit::ToolDef) -> Self {
        Self {
            name: t.name,
            description: t.description,
            doc: t.doc,
            input_schema: t.input_schema,
            read_only: t.read_only,
            concurrency_safe: t.concurrency_safe,
        }
    }
}

impl From<wit::HookDecision> for HookDecision {
    fn from(d: wit::HookDecision) -> Self {
        match d {
            wit::HookDecision::Proceed => Self::Proceed,
            wit::HookDecision::Block(why) => Self::Block(why),
            wit::HookDecision::AddContext(text) => Self::AddContext(text),
        }
    }
}

impl From<wit::AppError> for AppError {
    fn from(e: wit::AppError) -> Self {
        Self {
            code: e.code,
            message: e.message,
            // Detail that is not JSON is still detail; keep it as a string.
            data: e
                .data_json
                .map(|d| serde_json::from_str(&d).unwrap_or(Value::String(d))),
        }
    }
}
