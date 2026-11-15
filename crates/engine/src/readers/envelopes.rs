//! What a call was sent besides the messages, once per change.
//!
//! The rows of the list say enough to tell most changes: the model, the system blocks' names and
//! lengths, the number of tools. They do not say all of them: a block of the same length with other
//! words, or `max_tokens` or the thinking mode changed alone, are not seen. A call that begins an
//! envelope or changes it by the rows is taken in full; one that looks like the one before is not.

use std::collections::HashMap;
use std::sync::Mutex;

use harnix_contract::sync::lock;
use harnix_engine_api::readers::{Envelope, Envelopes, ReadError};
use serde_json::json;

use super::trace::run_of;
use super::wire::{self, Detail, RunInfo};
use super::{time, ProtocolReader};
use crate::translate::SESSION_CALL;

/// At most this many roots' envelopes kept; past it, the one asked about longest ago goes.
const KEPT_ROOTS: usize = 16;
/// Each root's cap of envelopes kept.
const KEPT_PER_ROOT: usize = 256;
/// A call's size, to the byte, is not in a row: its block names and lengths are.
type Coarse = (String, String, Vec<(Option<String>, Option<usize>)>, Option<usize>);

fn coarse(row: &wire::Row) -> Coarse {
    (
        row.provider.clone(),
        row.model.clone(),
        row.input.system.iter().map(|b| (b.source.clone(), b.chars)).collect(),
        row.input.tools,
    )
}

/// Full envelopes by root, generation and call index.
#[derive(Default)]
pub(super) struct Kept(Mutex<Roots>);

#[derive(Default)]
struct Roots {
    by_root: HashMap<String, Held>,
    clock: u64,
}

#[derive(Default)]
struct Held {
    generation: u64,
    by_index: HashMap<usize, Envelope>,
    asked: u64,
}

impl Kept {
    fn get(&self, root: &str, generation: u64, index: usize) -> Option<Envelope> {
        let mut roots = lock(&self.0);
        roots.clock += 1;
        let clock = roots.clock;
        let held = roots.by_root.get_mut(root)?;
        held.asked = clock;
        (held.generation == generation).then(|| held.by_index.get(&index).cloned()).flatten()
    }

    fn put(&self, root: &str, generation: u64, index: usize, envelope: Envelope) {
        let mut roots = lock(&self.0);
        roots.clock += 1;
        let clock = roots.clock;
        let held = roots.by_root.entry(root.to_string()).or_default();
        if held.generation != generation {
            *held = Held { generation, ..Held::default() };
        }
        held.asked = clock;
        if held.by_index.len() < KEPT_PER_ROOT {
            held.by_index.insert(index, envelope);
        }
        if roots.by_root.len() > KEPT_ROOTS {
            let oldest = roots.by_root.iter().min_by_key(|(_, h)| h.asked).map(|(id, _)| id.clone());
            if let Some(id) = oldest {
                roots.by_root.remove(&id);
            }
        }
    }
}

impl ProtocolReader {
    pub(super) async fn envelopes_of(&self, session: &str) -> Result<Envelopes, ReadError> {
        let root = self.calls.root_of(self, session, true).await?;
        let kept = self.calls.read(self, &root, true).await?;
        let Some(run) = kept.run.clone() else {
            return Ok(Envelopes::new(false, Vec::new(), 0, 0, 0));
        };
        let (mut headers, mut calls, mut auxiliary) = (Vec::new(), 0u64, 0u64);
        let mut previous: Option<Coarse> = None;
        for row in kept.rows.iter().filter(|r| r.session_id == session) {
            if row.purpose.is_some() {
                auxiliary += 1;
                continue;
            }
            calls += 1;
            let now = coarse(row);
            let reason = match &previous {
                None => "initial",
                Some(before) if *before != now => "changed",
                Some(_) => continue,
            };
            previous = Some(now);
            headers.push(self.envelope_at(&root, &run, row, reason).await?);
        }
        Ok(Envelopes::new(true, headers, calls, auxiliary, kept.damaged).run(run_of(&kept)))
    }

    /// The envelope of the call `row` is, from what was kept or the call in full.
    async fn envelope_at(&self, root: &str, run: &RunInfo, row: &wire::Row, reason: &str) -> Result<Envelope, ReadError> {
        if let Some(mut held) = self.envelopes.get(root, run.generation, row.index) {
            held.reason = reason.to_string();
            return Ok(held);
        }
        let params = json!({"session_id": root, "index": row.index, "generation": run.generation});
        let detail: Detail = match self.send(SESSION_CALL, params).await {
            Ok(answer) => wire::parse(answer, SESSION_CALL)?,
            Err(e) if wire::restarted(&e).is_some() => {
                self.calls.forget(root);
                return Err(ReadError::Missing("the recording was started over: read the envelopes again".into()));
            }
            Err(e) => return Err(wire::read_error(e)),
        };
        let request = detail.request;
        let mut budget = 16 * 1024 * 1024;
        let system = self.system_of(root, request.system, &mut budget).await;
        let tools = self.tools_of(root, request.tools, &mut budget).await;
        let envelope = Envelope::new(
            reason,
            request.turn,
            request.step,
            time::ms(&row.ts).unwrap_or(0),
            request.provider,
            request.model,
            request.max_tokens,
            request.thinking_mode,
            system,
            tools,
        );
        self.envelopes.put(root, run.generation, row.index, envelope.clone());
        Ok(envelope)
    }
}
