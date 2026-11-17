//! The conformance suite: what every [`SessionHost`] must do, checked against a live one.
//!
//! The checks do not depend on what a model says — only on the host's contract (`runner::host`):
//! a key opens once, a send starts exactly one turn and that turn settles, a permission question
//! can be answered, a closed session is gone. A host gives a [`Subject`]: itself, a profile its
//! sessions can be opened on, and messages that make a session do something known — one that
//! makes it answer and stop, and (when it can) one that makes it call a guarded tool so a
//! question comes. Harnix runs it against its hub with a recording; `ScriptedHost` runs it with
//! a script (`tests/conformance.rs`), which keeps the suite itself honest.
//!
//! ```no_run
//! # use std::sync::Arc;
//! # use harnix_flow::conformance::{run_all, Subject};
//! # use harnix_flow::runner::ScriptedHost;
//! # async fn go() {
//! let host = ScriptedHost::new();
//! let subject = Subject::new(host, "flow-harnix").idle_turn("Say hello and stop.");
//! let failures = run_all(&subject).await;
//! assert!(failures.is_empty(), "{failures:?}");
//! # }
//! ```

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use serde_json::json;

use crate::kernel::host::Delivery;
use crate::runner::host::{Decision, Frame, HostError, Note, OpenSpec, SessionHost, SessionId, Watch};

/// A host under test, and what makes its sessions do something known.
pub struct Subject {
    pub host: Arc<dyn SessionHost>,
    /// A profile the host opens sessions on.
    pub profile: String,
    /// A project the host accepts, for the trust check; none skips it.
    pub project: Option<PathBuf>,
    /// A message after which the session says something and stops.
    pub idle_turn: String,
    /// A message after which the session calls a tool the host asks about (`asks` lists it), so
    /// a [`Frame::Asked`] comes; none skips that check.
    pub asking_turn: Option<(String, String)>,
    /// How long one turn may take to settle.
    pub turn_timeout: Duration,
}

impl Subject {
    pub fn new(host: Arc<dyn SessionHost>, profile: impl Into<String>) -> Self {
        Self {
            host,
            profile: profile.into(),
            project: None,
            idle_turn: "Say hello and stop.".into(),
            asking_turn: None,
            turn_timeout: Duration::from_secs(60),
        }
    }

    pub fn idle_turn(mut self, text: impl Into<String>) -> Self {
        self.idle_turn = text.into();
        self
    }

    /// A message that makes the session call `tool`, which the session is opened asking about.
    pub fn asking_turn(mut self, text: impl Into<String>, tool: impl Into<String>) -> Self {
        self.asking_turn = Some((text.into(), tool.into()));
        self
    }

    pub fn project(mut self, root: impl Into<PathBuf>) -> Self {
        self.project = Some(root.into());
        self
    }

    pub fn turn_timeout(mut self, t: Duration) -> Self {
        self.turn_timeout = t;
        self
    }

    /// A spec of a fresh key: no two runs of the suite against a host that keeps its sessions
    /// collide.
    fn spec(&self, what: &str) -> OpenSpec {
        let run = format!("conformance-{}", uuid::Uuid::new_v4().simple());
        let mut spec = OpenSpec::new(format!("flow:{run}/conformance/{what}@t1"), run, self.profile.clone(), Delivery::FlowSubmit);
        spec.project_root = self.project.clone();
        spec
    }
}

/// One check that did not hold.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    pub check: &'static str,
    pub why: String,
}

/// Every check, each against a session of its own; the failures, none when the host conforms.
pub async fn run_all(s: &Subject) -> Vec<Failure> {
    let mut out = vec![];
    let checks: [(&'static str, Check); 7] = [
        ("open_is_idempotent", Box::new(|s| Box::pin(open_is_idempotent(s)))),
        ("lookup_of_nothing_is_none", Box::new(|s| Box::pin(lookup_of_nothing_is_none(s)))),
        ("a_send_starts_one_turn_that_settles", Box::new(|s| Box::pin(a_send_starts_one_turn_that_settles(s)))),
        ("interrupt_of_an_idle_session_is_ok", Box::new(|s| Box::pin(interrupt_of_an_idle_session_is_ok(s)))),
        ("a_question_is_asked_and_answered", Box::new(|s| Box::pin(a_question_is_asked_and_answered(s)))),
        ("a_closed_session_is_gone", Box::new(|s| Box::pin(a_closed_session_is_gone(s)))),
        ("a_note_and_trust_are_answered", Box::new(|s| Box::pin(a_note_and_trust_are_answered(s)))),
    ];
    for (name, check) in checks {
        if let Err(why) = check(s).await {
            out.push(Failure { check: name, why });
        }
    }
    out
}

type Check = Box<dyn for<'a> Fn(&'a Subject) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), String>> + Send + 'a>> + Send + Sync>;

async fn open(s: &Subject, spec: OpenSpec) -> Result<SessionId, String> {
    s.host.open(spec).await.map(|o| o.id).map_err(|e| format!("open: {e}"))
}

async fn next(w: &mut Watch, timeout: Duration) -> Result<Frame, String> {
    match tokio::time::timeout(timeout, w.frames.next()).await {
        Ok(Some(f)) => Ok(f),
        Ok(None) => Err("the stream ended".into()),
        Err(_) => Err(format!("no frame within {timeout:?}")),
    }
}

/// The next frame that is not a `Lagged`.
async fn next_frame(w: &mut Watch, timeout: Duration) -> Result<Frame, String> {
    loop {
        match next(w, timeout).await? {
            Frame::Lagged => continue,
            f => return Ok(f),
        }
    }
}

async fn open_is_idempotent(s: &Subject) -> Result<(), String> {
    let spec = s.spec("open");
    let first = s.host.open(spec.clone()).await.map_err(|e| format!("open: {e}"))?;
    if first.existed {
        return Err("a fresh key is said to have existed".into());
    }
    let again = s.host.open(spec.clone()).await.map_err(|e| format!("open again: {e}"))?;
    if again.id != first.id || !again.existed {
        return Err(format!("the same key opened as {:?} (existed {}) then {:?} (existed {})", first.id, first.existed, again.id, again.existed));
    }
    let mut other = spec;
    other.main = !other.main;
    match s.host.open(other).await {
        Err(HostError::Refused(_)) => {}
        Ok(o) => return Err(format!("the same key with another spec opened as {:?}", o.id)),
        Err(e) => return Err(format!("the same key with another spec: {e}, not Refused")),
    }
    s.host.close(&first.id).await.map_err(|e| format!("close: {e}"))?;
    Ok(())
}

async fn lookup_of_nothing_is_none(s: &Subject) -> Result<(), String> {
    let id = format!("conformance-nothing-{}", uuid::Uuid::new_v4().simple());
    match s.host.lookup(&id).await {
        Ok(None) | Err(HostError::Gone(_)) => Ok(()),
        Ok(Some(info)) => Err(format!("a session nobody opened is there: {info:?}")),
        Err(e) => Err(format!("lookup: {e}, not None or Gone")),
    }
}

async fn a_send_starts_one_turn_that_settles(s: &Subject) -> Result<(), String> {
    let id = open(s, s.spec("send")).await?;
    let found = s.host.lookup(&id).await.map_err(|e| format!("lookup: {e}"))?;
    if found.is_none() {
        return Err("a session just opened is not found".into());
    }
    let mut w = s.host.watch(&id).await.map_err(|e| format!("watch: {e}"))?;
    if w.info.turn_running {
        return Err("a session just opened has a turn running".into());
    }
    s.host.send(&id, &s.idle_turn).await.map_err(|e| format!("send: {e}"))?;
    match next_frame(&mut w, s.turn_timeout).await? {
        Frame::TurnStarted { text, .. } if text == s.idle_turn => {}
        f => return Err(format!("the first frame after a send is {f:?}, not TurnStarted with the text sent")),
    }
    let mut settled = 0;
    let mut other_turns = 0;
    loop {
        match next_frame(&mut w, s.turn_timeout).await? {
            Frame::TurnSettled { .. } => {
                settled += 1;
                break;
            }
            Frame::TurnStarted { .. } => other_turns += 1,
            _ => {}
        }
    }
    if other_turns > 0 {
        return Err(format!("{other_turns} more turns started from one send"));
    }
    // Nothing more comes of it: no second settlement.
    if let Ok(Frame::TurnSettled { .. }) = next_frame(&mut w, Duration::from_millis(200)).await {
        settled += 1;
    }
    if settled != 1 {
        return Err(format!("{settled} settlements for one send"));
    }
    s.host.close(&id).await.map_err(|e| format!("close: {e}"))?;
    Ok(())
}

async fn interrupt_of_an_idle_session_is_ok(s: &Subject) -> Result<(), String> {
    let id = open(s, s.spec("interrupt")).await?;
    s.host.interrupt(&id).await.map_err(|e| format!("interrupt of an idle session: {e}"))?;
    s.host.close(&id).await.map_err(|e| format!("close: {e}"))?;
    Ok(())
}

async fn a_question_is_asked_and_answered(s: &Subject) -> Result<(), String> {
    let Some((text, tool)) = &s.asking_turn else {
        return Ok(());
    };
    let mut spec = s.spec("ask");
    spec.asks = vec![tool.clone()];
    let id = open(s, spec).await?;
    let mut w = s.host.watch(&id).await.map_err(|e| format!("watch: {e}"))?;
    s.host.send(&id, text).await.map_err(|e| format!("send: {e}"))?;
    let ask = loop {
        match next_frame(&mut w, s.turn_timeout).await? {
            Frame::Asked { ask, tool: t, .. } if t == *tool => break ask,
            Frame::Asked { tool: t, .. } => return Err(format!("asked about {t}, not {tool}")),
            Frame::TurnSettled { .. } => return Err("the turn settled without asking".into()),
            _ => {}
        }
    };
    s.host.answer(&id, &ask, Decision::Allow).await.map_err(|e| format!("answer: {e}"))?;
    let (mut result, mut settled) = (false, false);
    while !settled {
        match next_frame(&mut w, s.turn_timeout).await? {
            Frame::ToolResult { .. } => result = true,
            Frame::TurnSettled { .. } => settled = true,
            _ => {}
        }
    }
    if !result {
        return Err("no ToolResult came after the question was allowed".into());
    }
    // An answer to a question whose turn is over is nothing to mind.
    match s.host.answer(&id, &ask, Decision::Deny { reason: "late".into() }).await {
        Ok(()) | Err(HostError::Gone(_)) => {}
        Err(e) => return Err(format!("a late answer: {e}, not Ok or Gone")),
    }
    s.host.close(&id).await.map_err(|e| format!("close: {e}"))?;
    Ok(())
}

async fn a_closed_session_is_gone(s: &Subject) -> Result<(), String> {
    let id = open(s, s.spec("close")).await?;
    let mut w = s.host.watch(&id).await.map_err(|e| format!("watch: {e}"))?;
    s.host.close(&id).await.map_err(|e| format!("close: {e}"))?;
    match tokio::time::timeout(s.turn_timeout, async {
        while let Some(f) = w.frames.next().await {
            if matches!(f, Frame::TurnStarted { .. }) {
                return Err("a turn started in a closed session".to_string());
            }
        }
        Ok(())
    })
    .await
    {
        Ok(r) => r?,
        Err(_) => return Err("the stream of a closed session did not end".into()),
    }
    match s.host.send(&id, &s.idle_turn).await {
        Err(HostError::Gone(_)) => {}
        Ok(()) => return Err("a send to a closed session went through".into()),
        Err(e) => return Err(format!("a send to a closed session: {e}, not Gone")),
    }
    match s.host.lookup(&id).await {
        Ok(None) | Err(HostError::Gone(_)) => Ok(()),
        Ok(Some(_)) => Err("a closed session is still looked up".into()),
        Err(e) => Err(format!("lookup of a closed session: {e}")),
    }
}

async fn a_note_and_trust_are_answered(s: &Subject) -> Result<(), String> {
    let id = open(s, s.spec("note")).await?;
    s.host
        .note(&id, Note { event: "proposal".into(), payload: json!({"kind": "cancel", "reason": "conformance"}) })
        .await
        .map_err(|e| format!("note: {e}"))?;
    if let Some(p) = &s.project {
        s.host.project_trusted(p).await.map_err(|e| format!("project_trusted: {e}"))?;
    }
    s.host.close(&id).await.map_err(|e| format!("close: {e}"))?;
    Ok(())
}
