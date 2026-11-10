//! The hub's side of the instances: the links it was handed, how far each engine is trusted, and
//! what the hub does as one comes, goes or is replaced (concept §3.1.1).
//!
//! The instance registry builds and lists instances; the hub is the only holder of their links.
//! Each link is read by one router task from the moment it is handed over — before the registry
//! lists the instance — so no caller can reach an instance whose events nobody reads.
//!
//! # An extension's engine
//!
//! An instance whose engine runs in an extension's own process is under third-party control, and
//! the hub holds it to what the product's own engines are not:
//!
//! - its engine-wide events are not fanned out to every client: they serve the hub's own account
//!   of turns, and nothing else;
//! - its frames are bounded ([`FRAME_MAX_BYTES`], [`FRAMES_PER_SECOND`], and a session's buffer
//!   [`EXTENSION_BUFFER_FRAMES`]): what is over is dropped and audited, and the registry is told,
//!   which stops an engine that keeps at it;
//! - what it says of a session is held to what a session can say (`SessionInfo::harden`);
//! - nobody reaches its settings or its plugins: its carrier wrote them before it started;
//! - its sessions are marked `trust: "extension"` wherever they are shown, and a shell draws
//!   them as untrusted content.

use std::sync::{Arc, Mutex as StdMutex};

use harnix_contract::sync::{lock, read, write};
use harnix_contract::Subject;
use harnix_engine_api::link::EngineLink;
use harnix_engine_api::ops::EngineOps;
use harnix_kernel::instances::{holds, Side};
use harnix_kernel::{InstanceObserver, InstanceTrust, TopicChanged};
use serde_json::{json, Value};

use crate::Hub;

/// The largest frame an extension's engine may send.
pub const FRAME_MAX_BYTES: usize = 1024 * 1024;
/// How many frames a second an extension's engine may send.
pub const FRAMES_PER_SECOND: u32 = 200;
/// How many frames of one of an extension's engine's sessions are kept for catch-up.
pub const EXTENSION_BUFFER_FRAMES: usize = 4096;

/// One instance's link, and the task reading its events.
pub(crate) struct Linked {
    pub link: Arc<dyn EngineLink>,
    pub trust: InstanceTrust,
    router: StdMutex<Option<tokio::task::JoinHandle<()>>>,
    flood: StdMutex<Flood>,
}

impl Linked {
    pub(crate) fn ops(&self) -> &dyn EngineOps {
        self.link.ops()
    }

    pub(crate) fn is_extension(&self) -> bool {
        self.trust.is_extension()
    }

    /// Stop reading the link. Shutting it down is the registry's.
    pub(crate) fn stop(&self) {
        if let Some(task) = lock(&self.router).take() {
            task.abort();
        }
    }
}

impl Drop for Linked {
    fn drop(&mut self) {
        self.stop();
    }
}

/// An extension's engine's frames, counted by the second.
#[derive(Default)]
struct Flood {
    /// The second being counted, and how many frames came in it.
    second: u64,
    frames: u32,
    /// The last second something was over, and how many seconds in a row were.
    over: Option<u64>,
    in_a_row: u32,
}

/// What became of one frame of an extension's engine.
pub(crate) enum Bounded {
    Within,
    /// Over: dropped. The first time in a second, with how many seconds in a row it has been.
    Dropped(Option<u32>),
    /// Within again, after being over: the registry is to hear it.
    Calmed,
}

impl Flood {
    fn admit(&mut self, now_s: u64, bytes: usize) -> Bounded {
        if now_s != self.second {
            self.second = now_s;
            self.frames = 0;
        }
        self.frames += 1;
        let over = bytes > FRAME_MAX_BYTES || self.frames > FRAMES_PER_SECOND;
        if !over {
            let calmed = self.over.is_some_and(|last| now_s > last + 1) && self.in_a_row > 0;
            if calmed {
                self.in_a_row = 0;
                return Bounded::Calmed;
            }
            return Bounded::Within;
        }
        if self.over == Some(now_s) {
            return Bounded::Dropped(None);
        }
        self.in_a_row = match self.over {
            Some(last) if last + 1 == now_s => self.in_a_row + 1,
            _ => 1,
        };
        self.over = Some(now_s);
        Bounded::Dropped(Some(self.in_a_row))
    }
}

impl Hub {
    /// Start reading `link`'s events and route `id` to it, in place of whatever link it had.
    fn take(&self, id: &str, link: Arc<dyn EngineLink>, trust: InstanceTrust) {
        let hub = self.arc();
        let router = link.take_events().map(|events| tokio::spawn(hub.route(id.to_string(), events)));
        let linked = Arc::new(Linked {
            link,
            trust,
            router: StdMutex::new(router),
            flood: StdMutex::new(Flood::default()),
        });
        if let Some(previous) = write(&self.links).insert(id.to_string(), linked) {
            previous.stop();
        }
    }

    /// How far the hub trusts an instance's engine.
    pub fn trust(&self, id: &str) -> Option<InstanceTrust> {
        self.linked(id).map(|l| l.trust.clone())
    }

    /// Whether an instance's engine is an extension's.
    pub(crate) fn is_extension(&self, id: &str) -> bool {
        self.linked(id).is_some_and(|l| l.is_extension())
    }

    /// Whether to route one frame of `instance`'s, `bytes` long: always, for the product's own
    /// engines; within bounds, for an extension's. What is dropped is audited, and the registry
    /// told.
    pub(crate) fn bounded(&self, instance: &str, session: &str, bytes: usize) -> bool {
        let Some(linked) = read(&self.links).get(instance).cloned() else {
            return true;
        };
        if !linked.is_extension() {
            return true;
        }
        let now_s = crate::now_ms() / 1000;
        let bounded = lock(&linked.flood).admit(now_s, bytes);
        match bounded {
            Bounded::Within => true,
            Bounded::Calmed => {
                self.instances.flooding(instance, 0);
                true
            }
            Bounded::Dropped(None) => false,
            Bounded::Dropped(Some(in_a_row)) => {
                tracing::warn!(instance, session, bytes, in_a_row, "an extension's engine sent more than it may; dropped");
                if let Some(hook) = read(&self.audit).as_ref() {
                    let subject = Subject::Engine { instance: instance.to_string() };
                    hook(&subject, "flooding", &format!("{instance}/{session}"), &format!("{in_a_row} in a row"));
                }
                self.instances.flooding(instance, in_a_row);
                false
            }
        }
    }
}

#[async_trait::async_trait]
impl InstanceObserver<dyn EngineLink> for Hub {
    async fn added(&self, id: &str, link: Arc<dyn EngineLink>, trust: InstanceTrust) {
        debug_assert!(!holds(Side::Registry), "the registry calls the hub holding its own lock");
        self.take(id, link, trust);
    }

    async fn removed(&self, id: &str) {
        debug_assert!(!holds(Side::Registry), "the registry calls the hub holding its own lock");
        if let Some(linked) = write(&self.links).remove(id) {
            linked.stop();
        }
        self.inner.lock().await.sessions.retain(|key, _| key.0 != id);
        let hub = self.arc();
        hub.refresh("harnix.sessions.status", json!({}));
    }

    async fn restarted(&self, id: &str, link: Arc<dyn EngineLink>) {
        debug_assert!(!holds(Side::Registry), "the registry calls the hub holding its own lock");
        let trust = self.trust(id).unwrap_or(InstanceTrust::Product);
        self.take(id, link, trust);
        // Every subscription belonged to the old link.
        tokio::spawn(self.arc().resync_instance(id.to_string()));
    }

    fn space(&self, claimed: Option<&str>) -> Option<String> {
        let spaces = lock(&self.spaces);
        let space = claimed.map(str::to_string).or_else(|| spaces.default_space().map(str::to_string))?;
        spaces.exists(&space).then_some(space)
    }
}

/// A computed topic's owner says it changed: computed again soon, if anyone watches it.
impl TopicChanged for Hub {
    fn changed(&self, topic: &str, params: Value) {
        self.arc().refresh(topic, params);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_over_the_rate_are_dropped_and_counted_once_a_second() {
        let mut flood = Flood::default();
        for _ in 0..FRAMES_PER_SECOND {
            assert!(matches!(flood.admit(10, 10), Bounded::Within));
        }
        assert!(matches!(flood.admit(10, 10), Bounded::Dropped(Some(1))));
        assert!(matches!(flood.admit(10, 10), Bounded::Dropped(None)), "once a second");
        // The next second starts counting again; over again, it is two in a row.
        for _ in 0..FRAMES_PER_SECOND {
            assert!(matches!(flood.admit(11, 10), Bounded::Within));
        }
        assert!(matches!(flood.admit(11, 10), Bounded::Dropped(Some(2))));
        // A second with nothing over between: the count starts again, and calm is said once.
        assert!(matches!(flood.admit(13, 10), Bounded::Calmed));
        assert!(matches!(flood.admit(13, 10), Bounded::Within));
        assert!(matches!(flood.admit(14, FRAME_MAX_BYTES + 1), Bounded::Dropped(Some(1))), "too large is over too");
    }
}
