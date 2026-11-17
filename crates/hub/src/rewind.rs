//! Going back to an earlier point of a session: its files, its conversation or both, as far as the
//! engine kept them (`harnix.session.rewind`, `docs/harnix-app.md` §5.1).
//!
//! The engine does the rewinding and keeps its log append-only: the turns it takes out stay on
//! disk, marked void, and the history it answers no longer has them. What the hub holds of the
//! session is drawn past that point — the catch-up buffer, the entries it filed for those turns —
//! so it is dropped here, and every watcher attaches again and reads the history as it now is.
//! A rewind writes project files: a person confirms it after a dry run shows what it would do,
//! which is the interface's half.

use std::sync::Arc;

use harnix_contract::sync::lock;
use harnix_contract::wire::{self, RewindParams, RewindResult, RewindTo, SessionResync};
use harnix_contract::{Frame, RpcError, Subject};
use harnix_engine_link::ops::{codes, Entry};
use harnix_kernel::{CallContext, Rpc};
use serde_json::{json, Value};

use crate::{notification, Hub, SessionKey};

impl Hub {
    /// `harnix.session.rewind {instance?, session_id, space?, to, scope?, dry_run?, fork_first?,
    /// force?, allow_partial?}`: a device rewinds only a session of the space it claims; an
    /// extension holds `engine.control`. Refused while a turn of the session runs or waits.
    pub(crate) async fn rewind(self: &Arc<Self>, cx: &CallContext, params: Value) -> Rpc<Value> {
        let mut ask: RewindParams =
            serde_json::from_value(params.clone()).map_err(|e| RpcError::invalid_params(format!("harnix.session.rewind: {e}")))?;
        let key = self.key_of(&params)?;
        if matches!(cx.subject, Subject::Device { .. }) {
            self.within_own_space(&key, &params, "a session is rewound")?;
        }
        if let Some(slot) = self.slot(&key.0) {
            let (space, crosses) = self.claim_of(&params);
            crate::spaces::may_use(&slot, cx, &space, crosses)?;
        }
        let linked = self.engine(&key.0)?;
        {
            // Checked and taken under one lock: two rewinds of one session do not both start.
            let mut guard = self.inner.lock().await;
            let state = guard.sessions.entry(key.clone()).or_default();
            if state.running_turn.is_some() || !state.queue.is_empty() || state.rewinding {
                return Err(RpcError::with_data(
                    codes::SESSION_BUSY,
                    "the session has a turn running or waiting, or a rewind under way; rewind once it is idle",
                    json!({"session_id": key.1, "current_turn_id": state.running_turn}),
                ));
            }
            state.rewinding = !ask.dry_run;
        }
        let outcome = async {
            if let RewindTo::BeforeTurn(turn) = &ask.to {
                // The engine lists no turn a rewind of the conversation took out.
                let live = linked.ops().live_turns(&key.1).await?;
                ask.to = before(&live, turn)?;
            }
            let done = linked.ops().rewind(&key.1, &ask).await?;
            // The engine's frame for it is routed before the hub says it is over, so the frame is
            // not taken for a rewind somebody else made.
            if !done.dry_run {
                self.barrier(&key.0).await;
            }
            Ok::<_, RpcError>(done)
        }
        .await;
        if let Some(state) = self.inner.lock().await.sessions.get_mut(&key) {
            state.rewinding = false;
        }
        let mut done = outcome?;
        done.instance = key.0.clone();
        if !done.dry_run {
            self.rewound(&key, &cx.subject, &done).await;
        }
        Ok(wire::params(&done))
    }

    /// After a rewind: the changes of the turns it took out are voided in the log, a fork it made
    /// first lands beside the session, and every watcher reads the session again.
    async fn rewound(self: &Arc<Self>, key: &SessionKey, subject: &Subject, done: &RewindResult) {
        let removed: Vec<String> =
            done.conversation.iter().flat_map(|c| c.removed_turns.iter().map(|t| t.turn_id.clone())).collect();
        if !removed.is_empty() {
            let entry = Entry::new(crate::changes::NS, crate::changes::REWIND, crate::changes::voided(&done.rewind_id, &removed));
            let filed = match self.engine(&key.0) {
                Ok(linked) => linked.ops().append_entry(&key.1, entry).await.map(|_| ()),
                Err(e) => Err(e),
            };
            match filed {
                // Its frame is routed before the buffer is emptied: none of this rewind's is left in it.
                Ok(()) => self.barrier(&key.0).await,
                Err(e) => tracing::warn!(error = %e.message, "the changes of rewound turns not voided in the log"),
            }
        }
        if let Some(fork) = &done.fork_session_id {
            let new = (key.0.clone(), fork.clone());
            self.bind_space(&new, &self.space_of(key));
            self.note_starter(&new, subject);
            lock(&self.spaces).copy_session(key, &new);
        }
        self.redraw_rewound(key, done.conversation.as_ref().map(|c| c.messages_after)).await;
    }

    /// What the hub drew past a rewind's point is gone — or a message it drew was changed in place
    /// (`engine.session.editMessage`, `dropMessages`): the buffer is emptied, and the session's
    /// watchers attach again and read the history the engine answers now.
    pub(crate) async fn redraw_rewound(self: &Arc<Self>, key: &SessionKey, messages: Option<u64>) {
        let watchers = {
            let mut guard = self.inner.lock().await;
            let state = guard.sessions.entry(key.clone()).or_default();
            state.replay.clear();
            if let Some(messages) = messages {
                state.history_total_before_turn = messages;
            }
            state.watchers()
        };
        let resync = SessionResync { instance: key.0.clone(), session_id: key.1.clone() };
        self.fan_out(&watchers, Frame::session(&key.1, notification("harnix.session_resync", wire::params(&resync)))).await;
        self.refresh("harnix.instances.sessions", json!({"instance": key.0}));
    }
}

/// Where going back to just before `turn` goes: the end of the live turn before it, or the start
/// when it is the first.
fn before(live: &[String], turn: &str) -> Result<RewindTo, RpcError> {
    match live.iter().position(|t| t == turn) {
        Some(0) => Ok(RewindTo::Start),
        Some(at) => Ok(RewindTo::Turn(live[at - 1].clone())),
        None => Err(RpcError::invalid_params(format!("turn `{turn}` is not in the conversation (rewound, or never there)"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn before_a_turn_is_the_end_of_the_one_before_it_or_the_start() {
        let live: Vec<String> = ["t1", "t2", "t3"].map(String::from).to_vec();
        assert_eq!(before(&live, "t1").unwrap(), RewindTo::Start);
        assert_eq!(before(&live, "t3").unwrap(), RewindTo::Turn("t2".into()));
        assert!(before(&live, "t9").is_err());
    }
}
