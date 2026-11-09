//! Who is watching, and how a frame reaches them.
//!
//! A client here is a `dyn FrameSink` and nothing more. Whether that is one
//! bidirectional connection, two download-only streams or a poll queue is the
//! transport's business — and the fact that this file cannot tell is what
//! makes §3.3.3's fifth invariant ("adding a topology does not touch the hub")
//! checkable rather than aspirational.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use harnix_contract::sync::lock;
use harnix_contract::{Channel, Frame, FrameSink};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ClientId(pub u64);

#[derive(Default)]
pub struct Clients {
    sinks: HashMap<ClientId, Arc<dyn FrameSink>>,
    /// Clients whose downstream answered `false`: gone, and sent nothing more
    /// until the transport removes them (`Hub::remove_client`), which it does
    /// once it notices the same thing. Behind a lock of its own because a
    /// frame is sent with the hub's state only borrowed.
    gone: Mutex<HashSet<ClientId>>,
}

impl Clients {
    pub fn insert(&mut self, id: ClientId, sink: Arc<dyn FrameSink>) {
        lock(&self.gone).remove(&id);
        self.sinks.insert(id, sink);
    }

    pub fn remove(&mut self, id: ClientId) {
        self.sinks.remove(&id);
        lock(&self.gone).remove(&id);
    }

    /// Send to one client, if it still wants this channel.
    ///
    /// `false` from a downstream means it is gone (`FrameSink::deliver`): it
    /// is not sent to again. Ignoring that kept a closed connection counted
    /// as a watcher, every frame of every session it had watched handed to a
    /// queue nobody drained.
    pub async fn send(&self, id: ClientId, frame: &Frame) {
        let Some(sink) = self.sinks.get(&id) else {
            return;
        };
        self.deliver(id, sink, frame).await;
    }

    /// Send to a named set — a session's watchers.
    pub async fn send_many(&self, ids: &[ClientId], frame: &Frame) {
        for id in ids {
            self.send(*id, frame).await;
        }
    }

    /// Send to everyone. Only for host events: a client watching the sidebar
    /// must not receive every session's body just to learn which one is
    /// running (§3.3.1).
    pub async fn broadcast(&self, frame: &Frame) {
        debug_assert_eq!(frame.channel, Channel::HostEvents);
        for (id, sink) in &self.sinks {
            self.deliver(*id, sink, frame).await;
        }
    }

    async fn deliver(&self, id: ClientId, sink: &Arc<dyn FrameSink>, frame: &Frame) {
        if !sink.wants(frame.channel) || lock(&self.gone).contains(&id) {
            return;
        }
        if !sink.deliver(frame.clone()).await {
            tracing::debug!(client = id.0, "a client's downstream is gone; nothing more is sent to it");
            lock(&self.gone).insert(id);
        }
    }

    /// Whether a client's downstream has said it is gone.
    #[cfg(test)]
    pub fn is_gone(&self, id: ClientId) -> bool {
        lock(&self.gone).contains(&id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Takes `open` frames, then answers `false` as a closed connection does.
    struct Closing {
        open: usize,
        seen: AtomicUsize,
    }

    #[async_trait::async_trait]
    impl FrameSink for Closing {
        async fn deliver(&self, _frame: Frame) -> bool {
            self.seen.fetch_add(1, Ordering::SeqCst) < self.open
        }
    }

    #[tokio::test]
    async fn a_downstream_that_is_gone_is_not_sent_to_again() {
        let mut clients = Clients::default();
        let gone = Arc::new(Closing { open: 1, seen: AtomicUsize::new(0) });
        let fine = Arc::new(Closing { open: usize::MAX, seen: AtomicUsize::new(0) });
        clients.insert(ClientId(1), gone.clone());
        clients.insert(ClientId(2), fine.clone());
        for _ in 0..5 {
            clients.broadcast(&Frame::host("x".into())).await;
            clients.send_many(&[ClientId(1), ClientId(2)], &Frame::session("s", "y".into())).await;
        }
        // One taken, one refused, and nothing after.
        assert_eq!(gone.seen.load(Ordering::SeqCst), 2);
        assert!(clients.is_gone(ClientId(1)));
        assert_eq!(fine.seen.load(Ordering::SeqCst), 10);
        assert!(!clients.is_gone(ClientId(2)));
        clients.remove(ClientId(1));
        assert!(!clients.is_gone(ClientId(1)));
    }
}
