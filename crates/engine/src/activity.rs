//! What a link has seen of each session since it was made: when a frame last named it, and whether
//! a turn of it runs. A recording changes only while its session works, so a reader asks the engine
//! again only when this says it may have.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use harnix_contract::sync::lock;

use crate::translate::Stir;

/// After a turn ends the engine may still call the model beside it (a title, memory, compaction):
/// for this long a reading is not taken as current.
const SETTLE: Duration = Duration::from_secs(30);
/// At most this many sessions remembered; past it, the ones seen longest ago go.
const KEPT: usize = 4096;

#[derive(Default)]
pub(crate) struct Activity(Mutex<Inner>);

#[derive(Default)]
struct Inner {
    sessions: HashMap<String, Seen>,
    connected: Option<Instant>,
}

#[derive(Clone, Copy)]
struct Seen {
    at: Instant,
    running: bool,
}

impl Activity {
    /// A frame named `session`.
    pub(crate) fn saw(&self, session: &str, stir: Stir) {
        self.saw_at(session, stir, Instant::now());
    }

    fn saw_at(&self, session: &str, stir: Stir, at: Instant) {
        let mut inner = lock(&self.0);
        match stir {
            Stir::Reconnect => inner.connected = Some(at),
            Stir::TurnStarted => {
                inner.sessions.insert(session.to_string(), Seen { at, running: true });
            }
            Stir::TurnEnded => {
                inner.sessions.insert(session.to_string(), Seen { at, running: false });
            }
            Stir::Other => {
                let running = inner.sessions.get(session).is_some_and(|s| s.running);
                inner.sessions.insert(session.to_string(), Seen { at, running });
            }
        }
        if inner.sessions.len() > KEPT {
            let oldest = inner.sessions.iter().filter(|(_, s)| !s.running).min_by_key(|(_, s)| s.at).map(|(id, _)| id.clone());
            if let Some(id) = oldest {
                inner.sessions.remove(&id);
            }
        }
    }

    /// The link took a connection: after a reconnection anything may have changed.
    pub(crate) fn connected(&self) {
        lock(&self.0).connected = Some(Instant::now());
    }

    /// Whether what was read of `session` at `read_at` may be behind now: the link connected
    /// again since, a turn of it runs, or something was seen of it less than [`SETTLE`] before the
    /// reading.
    pub(crate) fn may_have_changed(&self, session: &str, read_at: Instant) -> bool {
        let inner = lock(&self.0);
        if inner.connected.is_some_and(|c| c > read_at) {
            return true;
        }
        match inner.sessions.get(session) {
            None => false,
            Some(seen) => seen.running || seen.at + SETTLE > read_at,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_session_nothing_was_seen_of_has_not_changed() {
        let activity = Activity::default();
        assert!(!activity.may_have_changed("s", Instant::now()));
    }

    #[test]
    fn a_running_turn_and_the_settling_after_it_count_as_change() {
        let activity = Activity::default();
        let start = Instant::now();
        activity.saw_at("s", Stir::TurnStarted, start);
        let read = start + Duration::from_secs(100);
        assert!(activity.may_have_changed("s", read), "a turn runs");
        activity.saw_at("s", Stir::TurnEnded, start + Duration::from_secs(101));
        assert!(activity.may_have_changed("s", read), "read before the turn ended");
        assert!(activity.may_have_changed("s", start + Duration::from_secs(110)), "within the settling time");
        assert!(!activity.may_have_changed("s", start + Duration::from_secs(140)), "read after it settled");
        assert!(!activity.may_have_changed("other", read));
    }

    #[test]
    fn a_frame_of_a_running_session_keeps_it_running() {
        let activity = Activity::default();
        let start = Instant::now();
        activity.saw_at("s", Stir::TurnStarted, start);
        activity.saw_at("s", Stir::Other, start + Duration::from_secs(1));
        assert!(activity.may_have_changed("s", start + Duration::from_secs(500)));
    }

    #[test]
    fn a_reconnection_makes_every_earlier_reading_stale() {
        let activity = Activity::default();
        let read = Instant::now();
        assert!(!activity.may_have_changed("s", read));
        activity.connected();
        assert!(activity.may_have_changed("s", read));
        assert!(!activity.may_have_changed("s", Instant::now() + Duration::from_millis(1)));
    }

    #[test]
    fn past_the_limit_the_one_seen_longest_ago_goes() {
        let activity = Activity::default();
        let start = Instant::now();
        for n in 0..=KEPT {
            activity.saw_at(&format!("s{n}"), Stir::TurnEnded, start + Duration::from_millis(n as u64));
        }
        assert_eq!(lock(&activity.0).sessions.len(), KEPT);
        assert!(!lock(&activity.0).sessions.contains_key("s0"));
    }
}
