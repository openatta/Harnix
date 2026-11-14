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
//! - its frames are bounded ([`FRAME_MAX_BYTES`], [`FRAMES_PER_SECOND`] with bursts of
//!   [`FRAMES_BURST`], and a session's buffer [`EXTENSION_BUFFER_FRAMES`]): what is over is
//!   dropped and audited — but never the frame that ends a turn — and the registry is told, which
//!   stops an engine that keeps at it;
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
/// How many frames a second an extension's engine may send, kept up.
pub const FRAMES_PER_SECOND: u32 = 200;
/// How many frames an extension's engine may send at once, above [`FRAMES_PER_SECOND`]: a turn's
/// tail (a sub-agent's whole transcript, thinking streamed beside the answer) comes in one go, and
/// a well-behaved engine is not to lose its end to a bound meant for a hostile one.
pub const FRAMES_BURST: u32 = 2000;
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

/// What one frame draws from the bucket, in thousandths so that it refills by the millisecond.
const FRAME_COST: u64 = 1000;

/// An extension's engine's frames: a bucket of [`FRAMES_BURST`] that refills at
/// [`FRAMES_PER_SECOND`], and the seconds in which it was over.
#[derive(Default)]
struct Flood {
    /// How much of the bucket is drawn, in [`FRAME_COST`]s (`0`: full), as of `at_ms`.
    drawn: u64,
    at_ms: u64,
    /// The last second something was over, and how many seconds in a row were.
    over: Option<u64>,
    in_a_row: u32,
}

/// What became of one frame of an extension's engine.
pub(crate) enum Bounded {
    Within,
    /// Over: dropped. The first time in a second, with how many seconds in a row it has been.
    Dropped(Option<u32>),
    /// Over, but it ends a turn, which clients need to settle: routed, and otherwise counted as
    /// [`Bounded::Dropped`] is.
    Kept(Option<u32>),
    /// Within again, after being over: the registry is to hear it.
    Calmed,
}

impl Flood {
    /// One frame, `bytes` long, at `now_ms`; `ends_turn`: it is how the engine says a turn ended.
    fn admit(&mut self, now_ms: u64, bytes: usize, ends_turn: bool) -> Bounded {
        let refilled = now_ms.saturating_sub(self.at_ms) * u64::from(FRAMES_PER_SECOND);
        self.drawn = self.drawn.saturating_sub(refilled);
        self.at_ms = self.at_ms.max(now_ms);
        let empty = self.drawn + FRAME_COST > u64::from(FRAMES_BURST) * FRAME_COST;
        if !empty {
            self.drawn += FRAME_COST;
        }
        let now_s = now_ms / 1000;
        let over = bytes > FRAME_MAX_BYTES || empty;
        if !over {
            let calmed = self.over.is_some_and(|last| now_s > last + 1) && self.in_a_row > 0;
            if calmed {
                self.in_a_row = 0;
                return Bounded::Calmed;
            }
            return Bounded::Within;
        }
        let told = (self.over != Some(now_s)).then(|| {
            self.in_a_row = match self.over {
                Some(last) if last + 1 == now_s => self.in_a_row + 1,
                _ => 1,
            };
            self.over = Some(now_s);
            self.in_a_row
        });
        // Too large is dropped whatever it says.
        if ends_turn && bytes <= FRAME_MAX_BYTES {
            Bounded::Kept(told)
        } else {
            Bounded::Dropped(told)
        }
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
    /// engines; within bounds, for an extension's, or when it ends a turn (`ends_turn`) and is not
    /// too large. What is over is audited, and the registry told.
    pub(crate) fn bounded(&self, instance: &str, session: &str, bytes: usize, ends_turn: bool) -> bool {
        let Some(linked) = read(&self.links).get(instance).cloned() else {
            return true;
        };
        if !linked.is_extension() {
            return true;
        }
        let bounded = lock(&linked.flood).admit(crate::now_ms(), bytes, ends_turn);
        let (routed, told) = match bounded {
            Bounded::Within => return true,
            Bounded::Calmed => {
                self.instances.flooding(instance, 0);
                return true;
            }
            Bounded::Dropped(told) => (false, told),
            Bounded::Kept(told) => (true, told),
        };
        if let Some(in_a_row) = told {
            tracing::warn!(instance, session, bytes, in_a_row, routed, "an extension's engine sent more than it may");
            if let Some(hook) = read(&self.audit).as_ref() {
                let subject = Subject::Engine { instance: instance.to_string() };
                hook(&subject, "flooding", &format!("{instance}/{session}"), &format!("{in_a_row} in a row"));
            }
            self.instances.flooding(instance, in_a_row);
        }
        routed
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
        self.forget_home(id);
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

    async fn engaged(&self, id: &str) -> bool {
        debug_assert!(!holds(Side::Registry), "the registry calls the hub holding its own lock");
        let guard = self.inner.lock().await;
        guard.sessions.iter().filter(|(key, _)| key.0 == id).any(|(_, s)| {
            !s.watchers.is_empty()
                || s.running_turn.is_some()
                || !s.queue.is_empty()
                || !s.live_tasks.is_empty()
                || !s.pending_prompts.is_empty()
        })
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

    /// `per_second` frames a second for `seconds`, from `start_ms`, evenly spread; what became of
    /// each.
    fn steady(flood: &mut Flood, start_ms: u64, seconds: u64, per_second: u64) -> Vec<Bounded> {
        (0..seconds * per_second).map(|n| flood.admit(start_ms + n * 1000 / per_second, 10, false)).collect()
    }

    #[test]
    fn a_burst_within_the_bucket_is_admitted_whole() {
        let mut flood = Flood::default();
        for _ in 0..1000 {
            assert!(matches!(flood.admit(10_000, 10, false), Bounded::Within));
        }
        // What is left of the bucket, and nothing more till it refills.
        for _ in 0..FRAMES_BURST - 1000 {
            assert!(matches!(flood.admit(10_000, 10, false), Bounded::Within));
        }
        assert!(matches!(flood.admit(10_000, 10, false), Bounded::Dropped(Some(1))));
        assert!(matches!(flood.admit(10_000, 10, false), Bounded::Dropped(None)), "told once a second");
        // It refills at the rate: half a second later, half a second's worth.
        let refilled = (0..FRAMES_PER_SECOND).filter(|_| matches!(flood.admit(10_500, 10, false), Bounded::Within)).count();
        assert_eq!(refilled, FRAMES_PER_SECOND as usize / 2);
    }

    #[test]
    fn the_rate_kept_up_is_admitted_and_twice_it_is_dropped_and_escalates() {
        let mut flood = Flood::default();
        let kept_up = steady(&mut flood, 10_000, 30, u64::from(FRAMES_PER_SECOND));
        assert!(kept_up.iter().all(|b| matches!(b, Bounded::Within)));

        let mut flood = Flood::default();
        let twice = steady(&mut flood, 10_000, 20, 2 * u64::from(FRAMES_PER_SECOND));
        let told: Vec<u32> = twice.iter().filter_map(|b| if let Bounded::Dropped(Some(n)) = b { Some(*n) } else { None }).collect();
        // The burst holds it off for a while, and then it is over every second, in a row.
        assert!(told.len() >= 5, "{told:?}");
        assert_eq!(told, (1..=told.len() as u32).collect::<Vec<_>>(), "seconds in a row");
        let dropped = twice.iter().filter(|b| matches!(b, Bounded::Dropped(_))).count();
        assert!(dropped > twice.len() / 5, "{dropped} of {}", twice.len());
        // A second with nothing over between: the count starts again, and calm is said once.
        assert!(matches!(flood.admit(40_000, 10, false), Bounded::Calmed));
        assert!(matches!(flood.admit(40_000, 10, false), Bounded::Within));
        assert!(matches!(flood.admit(41_000, FRAME_MAX_BYTES + 1, false), Bounded::Dropped(Some(1))), "too large is over too");
    }

    #[test]
    fn the_frame_that_ends_a_turn_is_kept_when_the_bucket_is_empty() {
        let mut flood = Flood::default();
        for _ in 0..FRAMES_BURST {
            assert!(matches!(flood.admit(10_000, 10, false), Bounded::Within));
        }
        assert!(matches!(flood.admit(10_000, 10, false), Bounded::Dropped(Some(1))));
        // Kept, and still over: it is not told twice in the second, and the next is.
        assert!(matches!(flood.admit(10_000, 10, true), Bounded::Kept(None)));
        assert!(matches!(flood.admit(11_000, 10, true), Bounded::Within), "a second refilled the bucket");
        for _ in 1..FRAMES_PER_SECOND {
            assert!(matches!(flood.admit(11_000, 10, false), Bounded::Within));
        }
        assert!(matches!(flood.admit(11_000, 10, true), Bounded::Kept(Some(2))));
        // It draws from the bucket like any other.
        let mut flood = Flood::default();
        for _ in 0..FRAMES_BURST {
            assert!(matches!(flood.admit(10_000, 10, true), Bounded::Within));
        }
        assert!(matches!(flood.admit(10_000, 10, false), Bounded::Dropped(Some(1))));
        // Too large is dropped, whatever it says.
        assert!(matches!(flood.admit(20_000, FRAME_MAX_BYTES + 1, true), Bounded::Dropped(_)));
    }
}
