//! When a child that exited is started again, and when it is not.

use std::time::Duration;

/// A schedule of waits between starts, and what counts as a child that will not stay up.
#[derive(Debug, Clone)]
pub struct RestartPolicy {
    /// The wait before the first start again, the second, and so on; the last one repeats.
    pub backoff: Vec<Duration>,
    /// A run at least this long starts the schedule over.
    pub reset_after: Duration,
    /// A run shorter than this is a quick exit.
    pub quick: Duration,
    /// This many quick exits in a row and the child is not started again: something about it is
    /// wrong, and a restart loop would hide what.
    pub give_up_after: u32,
}

impl RestartPolicy {
    /// What a link keeps a spawned Agix daemon by: 0.2 s doubling to 5 s, five quick exits (under
    /// 5 s) in a row. The schedule starts over when the link says so, not by how long a run was.
    pub fn daemon() -> Self {
        Self {
            backoff: [200, 400, 800, 1600, 3200, 5000].map(Duration::from_millis).to_vec(),
            reset_after: Duration::MAX,
            quick: Duration::from_secs(5),
            give_up_after: 5,
        }
    }

    /// What an extension process is kept by: 1, 2, 4, 8, 16, 30, 60 s; a run of 120 s starts the
    /// schedule over; three exits within 60 s of starting in a row and it is set aside.
    pub fn extension() -> Self {
        Self {
            backoff: [1, 2, 4, 8, 16, 30, 60].map(Duration::from_secs).to_vec(),
            reset_after: Duration::from_secs(120),
            quick: Duration::from_secs(60),
            give_up_after: 3,
        }
    }

    pub fn restarts(&self) -> Restarts {
        Restarts { policy: self.clone(), step: 0, quick_exits: 0 }
    }
}

/// What to do about a child that exited.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    Restart { after: Duration },
    GiveUp { reason: String },
}

/// One child's record against a [`RestartPolicy`].
#[derive(Debug, Clone)]
pub struct Restarts {
    policy: RestartPolicy,
    step: usize,
    quick_exits: u32,
}

impl Restarts {
    /// The wait before the next start; the one after it is longer, up to the end of the schedule.
    pub fn next_backoff(&mut self) -> Duration {
        let last = self.policy.backoff.len().saturating_sub(1);
        let wait = self.policy.backoff.get(self.step.min(last)).copied().unwrap_or_default();
        self.step = (self.step + 1).min(last + 1);
        wait
    }

    pub fn reset_backoff(&mut self) {
        self.step = 0;
    }

    /// Count a quick exit. True once there have been too many in a row.
    pub fn quick_exit(&mut self) -> bool {
        self.quick_exits += 1;
        self.quick_exits >= self.policy.give_up_after
    }

    /// A run lasted `ran_for`: long enough to count as healthy ends the run of quick exits, and
    /// a long enough one starts the schedule over.
    pub fn ran_for(&mut self, ran_for: Duration) {
        if ran_for >= self.policy.quick {
            self.quick_exits = 0;
        }
        if ran_for >= self.policy.reset_after {
            self.reset_backoff();
        }
    }

    /// The child exited after `ran_for`.
    pub fn exited(&mut self, ran_for: Duration) -> Decision {
        self.ran_for(ran_for);
        if ran_for < self.policy.quick && self.quick_exit() {
            return Decision::GiveUp {
                reason: format!(
                    "it exited within {}s of starting {} times in a row",
                    self.policy.quick.as_secs_f64(),
                    self.quick_exits
                ),
            };
        }
        Decision::Restart { after: self.next_backoff() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secs(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    #[test]
    fn the_wait_doubles_along_the_schedule_and_stays_at_its_end() {
        let mut r = RestartPolicy::extension().restarts();
        let waits: Vec<_> = (0..9).map(|_| r.next_backoff()).collect();
        assert_eq!(waits, [1, 2, 4, 8, 16, 30, 60, 60, 60].map(secs));
        let mut d = RestartPolicy::daemon().restarts();
        let waits: Vec<_> = (0..8).map(|_| d.next_backoff()).collect();
        assert_eq!(waits.iter().map(Duration::as_millis).collect::<Vec<_>>(), [200, 400, 800, 1600, 3200, 5000, 5000, 5000]);
    }

    #[test]
    fn the_third_quick_exit_in_a_row_is_given_up_on() {
        let mut r = RestartPolicy::extension().restarts();
        assert!(matches!(r.exited(secs(1)), Decision::Restart { after } if after == secs(1)));
        assert!(matches!(r.exited(secs(1)), Decision::Restart { after } if after == secs(2)));
        assert!(matches!(r.exited(secs(1)), Decision::GiveUp { .. }));
    }

    #[test]
    fn a_run_of_120_seconds_starts_the_schedule_and_the_count_over() {
        let mut r = RestartPolicy::extension().restarts();
        r.exited(secs(1));
        r.exited(secs(1));
        // Healthy for long enough: both begin again.
        assert!(matches!(r.exited(secs(120)), Decision::Restart { after } if after == secs(1)));
        assert!(matches!(r.exited(secs(1)), Decision::Restart { after } if after == secs(2)));
        assert!(matches!(r.exited(secs(1)), Decision::Restart { .. }));
    }

    /// Past the quick-exit mark but short of the reset: the count of quick exits ends, the
    /// schedule goes on.
    #[test]
    fn a_run_between_the_two_marks_ends_the_count_but_not_the_schedule() {
        let mut r = RestartPolicy::extension().restarts();
        r.exited(secs(1));
        r.exited(secs(1));
        assert!(matches!(r.exited(secs(90)), Decision::Restart { after } if after == secs(4)));
        assert!(matches!(r.exited(secs(1)), Decision::Restart { after } if after == secs(8)));
    }

    #[test]
    fn the_daemon_policy_gives_up_on_the_fifth_and_resets_only_when_told() {
        let mut r = RestartPolicy::daemon().restarts();
        for _ in 0..4 {
            assert!(!r.quick_exit());
        }
        assert!(r.quick_exit());
        let mut r = RestartPolicy::daemon().restarts();
        r.next_backoff();
        r.next_backoff();
        r.ran_for(secs(3600));
        assert_eq!(r.next_backoff(), Duration::from_millis(800));
        r.reset_backoff();
        assert_eq!(r.next_backoff(), Duration::from_millis(200));
    }
}
