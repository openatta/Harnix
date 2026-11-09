//! What a resident instance holds on the host — its subscriptions and timers — and undoing it
//! when that instance ends.

use super::{deliver::Delivery, lock, Shared};
use serde_json::Value;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

/// Timers one backend instance may have armed at once.
pub const MAX_TIMERS: usize = 64;
/// Hub subscriptions one backend instance may hold at once.
pub const MAX_SUBSCRIPTIONS: usize = 64;
/// Longer than this is a timer that will not survive a restart anyway.
const MAX_TIMER_DELAY: Duration = Duration::from_secs(30 * 24 * 3600);

impl Shared {
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

    pub(super) fn fire(&self, id: &str, generation: u64) {
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
    pub(super) async fn release_bindings(&self) {
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
