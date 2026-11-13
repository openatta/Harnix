//! Keeping one child running: start it, watch it, start it again by the policy.

use std::io;
use std::process::ExitStatus;
use std::time::{Duration, Instant};

use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;

use crate::child::ChildSpec;
use crate::policy::{Decision, RestartPolicy};

/// How long a child gets to finish on its own once its stdin is closed.
const STOP_GRACE: Duration = Duration::from_secs(5);

/// What happened to the child, in order.
#[derive(Debug, Clone)]
pub enum ChildEvent {
    Started { pid: u32 },
    /// The spec could not be made or the program could not be started.
    SpawnFailed { error: String },
    Exited { status: ExitStatus, ran_for: Duration },
    /// It will be started again after the wait.
    Restarting { after: Duration },
    /// It is not started again.
    GaveUp { reason: String },
    /// [`Supervisor::stop`] ended it.
    Stopped,
}

/// Keeps one child for one run of whatever it serves: an exit nobody asked for is started again
/// by the policy; once [`Supervisor::stop`] is called, nothing is. What outlives a run — a record
/// of crashes across sleeping and waking, say — is the owner's to keep.
pub struct Supervisor {
    events: mpsc::UnboundedReceiver<ChildEvent>,
    stop: watch::Sender<bool>,
    kill: watch::Sender<u64>,
    task: JoinHandle<()>,
}

impl Supervisor {
    /// Start keeping a child. `make` is asked for a spec every time one is to be started: a spec
    /// holds descriptors, which a start uses up.
    pub fn start(mut make: impl FnMut() -> io::Result<ChildSpec> + Send + 'static, policy: RestartPolicy) -> Self {
        let (tx, events) = mpsc::unbounded_channel();
        let (stop, stopped) = watch::channel(false);
        let (kill, mut killed) = watch::channel(0u64);
        let task = tokio::spawn(async move {
            let mut restarts = policy.restarts();
            let mut stopped = stopped;
            loop {
                let started = Instant::now();
                let spawned = make().and_then(ChildSpec::spawn);
                let mut child = match spawned {
                    Ok(child) => child,
                    Err(e) => {
                        let _ = tx.send(ChildEvent::SpawnFailed { error: e.to_string() });
                        if !retry(&mut restarts, None, Duration::ZERO, &tx, &mut stopped).await {
                            return;
                        }
                        continue;
                    }
                };
                let _ = tx.send(ChildEvent::Started { pid: child.id().unwrap_or(0) });
                // A kill asked for before this child started is not this child's.
                killed.borrow_and_update();
                let status = tokio::select! {
                    biased;
                    _ = stop_asked(&mut stopped) => {
                        child.stop(STOP_GRACE).await;
                        let _ = tx.send(ChildEvent::Stopped);
                        return;
                    }
                    _ = killed.changed() => {
                        let _ = child.kill().await;
                        child.wait().await
                    }
                    status = child.wait() => status,
                };
                child.reap().await;
                let ran_for = started.elapsed();
                let code = match status {
                    Ok(status) => {
                        let _ = tx.send(ChildEvent::Exited { status, ran_for });
                        status.code()
                    }
                    Err(e) => {
                        let _ = tx.send(ChildEvent::SpawnFailed { error: e.to_string() });
                        None
                    }
                };
                if !retry(&mut restarts, code, ran_for, &tx, &mut stopped).await {
                    return;
                }
            }
        });
        Self { events, stop, kill, task }
    }

    /// The next event; `None` once the supervisor has finished (gave up, or was stopped).
    pub async fn next_event(&mut self) -> Option<ChildEvent> {
        self.events.recv().await
    }

    /// Kill the child that runs now, outright: it counts as an exit like any other, and is
    /// started again by the policy. For one that is up but does not answer.
    pub fn kill(&self) {
        self.kill.send_modify(|n| *n = n.wrapping_add(1));
    }

    /// End the child — stdin closed, a grace, TERM, a shorter grace, KILL, its session reaped —
    /// and stop starting it.
    pub async fn stop(mut self) {
        let _ = self.stop.send(true);
        let _ = (&mut self.task).await;
    }
}

impl Drop for Supervisor {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// Decide, announce, wait. False when the supervisor is done: it gave up or was stopped.
async fn retry(
    restarts: &mut crate::policy::Restarts,
    code: Option<i32>,
    ran_for: Duration,
    tx: &mpsc::UnboundedSender<ChildEvent>,
    stopped: &mut watch::Receiver<bool>,
) -> bool {
    match restarts.exited_with(code, ran_for) {
        Decision::GiveUp { reason } => {
            tracing::error!(%reason, "child given up on");
            let _ = tx.send(ChildEvent::GaveUp { reason });
            false
        }
        Decision::Restart { after } => {
            let _ = tx.send(ChildEvent::Restarting { after });
            tokio::select! {
                _ = tokio::time::sleep(after) => true,
                _ = stop_asked(stopped) => {
                    let _ = tx.send(ChildEvent::Stopped);
                    false
                }
            }
        }
    }
}

/// Resolves once [`Supervisor::stop`] was called (or the supervisor was dropped).
async fn stop_asked(stopped: &mut watch::Receiver<bool>) {
    while !*stopped.borrow_and_update() {
        if stopped.changed().await.is_err() {
            return;
        }
    }
}
