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
//!
//! The first start is the carrier's caller's: whatever ends it before the process is ready is the
//! answer to that start, and nothing is tried again.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use harnix_ext::{codes, ExtId, Rpc, RpcError};
use harnix_host_api::{Availability, ChildEvent, ChildHandle, ExtensionHandle, Installed};
use harnix_wasm_port::rpc::{ControlClient, Wire};
use harnix_wasm_port::socket::{now_ms, CallMeta, Options, SocketWire};
use harnix_wasm_port::{Control, Place, Registration, Started, Stop};
use serde_json::{json, Value};
use tokio::sync::{mpsc, oneshot};

use crate::kernel::{start_of, Connection, Parts};
use crate::registry::Loaded;
use crate::sync::lock;
use crate::{unavailable, Inner, CRASHES, CRASH_WINDOW, READY_WITHIN};

/// How often a ready process is asked how it is.
pub const PING_EVERY: Duration = Duration::from_secs(15);

/// How long it has to answer.
pub const PING_WITHIN: Duration = Duration::from_secs(5);

/// Answers missed in a row after which it is killed.
pub const PING_MISSES: u32 = 3;

/// How long a stopping process has to stop its package and answer `ctl.shutdown`.
const STOP_WITHIN: Duration = Duration::from_secs(5);

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
}

pub(crate) enum Command {
    /// End the run: its package stopped in the process, the process asked to go, then ended.
    Stop(oneshot::Sender<()>),
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
    mut child: ChildHandle,
    mut commands: mpsc::UnboundedReceiver<Command>,
    ready: oneshot::Sender<Rpc<Started>>,
) {
    let mut ready = Some(ready);
    let mut up: Option<Up> = None;
    let mut ping = tokio::time::interval_at(tokio::time::Instant::now() + PING_EVERY, PING_EVERY);
    let id = package.id.to_string();
    loop {
        tokio::select! {
            command = commands.recv() => {
                if let Some(up) = up.take() {
                    shut(&package, up).await;
                }
                child.stop().await;
                gone(&me, &package);
                if let Some(ready) = ready.take() {
                    let _ = ready.send(Err(unavailable("stopped", format!("`{id}` was stopped while it started"))));
                }
                if let Some(Command::Stop(done)) = command {
                    let _ = done.send(());
                }
                return;
            }
            why = violated(&mut up) => {
                // Past its consent after it was ready: the same as before.
                if let Some(up) = up.take() {
                    up.connection.end().await;
                    up.wire.close("it registered what was not consented to").await;
                }
                fail(&me, &package, &why, &mut ready, "failed");
                child.stop().await;
                return;
            }
            _ = ping.tick(), if up.is_some() => {
                let Some(now) = up.as_mut() else { continue };
                match asked(&now.control, &id).await {
                    Ok(_) => now.misses = 0,
                    Err(e) => {
                        now.misses += 1;
                        tracing::warn!(package = %id, misses = now.misses, error = %e.message, "its extension process did not answer");
                        if now.misses >= PING_MISSES {
                            child.kill();
                        }
                    }
                }
            }
            event = child.next() => {
                let Some(event) = event else {
                    fail(&me, &package, "its supervisor finished", &mut ready, "faulted");
                    return;
                };
                match event {
                    ChildEvent::Started { pid, wire, .. } => {
                        package.set(|p| {
                            p.state = "starting";
                            p.pid = Some(pid);
                            p.starts += 1;
                            p.restarting_in_ms = None;
                        });
                        me.moved();
                        match connect(&me, &package, wire).await {
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
                                up = Some(now);
                            }
                            Err(NotUp::Refused(why)) => {
                                fail(&me, &package, &why, &mut ready, "failed");
                                child.stop().await;
                                return;
                            }
                            Err(NotUp::Crashed(why)) => {
                                tracing::warn!(package = %id, pid, %why, "its extension process did not come up");
                                package.set(|p| p.reason = Some(why.clone()));
                                if ready.is_some() {
                                    fail(&me, &package, &why, &mut ready, "failed");
                                    child.stop().await;
                                    return;
                                }
                                child.kill();
                            }
                        }
                    }
                    ChildEvent::Exited { code, ran_for } => {
                        if let Some(up) = up.take() {
                            up.connection.end().await;
                            package.set(|p| p.withdraws += up.connection.told.withdraws.load(std::sync::atomic::Ordering::Relaxed));
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
                        if me.crashed(&id, ran_for) {
                            let why = format!(
                                "its process exited {CRASHES} times in a row within {}s of starting (last: {})",
                                CRASH_WINDOW.as_secs(),
                                code.map_or("killed by a signal".to_string(), |c| format!("exit {c}")),
                            );
                            fail(&me, &package, &why, &mut ready, "faulted");
                            child.stop().await;
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
                        fail(&me, &package, &format!("given up on: {reason}"), &mut ready, "faulted");
                        return;
                    }
                    ChildEvent::Stopped => return,
                    _ => {}
                }
            }
        }
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

/// The product's end of a start's wire, taken to ready: its hello, its registration, its start.
async fn connect(me: &Arc<Inner>, package: &Arc<Package>, stream: std::os::unix::net::UnixStream) -> Result<(Up, Started), NotUp> {
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

    // Put in place before it first starts: into its own engine's instances, none of the product's.
    if package.phase().placed.is_none() && package.parts.get("agent").is_none() {
        let app_dir = package.installed.pkg_dir().parent().map(std::path::Path::to_path_buf).unwrap_or_default();
        let place = Place { app: id.clone(), app_dir, version: package.installed.version().to_string(), instances: Some(Vec::new()) };
        match control.place(CallMeta::default().with_deadline(Some(deadline_ms)), place).await {
            Ok(placement) => package.set(|p| p.placed = Some(placement.parts)),
            Err(e) => return Err(failed(&wire, NotUp::Refused(format!("it was not put in place: {}", e.message))).await),
        }
    }

    // Its start: its `proc.register` and its backend's `init` come before the answer.
    let mut violation = connection.violation();
    let start = start_of(&package.installed, package.placed());
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
