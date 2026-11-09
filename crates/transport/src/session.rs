//! What a handshake establishes, and what every channel of it shares.
//!
//! **A credential belongs to the subject, not to a connection** (concept
//! §3.3.3, item 2). One handshake produces one credential and one client identity; every
//! channel opened afterwards carries that credential rather than negotiating
//! again. In the single-duplex topology that is one connection holding all
//! five semantics; in split streams it is three connections holding one
//! identity between them.
//!
//! This is the piece the second topology forced into the open. Client
//! identity used to be "which connection is this", which works exactly as
//! long as there is one — and then `harnix.attach` arrives on a POST while the
//! frames have to reach an event stream that is a different socket entirely.
//! Tying identity to the credential fixes that, and it is also what makes
//! revoking a device able to cut all of its channels at once
//! ([`Sessions::end_subject`]).
//!
//! # Every queue is bounded
//!
//! The hub hands a frame to a [`Downstream`] and does not wait: it sends every
//! frame in one order, one recipient after another, so a downstream that
//! blocked would hold up every other client behind it (`FrameSink::deliver`
//! must return at once). A queue that never refuses is how one stuck reader — a
//! background tab, a proxy holding a connection open with nobody behind it —
//! grows this process without limit. So each queue holds [`OUTBOUND_FRAMES`],
//! and a full one ends the client: `deliver` answers `false`, the hub stops
//! sending to it, and the connection closes, so the client reconnects and
//! catches up from the hub's buffer rather than from a backlog.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use harnix_contract::sync::lock;
use harnix_contract::{Channel, Frame, FrameSink, Subject, Topology};
use harnix_hub::ClientId;
use tokio::sync::{mpsc, watch};

/// What this layer needs of the hub: to register a client, describe it, and
/// forget it. A trait so the topologies can be exercised against something
/// smaller than a hub with engines behind it.
#[async_trait::async_trait]
pub trait ClientRegistry: Send + Sync {
    async fn add_client(&self, sink: Arc<dyn FrameSink>) -> ClientId;
    fn describe_client(&self, id: ClientId, description: serde_json::Value);
    async fn remove_client(&self, id: ClientId);
}

#[async_trait::async_trait]
impl ClientRegistry for harnix_hub::Hub {
    async fn add_client(&self, sink: Arc<dyn FrameSink>) -> ClientId {
        harnix_hub::Hub::add_client(self, sink).await
    }

    fn describe_client(&self, id: ClientId, description: serde_json::Value) {
        harnix_hub::Hub::describe_client(self, id, description)
    }

    async fn remove_client(&self, id: ClientId) {
        harnix_hub::Hub::remove_client(self, id).await
    }
}

/// Why a handshake is over, for the connection that has to say so.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Ended {
    /// Its device was revoked.
    Revoked,
    /// It stopped reading, and its queue filled.
    Overflowed,
    /// The process is shutting down.
    ShuttingDown,
    /// Its connection closed, or it sat idle.
    Closed,
}

/// Frames one client may have waiting before it is taken to be gone.
///
/// Catch-up does not travel here — it is the reply to `harnix.attach` — so
/// this is only what arrives live while the reader is behind. A healthy
/// reader drains at the speed of its connection; this many waiting is one
/// that is not reading at all.
pub const OUTBOUND_FRAMES: usize = 4096;

/// How long a split-streams handshake outlives its last sign of use: neither
/// event stream open, and no `/rpc` in this long.
///
/// A page that is closed says nothing; its streams simply stop. Without an
/// end, each page load left a credential, a hub client and its queues behind
/// for good, and every host event went to all of them. Long enough that an
/// event stream's own reconnect, or a laptop lid closed over lunch, does not
/// cost the page its identity.
pub const SPLIT_IDLE: Duration = Duration::from_secs(5 * 60);

/// One established handshake.
pub struct Established {
    pub subject: Subject,
    /// What was negotiated: a split-streams handshake ends when it is idle
    /// ([`Sessions::sweep`]), a single-duplex one when its connection does.
    pub topology: Topology,
    /// The hub's handle for this client. Frames addressed to it arrive at
    /// [`Downstream`], which puts them on whichever connection is draining
    /// them now.
    pub client: ClientId,
    pub sink: Arc<Downstream>,
    /// The last request, or the last moment an event stream was open.
    last_seen: Mutex<Instant>,
}

impl Established {
    pub fn touch(&self) {
        *lock(&self.last_seen) = Instant::now();
    }

    /// Changes once this handshake is over, to say why.
    pub fn ended(&self) -> watch::Receiver<Option<Ended>> {
        self.sink.ended.subscribe()
    }

    pub fn is_ended(&self) -> bool {
        self.sink.ended.borrow().is_some()
    }
}

/// The downstream the hub sees. It knows nothing about connections.
pub struct Downstream {
    wire: Wire,
    /// Which faces this client actually subscribed to. In split streams a
    /// read-only client may open only the host stream, and then session
    /// bodies are not merely ignored — they are never queued.
    faces: parking_faces::Faces,
    /// Set once, when the client is over; every channel of it watches this.
    ended: watch::Sender<Option<Ended>>,
}

/// Where frames go.
enum Wire {
    /// One bidirectional connection carries both faces, so frames go straight
    /// to its own queue — the alternative was two channels and two tasks per
    /// connection whose entire job was to copy one queue into another. That
    /// showed up as about 110 KB per idle connection against a 64 KB budget,
    /// which is how it was found.
    Single(mpsc::Sender<String>),
    /// Each face on its own download-only stream, and each stream can drop
    /// and come back: the sender is replaced by every `GET /events/*`.
    /// `None` while that face has no reader.
    Split {
        session: Mutex<Option<mpsc::Sender<String>>>,
        host: Mutex<Option<mpsc::Sender<String>>>,
    },
}

/// A tiny bitset, spelled out rather than pulled in.
mod parking_faces {
    use std::sync::atomic::{AtomicU8, Ordering};

    #[derive(Default)]
    pub struct Faces(AtomicU8);

    const SESSION: u8 = 1;
    const HOST: u8 = 2;

    impl Faces {
        pub fn all() -> Self {
            Self(AtomicU8::new(SESSION | HOST))
        }

        pub fn none() -> Self {
            Self(AtomicU8::new(0))
        }

        pub fn add_session(&self) {
            self.0.fetch_or(SESSION, Ordering::Relaxed);
        }

        pub fn add_host(&self) {
            self.0.fetch_or(HOST, Ordering::Relaxed);
        }

        pub fn wants_session(&self) -> bool {
            self.0.load(Ordering::Relaxed) & SESSION != 0
        }

        pub fn wants_host(&self) -> bool {
            self.0.load(Ordering::Relaxed) & HOST != 0
        }
    }
}

/// Which download-only stream.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Face {
    Session,
    Host,
}

impl Downstream {
    /// The first reason sticks: a revoked client whose connection then
    /// closes was revoked.
    fn end(&self, why: Ended) {
        self.ended.send_if_modified(|ended| {
            let first = ended.is_none();
            if first {
                *ended = Some(why);
            }
            first
        });
        // The streams' senders go now, so a reader still attached sees its
        // stream end instead of waiting on frames that will never come.
        if let Wire::Split { session, host } = &self.wire {
            lock(session).take();
            lock(host).take();
        }
    }

    /// Whether a reader is attached to either stream right now. Always true
    /// for the one connection that carries both.
    fn has_reader(&self) -> bool {
        match &self.wire {
            Wire::Single(tx) => !tx.is_closed(),
            Wire::Split { session, host } => [session, host]
                .iter()
                .any(|slot| lock(slot).as_ref().is_some_and(|tx| !tx.is_closed())),
        }
    }

    /// Hand one frame on, without waiting. `false` when the client is gone or
    /// too far behind to catch up from a queue.
    fn put(&self, face: Face, payload: String) -> bool {
        if self.ended.borrow().is_some() {
            return false;
        }
        let tx = match &self.wire {
            Wire::Single(tx) => Some(tx.clone()),
            Wire::Split { session, host } => {
                let slot = if face == Face::Session { session } else { host };
                lock(slot).clone()
            }
        };
        let Some(tx) = tx else {
            // A face whose stream has dropped and not come back yet: the
            // client is still here, and catches up when it re-attaches.
            return true;
        };
        match tx.try_send(payload) {
            Ok(()) => true,
            Err(mpsc::error::TrySendError::Full(_)) => {
                tracing::warn!(
                    queued = OUTBOUND_FRAMES,
                    "a client stopped reading; it is disconnected and will catch up on reconnect"
                );
                self.end(Ended::Overflowed);
                false
            }
            // The one connection is gone; a split stream's reader is gone
            // until the next `GET` replaces it.
            Err(mpsc::error::TrySendError::Closed(_)) => !matches!(self.wire, Wire::Single(_)),
        }
    }
}

#[async_trait::async_trait]
impl FrameSink for Downstream {
    async fn deliver(&self, frame: Frame) -> bool {
        match frame.channel {
            Channel::SessionEvents => self.put(Face::Session, frame.payload.to_string()),
            Channel::HostEvents => self.put(Face::Host, frame.payload.to_string()),
            // Unary replies travel back on the request that asked; bulk goes
            // around every channel. Neither reaches a sink.
            _ => true,
        }
    }

    fn wants(&self, channel: Channel) -> bool {
        match channel {
            Channel::SessionEvents => self.faces.wants_session(),
            Channel::HostEvents => self.faces.wants_host(),
            _ => false,
        }
    }
}

/// Every established handshake, by credential.
///
/// A `std` lock, never held across an await: revoking a device ends its
/// credentials before the revoke call answers, from a callback that cannot
/// wait.
#[derive(Default)]
pub struct Sessions {
    by_credential: Mutex<HashMap<String, Arc<Established>>>,
    next: AtomicU64,
}

impl Sessions {
    /// Establish one.
    ///
    /// `single_wire` is the connection's own sender, for a topology that
    /// carries both faces on one connection: frames go straight to it, and no
    /// queue or task is created to move them. `None` splits the faces onto
    /// the streams that will be opened for them ([`Sessions::open`]).
    pub async fn establish(
        &self,
        hub: &dyn ClientRegistry,
        subject: Subject,
        topology: Topology,
        single_wire: Option<mpsc::Sender<String>>,
        user_agent: &str,
    ) -> (String, Arc<Established>) {
        let (wire, faces) = match single_wire {
            // Both faces, from the start: there is one wire and it carries
            // everything.
            Some(wire) => (Wire::Single(wire), parking_faces::Faces::all()),
            // Neither: opening a stream is how a client says which face it
            // wants.
            None => (
                Wire::Split {
                    session: Mutex::new(None),
                    host: Mutex::new(None),
                },
                parking_faces::Faces::none(),
            ),
        };
        let sink = Arc::new(Downstream {
            wire,
            faces,
            ended: watch::channel(None).0,
        });
        let client = hub.add_client(sink.clone()).await;
        // What the system space lists as a connected user: the device, how it is connected, what
        // it says it is, and since when.
        let device = match &subject {
            Subject::Device { id } => id.clone(),
            other => format!("{other:?}"),
        };
        let since_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        hub.describe_client(
            client,
            serde_json::json!({
                "device": device,
                "topology": topology.as_str(),
                "user_agent": user_agent,
                "since_ms": since_ms,
            }),
        );
        let established = Arc::new(Established {
            subject,
            topology,
            client,
            sink,
            last_seen: Mutex::new(Instant::now()),
        });
        // Opaque and unguessable; it is the only thing a later channel shows.
        let credential = format!(
            "{}-{}",
            uuid::Uuid::new_v4().simple(),
            self.next.fetch_add(1, Ordering::Relaxed)
        );
        lock(&self.by_credential).insert(credential.clone(), established.clone());
        (credential, established)
    }

    /// The handshake a credential names, while it lasts. Asking is a sign of
    /// use: an idle split handshake is measured from here.
    pub fn get(&self, credential: &str) -> Option<Arc<Established>> {
        let established = lock(&self.by_credential).get(credential).cloned()?;
        if established.is_ended() {
            return None;
        }
        established.touch();
        Some(established)
    }

    /// Open one face's stream: the receiving end of a fresh queue.
    ///
    /// Opening a stream is how a client subscribes to that face. It is not a
    /// formality: a client that opens only the host stream never has session
    /// bodies queued for it at all, which is the read-only case the two faces
    /// exist for.
    ///
    /// **A stream that dropped can be opened again**, and a second open
    /// replaces the first, whose stream then ends. An event stream is
    /// reopened by the browser on its own when it drops; answering that with
    /// "this stream already has a reader" — for a reader that was gone — left
    /// the page deaf for good, because a browser does not retry a refusal.
    pub fn open(&self, credential: &str, face: Face) -> Option<mpsc::Receiver<String>> {
        let established = self.get(credential)?;
        let Wire::Split { session, host } = &established.sink.wire else {
            return None;
        };
        let (tx, rx) = mpsc::channel(OUTBOUND_FRAMES);
        match face {
            Face::Session => {
                established.sink.faces.add_session();
                *lock(session) = Some(tx);
            }
            Face::Host => {
                established.sink.faces.add_host();
                *lock(host) = Some(tx);
            }
        }
        Some(rx)
    }

    /// Drop one, and everything it was watching with it.
    pub async fn end(&self, hub: &dyn ClientRegistry, credential: &str) {
        let established = lock(&self.by_credential).remove(credential);
        if let Some(established) = established {
            established.sink.end(Ended::Closed);
            hub.remove_client(established.client).await;
        }
    }

    /// End every credential `subject` holds, now: each of its channels sees
    /// its end, and no credential of it is accepted again. What revoking a
    /// device is. Returns the hub clients to forget, which is the one part
    /// that has to wait.
    pub fn end_subject(&self, subject: &Subject) -> Vec<ClientId> {
        self.end_where(|e| &e.subject == subject, Ended::Revoked)
    }

    /// End every handshake, for a process on its way out: each connection
    /// is told so and closes, rather than carrying calls to modules that have
    /// stopped.
    pub fn end_all(&self) -> Vec<ClientId> {
        self.end_where(|_| true, Ended::ShuttingDown)
    }

    fn end_where(&self, which: impl Fn(&Established) -> bool, why: Ended) -> Vec<ClientId> {
        let ended: Vec<Arc<Established>> = {
            let mut all = lock(&self.by_credential);
            let credentials: Vec<String> = all
                .iter()
                .filter(|(_, e)| which(e))
                .map(|(c, _)| c.clone())
                .collect();
            credentials.iter().filter_map(|c| all.remove(c)).collect()
        };
        ended
            .iter()
            .map(|established| {
                established.sink.end(why);
                established.client
            })
            .collect()
    }

    /// End the split-streams handshakes nobody is using — no stream open,
    /// nothing asked since `now - idle` — and any that ended on their own (a
    /// queue that filled). A single-duplex handshake is its connection's to
    /// end. Returns how many ended.
    pub async fn sweep(&self, hub: &dyn ClientRegistry, now: Instant, idle: Duration) -> usize {
        let ended: Vec<Arc<Established>> = {
            let mut all = lock(&self.by_credential);
            let gone: Vec<String> = all
                .iter()
                .filter(|(_, e)| {
                    e.is_ended()
                        || (e.topology == Topology::SplitStreams
                            && !e.sink.has_reader()
                            && now.saturating_duration_since(*lock(&e.last_seen)) >= idle)
                })
                .map(|(c, _)| c.clone())
                .collect();
            gone.iter().filter_map(|c| all.remove(c)).collect()
        };
        for established in &ended {
            established.sink.end(Ended::Closed);
            hub.remove_client(established.client).await;
        }
        ended.len()
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        lock(&self.by_credential).len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hub() -> Arc<crate::tests::FakeHub> {
        Arc::new(crate::tests::FakeHub::default())
    }

    fn device(id: &str) -> Subject {
        Subject::Device { id: id.into() }
    }

    async fn split(sessions: &Sessions, hub: &dyn ClientRegistry, who: &str) -> (String, Arc<Established>) {
        sessions.establish(hub, device(who), Topology::SplitStreams, None, "test").await
    }

    /// A stream that dropped is opened again, and frames reach the new one;
    /// the old reader's stream ends rather than splitting frames with it.
    #[tokio::test]
    async fn a_dropped_stream_reopens_and_receives() {
        let (hub, sessions) = (hub(), Sessions::default());
        let (credential, established) = split(&sessions, &*hub, "a").await;

        let first = sessions.open(&credential, Face::Host).expect("opened");
        drop(first);
        // Nobody reading: the client is not gone, the frame simply has no reader.
        assert!(established.sink.deliver(Frame::host("lost".into())).await);

        let mut second = sessions.open(&credential, Face::Host).expect("opened again");
        let mut third = sessions.open(&credential, Face::Host).expect("and again");
        assert!(established.sink.deliver(Frame::host("one".into())).await);
        assert_eq!(third.recv().await.as_deref(), Some("one"));
        // Replaced: the earlier reader's stream is over.
        assert_eq!(second.recv().await, None);
    }

    /// A face nobody opened gets nothing queued.
    #[tokio::test]
    async fn only_the_faces_opened_are_wanted() {
        let (hub, sessions) = (hub(), Sessions::default());
        let (credential, established) = split(&sessions, &*hub, "a").await;
        assert!(!established.sink.wants(Channel::SessionEvents));
        let _host = sessions.open(&credential, Face::Host).unwrap();
        assert!(established.sink.wants(Channel::HostEvents));
        assert!(!established.sink.wants(Channel::SessionEvents));
    }

    /// Revoking a device ends every credential it holds at once, and those of
    /// no one else.
    #[tokio::test]
    async fn ending_a_subject_ends_all_of_its_channels() {
        let (hub, sessions) = (hub(), Sessions::default());
        let (a1, e1) = split(&sessions, &*hub, "dev-a").await;
        let (tx, mut wire) = mpsc::channel(8);
        let (a2, e2) = sessions
            .establish(&*hub, device("dev-a"), Topology::SingleDuplex, Some(tx), "test")
            .await;
        let (b, _) = split(&sessions, &*hub, "dev-b").await;
        let mut stream = sessions.open(&a1, Face::Session).unwrap();
        let mut ended = e2.ended();

        let clients = sessions.end_subject(&device("dev-a"));
        assert_eq!(clients.len(), 2);
        assert!(sessions.get(&a1).is_none() && sessions.get(&a2).is_none());
        assert!(sessions.get(&b).is_some());
        // The open stream ends, the connection is told, and nothing more is delivered.
        assert_eq!(stream.recv().await, None);
        assert_eq!(*ended.wait_for(|e| e.is_some()).await.unwrap(), Some(Ended::Revoked));
        assert!(!e1.sink.deliver(Frame::host("x".into())).await);
        assert!(!e2.sink.deliver(Frame::host("x".into())).await);
        assert!(wire.try_recv().is_err());
    }

    /// A queue that fills ends the client: `false` to the hub, and the
    /// connection is told to close. Nothing grows past the ceiling.
    #[tokio::test]
    async fn a_reader_that_stops_reading_is_disconnected() {
        let (hub, sessions) = (hub(), Sessions::default());
        let (tx, _wire) = mpsc::channel(OUTBOUND_FRAMES);
        let (credential, established) = sessions
            .establish(&*hub, device(harnix_contract::LOCAL_DEVICE), Topology::SingleDuplex, Some(tx), "test")
            .await;
        for _ in 0..OUTBOUND_FRAMES {
            assert!(established.sink.deliver(Frame::host("f".into())).await);
        }
        assert!(!established.sink.deliver(Frame::host("one too many".into())).await);
        assert_eq!(*established.ended().borrow(), Some(Ended::Overflowed));
        assert!(sessions.get(&credential).is_none());

        // The same on a split stream.
        let (credential, established) = split(&sessions, &*hub, "a").await;
        let _stream = sessions.open(&credential, Face::Host).unwrap();
        for _ in 0..OUTBOUND_FRAMES {
            assert!(established.sink.deliver(Frame::host("f".into())).await);
        }
        assert!(!established.sink.deliver(Frame::host("one too many".into())).await);
        assert!(sessions.get(&credential).is_none());
    }

    /// The single connection gone is the client gone.
    #[tokio::test]
    async fn a_closed_connection_is_gone() {
        let (hub, sessions) = (hub(), Sessions::default());
        let (tx, wire) = mpsc::channel(8);
        let (_, established) = sessions
            .establish(&*hub, device(harnix_contract::LOCAL_DEVICE), Topology::SingleDuplex, Some(tx), "test")
            .await;
        drop(wire);
        assert!(!established.sink.deliver(Frame::host("x".into())).await);
    }

    /// An idle split handshake ends; one with a stream open, one recently
    /// used, and a single-duplex one do not.
    #[tokio::test]
    async fn idle_split_handshakes_are_swept() {
        let (hub, sessions) = (hub(), Sessions::default());
        let (idle, _) = split(&sessions, &*hub, "a").await;
        let (streaming, _) = split(&sessions, &*hub, "b").await;
        let _stream = sessions.open(&streaming, Face::Host).unwrap();
        let (tx, _wire) = mpsc::channel(8);
        let (duplex, _) = sessions
            .establish(&*hub, device("c"), Topology::SingleDuplex, Some(tx), "test")
            .await;

        // Not idle long enough yet.
        assert_eq!(sessions.sweep(&*hub, Instant::now(), SPLIT_IDLE).await, 0);
        let later = Instant::now() + SPLIT_IDLE + Duration::from_secs(1);
        assert_eq!(sessions.sweep(&*hub, later, SPLIT_IDLE).await, 1);
        assert!(sessions.get(&idle).is_none());
        assert!(sessions.get(&streaming).is_some());
        assert!(sessions.get(&duplex).is_some());
        assert_eq!(sessions.len(), 2);
        // And the hub was told to forget it.
        assert_eq!(hub.removed().len(), 1);
    }
}
