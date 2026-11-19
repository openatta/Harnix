//! Per-session state, and what arrives from the engine.

use std::collections::{HashSet, VecDeque};
use std::sync::Arc;

use harnix_contract::wire::{self, AttachResult, EventFrame, ToolCallOutcome};
use harnix_contract::{Frame, RpcError};
use harnix_engine_link::events::{Completion, FileWrite, Joins, Prompt, SessionEventKind};
use harnix_contract::sync::lock;
use harnix_engine_link::ops::{SessionInfo, SessionSetup};
use harnix_kernel::CallContext;
use serde_json::{json, Value};

use crate::clients::ClientId;
use crate::replay::Buffer;
use crate::{notification, without_instance, Hub, InnerLock, SessionKey};

/// The event the hub writes into a session's stream for a person's own message: the engine
/// emits none, and every other watcher needs to see it.
pub(crate) const USER_MESSAGE: &str = "user_message";

pub(crate) struct QueueItem {
    pub item_id: String,
    pub message: String,
    pub attachments: Value,
    /// The calls the send was made on behalf of, the sender's own included: what the turn it
    /// starts is on behalf of.
    pub chain: harnix_kernel::Chain,
}

impl QueueItem {
    /// What a client draws for a message that is waiting its turn.
    ///
    /// Attachments are in it because the row is the only place a queued
    /// message is visible: without them a "look at this file" waiting behind
    /// a running turn draws as a bare question, and there is no other call
    /// that would tell the client otherwise. They reach the turn either way —
    /// what was missing was the client being able to see that they would.
    pub fn view(&self) -> wire::QueueItem {
        wire::QueueItem { item_id: self.item_id.clone(), message: self.message.clone(), attachments: self.attachments.clone() }
    }
}

#[derive(Default)]
pub(crate) struct SessionState {
    pub subscribed: bool,
    pub seq: u64,
    pub replay: Buffer,
    pub running_turn: Option<String>,
    /// When the running turn began, in ms since the epoch: what an elapsed clock counts from.
    pub turn_started_ms: Option<u64>,
    pub queue: VecDeque<QueueItem>,
    pub pending_prompts: Vec<Prompt>,
    /// The asks an answer is on its way to the engine for, by `prompt_id`: the first answer through
    /// the hub claims its ask, and only that one is told to every watcher (`ask_answered`).
    pub answering: HashSet<String>,
    pub watchers: HashSet<ClientId>,
    /// Projected-message count before the running turn began, so settling can
    /// tell whether the transcript has caught up.
    pub history_total_before_turn: u64,
    /// How many times the session had been compacted then. A compaction
    /// rewrites the transcript shorter, so a count that did not grow can
    /// still be a transcript that caught up; this is what tells them apart.
    pub compactions_before_turn: u64,
    /// The running turn's own `compact` event went by: it compacted the
    /// transcript. Said by the turn itself, so it holds where the counts read
    /// around a turn the hub only heard of may already be the compacted ones.
    pub compacted: bool,
    /// The session's tasks that were announced and have not finished, by id:
    /// work that outlives the turn that started it, which is what tells a
    /// session that is idle apart from one that still has something running.
    pub live_tasks: std::collections::BTreeSet<String>,
    /// The session's goal as the hub last heard it (`goal_changed`, or a goal call's answer):
    /// `None` is not heard, `Some(Value::Null)` is no goal.
    pub goal: Option<Value>,
    /// The permission mode it is in as the hub last heard it (read when it was first opened, a
    /// change's answer, or the engine's event for a change): `None` is not heard.
    pub permission_mode: Option<String>,
    /// Each team's latest `team_progress` frame, by `team_id`: where its run stands.
    pub teams: std::collections::BTreeMap<String, Value>,
    /// The running turn is not driven by a loop of ours: the engine started
    /// it (a schedule), or the link that was running it went away. Its end
    /// arrives as `turn_completed` or is found by a resync.
    pub external: bool,
    /// A rewind of the hub's is under way: the engine's frame for it is the hub's to follow up.
    pub rewinding: bool,
    /// How the running turn ended, once the engine has said: what settling reports as the
    /// turn's `outcome` (ipdrun-17).
    pub last_complete: Option<Completion>,
    /// The running turn's answer and tool calls, as its events went by: what settling reports as
    /// `outcome.text` and `outcome.tool_calls` (ipdrun-41).
    pub answer: Answer,
}

/// What a turn said, which tools it called and which files they wrote, gathered from its events.
#[derive(Default)]
pub(crate) struct Answer {
    pub text: String,
    pub truncated: bool,
    pub tool_calls: Vec<ToolCallOutcome>,
    /// In the order written, a sub-agent's among them: what the turn's changes entry is made of.
    pub wrote: Vec<FileWrite>,
}

/// The most answer text `outcome.text` carries; the rest is in `engine.session.history`.
pub(crate) const ANSWER_TEXT_MAX: usize = 64 * 1024;

impl Answer {
    pub fn take_event(&mut self, kind: &SessionEventKind) {
        match kind {
            SessionEventKind::Fragment(f) if f.joins == Joins::Answer => {
                let text = f.text.as_str();
                let room = ANSWER_TEXT_MAX.saturating_sub(self.text.len());
                if text.len() <= room {
                    self.text.push_str(text);
                } else {
                    let cut = (0..=room).rev().find(|i| text.is_char_boundary(*i)).unwrap_or(0);
                    self.text.push_str(&text[..cut]);
                    self.truncated = true;
                }
            }
            SessionEventKind::ToolCall { id, name } => {
                self.tool_calls.push(ToolCallOutcome { id: id.clone(), name: name.clone(), is_error: None })
            }
            SessionEventKind::Wrote(wrote) => self.wrote.extend(wrote.iter().cloned()),
            SessionEventKind::ToolResult { id, is_error, wrote } => {
                self.wrote.extend(wrote.iter().cloned());
                if let Some(call) = self.tool_calls.iter_mut().find(|c| c.id == *id) {
                    call.is_error = Some(*is_error);
                }
            }
            _ => {}
        }
    }
}

impl SessionState {
    pub fn watchers(&self) -> Vec<ClientId> {
        self.watchers.iter().copied().collect()
    }

    pub fn queue_view(&self) -> Vec<wire::QueueItem> {
        self.queue.iter().map(QueueItem::view).collect()
    }

    /// The questions waiting on a person, as the engine put them.
    pub fn prompts_view(&self) -> Vec<Value> {
        self.pending_prompts.iter().map(|p| p.value.clone()).collect()
    }
}

/// Assign a seq, buffer for catch-up, fold the projections, fan out.
///
/// Sequence assignment, buffering and the choice of recipients happen under
/// one lock, so a client attaching between two frames either finds a frame in
/// its replay snapshot or receives it live — never both, never neither. The
/// sending itself happens after the lock, in seq order (`send_after`).
///
/// A session of an extension's engine keeps fewer frames for catch-up, and its frames say so.
pub(crate) async fn route_event(
    inner: &Arc<InnerLock>,
    instance: &str,
    session_id: &str,
    turn_id: &str,
    kind: &SessionEventKind,
    event: Value,
    extension: bool,
) {
    let mut guard = inner.lock().await;
    let state = guard
        .sessions
        .entry((instance.to_string(), session_id.to_string()))
        .or_default();
    if extension {
        state.replay.hold_to(crate::links::EXTENSION_BUFFER_FRAMES);
    }
    match kind {
        SessionEventKind::Compacted => state.compacted = true,
        // What was drawn past the point it went back to is gone, its frames with it; the frame
        // itself goes to whoever watches, and no one catches up on it.
        SessionEventKind::Rewound { messages } => {
            state.replay.clear();
            state.history_total_before_turn = *messages;
        }
        SessionEventKind::Asked(prompt) => state.pending_prompts.push(prompt.clone()),
        SessionEventKind::GoalChanged { goal } => state.goal = Some(goal.clone()),
        SessionEventKind::PermissionModeChanged { mode } => state.permission_mode = Some(mode.clone()),
        SessionEventKind::TeamProgress { team_id } => {
            state.teams.insert(team_id.clone(), event.clone());
        }
        SessionEventKind::AskResolved { prompt_id, .. } => {
            state.pending_prompts.retain(|p| p.prompt_id.as_deref() != Some(prompt_id.as_str()));
        }
        // Answered, or refused on its timeout: the tool went on either way.
        SessionEventKind::ToolResult { id, .. } => {
            state.pending_prompts.retain(|p| p.tool_use_id.as_deref() != Some(id.as_str()));
            state.answer.take_event(kind);
        }
        // Nothing is waiting on those answers any more.
        SessionEventKind::TurnComplete(completion) => {
            state.pending_prompts.clear();
            state.last_complete = Some(completion.clone());
        }
        _ => state.answer.take_event(kind),
    }
    state.seq += 1;
    let seq = state.seq;
    if !matches!(kind, SessionEventKind::Rewound { .. }) {
        state.replay.push(seq, turn_id, kind, &event);
    }
    let watchers = state.watchers();
    let params = EventFrame {
        instance: instance.to_string(),
        session_id: session_id.to_string(),
        seq,
        turn_id: turn_id.to_string(),
        event,
        trust: extension.then_some(wire::EngineTrust::Extension),
    };
    let frame = Frame::session(session_id.to_string(), notification("harnix.event", wire::params(&params)));
    let out = guard.clients.to_many(&watchers, frame);
    crate::send_after(guard, [out]).await;
}

impl Hub {
    /// Open a session for one client: subscribe once, hub-wide, resuming the
    /// session first if it only exists on disk; read its shape and transcript
    /// depth; then hand back the replay snapshot under the same lock that
    /// registers the watcher.
    ///
    /// The order of those three is not interchangeable. Subscribing before
    /// reading is what closes the seam between what history holds and what the
    /// buffer holds.
    pub(crate) async fn attach(
        &self,
        cx: &CallContext,
        params: Value,
    ) -> Result<Value, RpcError> {
        let key = self.key_of(&params)?;
        let (instance, session_id) = (key.0.clone(), key.1.clone());
        let client = client_of(cx)?;

        // Reading a session answers for a cold one too — and it is where the scene comes from,
        // which a resume needs.
        let linked = self.engine(&instance)?;
        let mut info = linked.ops().get(&session_id).await?;
        let extension = linked.is_extension();
        if extension {
            info.harden();
        }
        self.ensure_subscribed(&key, &info).await?;
        self.read_permission_mode(&key, &info).await;
        let total = self.history_total(&key).await;
        let compactions = info.compactions();
        let mut session = info.into_value();
        lock(&self.spaces).decorate(&key, &mut session);

        let mut guard = self.inner.lock().await;
        let state = guard.sessions.entry(key).or_default();
        state.watchers.insert(client);
        if let (Some(mode), Some(obj)) = (&state.permission_mode, session.as_object_mut()) {
            obj.insert("permission_mode".into(), json!(mode));
        }
        // What the client draws from history, and what from the buffer, must
        // not overlap. A running turn is written to the transcript step by
        // step, so part of it is already in history — history is therefore
        // cut at where the turn began and the whole turn comes from the
        // buffer. A settled turn already in the transcript comes from history
        // alone; one settled but not yet written comes from the buffer.
        let running = state.running_turn.is_some();
        let (history_total, serve_replay) = if running {
            (state.history_total_before_turn, true)
        } else {
            (
                total,
                total <= state.history_total_before_turn
                    && compactions == state.compactions_before_turn
                    && !state.compacted,
            )
        };
        let replay = if serve_replay {
            state.replay.snapshot()
        } else {
            Vec::new()
        };
        let attached = AttachResult {
            instance,
            session,
            history_total,
            replay,
            truncated: state.replay.truncated,
            pending_prompts: state.prompts_view(),
            running_turn: state.running_turn.clone(),
            queue: state.queue_view(),
            seq: state.seq,
            trust: extension.then_some(wire::EngineTrust::Extension),
        };
        Ok(wire::params(&attached))
    }

    /// Subscribe the hub to a session once, resuming it first if it exists
    /// only on disk. `info` is what the engine says of it. Returns the engine's
    /// watermark when this call subscribed, `None` when the hub already was.
    ///
    /// Every way into a session goes through here — a client attaching, a
    /// send, a resync — so whichever comes first, the hub is subscribed
    /// before anyone watches and before any turn runs.
    pub(crate) async fn ensure_subscribed(
        &self,
        key: &SessionKey,
        info: &SessionInfo,
    ) -> Result<Option<u64>, RpcError> {
        let session_id = key.1.as_str();
        let subscribed = {
            let guard = self.inner.lock().await;
            guard.sessions.get(key).is_some_and(|s| s.subscribed)
        };
        if subscribed {
            return Ok(None);
        }
        let linked = self.engine(&key.0)?;
        let ops = linked.ops();
        let subscribed = match ops.subscribe(session_id).await? {
            Some(subscribed) => subscribed,
            // Only live sessions have subscribers. One that exists on disk but
            // not in memory has to be brought back first — opening a past
            // conversation is otherwise impossible. Recorded as every resume is,
            // by the link.
            None => {
                ops.resume(session_id, info).await?;
                ops.subscribe(session_id).await?.ok_or_else(|| {
                    RpcError::new(harnix_engine_link::ops::codes::SESSION_NOT_FOUND, "the session was resumed and is not there")
                })?
            }
        };
        let mut guard = self.inner.lock().await;
        let state = guard.sessions.entry(key.clone()).or_default();
        state.subscribed = true;
        // Replaced, not extended: after a resync these are the same asks
        // the hub already held.
        state.pending_prompts = subscribed.pending;
        drop(guard);
        // What it waits on, heard afresh.
        self.arc().refresh(wire::INBOX, json!({}));
        self.reconcile_skill_dirs(key, info).await;
        Ok(subscribed.last_seq)
    }

    pub(crate) async fn detach(
        &self,
        cx: &CallContext,
        params: Value,
    ) -> Result<Value, RpcError> {
        let key = self.key_of(&params)?;
        let client = client_of(cx)?;
        let mut guard = self.inner.lock().await;
        if let Some(state) = guard.sessions.get_mut(&key) {
            state.watchers.remove(&client);
        }
        Ok(json!({"detached": true}))
    }

    /// The permission mode of a session the hub is subscribed to, read from the engine when the hub
    /// has not heard it: what the engine says of a session (`EngineOps::get`) does not say it. A
    /// change heard meanwhile is newer than what was read, and stays. Not read is not known:
    /// nothing is shown.
    async fn read_permission_mode(&self, key: &SessionKey, info: &SessionInfo) {
        let known = self.inner.lock().await.sessions.get(key).is_some_and(|s| s.permission_mode.is_some());
        if known {
            return;
        }
        let read = match self.engine(&key.0) {
            Ok(linked) => linked.ops().permission_mode(&key.1, info).await,
            Err(e) => Err(e),
        };
        match read {
            Ok(Some(mode)) => {
                let mut guard = self.inner.lock().await;
                let state = guard.sessions.entry(key.clone()).or_default();
                state.permission_mode.get_or_insert(mode);
            }
            Ok(None) => {}
            Err(e) => tracing::debug!(instance = %key.0, session = %key.1, error = %e.message, "the session's permission mode was not read"),
        }
    }

    /// A permission mode heard in an answer: kept for the session's row.
    pub(crate) async fn permission_mode_heard(self: &Arc<Self>, key: &SessionKey, mode: String) {
        self.inner.lock().await.sessions.entry(key.clone()).or_default().permission_mode = Some(mode);
        self.refresh("harnix.instances.sessions", json!({"instance": key.0}));
    }

    /// A goal heard in an answer: kept for the session's row.
    pub(crate) async fn goal_heard(self: &Arc<Self>, key: &SessionKey, goal: Value) {
        self.inner.lock().await.sessions.entry(key.clone()).or_default().goal = Some(goal);
        self.refresh("harnix.instances.sessions", json!({"instance": key.0}));
    }

    /// A prompt was answered: it no longer waits, and the session's row says so.
    pub(crate) async fn answered(self: &Arc<Self>, key: &SessionKey, which: impl Fn(&Prompt) -> bool) {
        if let Some(state) = self.inner.lock().await.sessions.get_mut(key) {
            state.pending_prompts.retain(|p| !which(p));
        }
        self.refresh("harnix.instances.sessions", json!({"instance": key.0}));
    }

    /// Intercepted for where the new session lands, who started it, and what it is set up with:
    /// the scene layer it is created in, expanded (`scenes.rs`), and the skills its space sees. It
    /// records as every session does, by the link.
    pub(crate) async fn create(
        self: &std::sync::Arc<Self>,
        cx: &harnix_kernel::CallContext,
        mut params: Value,
    ) -> Result<Value, RpcError> {
        crate::refuse_hub_owned_options(&params)?;
        let subject = &cx.subject;
        let space = self.place(cx, &mut params).await?;
        let instance = self.instance_of(&params)?;
        let linked = self.engine(&instance)?;
        let layer = crate::scenes::layer_for(&linked.link.scene_layers(), &params)?;
        let setup = SessionSetup::new()
            .scene(layer.clone())
            .skill_dirs(self.skill_dirs_for(&instance, &space))
            .person(matches!(subject, harnix_contract::Subject::Device { .. }));
        let created = self.create_listing(linked.ops(), without_instance(params), setup).await?;
        let mut value = created.value;
        if let Some(sid) = &created.session_id {
            let key = (instance.clone(), sid.clone());
            self.bind_space(&key, &space);
            self.note_starter(&key, subject);
            let mut spaces = lock(&self.spaces);
            if let Some(layer) = &layer {
                spaces.note_layer(&key, &layer.id);
            }
            spaces.decorate(&key, &mut value);
        }
        if let Some(obj) = value.as_object_mut() {
            obj.insert("instance".into(), json!(instance));
            obj.insert("space".into(), json!(space));
        }
        Ok(value)
    }

    /// How many times the engine has compacted the session.
    pub(crate) async fn compactions(&self, key: &SessionKey) -> u64 {
        match self.engine(&key.0) {
            Ok(linked) => linked.ops().get(&key.1).await.map(|info| info.compactions()).unwrap_or(0),
            Err(_) => 0,
        }
    }

    pub(crate) async fn history_total(&self, key: &SessionKey) -> u64 {
        match self.engine(&key.0) {
            Ok(linked) => linked.ops().history_len(&key.1).await.unwrap_or(0),
            Err(_) => 0,
        }
    }

    pub(crate) async fn fan_out(&self, watchers: &[ClientId], frame: Frame) {
        let guard = self.inner.lock().await;
        let out = guard.clients.to_many(watchers, frame);
        crate::send_after(guard, [out]).await;
    }
}

/// Claim an ask for the answer on its way to the engine: true when it is still waiting and no other
/// answer is on its way. Whoever answers first is the answer, and the claim is how the hub knows
/// which one that was: the engine answers a later one with the same silent success.
pub(crate) async fn claim_ask(inner: &Arc<InnerLock>, key: &SessionKey, prompt_id: &str) -> bool {
    let mut guard = inner.lock().await;
    let Some(state) = guard.sessions.get_mut(key) else { return false };
    let waiting = state.pending_prompts.iter().any(|p| p.prompt_id.as_deref() == Some(prompt_id));
    waiting && state.answering.insert(prompt_id.to_string())
}

/// The claimed answer is back from the engine, taken or refused (a form that does not fit its
/// schema stays asked, and may be answered again).
pub(crate) async fn release_ask(inner: &Arc<InnerLock>, key: &SessionKey, prompt_id: &str) {
    if let Some(state) = inner.lock().await.sessions.get_mut(key) {
        state.answering.remove(prompt_id);
    }
}

/// The engine took the claimed answer: written into the session's stream as the hub's own event,
/// so every watcher's card shows it, and a client that attaches later finds it in the replay.
pub(crate) async fn tell_answered(inner: &Arc<InnerLock>, key: &SessionKey, event: Value, extension: bool) {
    let turn = inner.lock().await.sessions.get(key).and_then(|s| s.running_turn.clone()).unwrap_or_default();
    route_event(inner, &key.0, &key.1, &turn, &SessionEventKind::Other, event, extension).await;
}

/// Which client this call is on behalf of.
///
/// Watching is per-client, and a subject may have several. The transport
/// says which connection a call came on; nothing else may, which is why a
/// call that came on none is an error rather than a default.
pub(crate) fn client_of(cx: &CallContext) -> Result<ClientId, RpcError> {
    cx.client.ok_or_else(|| {
        RpcError::invalid_params(format!(
            "{} called a watching method outside a client connection",
            cx.subject.label()
        ))
    })
}


#[cfg(test)]
mod tests {
    use super::*;
    use harnix_engine_link::events::Waiting;

    fn asked(id: &str) -> SessionEventKind {
        SessionEventKind::Asked(Prompt::new(json!({"prompt_id": id}), Some(id.into()), None, Waiting::Approval))
    }

    /// A question the engine says stopped waiting is no longer pending; the others stay.
    #[tokio::test]
    async fn a_resolved_question_is_not_pending_any_more() {
        let inner = Arc::new(InnerLock::default());
        let key = ("main".to_string(), "s".to_string());
        for id in ["p1", "p2"] {
            route_event(&inner, "main", "s", "t", &asked(id), json!({"kind": "prompt"}), false).await;
        }
        let pending = |inner: &Arc<InnerLock>| {
            let inner = inner.clone();
            let key = key.clone();
            async move {
                let guard = inner.lock().await;
                guard.sessions[&key].pending_prompts.iter().map(|p| p.prompt_id.clone().unwrap()).collect::<Vec<_>>()
            }
        };
        assert_eq!(pending(&inner).await, ["p1", "p2"]);
        let resolved = SessionEventKind::AskResolved { prompt_id: "p1".into(), answered: false };
        route_event(&inner, "main", "s", "t", &resolved, json!({"kind": "prompt_resolved"}), false).await;
        assert_eq!(pending(&inner).await, ["p2"]);
    }

    /// The first answer claims its ask; a second while it is on its way, or one to an ask nobody
    /// is waiting on, does not. A refused answer gives the claim back.
    #[tokio::test]
    async fn the_first_answer_claims_its_ask() {
        let inner = Arc::new(InnerLock::default());
        let key = ("main".to_string(), "s".to_string());
        assert!(!claim_ask(&inner, &key, "p1").await, "a session the hub has not heard of");
        route_event(&inner, "main", "s", "t", &asked("p1"), json!({"kind": "prompt"}), false).await;
        assert!(claim_ask(&inner, &key, "p1").await);
        assert!(!claim_ask(&inner, &key, "p1").await, "another answer on its way");
        assert!(!claim_ask(&inner, &key, "p2").await, "not asked");
        release_ask(&inner, &key, "p1").await;
        assert!(claim_ask(&inner, &key, "p1").await, "the first was refused");

        tell_answered(&inner, &key, json!({"kind": "ask_answered", "prompt_id": "p1"}), false).await;
        let guard = inner.lock().await;
        let state = &guard.sessions[&key];
        assert_eq!(state.seq, 2);
        assert_eq!(state.pending_prompts.len(), 1, "the engine's own frame resolves it");
    }
}
