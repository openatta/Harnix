//! The resident backend instance: started, woken from hibernation, judged after every call into
//! it, and replaced when it can no longer be trusted.

use super::{holding, with_deadline, Abort, CallFailure, Inner};
use crate::sync::lock;
use crate::bindings::exports::harnix::app::backend;
use crate::state::{AppState, Role};
use std::sync::atomic::Ordering;
use std::time::Duration;
use wasmtime::Store;

pub(super) struct Resident {
    pub(super) store: Store<AppState>,
    pub(super) guest: backend::Guest,
}

/// Why the resident instance is being replaced, for the counter it goes on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Replaced {
    /// A call was abandoned at its deadline.
    Recycle,
    /// It crashed.
    Rebuild,
    /// Its files crossed the storage quota (`storage.rs`).
    Storage,
}

impl Inner {
    /// A fresh resident instance with `init` run on it.
    pub(super) async fn instantiate_resident(&self) -> Result<Resident, CallFailure> {
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
    pub(super) async fn wake(
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

    /// Judge a call into the resident instance, replacing the instance when
    /// it can no longer be trusted — or when its files crossed the storage
    /// quota, so that the instance that takes over has `/data` as the quota
    /// now says.
    pub(super) async fn settle_resident<T>(
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
}
