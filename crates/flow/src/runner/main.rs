//! The run's main session — the flow's main Agent.
//!
//! It is opened when the run is created, where the flow's `main` places it (the runner's main
//! profile by default) — in the run's project when the run is a task (`session_root`) — and
//! recorded as the run's `@main` session key, so the session tasks bound to `main` and the
//! sub-agent tasks take their turns in it (the shared session's lock, `flow:<run>#@main`).
//!
//! Every flow session — the main one and a station's own — asks about every guarded tool and
//! every tool a permission of the flow names. While a turn works on a task, the runner answers
//! those questions itself from that task's permissions: `allow` is permitted, `deny` refused with
//! its reason, `ask` left to a person — the run waits for `approval` until it is answered. A
//! station's limit so holds in a session that every station shares. A turn that works on no task
//! (a person talking to the main Agent) is asked of the person as ever. The main session's watch
//! also counts each turn's tokens on the task it worked on, or on the run (`Engine::account`), and
//! notes the sub-agents it starts.

use std::sync::Arc;

use futures::StreamExt;
use serde_json::{json, Value};

use super::dispatch::settle_quiet;
use super::host::{Decision, Frame, OpenSpec, SessionId, Watch};
use super::session::{asks_of, session_root};
use super::state::Binding;
use super::Runner;
use crate::kernel::def::Guard;
use crate::kernel::def::MAIN_SESSION_KEY;
use crate::kernel::parts::{judge_under, Call, Effect, Verdict};
use crate::kernel::run::{RunRecord, Usage};

/// The key the main session is opened under, and the lock its turns take (the shared-session
/// lock of the `@main` key).
pub(crate) fn main_key(run: &str) -> String {
    format!("flow:{run}#{MAIN_SESSION_KEY}")
}

/// What a main session is opened with (what its tasks may do is answered turn by turn).
pub(crate) fn main_settings(scene: &str, p: &crate::Placement, instance: Option<&str>) -> Value {
    json!({
        "scene": scene, "delivery": crate::Delivery::FlowSubmit, "main": true,
        "model": p.model, "provider": p.provider, "effort": p.effort, "space": p.space, "template": p.template,
        "instance": instance.map(String::from).or_else(|| p.instance.clone()),
    })
}

/// Open the run's main session if it has none (or the one it had is gone), under the main
/// session's lock; then watch it.
pub(crate) async fn ensure_main(rn: &Arc<Runner>, run: &str) -> Result<SessionId, String> {
    let lock_name = main_key(run);
    let lock = rn.state.session_lock(&lock_name);
    let r = {
        let _g = lock.clone().lock_owned().await;
        open_or_make(rn, run).await
    };
    rn.state.release_lock(&lock_name, lock);
    if let Ok(s) = &r {
        watch(rn, run, s, true).await;
    }
    r
}

/// With the main session's lock held: the session the run names if it is still there, else a
/// new one recorded on the run. Its replay is the runner's `main_recording` for the run — not a
/// task's: whichever comes first opens it.
pub(crate) async fn open_or_make(rn: &Arc<Runner>, run: &str) -> Result<SessionId, String> {
    let engine = rn.engine();
    let host = rn.host();
    let rec = engine.record(run).await.map_err(|e| e.to_string())?;
    if let Some(id) = rec.main_session() {
        if let Ok(Some(_)) = host.lookup(&id.to_string()).await {
            return Ok(id.to_string());
        }
    }
    let (placement, scene) = engine.main_placement(&rec);
    let scene = scene.ok_or("the flow names no main scene and the runner has none")?;
    let wanted = main_settings(&scene, &placement, placement.template_instance(run).as_deref());
    let mut spec = OpenSpec::new(main_key(run), run, scene.clone(), crate::Delivery::FlowSubmit);
    spec.main = true;
    spec.title = Some(main_title(&rec));
    spec.project_root = session_root(rn, &rec, &scene)?;
    spec.placement = placement;
    spec.asks = asks_of(rn, &rec);
    spec.shown_in = rec.shown_in.clone();
    spec.recording = rn.cfg.main_recording.as_ref().and_then(|f| f(&rec));
    spec.tools = rn.tools().specs().to_vec();
    let opened = host.open(spec).await.map_err(|e| format!("no main session: {e}"))?;
    let attached = engine.attach_main(run, &opened.id, wanted).await;
    settle_quiet(rn, run, attached).await;
    Ok(opened.id)
}

/// The run whose main session this is: the one watching it while it runs, else the newest run
/// whose record names it; `None` for a session that is no run's main.
pub(crate) async fn run_of_main(rn: &Runner, id: &SessionId) -> Option<String> {
    if let Some(run) = rn.state.watched_run(id) {
        return Some(run);
    }
    let engine = rn.engine();
    for summary in engine.list(crate::kernel::run::RunQuery::default()).await.ok()? {
        if let Ok(rec) = engine.record(&summary.id).await {
            if rec.main_session() == Some(id.as_str()) {
                return Some(rec.id);
            }
        }
    }
    None
}

/// The main session's name in the session list: the run's title, else its flow's and its id.
fn main_title(rec: &RunRecord) -> String {
    let flow = if rec.def.title.is_empty() { &rec.flow } else { &rec.def.title };
    if rec.title.is_empty() {
        format!("{flow} · {}", rec.id)
    } else {
        format!("{} · {}", rec.title, rec.id)
    }
}

/// Watch a flow session while the run lasts, once: its questions answered (every session), its
/// turns counted and its sub-agents noted (the main one, `main`; a station's own session's turns
/// are counted by the task carrying it).
///
/// The session is watched before this returns, so nothing sent to it afterwards goes unseen.
pub(crate) async fn watch(rn: &Arc<Runner>, run: &str, s: &SessionId, main: bool) {
    if !rn.state.watch(s, run) {
        return;
    }
    let Ok(watching) = rn.host().watch(s).await else {
        rn.state.unwatch(s);
        return;
    };
    let (rn2, run, s) = (rn.clone(), run.to_string(), s.clone());
    rn.spawn(async move {
        watch_loop(&rn2, &run, &s, main, watching).await;
        rn2.state.unwatch(&s);
    });
}

/// The tool calls a watch keeps the input of, to judge the question that may follow.
const CALLS_KEPT: usize = 64;

async fn watch_loop(rn: &Arc<Runner>, run: &str, s: &SessionId, main: bool, mut watching: Watch) {
    // What it asks about (the pinned flow's), how the flow judges, and where the run works. Under
    // `strict` a question about any other tool is the host's own, and a person's to answer; under
    // `auto` the host's own questions of a task's turn (a write it holds back on its own: `.git`,
    // outside the project) are answered by the same rules.
    let engine = rn.engine();
    let Ok(rec) = engine.record(run).await else { return };
    let asked = asks_of(rn, &rec);
    // The session's project, else the run's directory: what a path is inside or outside of.
    let workspace = engine.workspace_for(&rec).and_then(|p| p.to_str().map(String::from));
    let root = watching
        .info
        .project_root
        .as_ref()
        .and_then(|p| p.to_str().map(String::from))
        .or_else(|| workspace.clone());
    // The project's data directory there is the host's: outside what the task works in, but for
    // what the host's sandbox lets a session write there.
    let (reserved, agent_dirs) = match (engine.layout(), &root) {
        (Some(l), Some(r)) => (l.reserved(r), l.agent_dirs_in(r)),
        _ => (vec![], vec![]),
    };
    let judging = Judging { guard: rec.def.guard, workspace, root, reserved, agent_dirs };
    // Turn → the task it was started for: read when the turn's first frame comes, while that
    // task is in hand (by its settlement it has often handed in and been let go).
    let mut turns: std::collections::HashMap<String, Option<Binding>> = std::collections::HashMap::new();
    // `Agent` calls of the main Agent: the call → what it named the sub-agent.
    let mut agent_calls: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    // Tool calls, newest last — (call, tool, input): what a question about a call is judged on.
    let mut inputs: std::collections::VecDeque<(String, String, Value)> = Default::default();
    // Questions held for a person: the call (the question's id when it names none) → the task.
    let mut held: std::collections::HashMap<String, Binding> = Default::default();
    loop {
        let frame = tokio::select! {
            f = watching.frames.next() => f,
            _ = rn.cancelled().cancelled() => return,
            _ = tokio::time::sleep(std::time::Duration::from_secs(5)) => {
                // A run that is over stops being watched.
                if rn.stopping() || !run_active(rn, run).await {
                    return;
                }
                continue;
            }
        };
        let Some(frame) = frame else { return };
        match frame {
            // A turn begins with its message: one the dispatcher sent for a task is that task's,
            // any other a person's.
            Frame::TurnStarted { turn, text } => {
                turns.insert(turn, rn.state.take_sent(s, &text));
            }
            Frame::TurnSettled { turn, usage, .. } => {
                let task = turns.remove(&turn).flatten().map(|b| (b.station, b.task));
                // Whatever was held is over with the turn.
                for (_, b) in held.drain() {
                    unhold(rn, &b).await;
                }
                if let (true, Some(u)) = (main, usage) {
                    count(rn, run, task, u).await;
                }
            }
            Frame::ToolCall { call, tool, input, .. } => {
                if main && tool == "Agent" {
                    let d = input.get("description").and_then(Value::as_str).unwrap_or_default();
                    agent_calls.insert(call.clone(), d.to_string());
                }
                inputs.push_back((call, tool, input));
                if inputs.len() > CALLS_KEPT {
                    inputs.pop_front();
                }
            }
            Frame::ToolResult { call, .. } => {
                if let Some(b) = held.remove(&call) {
                    unhold(rn, &b).await;
                }
            }
            Frame::Asked { turn, ask, call, tool, paths, .. } => {
                if !answers(&judging, &asked, &tool) {
                    continue;
                }
                // The turn's task; a turn begun before this watch saw it (a restart, a lag) is
                // the session's bound task's.
                let owner = match turns.get(&turn) {
                    Some(o) => o.clone(),
                    None => rn.state.binding(s),
                };
                // By the call; a sub-agent's question names none — its latest call of the tool.
                let input = match &call {
                    Some(id) => inputs.iter().rev().find(|(k, _, _)| k == id),
                    None => inputs.iter().rev().find(|(_, t, _)| *t == tool),
                }
                .map(|(_, _, v)| v.clone());
                if let Some(b) = owner {
                    let question = Question { ask: &ask, tool: &tool, paths: &paths };
                    let asked_person = answer(rn, s, &question, input.as_ref(), &judging, &b).await;
                    if asked_person {
                        held.insert(call.unwrap_or(ask), b);
                    }
                }
            }
            Frame::AgentStarted { call, agent, agent_type, .. } if main => {
                let description = call.as_ref().and_then(|c| agent_calls.get(c)).cloned();
                let flow = description
                    .as_deref()
                    .and_then(|d| d.strip_prefix(&format!("flow:{run}/")))
                    .and_then(|r| r.split_once('/'))
                    .map(|(st, t)| json!({"station": st, "task": t}));
                rn.state.note_agent(
                    run,
                    json!({
                        "agent": agent, "agent_type": agent_type, "call": call,
                        "description": description, "flow": flow, "status": Value::Null,
                    }),
                );
            }
            Frame::AgentEnded { agent, ok, .. } if main => {
                rn.state.note_agent(run, json!({"agent": agent, "status": if ok { "done" } else { "failed" }}));
            }
            _ => {}
        }
    }
}

async fn run_active(rn: &Runner, run: &str) -> bool {
    rn.engine().record(run).await.is_ok_and(|r| r.is_active())
}

async fn count(rn: &Arc<Runner>, run: &str, task: Option<(String, String)>, u: Usage) {
    let task = task.as_ref().map(|(st, t)| (st.as_str(), t.as_str()));
    let r = rn.engine().account(run, task, u).await;
    settle_quiet(rn, run, r).await;
}

/// How a session's questions are judged, from its run: the flow's guard, where the run works
/// (`${workspace}`, what relative path patterns are taken under) and what a path must be inside —
/// the session's project, else the run's directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Judging {
    pub guard: Guard,
    pub workspace: Option<String>,
    pub root: Option<String>,
    /// The project's data directory in the root (`ProjectLayout::reserved`).
    pub reserved: Vec<String>,
    /// What a session writes as its own in it (`ProjectLayout::agent_dirs_in`).
    pub agent_dirs: Vec<String>,
}

/// A permission question, as a frame carries it.
struct Question<'a> {
    ask: &'a str,
    tool: &'a str,
    paths: &'a [String],
}

/// Whether the runner answers a question of a task's turn: one about a tool the flow asks about;
/// under `auto` every permission question (the host's own too). The host sends only permission
/// questions: one for words (the model's, an MCP server's) is never the runner's.
pub(crate) fn answers(j: &Judging, asked: &[String], tool: &str) -> bool {
    j.guard == Guard::Auto || crate::kernel::parts::tool_matches(asked, tool)
}

/// What becomes of a question about a call of a task's turn: its permissions judge it.
fn verdict(rn: &Runner, b: &Binding, q: &Question<'_>, input: Option<&Value>, j: &Judging) -> Verdict {
    judge_under(
        j.guard,
        &b.permissions,
        &rn.cfg.guarded,
        &Call {
            tool: q.tool,
            input,
            paths: q.paths,
            workspace: j.workspace.as_deref(),
            root: j.root.as_deref(),
            reserved: &j.reserved,
            agent_dirs: &j.agent_dirs,
        },
        &b.station,
    )
}

/// The answer to give the host for a verdict; `None` for `ask` (a person answers).
pub(crate) fn decision(v: &Verdict) -> Option<Decision> {
    match v.effect {
        Effect::Allow => Some(Decision::Allow),
        Effect::Deny => Some(Decision::Deny { reason: v.reason.clone() }),
        Effect::Ask => None,
    }
}

/// Answer a question of a task's turn from its permissions. Returns whether it was left to a
/// person (the task is marked as waiting for them).
async fn answer(rn: &Arc<Runner>, s: &SessionId, q: &Question<'_>, input: Option<&Value>, j: &Judging, b: &Binding) -> bool {
    let v = verdict(rn, b, q, input, j);
    let Some(d) = decision(&v) else {
        let r = rn.engine().asking(&b.run, &b.station, &b.task, &b.token, Some((q.tool.to_string(), v.reason))).await;
        settle_quiet(rn, &b.run, r).await;
        return true;
    };
    if let Err(e) = rn.host().answer(s, q.ask, d).await {
        tracing::warn!(tool = q.tool, error = %e, "a session's question not answered");
    }
    false
}

/// A held call was answered (or its turn is over): the task works again.
async fn unhold(rn: &Arc<Runner>, b: &Binding) {
    let r = rn.engine().asking(&b.run, &b.station, &b.task, &b.token, None).await;
    settle_quiet(rn, &b.run, r).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_verdict_is_an_answer_unless_a_person_is_asked() {
        let v = |effect, reason: &str| Verdict { effect, reason: reason.into() };
        assert_eq!(decision(&v(Effect::Allow, "")), Some(Decision::Allow));
        assert_eq!(
            decision(&v(Effect::Deny, "station s does not allow Bash")),
            Some(Decision::Deny { reason: "station s does not allow Bash".into() })
        );
        assert_eq!(decision(&v(Effect::Ask, "a nod")), None);
    }

    #[test]
    fn under_auto_every_permission_question_of_a_task_is_answered_and_under_strict_only_the_asked_ones() {
        let asked = vec!["Write".to_string()];
        let j = |guard| Judging { guard, workspace: None, root: None, reserved: vec![], agent_dirs: vec![] };
        assert!(answers(&j(Guard::Strict), &asked, "Write"));
        assert!(!answers(&j(Guard::Strict), &asked, "Read"), "the host's own: a person's");
        assert!(answers(&j(Guard::Auto), &asked, "Read"));
        assert!(answers(&j(Guard::Auto), &asked, "Glob"));
    }

    #[test]
    fn a_main_session_is_named_after_its_run() {
        let rec: RunRecord = serde_json::from_value(json!({
            "id": "r-1", "flow": "f", "def_digest": "d", "subject": "s", "status": "active", "title": "Look",
            "rev": 1, "created_at": 0, "created_by": {"kind": "x", "id": "y"},
            "def": {"name": "f", "title": "Flow F", "stations": [{"id": "a", "exit": [{"kind": "confirm"}]}]},
            "stations": {}
        }))
        .unwrap();
        assert_eq!(main_title(&rec), "Look · r-1");
        let untitled = RunRecord { title: String::new(), ..rec };
        assert_eq!(main_title(&untitled), "Flow F · r-1");
        assert_eq!(main_key("r-1"), "flow:r-1#@main");
    }
}
