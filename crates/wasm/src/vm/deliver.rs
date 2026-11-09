//! Topic values and timer firings for the resident backend: queued, bounded, and drained in order
//! by one worker task per VM.

use super::{holding, with_deadline, Inner, Shared};
use crate::sync::lock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Weak;
use tokio::sync::mpsc;

/// Topic values and timer firings waiting for the backend. Past this the
/// backend is not keeping up, and dropping (and counting) beats growing
/// without bound.
pub(super) const DELIVERY_QUEUE: usize = 256;

pub(super) enum Delivery {
    Topic { topic: String, value_json: String },
    Timer(String),
}

/// The backend's delivery worker: one task per VM, so topic values and
/// timers reach the backend in the order they came. Holds the VM weakly; it
/// ends when the VM is dropped and the queue closes.
pub(super) async fn drain(vm: Weak<Inner>, mut queue: mpsc::Receiver<Delivery>) {
    while let Some(delivery) = queue.recv().await {
        let Some(inner) = vm.upgrade() else { break };
        inner.deliver(delivery).await;
    }
}

impl Inner {
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
    pub(super) fn enqueue(&self, delivery: Delivery) {
        self.pending.fetch_add(1, Ordering::AcqRel);
        if let Err(e) = self.deliveries.try_send(delivery) {
            self.pending.fetch_sub(1, Ordering::AcqRel);
            if let mpsc::error::TrySendError::Full(_) = e {
                lock(&self.counters).dropped_deliveries += 1;
                tracing::warn!(app = %self.app, "backend delivery queue full; dropped");
            }
        }
    }
}
