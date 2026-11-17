//! What each Agent of a session has used this run, summed from its recording's rows.
//!
//! A recording holds the calls of the whole tree of sessions — each row names its session and that
//! session's parent — so one read gives a session and every Agent under it. It covers the latest
//! run only (see the module above); the answer says which, and never adds runs up.
//!
//! The usage summary asks for many sessions at once, again and again. At most [`CONCURRENT`] are
//! read at a time, and a session the link has seen nothing of since it was read is not asked about
//! again ([`crate::activity`]). A sleeping instance is not woken for it: what was read before is
//! given, marked stale.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use futures::stream::{self, StreamExt};
use harnix_engine_api::readers::{AgentUsage, ReadError, SessionUsage};

use super::calls::Kept;
use super::trace::run_of;
use super::{time, ProtocolReader};

/// Sessions read at once: the connection takes 64 requests in flight, and the hub's turns use it too.
const CONCURRENT: usize = 4;

#[derive(Default)]
struct Agent {
    parent: Option<String>,
    calls: u64,
    auxiliary: u64,
    turns: BTreeSet<u32>,
    models: BTreeSet<String>,
    input: u64,
    cache_read: u64,
    cache_write: u64,
    output: u64,
    model_ms: u64,
    errors: u64,
    first_at: Option<u64>,
    last_at: Option<u64>,
}

/// `session` and every Agent under it, from the rows of its root's recording; `session` first.
fn fold(kept: &Kept, session: &str, stale: bool) -> SessionUsage {
    let mut order: Vec<String> = Vec::new();
    let mut agents: HashMap<String, Agent> = HashMap::new();
    for row in &kept.rows {
        let agent = agents.entry(row.session_id.clone()).or_insert_with(|| {
            order.push(row.session_id.clone());
            Agent::default()
        });
        if agent.parent.is_none() {
            agent.parent = row.parent_session_id.clone();
        }
        if row.purpose.is_some() {
            agent.auxiliary += 1;
        } else {
            agent.calls += 1;
            agent.turns.insert(row.turn);
        }
        if !row.model.is_empty() {
            agent.models.insert(row.model.clone());
        }
        let n = |key: &str| row.usage.as_ref().and_then(|u| u[key].as_u64()).unwrap_or(0);
        agent.input += n("input_tokens");
        agent.cache_read += n("cache_read_input_tokens");
        agent.cache_write += n("cache_creation_input_tokens");
        agent.output += n("output_tokens");
        agent.model_ms += row.timing.duration_ms.unwrap_or(0);
        if matches!(row.status.as_str(), "error" | "cancelled") {
            agent.errors += 1;
        }
        let sent = time::ms(&row.timing.sent_at).or_else(|| time::ms(&row.ts));
        if let Some(sent) = sent {
            agent.first_at = Some(agent.first_at.map_or(sent, |t| t.min(sent)));
            let ended = sent + row.timing.duration_ms.unwrap_or(0);
            agent.last_at = Some(agent.last_at.map_or(ended, |t| t.max(ended)));
        }
    }
    let tree = tree_of(kept, session);
    let under = |id: &str| tree.contains(id);
    order.sort_by_key(|id| id != session);
    let listed: Vec<AgentUsage> = order
        .iter()
        .filter(|id| under(id))
        .map(|id| {
            let a = &agents[id];
            let mut u = AgentUsage::new(id);
            u.parent_session_id = a.parent.clone();
            u.calls = a.calls;
            u.auxiliary = a.auxiliary;
            u.turns = a.turns.len();
            u.models = a.models.iter().cloned().collect();
            u.input_tokens = a.input;
            u.cache_read_tokens = a.cache_read;
            u.cache_write_tokens = a.cache_write;
            u.output_tokens = a.output;
            u.model_ms = a.model_ms;
            u.errors = a.errors;
            u.first_at = a.first_at;
            u.last_at = a.last_at;
            u
        })
        .collect();
    SessionUsage::new(kept.run.is_some(), listed).run(run_of(kept)).stale(stale && kept.run.is_some())
}

/// The sessions of `kept`'s rows under `session`: itself, and whatever its parents lead up to it.
pub(super) fn tree_of(kept: &Kept, session: &str) -> HashSet<String> {
    let mut parents: HashMap<&str, &str> = HashMap::new();
    for row in &kept.rows {
        if let Some(parent) = row.parent_session_id.as_deref() {
            parents.entry(row.session_id.as_str()).or_insert(parent);
        }
    }
    let under = |id: &str| {
        let mut at = id;
        for _ in 0..=16 {
            if at == session {
                return true;
            }
            match parents.get(at) {
                Some(parent) => at = parent,
                None => return false,
            }
        }
        false
    };
    kept.rows.iter().map(|r| r.session_id.as_str()).filter(|id| under(id)).map(str::to_string).collect()
}

impl ProtocolReader {
    pub(super) async fn usage_of(&self, sessions: &[String]) -> Result<BTreeMap<String, SessionUsage>, ReadError> {
        let read: Vec<(String, Result<SessionUsage, ReadError>)> = stream::iter(sessions.iter().cloned())
            .map(|session| async move {
                let one = self.usage_one(&session).await;
                (session, one)
            })
            .buffer_unordered(CONCURRENT)
            .collect()
            .await;
        let mut out = BTreeMap::new();
        for (session, one) in read {
            match one {
                Ok(usage) => {
                    out.insert(session, usage);
                }
                // The instance cannot be reached: no session of it can be read.
                Err(e @ ReadError::Unavailable(_)) => return Err(e),
                Err(e) => {
                    tracing::warn!(session, error = %e, "a session's usage was not read");
                    out.insert(session, SessionUsage::new(false, Vec::new()));
                }
            }
        }
        Ok(out)
    }

    async fn usage_one(&self, session: &str) -> Result<SessionUsage, ReadError> {
        let root = self.calls.root_of(self, session, false).await?;
        let kept = self.calls.read(self, &root, false).await?;
        Ok(fold(&kept, session, self.asleep()))
    }
}
