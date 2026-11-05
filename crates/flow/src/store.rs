//! Two [`RunStore`]s: [`MemStore`] for tests, [`FileStore`] for a directory.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use async_trait::async_trait;

use crate::kernel::host::{ChangeSet, RunStore, StoreError};
use crate::kernel::run::{History, RunQuery, RunRecord, RunSummary};

fn other(e: impl std::fmt::Display) -> StoreError {
    StoreError::Other(e.to_string())
}

/// Check every expected rev of a change set against `current`. All or nothing.
fn check_revs(cs: &ChangeSet, current: impl Fn(&str) -> Option<u64>) -> Result<(), StoreError> {
    for (rec, expected) in &cs.records {
        let have = current(&rec.id);
        if have != *expected {
            return Err(StoreError::Conflict(format!(
                "run {}: stored rev {have:?}, expected {expected:?}",
                rec.id
            )));
        }
    }
    Ok(())
}

// ── memory ───────────────────────────────────────────────────────────────────

#[derive(Default)]
struct Mem {
    runs: BTreeMap<String, RunRecord>,
    history: BTreeMap<String, Vec<History>>,
    counters: BTreeMap<String, u64>,
}

/// Runs in memory behind one mutex.
#[derive(Default)]
pub struct MemStore {
    inner: Mutex<Mem>,
}

impl MemStore {
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, Mem>, StoreError> {
        self.inner.lock().map_err(|_| other("store lock poisoned"))
    }
}

#[async_trait]
impl RunStore for MemStore {
    async fn alloc_id(&self, flow: &str) -> Result<String, StoreError> {
        let mut m = self.lock()?;
        let n = m.counters.entry(flow.to_string()).or_insert(0);
        *n += 1;
        Ok(format!("{flow}-{n}"))
    }

    async fn load(&self, run: &str) -> Result<Option<RunRecord>, StoreError> {
        Ok(self.lock()?.runs.get(run).cloned())
    }

    async fn list(&self, q: &RunQuery) -> Result<Vec<RunSummary>, StoreError> {
        let all = self.lock()?.runs.values().map(RunRecord::summary).collect();
        Ok(q.apply(all))
    }

    async fn commit(&self, cs: &ChangeSet) -> Result<(), StoreError> {
        let mut m = self.lock()?;
        check_revs(cs, |id| m.runs.get(id).map(|r| r.rev))?;
        for (rec, _) in &cs.records {
            m.runs.insert(rec.id.clone(), rec.clone());
        }
        for h in &cs.history {
            m.history.entry(h.run.clone()).or_default().push(h.clone());
        }
        Ok(())
    }

    async fn history(&self, run: &str) -> Result<Vec<History>, StoreError> {
        Ok(self.lock()?.history.get(run).cloned().unwrap_or_default())
    }
}

// ── files ────────────────────────────────────────────────────────────────────

/// Serialises commits of every [`FileStore`] in the process, so the rev comparison and
/// the write are one step.
static FILE_LOCK: Mutex<()> = Mutex::new(());

const COUNTERS: &str = "ids.json";

/// One directory: `<id>.json` per run (written to a temp file, then renamed),
/// `<id>.history.jsonl` appended alongside, and `ids.json` holding the id counters.
///
/// Atomicity is per process: revs are compared under a process-wide mutex. A commit that
/// touches several runs writes them one after another; a crash in between can leave
/// some written.
pub struct FileStore {
    dir: PathBuf,
}

impl FileStore {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn record_path(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{id}.json"))
    }

    fn history_path(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{id}.history.jsonl"))
    }

    fn read_record(&self, id: &str) -> Result<Option<RunRecord>, StoreError> {
        if id.is_empty() || id.contains(['/', '\\']) || id.starts_with('.') {
            return Ok(None);
        }
        match std::fs::read(self.record_path(id)) {
            Ok(bytes) => serde_json::from_slice(&bytes).map(Some).map_err(other),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(other(e)),
        }
    }

    fn write_atomic(&self, path: &Path, bytes: &[u8]) -> Result<(), StoreError> {
        let tmp = path.with_extension(format!("tmp-{}", std::process::id()));
        std::fs::write(&tmp, bytes).map_err(other)?;
        std::fs::rename(&tmp, path).map_err(other)
    }

    fn ensure_dir(&self) -> Result<(), StoreError> {
        std::fs::create_dir_all(&self.dir).map_err(other)
    }
}

fn guard() -> Result<std::sync::MutexGuard<'static, ()>, StoreError> {
    FILE_LOCK
        .lock()
        .map_err(|_| other("file store lock poisoned"))
}

#[async_trait]
impl RunStore for FileStore {
    async fn alloc_id(&self, flow: &str) -> Result<String, StoreError> {
        let _g = guard()?;
        self.ensure_dir()?;
        let path = self.dir.join(COUNTERS);
        let mut counters: BTreeMap<String, u64> = match std::fs::read(&path) {
            Ok(b) => serde_json::from_slice(&b).map_err(other)?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => BTreeMap::new(),
            Err(e) => return Err(other(e)),
        };
        let n = counters.entry(flow.to_string()).or_insert(0);
        *n += 1;
        let id = format!("{flow}-{n}");
        let bytes = serde_json::to_vec_pretty(&counters).map_err(other)?;
        self.write_atomic(&path, &bytes)?;
        Ok(id)
    }

    async fn load(&self, run: &str) -> Result<Option<RunRecord>, StoreError> {
        self.read_record(run)
    }

    async fn list(&self, q: &RunQuery) -> Result<Vec<RunSummary>, StoreError> {
        let entries = match std::fs::read_dir(&self.dir) {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
            Err(e) => return Err(other(e)),
        };
        let mut all = vec![];
        for entry in entries {
            let name = entry
                .map_err(other)?
                .file_name()
                .to_string_lossy()
                .into_owned();
            let Some(id) = name.strip_suffix(".json") else {
                continue;
            };
            if name == COUNTERS {
                continue;
            }
            if let Some(r) = self.read_record(id)? {
                all.push(r.summary());
            }
        }
        Ok(q.apply(all))
    }

    async fn commit(&self, cs: &ChangeSet) -> Result<(), StoreError> {
        let _g = guard()?;
        self.ensure_dir()?;
        let mut current = BTreeMap::new();
        for (rec, _) in &cs.records {
            current.insert(rec.id.clone(), self.read_record(&rec.id)?.map(|r| r.rev));
        }
        check_revs(cs, |id| current.get(id).copied().flatten())?;
        for (rec, _) in &cs.records {
            let bytes = serde_json::to_vec_pretty(rec).map_err(other)?;
            self.write_atomic(&self.record_path(&rec.id), &bytes)?;
        }
        let mut by_run: BTreeMap<&str, String> = BTreeMap::new();
        for h in &cs.history {
            let line = serde_json::to_string(h).map_err(other)?;
            let buf = by_run.entry(h.run.as_str()).or_default();
            buf.push_str(&line);
            buf.push('\n');
        }
        for (run, lines) in by_run {
            let mut f = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(self.history_path(run))
                .map_err(other)?;
            f.write_all(lines.as_bytes()).map_err(other)?;
        }
        Ok(())
    }

    async fn history(&self, run: &str) -> Result<Vec<History>, StoreError> {
        let text = match std::fs::read_to_string(self.history_path(run)) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
            Err(e) => return Err(other(e)),
        };
        text.lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| serde_json::from_str(l).map_err(other))
            .collect()
    }
}
