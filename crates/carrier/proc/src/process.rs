//! Keeping one package's extension process, for one run of it: from each start to ready — a hello
//! of this build, a registration within what was consented to, its start answered — then asked
//! how it is, and after each exit counted, until it is stopped, refused or set aside.
//!
//! | what | then |
//! |---|---|
//! | not ready within [`READY_WITHIN`] | killed, and counted as a crash |
//! | three answers to [`PING_EVERY`]'s question missed | killed, and counted as a crash |
//! | an exit within [`CRASH_WINDOW`] of starting, [`CRASHES`] in a row | set aside: `faulted` |
//! | another build's hello, a registration past its consent | ended, `failed`, not tried again |
//! | its start refused by the process | ended, `failed`, not tried again |
//! | its supervisor gave up (an exit 78: it could not be set up) | `faulted` |
//! | its own engine kept flooding the hub | killed, and counted as a crash however long it ran |
//! | on demand, idle: nothing in flight, its backend let go, its engine unused | let go: `hibernating` |
//!
//! The first start is the carrier's caller's: whatever ends it before the process is ready is the
//! answer to that start, and nothing is tried again.
//!
//! With an engine of its own, each start hands over its engine's stream as well: the run is ready
//! only once the adopted instance (`proc.<id>`) has taken it, and an exit leaves the instance
//! degraded until the next run. A process let go while idle is started again, with a new
//! supervisor, by the first thing that needs it — a method, a tool, a call on its engine — which
//! waits for it to be ready ([`Command::Wake`]); reading its lists wakes nothing.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use harnix_ext::{codes, ExtId, Rpc, RpcError};
use harnix_host_api::{
    AdoptedEngine, Availability, ChildEvent, ChildHandle, ChildRequest, EngineNote, ExtensionHandle, HostServices, Installed,
    Waker,
};
use harnix_wasm_port::rpc::{ControlClient, Wire};
use harnix_wasm_port::socket::{now_ms, CallMeta, Options, SocketWire};
use harnix_wasm_port::{Control, Place, Registration, Started, Stop, TurnChain, TurnRef};
use serde_json::{json, Value};
use tokio::sync::{mpsc, oneshot};

use crate::kernel::{start_of, Connection, Parts};
use crate::registry::Registry;
use harnix_carrier_shared::Loaded;
use harnix_carrier_shared::sync::lock;
use crate::{unavailable, Inner, CRASHES, CRASH_WINDOW, READY_WITHIN};

/// How often a ready process is asked how it is.
pub const PING_EVERY: Duration = Duration::from_secs(15);

/// How long it has to answer.
pub const PING_WITHIN: Duration = Duration::from_secs(5);

/// Answers missed in a row after which it is killed.
pub const PING_MISSES: u32 = 3;

/// How long a stopping process has to stop its package and answer `ctl.shutdown`.
const STOP_WITHIN: Duration = Duration::from_secs(5);

/// How long a turn started on a sleeping engine waits for its process, to be told of it.
const TELL_WITHIN: Duration = Duration::from_secs(60);

/// One package started on this carrier, whether its process is up or not.
pub(crate) struct Package {
    pub id: ExtId,
    pub installed: Installed,
    pub handle: ExtensionHandle,
    pub loaded: Arc<Loaded>,
    /// What its process must register within.
    pub expected: Registration,
    /// Its record's `parts`, and once it was placed, what placing it put there.
    pub parts: Value,
    pub net: Vec<String>,
    pub commands: mpsc::UnboundedSender<Command>,
    pub phase: Mutex<Phase>,
    /// What its process is started with, each time.
    pub request: ChildRequest,
    /// Its own engine, adopted, when it has `agent/` or a `[[space]]`.
    pub engine: Option<Arc<dyn AdoptedEngine>>,
    /// What its process holds it to ([`harnix_wasm_port::Start::limits`]).
    pub limits: harnix_wasm_port::Limits,
    /// How long it idles before its process is let go: an on-demand backend's, as long as its
    /// backend idles before it hibernates.
    pub idle: Option<Duration>,
}

pub(crate) enum Command {
    /// End the run: its package stopped in the process, the process asked to go, then ended.
    Stop(oneshot::Sender<()>),
    /// Be up: started again if it was let go while idle; answered once it is ready.
    Wake(oneshot::Sender<Rpc<()>>),
    /// Its engine kept flooding: end the process that runs now.
    Flooded,
}

/// What wakes a package's process let go while idle: its keeper, asked.
pub(crate) struct Wakes(pub mpsc::UnboundedSender<Command>);

#[async_trait::async_trait]
impl Waker for Wakes {
    async fn wake(&self) -> Rpc<()> {
        let (done, woken) = oneshot::channel();
        self.0
            .send(Command::Wake(done))
            .map_err(|_| unavailable("stopped", "its extension process is stopped"))?;
        woken.await.unwrap_or_else(|_| Err(unavailable("stopped", "its extension process is stopped")))
    }
}

/// Where its process is, as its status says.
#[derive(Debug, Clone)]
pub(crate) struct Phase {
    pub state: &'static str,
    pub reason: Option<String>,
    pub pid: Option<u32>,
    pub starts: u32,
    pub last_exit: Option<Value>,
    pub restarting_in_ms: Option<u64>,
    /// What placing it put where, once it was.
    pub placed: Option<Value>,
    /// `proc.withdraw`s it sent, ignored.
    pub withdraws: u64,
    /// The last lines its process wrote to its stderr, as of its last run that wrote any.
    pub stderr: Vec<String>,
}

impl Default for Phase {
    fn default() -> Self {
        Self {
            state: "starting",
            reason: None,
            pid: None,
            starts: 0,
            last_exit: None,
            restarting_in_ms: None,
            placed: None,
            withdraws: 0,
            stderr: Vec::new(),
        }
    }
}

impl Phase {
    pub fn describe(&self) -> Value {
        json!({
            "pid": self.pid,
            "starts": self.starts,
            "last_exit": self.last_exit,
            "restarting_in_ms": self.restarting_in_ms,
            "withdraws_ignored": self.withdraws,
            "stderr": self.stderr,
        })
    }
}

impl Package {
    pub fn phase(&self) -> Phase {
        lock(&self.phase).clone()
    }

    fn set(&self, f: impl FnOnce(&mut Phase)) {
        f(&mut lock(&self.phase));
    }

    /// Its process's last lines of stderr, as its run keeps them now; true when they changed. A run
    /// that wrote none yet leaves the last run's.
    fn tail(&self, child: &Option<ChildHandle>) -> bool {
        let Some(child) = child else { return false };
        let lines = child.stderr();
        let mut phase = lock(&self.phase);
        if lines.is_empty() || lines == phase.stderr {
            return false;
        }
        phase.stderr = lines;
        true
    }

    /// What its record's `parts` say from now on.
    pub fn placed(&self) -> Value {
        lock(&self.phase).placed.clone().unwrap_or_else(|| self.parts.clone())
    }

    /// End its run, and wait until it has ended.
    pub async fn stop(&self) {
        let (done, ended) = oneshot::channel();
        if self.commands.send(Command::Stop(done)).is_ok() {
            let _ = ended.await;
        }
    }
}

/// Its process's own status, asked within [`PING_WITHIN`].
pub(crate) async fn asked(control: &Arc<dyn Control>, id: &str) -> Rpc<Value> {
    let meta = CallMeta::default().with_deadline(Some(now_ms() + PING_WITHIN.as_millis() as u64));
    control.status(meta, id.to_string()).await
}

/// A process that is up.
struct Up {
    wire: Arc<SocketWire>,
    connection: Arc<Connection>,
    control: Arc<dyn Control>,
    violation: tokio::sync::watch::Receiver<Option<String>>,
    misses: u32,
}

/// Why a start of its process did not come up.
enum NotUp {
    /// The product found it is not what was consented to, or the process refused its start:
    /// ended, and not tried again.
    Refused(String),
    /// It did not come up in time, or went: killed (if it is still there), counted as a crash.
    Crashed(String),
}

/// Keep `package`'s process `child` until it is stopped, refused or set aside. `ready` is told
/// how its first start went.
pub(crate) async fn keep(
    me: Arc<Inner>,
    package: Arc<Package>,
    child: ChildHandle,
    mut commands: mpsc::UnboundedReceiver<Command>,
    ready: oneshot::Sender<Rpc<Started>>,
) {
    let mut ready = Some(ready);
    // `None` while its process is let go while idle.
    let mut child = Some(child);
    let mut up: Option<Up> = None;
    // Who asked it to be up, until it is.
    let mut waiting: Vec<oneshot::Sender<Rpc<()>>> = Vec::new();
    // The run that is up was ended for flooding: its exit is a crash.
    let mut flooded = false;
    let mut ping = tokio::time::interval_at(tokio::time::Instant::now() + PING_EVERY, PING_EVERY);
    let mut idle = package.idle.map(|idle| {
        // Often enough that it goes within a quarter of `idle` of being due.
        let every = (idle / 4).clamp(Duration::from_millis(50), Duration::from_secs(15));
        tokio::time::interval_at(tokio::time::Instant::now() + every, every)
    });
    let id = package.id.to_string();
    loop {
        tokio::select! {
            command = commands.recv() => match command {
                Some(Command::Wake(done)) => {
                    if up.is_some() {
                        let _ = done.send(Ok(()));
                        continue;
                    }
                    waiting.push(done);
                    if child.is_none() {
                        match respawn(&me, &package) {
                            Ok(again) => {
                                tracing::info!(package = %id, "waking its extension process");
                                child = Some(again);
                                package.set(|p| p.state = "starting");
                                me.moved();
                            }
                            Err(e) => answer(&mut waiting, Err(e)),
                        }
                    }
                }
                Some(Command::Flooded) => {
                    if let (Some(_), Some(child)) = (&up, &child) {
                        tracing::warn!(package = %id, "its engine keeps flooding; ending its extension process");
                        flooded = true;
                        child.kill();
                    }
                }
                stop => {
                    if let Some(up) = up.take() {
                        shut(&package, up).await;
                    }
                    if let Some(child) = child.take() {
                        child.stop().await;
                    }
                    gone(&me, &package);
                    answer(&mut waiting, Err(unavailable("stopped", format!("`{id}` was stopped"))));
                    if let Some(ready) = ready.take() {
                        let _ = ready.send(Err(unavailable("stopped", format!("`{id}` was stopped while it started"))));
                    }
                    if let Some(Command::Stop(done)) = stop {
                        let _ = done.send(());
                    }
                    return;
                }
            },
            why = violated(&mut up) => {
                package.tail(&child);
                // Past its consent after it was ready: the same as before.
                if let Some(up) = up.take() {
                    up.connection.end().await;
                    up.wire.close("it registered what was not consented to").await;
                }
                fail(&me, &package, &why, &mut ready, "failed");
                answer(&mut waiting, Err(unavailable("failed", why)));
                if let Some(child) = child.take() {
                    child.stop().await;
                }
                return;
            }
            _ = ping.tick(), if up.is_some() => {
                if package.tail(&child) {
                    me.moved();
                }
                let Some(now) = up.as_mut() else { continue };
                match asked(&now.control, &id).await {
                    Ok(_) => now.misses = 0,
                    Err(e) => {
                        now.misses += 1;
                        tracing::warn!(package = %id, misses = now.misses, error = %e.message, "its extension process did not answer");
                        if now.misses >= PING_MISSES {
                            if let Some(child) = &child {
                                child.kill();
                            }
                        }
                    }
                }
            }
            _ = tick(&mut idle), if up.is_some() => {
                let (Some(now), Some(idle)) = (up.as_ref(), package.idle) else { continue };
                if !quiet(&me, &package, now, idle).await {
                    continue;
                }
                // Its sessions as they list now are kept to answer with; its process goes.
                tracing::info!(package = %id, "its extension process is idle; letting it go");
                if let Some(engine) = &package.engine {
                    engine.hibernate().await;
                }
                if let Some(now) = up.take() {
                    shut(&package, now).await;
                }
                if let Some(child) = child.take() {
                    child.stop().await;
                }
                me.registry.sleep(&id, Arc::new(Wakes(package.commands.clone())));
                package.set(|p| {
                    p.state = "hibernating";
                    p.pid = None;
                });
                me.moved();
            }
            event = next(&mut child) => {
                // Read at each event: an exit's last lines are in by the time it is told.
                package.tail(&child);
                let Some(event) = event else {
                    fail(&me, &package, "its supervisor finished", &mut ready, "faulted");
                    answer(&mut waiting, Err(unavailable("faulted", "its supervisor finished")));
                    return;
                };
                match event {
                    ChildEvent::Started { pid, wire, engine } => {
                        package.set(|p| {
                            p.state = "starting";
                            p.pid = Some(pid);
                            p.starts += 1;
                            p.restarting_in_ms = None;
                        });
                        me.moved();
                        match connect(&me, &package, wire, engine).await {
                            Ok((now, started)) => {
                                me.registry.insert(package.loaded.clone(), now.control.clone());
                                package.handle.mark(Availability::Ready);
                                package.set(|p| {
                                    p.state = "running";
                                    p.reason = None;
                                });
                                tracing::info!(package = %id, pid, "its extension process is ready");
                                if let Some(ready) = ready.take() {
                                    let _ = ready.send(Ok(started));
                                } else {
                                    me.refresh();
                                }
                                answer(&mut waiting, Ok(()));
                                up = Some(now);
                            }
                            Err(NotUp::Refused(why)) => {
                                package.tail(&child);
                                fail(&me, &package, &why, &mut ready, "failed");
                                answer(&mut waiting, Err(unavailable("failed", why)));
                                if let Some(child) = child.take() {
                                    child.stop().await;
                                }
                                return;
                            }
                            Err(NotUp::Crashed(why)) => {
                                tracing::warn!(package = %id, pid, %why, "its extension process did not come up");
                                package.set(|p| p.reason = Some(why.clone()));
                                if ready.is_some() {
                                    package.tail(&child);
                                    fail(&me, &package, &why, &mut ready, "failed");
                                    answer(&mut waiting, Err(unavailable("failed", why)));
                                    if let Some(child) = child.take() {
                                        child.stop().await;
                                    }
                                    return;
                                }
                                if let Some(child) = &child {
                                    child.kill();
                                }
                            }
                        }
                    }
                    ChildEvent::Exited { code, ran_for } => {
                        if let Some(up) = up.take() {
                            up.connection.end().await;
                            package.set(|p| p.withdraws += up.connection.told.withdraws.load(std::sync::atomic::Ordering::Relaxed));
                        }
                        if let Some(engine) = &package.engine {
                            engine.detach("its extension process exited").await;
                        }
                        let was_up = me.registry.remove(&id).is_some();
                        package.handle.mark(Availability::Starting);
                        package.set(|p| {
                            p.state = "starting";
                            p.pid = None;
                            p.last_exit = Some(json!({"code": code, "ran_ms": ran_for.as_millis() as u64}));
                        });
                        tracing::warn!(package = %id, ?code, ran_ms = ran_for.as_millis() as u64, "its extension process exited");
                        if was_up {
                            me.refresh();
                        }
                        // Ended for flooding, it crashed however long it ran.
                        let ran_for = if std::mem::take(&mut flooded) { Duration::ZERO } else { ran_for };
                        if me.crashed(&id, ran_for) {
                            let why = format!(
                                "its process exited {CRASHES} times in a row within {}s of starting (last: {})",
                                CRASH_WINDOW.as_secs(),
                                code.map_or("killed by a signal".to_string(), |c| format!("exit {c}")),
                            );
                            fail(&me, &package, &why, &mut ready, "faulted");
                            answer(&mut waiting, Err(unavailable("faulted", why)));
                            if let Some(child) = child.take() {
                                child.stop().await;
                            }
                            return;
                        }
                        me.moved();
                    }
                    ChildEvent::Restarting { after } => {
                        package.set(|p| p.restarting_in_ms = Some(after.as_millis() as u64));
                        me.moved();
                    }
                    ChildEvent::SpawnFailed { error } => {
                        tracing::warn!(package = %id, %error, "its extension process was not started");
                        package.set(|p| p.reason = Some(error));
                    }
                    ChildEvent::GaveUp { reason } => {
                        let why = format!("given up on: {reason}");
                        fail(&me, &package, &why, &mut ready, "faulted");
                        answer(&mut waiting, Err(unavailable("faulted", why)));
                        return;
                    }
                    ChildEvent::Stopped => return,
                    _ => {}
                }
            }
        }
    }
}

/// What happened to the run next; never, while its process is let go.
async fn next(child: &mut Option<ChildHandle>) -> Option<ChildEvent> {
    match child.as_mut() {
        Some(child) => child.next().await,
        None => std::future::pending().await,
    }
}

/// The idle check's next tick; never, for a package that is kept up.
async fn tick(idle: &mut Option<tokio::time::Interval>) {
    match idle.as_mut() {
        Some(idle) => {
            idle.tick().await;
        }
        None => std::future::pending().await,
    }
}

/// Tell everyone waiting for it to be up how that went.
fn answer(waiting: &mut Vec<oneshot::Sender<Rpc<()>>>, how: Rpc<()>) {
    for done in waiting.drain(..) {
        let _ = done.send(how.clone());
    }
}

/// A new run of a process let go while idle: a new supervisor, as its first was.
fn respawn(me: &Inner, package: &Package) -> Rpc<ChildHandle> {
    let cx = me.cx().ok_or_else(|| unavailable("stopped", "the packages' component is not bound"))?;
    cx.service::<HostServices>()?.spawn_child(&package.id, package.request.clone())
}

/// Whether its process can be let go: nothing of the product's in flight into it for `idle`, its
/// backend let go by its own host, and nobody using its engine.
async fn quiet(me: &Inner, package: &Package, up: &Up, idle: Duration) -> bool {
    let id = package.id.as_str();
    if !me.registry.idle_for(id).is_some_and(|quiet| quiet >= idle) {
        return false;
    }
    let hibernated = asked(&up.control, id).await.is_ok_and(|status| status["state"] == "hibernating");
    if !hibernated {
        return false;
    }
    match &package.engine {
        Some(engine) => !engine.engaged().await,
        None => true,
    }
}

/// What its adopted engine tells: each turn the hub starts and settles on it, to its process; that
/// it floods, to its keeper. Until it is let go.
pub(crate) async fn tell(
    registry: Arc<Registry>,
    id: String,
    instance: String,
    mut notes: mpsc::UnboundedReceiver<EngineNote>,
    commands: mpsc::UnboundedSender<Command>,
) {
    while let Some(note) = notes.recv().await {
        match note {
            EngineNote::TurnStarted { session_id, chain } => {
                // A turn on a sleeping engine wakes its process: it is told once it is up.
                if let Some(control) = reached(&registry, &id).await {
                    control.turn_started(TurnChain { instance: instance.clone(), session_id, chain });
                }
            }
            EngineNote::TurnSettled { session_id } => {
                if let Some(control) = registry.control(&id) {
                    control.turn_settled(TurnRef { instance: instance.clone(), session_id });
                }
            }
            EngineNote::Flooded => {
                let _ = commands.send(Command::Flooded);
            }
            _ => {}
        }
    }
}

/// The process that runs `id` once it is up, waited for within [`TELL_WITHIN`] while it starts or
/// wakes; `None` for one that is not coming.
async fn reached(registry: &Registry, id: &str) -> Option<Arc<dyn Control>> {
    let deadline = Instant::now() + TELL_WITHIN;
    loop {
        if let Some(control) = registry.control(id) {
            return Some(control);
        }
        if Instant::now() >= deadline {
            return None;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Why a process that is up registered past its consent, once it did; never without one up.
async fn violated(up: &mut Option<Up>) -> String {
    match up.as_mut() {
        Some(up) => up.violation.wait_for(Option::is_some).await.ok().and_then(|w| w.clone()).unwrap_or_default(),
        None => std::future::pending().await,
    }
}

/// It is not running: out of what is running, and the page told.
fn gone(me: &Inner, package: &Package) {
    if me.registry.remove(package.id.as_str()).is_some() {
        me.refresh();
    }
    package.set(|p| {
        p.state = "stopped";
        p.pid = None;
    });
    me.moved();
}

/// The run is over for `why`: its rows stay and answer that it is set aside (`faulted`) or failed;
/// a first start is answered with it.
fn fail(me: &Inner, package: &Package, why: &str, ready: &mut Option<oneshot::Sender<Rpc<Started>>>, state: &'static str) {
    tracing::error!(package = %package.id, %why, state, "its extension process is set aside");
    if me.registry.remove(package.id.as_str()).is_some() {
        me.refresh();
    }
    if let Some(engine) = package.engine.clone() {
        let why = why.to_string();
        tokio::spawn(async move { engine.detach(&why).await });
    }
    package.handle.mark(Availability::Faulted(why.to_string()));
    package.set(|p| {
        p.state = state;
        p.reason = Some(why.to_string());
        p.pid = None;
    });
    if let Some(ready) = ready.take() {
        let mut e = RpcError::new(codes::REFUSED, format!("`{}`: {why}", package.id));
        e.data = Some(json!({"failure": state}));
        let _ = ready.send(Err(e));
    }
    me.moved();
}

/// The product's end of a start's wire, taken to ready: its hello, its engine's stream taken by the
/// adopted instance, its registration, its start.
async fn connect(
    me: &Arc<Inner>,
    package: &Arc<Package>,
    stream: std::os::unix::net::UnixStream,
    engine: Option<std::os::unix::net::UnixStream>,
) -> Result<(Up, Started), NotUp> {
    let id = package.id.to_string();
    let crashed = |why: String| NotUp::Crashed(why);
    stream.set_nonblocking(true).map_err(|e| crashed(format!("its wire: {e}")))?;
    let stream = tokio::net::UnixStream::from_std(stream).map_err(|e| crashed(format!("its wire: {e}")))?;
    let options = Options { deadline_cap: package.loaded.timeout, ..Options::default() };
    let parts = Parts {
        app: id.clone(),
        handle: package.handle.clone(),
        expected: package.expected.clone(),
        net: package.net.clone(),
        egress: me.egress.clone(),
        registry: me.registry.clone(),
        engine: package.engine.clone(),
    };
    let (wire, connection) = Connection::open(stream, parts, me.ours.clone(), options);
    let deadline = Instant::now() + READY_WITHIN;
    let deadline_ms = now_ms() + READY_WITHIN.as_millis() as u64;
    let failed = |up: &Arc<SocketWire>, why: NotUp| {
        let up = up.clone();
        async move {
            up.close("it did not come up").await;
            why
        }
    };

    // Its hello first: of this build, or refused.
    let told = connection.told.clone();
    let hello = async {
        loop {
            let seen = told.hello_seen.notified();
            if let Some(hello) = lock(&told.hello).clone() {
                return hello;
            }
            seen.await;
        }
    };
    let hello = tokio::select! {
        hello = hello => hello,
        why = wire.closed() => return Err(failed(&wire, crashed(format!("its wire closed before it said hello: {why}"))).await),
        _ = tokio::time::sleep_until(deadline.into()) => {
            return Err(failed(&wire, crashed(format!("no hello within {}s", READY_WITHIN.as_secs()))).await)
        }
    };
    if let Err(why) = hello {
        return Err(failed(&wire, NotUp::Refused(format!("its hello was refused: {why}"))).await);
    }
    let control: Arc<dyn Control> = Arc::new(ControlClient(wire.clone() as Arc<dyn Wire>));

    // Its own engine, served on the stream it was given: up before anything is asked of it.
    if let Some(adopted) = &package.engine {
        let Some(engine) = engine else {
            return Err(failed(&wire, crashed("it was started without its engine's stream".into())).await);
        };
        match tokio::time::timeout_at(deadline.into(), adopted.attach(engine)).await {
            Ok(Ok(())) => {}
            Ok(Err(e)) => return Err(failed(&wire, crashed(format!("its engine: {}", e.message))).await),
            Err(_) => return Err(failed(&wire, crashed(format!("its engine did not answer within {}s", READY_WITHIN.as_secs()))).await),
        }
    }

    // Put in place before it first starts: into its own engine, none of the product's instances.
    if package.phase().placed.is_none() && package.parts.get("agent").is_none() {
        let app_dir = package.installed.pkg_dir().parent().map(std::path::Path::to_path_buf).unwrap_or_default();
        let instances = package.engine.iter().map(|e| e.instance().to_string()).collect();
        let place = Place { app: id.clone(), app_dir, version: package.installed.version().to_string(), instances: Some(instances) };
        match control.place(CallMeta::default().with_deadline(Some(deadline_ms)), place).await {
            Ok(placement) => package.set(|p| p.placed = Some(placement.parts)),
            Err(e) => return Err(failed(&wire, NotUp::Refused(format!("it was not put in place: {}", e.message))).await),
        }
    }

    // Its start: its `proc.register` and its backend's `init` come before the answer.
    let mut violation = connection.violation();
    let start = start_of(&package.installed, package.placed(), package.limits.clone());
    let meta = CallMeta::default().with_deadline(Some(deadline_ms));
    let started = tokio::select! {
        started = control.start(meta, start) => started,
        why = async { violation.wait_for(Option::is_some).await.ok().and_then(|w| w.clone()) } => {
            return Err(failed(&wire, NotUp::Refused(why.unwrap_or_default())).await);
        }
    };
    let violated = violation.borrow().clone();
    if let Some(why) = violated {
        return Err(failed(&wire, NotUp::Refused(why)).await);
    }
    let started = match started {
        Ok(started) => started,
        Err(e) if e.code == codes::DEADLINE_EXCEEDED || e.code == codes::EXTENSION_UNAVAILABLE => {
            return Err(failed(&wire, crashed(format!("its start: {}", e.message))).await)
        }
        Err(e) => return Err(failed(&wire, NotUp::Refused(format!("it did not start: {}", e.message))).await),
    };
    if !connection.told.registered.load(std::sync::atomic::Ordering::Acquire) {
        return Err(failed(&wire, NotUp::Refused("it started without registering".into())).await);
    }
    Ok((Up { wire, connection, control, violation, misses: 0 }, started))
}

/// Stop it in its process and ask the process to go, each within [`STOP_WITHIN`]; the wire closes
/// after.
async fn shut(package: &Package, up: Up) {
    let id = package.id.to_string();
    let within = || CallMeta::default().with_deadline(Some(now_ms() + STOP_WITHIN.as_millis() as u64));
    if let Err(e) = up.control.stop(within(), Stop { app: id.clone(), parts: package.placed() }).await {
        tracing::warn!(package = %id, error = %e.message, "the package did not stop cleanly in its process");
    }
    let _ = up.control.shutdown(within(), "the product stops it".into()).await;
    up.connection.end().await;
    package.set(|p| p.withdraws += up.connection.told.withdraws.load(std::sync::atomic::Ordering::Relaxed));
    up.wire.close("stopped").await;
}
