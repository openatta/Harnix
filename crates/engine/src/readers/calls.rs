//! The calls of each root session's recording, as far as they were read.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use harnix_contract::sync::lock;
use harnix_engine_api::readers::ReadError;
use serde_json::json;

use super::wire::{self, CallList, Lineage, Place, Row, RunInfo};
use super::ProtocolReader;
use crate::translate::{SESSION_CALLS, SESSION_GET};

/// At most this many roots kept; past it, the one asked about longest ago goes.
const KEPT_ROOTS: usize = 128;
/// A page: the protocol's most.
const PAGE: usize = 500;
/// How often the engine is asked again about one root, at most, however often it is asked of.
pub(super) const FRESH: Duration = Duration::from_millis(1000);
/// How far up from a sub-agent's session its root is looked for.
const DEPTH: usize = 8;
/// At most this many sessions' roots kept.
const KEPT_PLACES: usize = 4096;

/// The calls of one root session's recording, as far as they were read.
#[derive(Clone, Default)]
pub(super) struct Kept {
    pub run: Option<RunInfo>,
    /// By `index`: every session of the tree.
    pub rows: Vec<Row>,
    pub next: usize,
    pub damaged: usize,
    /// A generation other than `run`'s was seen for this root before.
    pub earlier_run: bool,
    read_at: Option<Instant>,
    asked: u64,
}

impl Kept {
    /// A call that is still streaming is somewhere from `next` on: those rows are read again.
    fn open(&self) -> bool {
        self.rows.get(self.next..).is_some_and(|rows| rows.iter().any(|r| r.status == "in_flight"))
    }
}

pub(super) struct Calls {
    by_root: Mutex<HashMap<String, Kept>>,
    /// Each session's root, as [`Calls::root_of`] found it.
    roots: Mutex<HashMap<String, String>>,
    clock: AtomicU64,
    /// However often asked, the engine is asked again about one root at most this often.
    fresh: Duration,
}

impl Calls {
    pub(super) fn new(fresh: Duration) -> Self {
        Self { by_root: Mutex::default(), roots: Mutex::default(), clock: AtomicU64::new(0), fresh }
    }

    fn tick(&self) -> u64 {
        self.clock.fetch_add(1, Ordering::Relaxed)
    }

    /// What was read of `root`'s recording, read on as far as the engine has it when it may have
    /// moved. `wake` false (for usage): a sleeping instance is not asked, what was kept is
    /// returned.
    pub(super) async fn read(&self, r: &ProtocolReader, root: &str, wake: bool) -> Result<Kept, ReadError> {
        let mut kept = lock(&self.by_root).get(root).cloned();
        if let Some(k) = &mut kept {
            k.asked = self.tick();
            if let Some(at) = k.read_at {
                let fresh = at.elapsed() < self.fresh;
                if fresh || (!k.open() && !r.activity.may_have_changed(root, at)) {
                    lock(&self.by_root).insert(root.to_string(), k.clone());
                    return Ok(k.clone());
                }
            }
        }
        if !wake && r.asleep() {
            return Ok(kept.unwrap_or_default());
        }
        let mut kept = kept.unwrap_or_default();
        let read_at = Instant::now();
        let mut started_over = false;
        let mut from = kept.next;
        loop {
            let mut params = json!({"session_id": root, "from": from, "limit": PAGE});
            if let Some(run) = &kept.run {
                params["generation"] = json!(run.generation);
            }
            let list: CallList = match r.send(SESSION_CALLS, params).await {
                Ok(answer) => wire::parse(answer, SESSION_CALLS)?,
                Err(e) if wire::restarted(&e).is_some() => {
                    if started_over {
                        return Err(ReadError::Unreadable("the recording was started over again while it was read".into()));
                    }
                    started_over = true;
                    kept = Kept { earlier_run: true, asked: kept.asked, ..Kept::default() };
                    from = 0;
                    continue;
                }
                Err(e) => return Err(wire::read_error(e)),
            };
            let Some(run) = list.recording else {
                kept = Kept { asked: kept.asked, ..Kept::default() };
                break;
            };
            kept.run.get_or_insert(run);
            let none = list.calls.is_empty();
            for row in list.calls {
                match row.index.cmp(&kept.rows.len()) {
                    std::cmp::Ordering::Less => {
                        let at = row.index;
                        kept.rows[at] = row;
                    }
                    std::cmp::Ordering::Equal => kept.rows.push(row),
                    std::cmp::Ordering::Greater => {
                        tracing::warn!(root, index = row.index, kept = kept.rows.len(), "the engine skipped a call: the recording is read in part");
                        kept.rows.push(row);
                    }
                }
            }
            kept.next = list.next;
            kept.damaged = list.damaged;
            // Stopped at the same place: a call still streams, and is read again next time.
            if list.next >= list.total || list.next == from || none {
                break;
            }
            from = list.next;
        }
        kept.read_at = Some(read_at);
        let mut by_root = lock(&self.by_root);
        by_root.insert(root.to_string(), kept.clone());
        if by_root.len() > KEPT_ROOTS {
            let oldest = by_root.iter().min_by_key(|(_, k)| k.asked).map(|(id, _)| id.clone());
            if let Some(id) = oldest {
                by_root.remove(&id);
            }
        }
        Ok(kept)
    }

    /// Forget what was read of `root`: its recording was started over.
    pub(super) fn forget(&self, root: &str) {
        lock(&self.by_root).remove(root);
    }

    /// The session whose recording holds `session`'s calls: the primary session its sub-agents
    /// hang from. A primary session records under its own id whether or not it has called the
    /// model yet — a fork too, though it names its source as its parent — and a sub-agent's calls
    /// are in its root's. Remembered once the engine said: where a session is in its tree never
    /// changes.
    pub(super) async fn root_of(&self, r: &ProtocolReader, session: &str, wake: bool) -> Result<String, ReadError> {
        let mut at = session.to_string();
        for _ in 0..=DEPTH {
            let known = lock(&self.roots).get(&at).cloned();
            if let Some(known) = known {
                return Ok(self.remember(session, known));
            }
            if !wake && r.asleep() {
                // Not asked while it sleeps: taken for its own root, and not remembered.
                return Ok(at);
            }
            let lineage: Lineage = r.ask(SESSION_GET, json!({"session_id": at})).await?;
            match lineage.place() {
                Place::Root => return Ok(self.remember(session, at)),
                Place::Under(parent) => at = parent,
                Place::Unsaid => {
                    tracing::warn!(session = %at, "the engine did not say where the session is in its tree; read as a root");
                    return Ok(at);
                }
            }
        }
        tracing::warn!(session, "no primary session within {DEPTH} parents; read from the last one");
        Ok(at)
    }

    fn remember(&self, session: &str, root: String) -> String {
        let mut roots = lock(&self.roots);
        if roots.len() >= KEPT_PLACES && !roots.contains_key(session) {
            // Any one: it is asked of the engine again when it is next read.
            if let Some(any) = roots.keys().next().cloned() {
                roots.remove(&any);
            }
        }
        roots.insert(session.to_string(), root.clone());
        root
    }
}
