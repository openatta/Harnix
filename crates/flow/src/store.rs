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
/// some written. The run summaries `list` answers from are read from the directory once
/// and then kept up to date by this store's own commits — one process per directory.
/// Disk work runs on the blocking pool when there is a Tokio runtime. A run id with a path
/// separator, or starting with `.`, names no run.
#[derive(Clone)]
pub struct FileStore {
    inner: std::sync::Arc<Files>,
}

struct Files {
    dir: PathBuf,
    /// id → summary, once read.
    index: Mutex<Option<BTreeMap<String, RunSummary>>>,
}

/// Run `f` off the async runtime's worker threads, when there is one.
async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, StoreError> + Send + 'static,
) -> Result<T, StoreError> {
    match tokio::runtime::Handle::try_current() {
        Ok(rt) => rt.spawn_blocking(f).await.map_err(other)?,
        Err(_) => f(),
    }
}

/// Whether `id` can be a file name in the directory.
fn safe_id(id: &str) -> bool {
    !id.is_empty() && !id.contains(['/', '\\', '\0']) && !id.starts_with('.')
}

impl FileStore {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self {
            inner: std::sync::Arc::new(Files {
                dir: dir.into(),
                index: Mutex::new(None),
            }),
        }
    }

    pub fn dir(&self) -> &Path {
        &self.inner.dir
    }
}

impl Files {
    fn record_path(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{id}.json"))
    }

    fn history_path(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{id}.history.jsonl"))
    }

    fn read_record(&self, id: &str) -> Result<Option<RunRecord>, StoreError> {
        if !safe_id(id) {
            return Ok(None);
        }
        match std::fs::read(self.record_path(id)) {
            Ok(bytes) => serde_json::from_slice(&bytes).map(Some).map_err(other),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(other(e)),
        }
    }

    /// Write to a temp file, flush it to disk, rename it over `path`: after a power cut the file
    /// is the old one or the new one, never empty. The directory is flushed too when the file is
    /// new — a rename over an existing file leaves a whole file there either way.
    fn write_atomic(&self, path: &Path, bytes: &[u8]) -> Result<(), StoreError> {
        let new = !path.exists();
        let tmp = path.with_extension(format!("tmp-{}", std::process::id()));
        let mut f = std::fs::File::create(&tmp).map_err(other)?;
        f.write_all(bytes).map_err(other)?;
        flush(&f).map_err(other)?;
        drop(f);
        std::fs::rename(&tmp, path).map_err(other)?;
        if new {
            self.sync_dir()?;
        }
        Ok(())
    }

    /// Flush the directory itself, which is where a rename or a new file is recorded.
    fn sync_dir(&self) -> Result<(), StoreError> {
        std::fs::File::open(&self.dir)
            .and_then(|d| flush(&d))
            .map_err(other)
    }

    fn ensure_dir(&self) -> Result<(), StoreError> {
        std::fs::create_dir_all(&self.dir).map_err(other)
    }

    fn lock_index(
        &self,
    ) -> Result<std::sync::MutexGuard<'_, Option<BTreeMap<String, RunSummary>>>, StoreError> {
        self.index
            .lock()
            .map_err(|_| other("file store index poisoned"))
    }

    /// Every run's summary, from the directory.
    fn scan(&self) -> Result<BTreeMap<String, RunSummary>, StoreError> {
        let entries = match std::fs::read_dir(&self.dir) {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
            Err(e) => return Err(other(e)),
        };
        let mut all = BTreeMap::new();
        for entry in entries {
            let name = entry
                .map_err(other)?
                .file_name()
                .to_string_lossy()
                .into_owned();
            let Some(id) = name.strip_suffix(".json") else {
                continue;
            };
            if name == COUNTERS || id.contains(".tmp-") {
                continue;
            }
            // One record that does not read — a crash's leftover, a file someone edited — is
            // that run's problem: listing every other run goes on.
            match self.read_record(id) {
                Ok(Some(r)) => {
                    all.insert(r.id.clone(), r.summary());
                }
                Ok(None) => {}
                Err(e) => tracing::warn!(run = id, dir = %self.dir.display(), error = %e, "a run record that does not read is left out"),
            }
        }
        Ok(all)
    }

    fn alloc_id(&self, flow: &str) -> Result<String, StoreError> {
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

    fn list(&self, q: &RunQuery) -> Result<Vec<RunSummary>, StoreError> {
        let mut index = self.lock_index()?;
        if index.is_none() {
            *index = Some(self.scan()?);
        }
        let all = index
            .as_ref()
            .map(|m| m.values().cloned().collect())
            .unwrap_or_default();
        Ok(q.apply(all))
    }

    fn commit(&self, cs: &ChangeSet) -> Result<(), StoreError> {
        let _g = guard()?;
        if let Some((bad, _)) = cs.records.iter().find(|(r, _)| !safe_id(&r.id)) {
            return Err(other(format!("{:?} cannot name a run", bad.id)));
        }
        if let Some(h) = cs.history.iter().find(|h| !safe_id(&h.run)) {
            return Err(other(format!("{:?} cannot name a run", h.run)));
        }
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
        if let Some(index) = self.lock_index()?.as_mut() {
            for (rec, _) in &cs.records {
                index.insert(rec.id.clone(), rec.summary());
            }
        }
        let mut by_run: BTreeMap<&str, String> = BTreeMap::new();
        for h in &cs.history {
            let line = serde_json::to_string(h).map_err(other)?;
            let buf = by_run.entry(h.run.as_str()).or_default();
            buf.push_str(&line);
            buf.push('\n');
        }
        for (run, lines) in by_run {
            let path = self.history_path(run);
            let new = !path.exists();
            let mut f = std::fs::OpenOptions::new()
                .create(true)
                .read(true)
                .append(true)
                .open(&path)
                .map_err(other)?;
            cut_torn_tail(&mut f).map_err(other)?;
            f.write_all(lines.as_bytes()).map_err(other)?;
            flush(&f).map_err(other)?;
            if new {
                self.sync_dir()?;
            }
        }
        Ok(())
    }

    fn history(&self, run: &str) -> Result<Vec<History>, StoreError> {
        if !safe_id(run) {
            return Ok(vec![]);
        }
        let text = match std::fs::read_to_string(self.history_path(run)) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
            Err(e) => return Err(other(e)),
        };
        let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
        let mut out = Vec::with_capacity(lines.len());
        for (n, line) in lines.iter().enumerate() {
            match serde_json::from_str(line) {
                Ok(h) => out.push(h),
                // The last line, half written when the process stopped: what came before stands.
                // The next commit cuts it off before appending.
                Err(e) if n + 1 == lines.len() && !text.ends_with('\n') => {
                    tracing::warn!(run, error = %e, "the history's last line was cut short; it is left out");
                }
                Err(e) => return Err(other(e)),
            }
        }
        Ok(out)
    }
}

/// Hand a file's writes to the drive: `fsync(2)`.
///
/// Not `File::sync_all` on macOS, where std makes it `F_FULLFSYNC`, which also empties the drive's
/// own cache: about 10 ms a call against a fraction of one, and a commit makes two or three under
/// the store's process-wide lock. That made commits slow enough that concurrent ones on a run
/// used up their retries and a gate's result was lost. Plain `fsync` is what Linux's gives and
/// what SQLite does by default.
fn flush(f: &std::fs::File) -> std::io::Result<()> {
    #[cfg(target_vendor = "apple")]
    {
        use std::os::unix::io::AsRawFd as _;
        // SAFETY: a valid descriptor for as long as `f` lives; fsync touches nothing else.
        if unsafe { libc::fsync(f.as_raw_fd()) } == 0 {
            Ok(())
        } else {
            Err(std::io::Error::last_os_error())
        }
    }
    #[cfg(not(target_vendor = "apple"))]
    {
        f.sync_data()
    }
}

/// A history file whose last line has no newline was cut short mid-append: drop that partial
/// line, so the next append starts on a line of its own instead of running into it.
fn cut_torn_tail(f: &mut std::fs::File) -> std::io::Result<()> {
    use std::io::{Read, Seek, SeekFrom};
    let len = f.metadata()?.len();
    if len == 0 {
        return Ok(());
    }
    let mut last = [0u8; 1];
    f.seek(SeekFrom::Start(len - 1))?;
    f.read_exact(&mut last)?;
    if last[0] == b'\n' {
        return Ok(());
    }
    // Back to the newline before the torn line, a block at a time.
    let mut end = len;
    let mut keep = 0;
    let mut block = vec![0u8; 4096];
    while end > 0 {
        let start = end.saturating_sub(block.len() as u64);
        let part = &mut block[..(end - start) as usize];
        f.seek(SeekFrom::Start(start))?;
        f.read_exact(part)?;
        if let Some(at) = part.iter().rposition(|b| *b == b'\n') {
            keep = start + at as u64 + 1;
            break;
        }
        end = start;
    }
    tracing::warn!(cut = len - keep, "a history line cut short is dropped before the next append");
    f.set_len(keep)
}

fn guard() -> Result<std::sync::MutexGuard<'static, ()>, StoreError> {
    FILE_LOCK
        .lock()
        .map_err(|_| other("file store lock poisoned"))
}

#[async_trait]
impl RunStore for FileStore {
    async fn alloc_id(&self, flow: &str) -> Result<String, StoreError> {
        let (files, flow) = (self.inner.clone(), flow.to_string());
        blocking(move || files.alloc_id(&flow)).await
    }

    async fn load(&self, run: &str) -> Result<Option<RunRecord>, StoreError> {
        let (files, run) = (self.inner.clone(), run.to_string());
        blocking(move || files.read_record(&run)).await
    }

    async fn list(&self, q: &RunQuery) -> Result<Vec<RunSummary>, StoreError> {
        let (files, q) = (self.inner.clone(), q.clone());
        blocking(move || files.list(&q)).await
    }

    async fn commit(&self, cs: &ChangeSet) -> Result<(), StoreError> {
        let (files, cs) = (self.inner.clone(), cs.clone());
        blocking(move || files.commit(&cs)).await
    }

    async fn history(&self, run: &str) -> Result<Vec<History>, StoreError> {
        let (files, run) = (self.inner.clone(), run.to_string());
        blocking(move || files.history(&run)).await
    }
}
