//! `schedules.json` in the extension's data directory: every schedule with its receipts, written
//! whole to a temporary file and renamed over the old one, so a crash leaves one or the other.
//! Not kept readable across versions: a file this version cannot read is set aside and the
//! schedules start empty (concept §10).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::wire::{Receipt, Schedule};

/// Receipts kept per schedule: the latest 200, of the last 30 days; the latest always.
pub const HISTORY_RECORDS: usize = 200;
pub const HISTORY_DAYS: u64 = 30;

/// One schedule as it is kept.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Kept {
    pub schedule: Schedule,
    /// Oldest first.
    #[serde(default)]
    pub history: Vec<Receipt>,
    /// When it was last delivered: where the minimum interval counts from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_delivered_at: Option<u64>,
}

impl Kept {
    /// Add or replace (by id) a receipt, and drop what is past keeping.
    pub fn record(&mut self, receipt: Receipt, now: u64) {
        match self.history.iter_mut().find(|r| r.id == receipt.id) {
            Some(kept) => *kept = receipt.clone(),
            None => self.history.push(receipt.clone()),
        }
        self.schedule.last_delivery = Some(receipt);
        let horizon = now.saturating_sub(HISTORY_DAYS * 86_400_000);
        let last = self.history.len().saturating_sub(1);
        let mut i = 0;
        self.history.retain(|r| {
            let keep = i == last || r.scheduled_at >= horizon;
            i += 1;
            keep
        });
        if self.history.len() > HISTORY_RECORDS {
            let extra = self.history.len() - HISTORY_RECORDS;
            self.history.drain(..extra);
        }
    }
}

/// The whole file.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Ledger {
    /// Bumped by every write.
    pub revision: u64,
    pub schedules: Vec<Kept>,
}

/// Where the ledger is kept; none in tests that keep it in memory.
pub struct File {
    path: Option<PathBuf>,
}

impl File {
    pub fn at(path: Option<PathBuf>) -> Self {
        Self { path }
    }

    /// What is there; nothing, when there is no file or it does not read.
    pub fn load(&self) -> Ledger {
        let Some(path) = &self.path else { return Ledger::default() };
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ledger::default(),
            Err(e) => {
                tracing::warn!(path = %path.display(), "schedules not read: {e}");
                return Ledger::default();
            }
        };
        match serde_json::from_str(&text) {
            Ok(ledger) => ledger,
            Err(e) => {
                let aside = path.with_extension("json.unread");
                tracing::warn!(path = %path.display(), aside = %aside.display(), "schedules not read, set aside: {e}");
                let _ = std::fs::rename(path, aside);
                Ledger::default()
            }
        }
    }

    /// Write it whole, with the next revision.
    pub fn save(&self, ledger: &mut Ledger) -> std::io::Result<()> {
        ledger.revision += 1;
        let Some(path) = &self.path else { return Ok(()) };
        write_atomic(path, &serde_json::to_vec_pretty(ledger).map_err(std::io::Error::other)?)
    }
}

fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let temp = path.with_extension("json.tmp");
    {
        let mut f = std::fs::File::create(&temp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    std::fs::rename(&temp, path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::{Concurrency, Missed, Outcome, SessionTarget, Target, Timing};

    fn kept() -> Kept {
        Kept {
            schedule: Schedule {
                id: "s".into(),
                title: "t".into(),
                enabled: true,
                timing: Timing::Daily { time: "09:00".into() },
                time_zone: "UTC".into(),
                target: Target::Session(SessionTarget { instance: "main".into(), session_id: "x".into(), prompt: "p".into() }),
                missed: Missed::CatchUpLatest,
                concurrency: Concurrency::Skip,
                space: "sessions".into(),
                project: None,
                created_by: "device:d".into(),
                created_at: 0,
                revision: 1,
                next_fire: None,
                last_delivery: None,
            },
            history: vec![],
            last_delivered_at: None,
        }
    }

    fn receipt(id: usize, at: u64) -> Receipt {
        Receipt {
            id: format!("r{id}"),
            scheduled_at: at,
            delivered_at: Some(at),
            manual: false,
            outcome: Outcome::Delivered,
            reason: None,
            target: None,
        }
    }

    #[test]
    fn history_keeps_200_of_the_last_30_days_and_always_the_latest() {
        let day = 86_400_000;
        let mut k = kept();
        for i in 0..250 {
            k.record(receipt(i, 100 * day + i as u64), 100 * day + 250);
        }
        assert_eq!(k.history.len(), HISTORY_RECORDS);
        assert_eq!(k.history.first().unwrap().id, "r50");
        assert_eq!(k.schedule.last_delivery.as_ref().unwrap().id, "r249");
        // A month later, one more: everything older goes.
        k.record(receipt(300, 140 * day), 140 * day);
        assert_eq!(k.history.len(), 1);
        // A receipt replaced by id stays one.
        let mut r = receipt(300, 140 * day);
        r.outcome = Outcome::Failed;
        k.record(r, 140 * day);
        assert_eq!(k.history.len(), 1);
        assert_eq!(k.history[0].outcome, Outcome::Failed);
        // The latest stays, however old.
        let mut old = kept();
        old.record(receipt(1, 0), 400 * day);
        assert_eq!(old.history.len(), 1);
    }

    #[test]
    fn written_whole_and_read_back_and_an_unreadable_one_set_aside() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("schedules.json");
        let file = File::at(Some(path.clone()));
        assert!(file.load().schedules.is_empty());
        let mut ledger = Ledger { revision: 0, schedules: vec![kept()] };
        file.save(&mut ledger).unwrap();
        file.save(&mut ledger).unwrap();
        let back = file.load();
        assert_eq!(back.revision, 2);
        assert_eq!(back.schedules[0].schedule, kept().schedule);
        assert!(!path.with_extension("json.tmp").exists());

        std::fs::write(&path, "{not json").unwrap();
        assert!(file.load().schedules.is_empty());
        assert!(path.with_extension("json.unread").exists());
    }
}
