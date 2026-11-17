//! A host played from a script: [`ScriptedHost`].
//!
//! Each session a run opens is a script of turns, one taken for each message sent to it; the
//! frames of a turn — its start, what it asks, how it ends — go to whoever watches. With it a
//! [`Runner`](super::Runner) over a `MemStore` drives a whole flow pack from start to finish with
//! no model, no Harnix and no network (`tests/runner.rs`, `examples/flowctl.rs`); the
//! conformance suite (`crate::conformance`) runs against it too, to keep the suite itself honest.
//!
//! Which script a session plays is read off its key: a task's own session plays its station's
//! ([`ScriptedHost::station`]), the main session [`ScriptedHost::main`]'s, a shared session its
//! key's ([`ScriptedHost::shared`]); a session with no script runs each turn until it is
//! interrupted.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use futures::channel::mpsc;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::host::{
    Decision, Frame, HostError, Note, OpenSpec, Opened, SessionHost, SessionId, SessionInfo, TurnEnd, Watch,
};

/// What one message makes a scripted session do.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Turn {
    /// The turn ends, with this structured output or none.
    Ends {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        output: Option<Value>,
    },
    /// The turn runs until it is interrupted.
    Runs,
    /// The turn calls `tool` on `paths` and asks; allowed, it goes on as `then` says; refused,
    /// it ends with nothing.
    Asks {
        tool: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        paths: Vec<String>,
        #[serde(default, skip_serializing_if = "Value::is_null")]
        input: Value,
        then: Box<Turn>,
    },
    /// The turn fails.
    Errors {
        retryable: bool,
        #[serde(default)]
        message: String,
    },
}

type OnSend = Box<dyn FnMut(&SessionId, &str) + Send>;

struct Session {
    spec: OpenSpec,
    /// The script it plays (`ScriptedHost::script_of`).
    script: String,
    watchers: Vec<mpsc::UnboundedSender<Frame>>,
    /// The turn running, when one is.
    running: Option<String>,
    /// A question waiting for its answer: the ask's id, the turn, the call it is about, and what
    /// follows when allowed.
    asked: Option<(String, String, String, Turn)>,
    closed: bool,
}

#[derive(Default)]
struct State {
    scripts: HashMap<String, VecDeque<Turn>>,
    sessions: HashMap<SessionId, Session>,
    by_key: HashMap<String, SessionId>,
    opened: Vec<OpenSpec>,
    sent: Vec<(SessionId, String)>,
    answers: Vec<(SessionId, String, Decision)>,
    interrupts: Vec<SessionId>,
    notes: Vec<(SessionId, Note)>,
    over: Vec<String>,
    trust: HashMap<PathBuf, bool>,
    seq: u64,
    on_send: Option<OnSend>,
    /// Every `open` is refused with this.
    refusing: Option<String>,
    unavailable: bool,
}

/// A [`SessionHost`] that answers from scripts, and remembers everything asked of it.
#[derive(Default)]
pub struct ScriptedHost {
    state: Mutex<State>,
}

impl ScriptedHost {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The turns the sessions of `station` play, in order, one for each message.
    pub fn station(&self, station: &str, turns: Vec<Turn>) -> &Self {
        self.lock().scripts.insert(format!("station:{station}"), turns.into());
        self
    }

    /// The turns the run's main session plays.
    pub fn main(&self, turns: Vec<Turn>) -> &Self {
        self.lock().scripts.insert("main".into(), turns.into());
        self
    }

    /// The turns a shared session (`session: per_run`, or a named key) plays.
    pub fn shared(&self, key: &str, turns: Vec<Turn>) -> &Self {
        self.lock().scripts.insert(format!("key:{key}"), turns.into());
        self
    }

    /// Called with each message sent, before its turn: what the session does about it besides.
    pub fn on_send(&self, f: impl FnMut(&SessionId, &str) + Send + 'static) -> &Self {
        self.lock().on_send = Some(Box::new(f));
        self
    }

    /// Whether `root` is trusted (nothing is until told).
    pub fn trust(&self, root: &Path, trusted: bool) -> &Self {
        self.lock().trust.insert(root.to_path_buf(), trusted);
        self
    }

    /// Refuse every `open` from now on with `why` (`None` opens again).
    pub fn refuse(&self, why: Option<&str>) -> &Self {
        self.lock().refusing = why.map(String::from);
        self
    }

    /// Answer every call `Unavailable` from now on.
    pub fn unavailable(&self, on: bool) -> &Self {
        self.lock().unavailable = on;
        self
    }

    /// Every spec opened, in order (an `open` that found its key already counts).
    pub fn opened(&self) -> Vec<OpenSpec> {
        self.lock().opened.clone()
    }

    /// Every message sent, in order.
    pub fn sent(&self) -> Vec<(SessionId, String)> {
        self.lock().sent.clone()
    }

    /// The messages sent to one session.
    pub fn sent_to(&self, id: &str) -> Vec<String> {
        self.lock().sent.iter().filter(|(s, _)| s == id).map(|(_, t)| t.clone()).collect()
    }

    pub fn answers(&self) -> Vec<(SessionId, String, Decision)> {
        self.lock().answers.clone()
    }

    pub fn interrupts(&self) -> Vec<SessionId> {
        self.lock().interrupts.clone()
    }

    pub fn notes(&self) -> Vec<(SessionId, Note)> {
        self.lock().notes.clone()
    }

    /// The runs told over, in order.
    pub fn over(&self) -> Vec<String> {
        self.lock().over.clone()
    }

    /// The session a key is open as.
    pub fn session_of(&self, key: &str) -> Option<SessionId> {
        self.lock().by_key.get(key).cloned()
    }

    /// Whether a turn is running in `id`.
    pub fn running(&self, id: &str) -> bool {
        self.lock().sessions.get(id).is_some_and(|s| s.running.is_some())
    }

    /// A frame from outside the script: a person's turn, one from before a restart.
    pub fn push(&self, id: &str, frame: Frame) {
        let mut g = self.lock();
        if let Some(s) = g.sessions.get_mut(id) {
            s.watchers.retain(|w| w.unbounded_send(frame.clone()).is_ok());
        }
    }

    /// The script a key's session plays: its station's, the main one's, or its shared key's.
    fn script_of(key: &str) -> String {
        let rest = key.strip_prefix("flow:").unwrap_or(key);
        if rest.ends_with("@main") {
            return "main".into();
        }
        if let Some((_, k)) = rest.split_once('#') {
            return format!("key:{k}");
        }
        match rest.split('/').nth(1) {
            Some(station) => format!("station:{station}"),
            None => "main".into(),
        }
    }
}

impl State {
    fn next(&mut self, what: &str) -> String {
        self.seq += 1;
        format!("{what}{}", self.seq)
    }

    fn emit(&mut self, id: &str, frame: Frame) {
        if let Some(s) = self.sessions.get_mut(id) {
            s.watchers.retain(|w| w.unbounded_send(frame.clone()).is_ok());
        }
    }

    /// Play `turn` in session `id`'s turn `t`.
    fn play(&mut self, id: &str, t: &str, turn: Turn) {
        match turn {
            Turn::Ends { output } => {
                if let Some(s) = self.sessions.get_mut(id) {
                    s.running = None;
                }
                self.emit(id, Frame::TurnSettled { turn: t.into(), end: TurnEnd::Done, output, usage: None });
            }
            Turn::Runs => {
                if let Some(s) = self.sessions.get_mut(id) {
                    s.running = Some(t.into());
                }
            }
            Turn::Asks { tool, paths, input, then } => {
                let call = self.next("call");
                let ask = self.next("ask");
                let input = if input.is_null() { json!({}) } else { input };
                self.emit(id, Frame::ToolCall { turn: t.into(), call: call.clone(), tool: tool.clone(), input, agent: None });
                self.emit(
                    id,
                    Frame::Asked { turn: t.into(), ask: ask.clone(), call: Some(call.clone()), tool, paths, agent: None },
                );
                if let Some(s) = self.sessions.get_mut(id) {
                    s.running = Some(t.into());
                    s.asked = Some((ask, t.into(), call, *then));
                }
            }
            Turn::Errors { retryable, message } => {
                if let Some(s) = self.sessions.get_mut(id) {
                    s.running = None;
                }
                let kind = if retryable { "rate_limited" } else { "no_credentials" };
                let end = TurnEnd::Error { retryable, kind: kind.into(), message };
                self.emit(id, Frame::TurnSettled { turn: t.into(), end, output: None, usage: None });
            }
        }
    }

    fn live(&self, id: &str) -> Result<&Session, HostError> {
        if self.unavailable {
            return Err(HostError::Unavailable("the scripted host is away".into()));
        }
        match self.sessions.get(id) {
            Some(s) if !s.closed => Ok(s),
            _ => Err(HostError::Gone(id.to_string())),
        }
    }
}

#[async_trait::async_trait]
impl SessionHost for ScriptedHost {
    async fn open(&self, spec: OpenSpec) -> Result<Opened, HostError> {
        let mut g = self.lock();
        if g.unavailable {
            return Err(HostError::Unavailable("the scripted host is away".into()));
        }
        if let Some(why) = &g.refusing {
            return Err(HostError::Refused(why.clone()));
        }
        g.opened.push(spec.clone());
        if let Some(id) = g.by_key.get(&spec.key).cloned() {
            let had = &g.sessions[&id];
            if had.closed {
                // A closed session's key opens a new one.
            } else if had.spec != spec {
                return Err(HostError::Refused(format!("key {} is open with another spec", spec.key)));
            } else {
                return Ok(Opened { id, existed: true });
            }
        }
        let id = g.next("s");
        let script = Self::script_of(&spec.key);
        g.by_key.insert(spec.key.clone(), id.clone());
        g.sessions.insert(
            id.clone(),
            Session { spec, script, watchers: vec![], running: None, asked: None, closed: false },
        );
        Ok(Opened { id, existed: false })
    }

    async fn lookup(&self, id: &SessionId) -> Result<Option<SessionInfo>, HostError> {
        let g = self.lock();
        if g.unavailable {
            return Err(HostError::Unavailable("the scripted host is away".into()));
        }
        Ok(g.sessions.get(id).filter(|s| !s.closed).map(|s| SessionInfo {
            project_root: s.spec.project_root.clone(),
            turn_running: s.running.is_some(),
        }))
    }

    async fn watch(&self, id: &SessionId) -> Result<Watch, HostError> {
        let mut g = self.lock();
        let info = {
            let s = g.live(id)?;
            SessionInfo { project_root: s.spec.project_root.clone(), turn_running: s.running.is_some() }
        };
        let (tx, rx) = mpsc::unbounded();
        if let Some(s) = g.sessions.get_mut(id) {
            s.watchers.push(tx);
        }
        Ok(Watch { frames: Box::pin(rx), info })
    }

    async fn send(&self, id: &SessionId, text: &str) -> Result<(), HostError> {
        let mut g = self.lock();
        g.live(id)?;
        g.sent.push((id.clone(), text.to_string()));
        if let Some(f) = g.on_send.as_mut() {
            f(id, text);
        }
        let turn = g.next("t");
        g.emit(id, Frame::TurnStarted { turn: turn.clone(), text: text.to_string() });
        let script = g.sessions[id].script.clone();
        let next = g.scripts.get_mut(&script).and_then(|s| s.pop_front()).unwrap_or(Turn::Runs);
        g.play(id, &turn, next);
        Ok(())
    }

    async fn interrupt(&self, id: &SessionId) -> Result<(), HostError> {
        let mut g = self.lock();
        g.live(id)?;
        g.interrupts.push(id.clone());
        let running = g.sessions.get_mut(id).and_then(|s| {
            s.asked = None;
            s.running.take()
        });
        if let Some(t) = running {
            g.emit(id, Frame::TurnSettled { turn: t, end: TurnEnd::Interrupted, output: None, usage: None });
        }
        Ok(())
    }

    async fn close(&self, id: &SessionId) -> Result<(), HostError> {
        let mut g = self.lock();
        g.live(id)?;
        if let Some(s) = g.sessions.get_mut(id) {
            s.closed = true;
            s.running = None;
            s.asked = None;
            s.watchers.clear();
        }
        Ok(())
    }

    async fn answer(&self, id: &SessionId, ask: &str, decision: Decision) -> Result<(), HostError> {
        let mut g = self.lock();
        g.live(id)?;
        g.answers.push((id.clone(), ask.to_string(), decision.clone()));
        let waiting = g.sessions.get_mut(id).and_then(|s| match &s.asked {
            Some((a, _, _, _)) if a == ask => s.asked.take(),
            _ => None,
        });
        let Some((_, turn, call, then)) = waiting else {
            return Ok(());
        };
        g.emit(id, Frame::ToolResult { turn: turn.clone(), call, agent: None });
        match decision {
            Decision::Allow => g.play(id, &turn, then),
            Decision::Deny { .. } => g.play(id, &turn, Turn::Ends { output: None }),
        }
        Ok(())
    }

    async fn note(&self, id: &SessionId, note: Note) -> Result<(), HostError> {
        let mut g = self.lock();
        g.live(id)?;
        g.notes.push((id.clone(), note));
        Ok(())
    }

    async fn project_trusted(&self, root: &Path) -> Result<bool, HostError> {
        let g = self.lock();
        if g.unavailable {
            return Err(HostError::Unavailable("the scripted host is away".into()));
        }
        Ok(g.trust.get(root).copied().unwrap_or(false))
    }

    async fn run_over(&self, run: &str) {
        self.lock().over.push(run.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::host::Delivery;
    use futures::StreamExt;

    fn spec(key: &str) -> OpenSpec {
        OpenSpec::new(key, "r", "flow-harnix", Delivery::FlowSubmit)
    }

    #[tokio::test]
    async fn a_key_opens_once_and_a_station_plays_its_script() {
        let h = ScriptedHost::new();
        h.station("a", vec![Turn::Ends { output: Some(json!({"n": 1})) }, Turn::Runs]);
        let first = h.open(spec("flow:r/a/a@tok")).await.unwrap();
        assert!(!first.existed);
        let again = h.open(spec("flow:r/a/a@tok")).await.unwrap();
        assert_eq!((again.id.as_str(), again.existed), (first.id.as_str(), true));
        let mut other = spec("flow:r/a/a@tok");
        other.main = true;
        assert!(matches!(h.open(other).await, Err(HostError::Refused(_))));

        let mut w = h.watch(&first.id).await.unwrap();
        assert!(!w.info.turn_running);
        h.send(&first.id, "do it").await.unwrap();
        assert_eq!(w.frames.next().await.unwrap(), Frame::TurnStarted { turn: "t2".into(), text: "do it".into() });
        assert!(matches!(w.frames.next().await.unwrap(), Frame::TurnSettled { end: TurnEnd::Done, output: Some(_), .. }));
        h.send(&first.id, "again").await.unwrap();
        let _ = w.frames.next().await;
        assert!(h.running(&first.id));
        h.interrupt(&first.id).await.unwrap();
        assert!(matches!(w.frames.next().await.unwrap(), Frame::TurnSettled { end: TurnEnd::Interrupted, .. }));
        assert_eq!(h.sent_to(&first.id), ["do it", "again"]);
        h.close(&first.id).await.unwrap();
        assert!(w.frames.next().await.is_none(), "the stream ends with the session");
        assert!(matches!(h.send(&first.id, "x").await, Err(HostError::Gone(_))));
        assert_eq!(h.lookup(&first.id).await.unwrap(), None);
    }

    #[tokio::test]
    async fn a_question_waits_for_its_answer() {
        let h = ScriptedHost::new();
        h.main(vec![Turn::Asks {
            tool: "Write".into(),
            paths: vec!["/p/a.txt".into()],
            input: Value::Null,
            then: Box::new(Turn::Ends { output: Some(json!({"ok": true})) }),
        }]);
        let s = h.open(spec("flow:r@main")).await.unwrap().id;
        let mut w = h.watch(&s).await.unwrap();
        h.send(&s, "go").await.unwrap();
        let _started = w.frames.next().await;
        assert!(matches!(w.frames.next().await.unwrap(), Frame::ToolCall { tool, .. } if tool == "Write"));
        let Frame::Asked { ask, paths, .. } = w.frames.next().await.unwrap() else { panic!() };
        assert_eq!(paths, ["/p/a.txt"]);
        assert!(h.running(&s));
        h.answer(&s, &ask, Decision::Allow).await.unwrap();
        assert!(matches!(w.frames.next().await.unwrap(), Frame::ToolResult { .. }));
        assert!(matches!(w.frames.next().await.unwrap(), Frame::TurnSettled { output: Some(_), .. }));
        assert_eq!(h.answers().len(), 1);
        h.note(&s, Note { event: "proposal".into(), payload: json!({}) }).await.unwrap();
        assert_eq!(h.notes().len(), 1);
        h.run_over("r").await;
        assert_eq!(h.over(), ["r"]);
    }

    #[test]
    fn scripts_are_read_off_keys() {
        assert_eq!(ScriptedHost::script_of("flow:r/a/t@tok"), "station:a");
        assert_eq!(ScriptedHost::script_of("flow:r#run"), "key:run");
        assert_eq!(ScriptedHost::script_of("flow:r@main"), "main");
    }
}
