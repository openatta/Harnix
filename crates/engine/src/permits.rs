//! A host tool's permission asks, answered by the tool: what `Tool::permission` says of the call.
//!
//! The engine asks about a host tool's call as it asks about an MCP tool's, with a line of its own
//! and without the call's input. The link knows the tool, so it answers for it: `Allow` is
//! permitted and `Deny` refused on the spot, neither reaching the hub; `Ask` goes on to the person
//! with the tool's own question beside the engine's (`tool_question`). The input comes from the
//! call's `tool_use` on the same stream — a delegate's mirrored in its progress — which may come
//! after the ask: an ask waits for it up to [`HOLD`], and goes on as it is if it does not come.
//!
//! An ask already waiting when the link subscribed has lost its input, and goes to the person; so
//! does one whose answer the engine did not take.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use harnix_contract::codes;
use harnix_contract::sync::lock;
use harnix_engine_api::{EngineEvent, Permission, Tool};
use serde_json::Value;
use tokio::sync::mpsc;

use crate::host::{HostSide, Wired};
use crate::translate::{self, PermitFrame};

/// How long an ask waits for the call it is about.
pub(crate) const HOLD: Duration = Duration::from_secs(2);
/// How many calls, and answered asks, are remembered: the oldest go first.
const KEPT: usize = 512;

/// A call and an ask by `(session, tool_use_id)`, the session the frame came on.
type Key = (String, String);
/// A host tool's call seen: its key, the tool, and its input.
type Seen = (Key, Arc<dyn Tool>, Value);

#[derive(Default)]
pub(crate) struct Permits {
    /// The host tools' calls seen and not yet ended, with their input.
    calls: StdMutex<VecDeque<Seen>>,
    /// Asks waiting for their call.
    held: StdMutex<HashMap<Key, Value>>,
    /// The asks answered here: the engine's word that they are resolved is not the hub's to hear.
    answered: StdMutex<VecDeque<String>>,
}

impl Permits {
    fn call(&self, key: &Key) -> Option<(Arc<dyn Tool>, Value)> {
        lock(&self.calls).iter().find(|(k, _, _)| k == key).map(|(_, t, i)| (t.clone(), i.clone()))
    }

    /// Whether `prompt` was answered here and is not resolved yet: forgotten either way.
    fn was_answered(&self, prompt: &str) -> bool {
        let mut answered = lock(&self.answered);
        match answered.iter().position(|p| p == prompt) {
            Some(i) => {
                answered.remove(i);
                true
            }
            None => false,
        }
    }

    pub(crate) fn clear(&self) {
        lock(&self.calls).clear();
        lock(&self.held).clear();
        lock(&self.answered).clear();
    }
}

/// What becomes of an ask the link can judge.
enum Judged {
    /// Answered here: not handed on.
    Answered,
    /// Handed on to the person, with the tool's question.
    Asked(Value),
}

impl HostSide {
    /// `frame` as the hub is to have it: `None` when it was the link's to take — an ask answered
    /// here, or held for its call — and the frame, or the ask it was about, otherwise. Never
    /// waits. `events` is where a held ask goes once it has waited long enough.
    pub(crate) fn screen(self: &Arc<Self>, frame: Value, wire: &Arc<dyn Wired>, events: &mpsc::Sender<EngineEvent>) -> Vec<Value> {
        let Some(seen) = translate::permit_frame(&frame) else { return vec![frame] };
        let permits = &self.permits;
        match seen {
            PermitFrame::ToolUse { session, id, name, input } => {
                let Some(tool) = self.tool(&name) else { return vec![frame] };
                let key = (session, id);
                {
                    let mut calls = lock(&permits.calls);
                    if calls.len() == KEPT {
                        calls.pop_front();
                    }
                    calls.push_back((key.clone(), tool.clone(), input.clone()));
                }
                let mut out = vec![frame];
                let held = lock(&permits.held).remove(&key);
                if let Some(ask) = held {
                    if let Judged::Asked(ask) = self.judge(&tool, &input, ask, &key.0, wire, events) {
                        out.push(ask);
                    }
                }
                out
            }
            PermitFrame::ToolResult { session, id } => {
                let key = (session, id);
                lock(&permits.calls).retain(|(k, _, _)| *k != key);
                vec![frame]
            }
            PermitFrame::Asked { session, tool_name, tool_use_id } => {
                if self.tool(&tool_name).is_none() {
                    return vec![frame];
                }
                let key = (session, tool_use_id);
                match permits.call(&key) {
                    Some((tool, input)) => match self.judge(&tool, &input, frame, &key.0, wire, events) {
                        Judged::Answered => Vec::new(),
                        Judged::Asked(ask) => vec![ask],
                    },
                    None => {
                        lock(&permits.held).insert(key.clone(), frame);
                        self.hold(key, events.clone());
                        Vec::new()
                    }
                }
            }
            PermitFrame::Resolved { prompt_id } => {
                if permits.was_answered(&prompt_id) {
                    Vec::new()
                } else {
                    vec![frame]
                }
            }
        }
    }

    /// Answer `ask` as the tool says of `input`, or hand it on with the tool's question. An
    /// answer the engine does not take hands the ask on to `events` as it is, unless it was
    /// resolved meanwhile or went with the connection.
    fn judge(
        self: &Arc<Self>,
        tool: &Arc<dyn Tool>,
        input: &Value,
        mut ask: Value,
        session: &str,
        wire: &Arc<dyn Wired>,
        events: &mpsc::Sender<EngineEvent>,
    ) -> Judged {
        let decision = match tool.permission(input) {
            Permission::Allow => Ok(()),
            Permission::Deny(why) => Err(why),
            Permission::Ask(question) => {
                translate::with_tool_question(&mut ask, &question);
                return Judged::Asked(ask);
            }
            _ => return Judged::Asked(ask),
        };
        let Some(prompt) = translate::prompt_id(&ask) else { return Judged::Asked(ask) };
        {
            let mut answered = lock(&self.permits.answered);
            if answered.len() == KEPT {
                answered.pop_front();
            }
            answered.push_back(prompt.clone());
        }
        let (host, wire, session, events) = (self.clone(), wire.clone(), session.to_string(), events.clone());
        let task = tokio::spawn(async move {
            let answer = translate::permit_answer(&session, &prompt, decision);
            let Err(e) = host.answer(&*wire, translate::SESSION_RESPOND, answer).await else { return };
            // Lost with the connection: the hub has it from the engine when it subscribes again.
            if e.code == codes::INSTANCE_UNAVAILABLE {
                return;
            }
            // Resolved first (it timed out, say): nothing waits for an answer.
            if !host.permits.was_answered(&prompt) {
                return;
            }
            tracing::error!(instance = %host.instance(), prompt = %prompt, error = %e.message, "the engine did not take the answer the tool gave; the person is asked");
            if let Some(event) = translate::event(ask) {
                let _ = events.send(event).await;
            }
        });
        self.keep(task.abort_handle());
        Judged::Answered
    }

    /// An ask whose call has not been seen: handed on as it is once [`HOLD`] has passed without it.
    fn hold(self: &Arc<Self>, key: Key, events: mpsc::Sender<EngineEvent>) {
        let host = Arc::downgrade(self);
        let task = tokio::spawn(async move {
            tokio::time::sleep(HOLD).await;
            let Some(host) = host.upgrade() else { return };
            let ask = lock(&host.permits.held).remove(&key);
            if let Some(event) = ask.and_then(translate::event) {
                let _ = events.send(event).await;
            }
        });
        self.keep(task.abort_handle());
    }
}
