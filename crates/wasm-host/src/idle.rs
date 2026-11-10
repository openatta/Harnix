//! Idle hibernation: an on-demand backend (`[backend] resident = "on-demand"`) is let go once all
//! of these hold, and started again by the next call into it:
//!
//! - it holds no subscription and has no timer armed (the VM's own count of them);
//! - nothing is queued for it or being delivered to it (the VM checks that as it lets go);
//! - no method call, tool call or hook of the application is in flight ([`Activity`]);
//! - the last of them ended `idle_secs` ago.
//!
//! Letting it go runs its `stop`; starting it again runs `init`, so what it keeps in its storage
//! comes back and what it held in memory does not. Neither is a fault or a recycle. A tool call
//! runs in an instance of its own whether the backend is up or not, so it wakes nothing — unless it
//! calls the backend (`host.call`), which is a method call like any.

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use harnix_contract::sync::lock;
use harnix_wasm::{AppVm, VmState};
use tokio_util::sync::CancellationToken;

/// What one application has in flight, and when the last of it ended.
pub(crate) struct Activity {
    busy: AtomicUsize,
    last: Mutex<Instant>,
}

impl Default for Activity {
    fn default() -> Self {
        Self {
            busy: AtomicUsize::new(0),
            last: Mutex::new(Instant::now()),
        }
    }
}

/// One call in flight, for as long as it is held.
pub(crate) struct Busy<'a>(&'a Activity);

impl Drop for Busy<'_> {
    fn drop(&mut self) {
        *lock(&self.0.last) = Instant::now();
        self.0.busy.fetch_sub(1, Ordering::AcqRel);
    }
}

impl Activity {
    pub(crate) fn hold(&self) -> Busy<'_> {
        self.busy.fetch_add(1, Ordering::AcqRel);
        Busy(self)
    }

    /// Nothing in flight, and nothing for `idle`.
    fn idle_for(&self, idle: Duration) -> bool {
        self.busy.load(Ordering::Acquire) == 0 && lock(&self.last).elapsed() >= idle
    }
}

/// Watch an on-demand backend until `done`: let it go each time it has been idle for `idle`, and
/// move the host's `generation` when it does.
pub(crate) fn watch(
    vm: &AppVm,
    activity: Arc<Activity>,
    idle: Duration,
    done: CancellationToken,
    generation: Arc<AtomicU64>,
) {
    let vm = vm.downgrade();
    // Often enough that it goes within a quarter of `idle` of being due, and no oftener than a
    // test's short idle needs.
    let every = (idle / 4).clamp(Duration::from_millis(50), Duration::from_secs(15));
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = done.cancelled() => break,
                _ = tokio::time::sleep(every) => {}
            }
            let Some(vm) = vm.upgrade() else { break };
            if vm.hibernating() || !activity.idle_for(idle) {
                continue;
            }
            let health = vm.health();
            if health.state != VmState::Running || health.subscriptions > 0 || health.timers > 0 {
                continue;
            }
            // The VM checks again, under the backend's own lock, that nothing holds it.
            vm.hibernate().await;
            generation.fetch_add(1, Ordering::Relaxed);
        }
    });
}
