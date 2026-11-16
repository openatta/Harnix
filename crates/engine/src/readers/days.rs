//! What the tree of Agents under a session used this run, summed per day and model, for a summary
//! over every session of an instance.
//!
//! Such a summary asks about far more sessions than the calls kept ([`super::calls`]) hold, so
//! what was summed is kept apart, small: per session and offset, one line per day and model, and
//! when it was summed. A session the link has seen nothing of since ([`crate::activity`]) is
//! answered from that, its recording not read again; one that moved is read and summed afresh.
//! It lives as long as the reader and is bounded ([`KEPT`]); nothing of it is written down.

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Instant;

use futures::stream::{self, StreamExt};
use harnix_contract::sync::lock;
use harnix_engine_api::readers::{DayUsage, DayWindow, ReadError, RecordingRun, SessionDays};

use super::calls::Kept;
use super::trace::run_of;
use super::usage::tree_of;
use super::{time, ProtocolReader};

/// At most this many sessions' sums kept (per offset asked); past it, the one asked about longest ago goes.
const KEPT: usize = 1024;
/// Sessions read at once, as for usage.
const CONCURRENT: usize = 4;

/// One session's sums, by day and model.
#[derive(Clone)]
struct Summed {
    recording: bool,
    run: Option<RecordingRun>,
    /// By (day from 1970-01-01, model).
    days: BTreeMap<(i64, String), DayUsage>,
    /// A call was still streaming: its figures are not final.
    open: bool,
    read_at: Instant,
    asked: u64,
}

impl Summed {
    fn answer(&self, window: &DayWindow, stale: bool) -> SessionDays {
        let from = time::day_of(window.since_ms, window.offset_minutes);
        let days = self.days.range((from, String::new())..).map(|(_, d)| d.clone()).collect();
        SessionDays::new(self.recording, days).run(self.run.clone()).stale(stale && self.recording)
    }
}

#[derive(Default)]
pub(super) struct Days {
    by: Mutex<HashMap<(String, i32), Summed>>,
    clock: AtomicU64,
}

impl Days {
    fn get(&self, key: &(String, i32)) -> Option<Summed> {
        let mut by = lock(&self.by);
        let tick = self.clock.fetch_add(1, Ordering::Relaxed);
        by.get_mut(key).map(|s| {
            s.asked = tick;
            s.clone()
        })
    }

    fn put(&self, key: (String, i32), mut summed: Summed) {
        summed.asked = self.clock.fetch_add(1, Ordering::Relaxed);
        let mut by = lock(&self.by);
        by.insert(key, summed);
        if by.len() > KEPT {
            let oldest = by.iter().min_by_key(|(_, s)| s.asked).map(|(k, _)| k.clone());
            if let Some(k) = oldest {
                by.remove(&k);
            }
        }
    }

    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        lock(&self.by).len()
    }
}

/// The rows of `kept` under `session`, summed by the day each call was sent at `offset_minutes`.
fn sum(kept: &Kept, session: &str, offset_minutes: i32, read_at: Instant) -> Summed {
    let tree = tree_of(kept, session);
    let mut days: BTreeMap<(i64, String), DayUsage> = BTreeMap::new();
    let mut open = false;
    for row in kept.rows.iter().filter(|r| tree.contains(&r.session_id)) {
        open |= row.status == "in_flight";
        // A call with no time cannot be placed on a day.
        let Some(sent) = time::ms(&row.timing.sent_at).or_else(|| time::ms(&row.ts)) else {
            continue;
        };
        let day = time::day_of(sent, offset_minutes);
        let d = days.entry((day, row.model.clone())).or_insert_with(|| DayUsage::new(time::date_of(day), row.model.clone()));
        if row.purpose.is_some() {
            d.auxiliary += 1;
        } else {
            d.calls += 1;
        }
        let n = |key: &str| row.usage.as_ref().and_then(|u| u[key].as_u64()).unwrap_or(0);
        d.input_tokens += n("input_tokens");
        d.cache_read_tokens += n("cache_read_input_tokens");
        d.cache_write_tokens += n("cache_creation_input_tokens");
        d.output_tokens += n("output_tokens");
        d.model_ms += row.timing.duration_ms.unwrap_or(0);
        if matches!(row.status.as_str(), "error" | "cancelled") {
            d.errors += 1;
        }
    }
    Summed { recording: kept.run.is_some(), run: run_of(kept), days, open, read_at, asked: 0 }
}

impl ProtocolReader {
    pub(super) async fn usage_days_of(&self, sessions: &[String], window: &DayWindow) -> Result<BTreeMap<String, SessionDays>, ReadError> {
        let read: Vec<(String, Result<SessionDays, ReadError>)> = stream::iter(sessions.iter().cloned())
            .map(|session| async move {
                let one = self.usage_days_one(&session, window).await;
                (session, one)
            })
            .buffer_unordered(CONCURRENT)
            .collect()
            .await;
        let mut out = BTreeMap::new();
        for (session, one) in read {
            match one {
                Ok(days) => {
                    out.insert(session, days);
                }
                Err(e @ ReadError::Unavailable(_)) => return Err(e),
                Err(e) => {
                    tracing::warn!(session, error = %e, "a session's usage by day was not read");
                    out.insert(session, SessionDays::new(false, Vec::new()));
                }
            }
        }
        Ok(out)
    }

    async fn usage_days_one(&self, session: &str, window: &DayWindow) -> Result<SessionDays, ReadError> {
        let root = self.calls.root_of(self, session, false).await?;
        let key = (session.to_string(), window.offset_minutes);
        let asleep = self.asleep();
        if let Some(summed) = self.days.get(&key) {
            if asleep || (!summed.open && !self.activity.may_have_changed(&root, summed.read_at)) {
                return Ok(summed.answer(window, asleep));
            }
        }
        let read_at = Instant::now();
        // Asleep, the calls kept answer and nothing is asked: summed, not kept as current.
        let kept = self.calls.read(self, &root, false).await?;
        let summed = sum(&kept, session, window.offset_minutes, read_at);
        let answer = summed.answer(window, asleep);
        if !asleep {
            self.days.put(key, summed);
        }
        Ok(answer)
    }
}
