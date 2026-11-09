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
use tokio::sync::{mpsc, oneshot};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ClientId(pub u64);

#[derive(Default)]
pub struct Clients {
    sinks: HashMap<ClientId, Arc<dyn FrameSink>>,
    /// Clients whose downstream answered `false`: gone, and sent nothing more
    /// until the transport removes them (`Hub::remove_client`), which it does
    /// once it notices the same thing. Shared with every [`Outgoing`], which
    /// delivers after the hub's lock is released.
    gone: Arc<Mutex<HashSet<ClientId>>>,
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

    /// A frame for a named set — a session's watchers — to send once the hub's lock is released.
    pub fn to_many(&self, ids: &[ClientId], frame: Frame) -> Outgoing {
        let to = ids
            .iter()
            .filter_map(|id| self.sinks.get(id).map(|sink| (*id, sink.clone())))
            .collect();
        self.outgoing(to, frame)
    }

    /// A frame for everyone. Only for host events: a client watching the sidebar
    /// must not receive every session's body just to learn which one is
    /// running (§3.3.1).
    pub fn to_all(&self, frame: Frame) -> Outgoing {
        debug_assert_eq!(frame.channel, Channel::HostEvents);
        let to = self.sinks.iter().map(|(id, sink)| (*id, sink.clone())).collect();
        self.outgoing(to, frame)
    }

    fn outgoing(&self, to: Vec<(ClientId, Arc<dyn FrameSink>)>, frame: Frame) -> Outgoing {
        Outgoing { to, gone: self.gone.clone(), frame }
    }

    /// Whether a client's downstream has said it is gone.
    #[cfg(test)]
    pub fn is_gone(&self, id: ClientId) -> bool {
        lock(&self.gone).contains(&id)
    }
}

/// Sends the hub's frames outside its lock, one batch after another in the order they were
/// handed over (under the lock): one task, started at the first batch.
pub struct Dispatch {
    tx: mpsc::UnboundedSender<Batch>,
    rx: Mutex<Option<mpsc::UnboundedReceiver<Batch>>>,
}

/// Frames to send, and who to tell once they are sent.
type Batch = (Vec<Outgoing>, oneshot::Sender<()>);

impl Default for Dispatch {
    fn default() -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        Self { tx, rx: Mutex::new(Some(rx)) }
    }
}

impl Dispatch {
    /// Queue a batch. The answer comes once it has been sent — or at once, for an empty one. A
    /// caller that stops waiting does not stop the batch.
    pub fn submit(&self, out: Vec<Outgoing>) -> oneshot::Receiver<()> {
        let (done, sent) = oneshot::channel();
        if out.iter().all(|o| o.to.is_empty()) {
            let _ = done.send(());
            return sent;
        }
        if let Some(mut rx) = lock(&self.rx).take() {
            tokio::spawn(async move {
                while let Some((batch, done)) = rx.recv().await {
                    for o in batch {
                        o.send().await;
                    }
                    let _ = done.send(());
                }
            });
        }
        let _ = self.tx.send((out, done));
        sent
    }
}

/// Who a frame goes to, decided under the hub's lock and sent after it.
///
/// The recipients are a snapshot taken with the frame's seq and its place in the replay buffer,
/// so a client attaching meanwhile either is in it or finds the frame in its replay — never both,
/// never neither. Sending outside the lock means a downstream that takes its time holds up the
/// frames behind it, not every session's state ([`Dispatch`]).
pub struct Outgoing {
    to: Vec<(ClientId, Arc<dyn FrameSink>)>,
    gone: Arc<Mutex<HashSet<ClientId>>>,
    frame: Frame,
}

impl Outgoing {
    /// Send to each recipient that still wants this channel.
    ///
    /// `false` from a downstream means it is gone (`FrameSink::deliver`): it
    /// is not sent to again. Ignoring that kept a closed connection counted
    /// as a watcher, every frame of every session it had watched handed to a
    /// queue nobody drained.
    pub async fn send(self) {
        for (id, sink) in &self.to {
            if !sink.wants(self.frame.channel) || lock(&self.gone).contains(id) {
                continue;
            }
            if !sink.deliver(self.frame.clone()).await {
                tracing::debug!(client = id.0, "a client's downstream is gone; nothing more is sent to it");
                lock(&self.gone).insert(*id);
            }
        }
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
            clients.to_all(Frame::host("x".into())).send().await;
            clients.to_many(&[ClientId(1), ClientId(2)], Frame::session("s", "y".into())).send().await;
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
