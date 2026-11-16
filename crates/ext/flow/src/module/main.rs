//! The run's main session — the flow's main Agent (xops-03 §1).
//!
//! It is opened when the run is created, where the flow's `main` places it (the module's main
//! scene in the module's space by default), and recorded as the run's `@main` session key, so
//! the session tasks bound to `main` and the sub-agent tasks take their turns in it (the shared
//! session's lock, `run#@main`).
//!
//! Every flow session — the main one and a station's own — `ask`s about every guarded tool and
//! every tool a permission of the flow names. While a turn works on a task, the module answers
//! those questions itself from that task's permissions (xops-04 §3): `allow` is permitted, `deny`
//! refused with its reason, `ask` left to a person — the run waits for `approval` until it is
//! answered. A station's limit so holds in a session that every station shares. A turn that works
//! on no task (a person talking to the main Agent) is asked of the person as ever. The main
//! session's watch also counts each turn's tokens on the task it worked on, or on the run
//! (`Engine::account`), and notes the sub-agents it starts.

use std::sync::Arc;

use harnix_ext::{Entry, Exposure, SessionRef, SessionSpec, TapEvent};
use serde_json::{json, Value};

use super::dispatch::settle_quiet;
use super::session::{place_at, SESSION_SETTINGS, TASK_PERMISSION_MODE};
use super::shared::{session_name, session_ref, Binding, Shared};
use crate::kernel::def::MAIN_SESSION_KEY;
use crate::kernel::parts::{judge, Call, Effect, Verdict};
use crate::kernel::run::{RunRecord, Usage};

/// The lock the main session's turns take (the shared-session lock of the `@main` key).
pub(crate) fn main_lock(run: &str) -> String {
    format!("{run}#{MAIN_SESSION_KEY}")
}

/// What a main session is opened with (what its tasks may do is answered turn by turn).
pub(crate) fn main_settings(scene: &str, p: &crate::Placement, instance: Option<&str>) -> Value {
    json!({
        "scene": scene, "delivery": crate::Delivery::FlowSubmit, "main": true,
        "model": p.model, "provider": p.provider, "space": p.space, "template": p.template,
        "instance": instance.map(String::from).or_else(|| p.instance.clone()),
    })
}

/// Open the run's main session if it has none (or the one it had is gone), under the main
/// session's lock; then watch it.
pub(crate) async fn ensure_main(sh: &Arc<Shared>, run: &str) -> Result<SessionRef, String> {
    let lock_name = main_lock(run);
    let lock = sh.session_lock(&lock_name);
    let r = {
        let _g = lock.clone().lock_owned().await;
        open_or_make(sh, run).await
    };
    sh.release_lock(&lock_name, lock);
    if let Ok(s) = &r {
        watch(sh, run, s, true).await;
    }
    r
}

/// With the main session's lock held: the session the run names if it opens, else a new one
/// recorded on the run. Its replay is the module's `main_recording` for the run — not a
/// task's: whichever comes first opens it.
pub(crate) async fn open_or_make(sh: &Arc<Shared>, run: &str) -> Result<SessionRef, String> {
    let engine = sh.engine().map_err(|e| e.message)?;
    let cx = sh.cx.get().ok_or("the flow module has not started")?;
    let rec = engine.record(run).await.map_err(|e| e.to_string())?;
    if let Some(name) = rec.main_session() {
        let s = session_ref(name);
        if let Ok(s) = cx.sessions().open(&s.instance, &s.session_id).await {
            return Ok(s);
        }
    }
    let (placement, scene) = crate::kernel::work::main_of(&rec, engine.main_scene());
    let scene = scene.ok_or("the flow names no main scene and the module has none")?;
    let (space, instance) = place_at(sh, &placement, run).await?;
    let wanted = main_settings(&scene, &placement, placement.template_instance(run).as_deref());
    let recording = sh.cfg.main_recording.as_ref().and_then(|f| f(&rec));
    // A scene that needs a project works in a directory of the run's own.
    let project_root = if sh.cfg.project_scenes.contains(&scene) {
        let dir = engine
            .workspace(run)
            .unwrap_or_else(|| crate::kernel::refs::workspace_of(&cx.data_dir().join("workspaces"), run));
        std::fs::create_dir_all(&dir).map_err(|e| format!("no workspace for the run: {e}"))?;
        Some(dir)
    } else {
        None
    };
    let spec = SessionSpec::new()
        .space(&space)
        .scene(&scene)
        .permission_mode(TASK_PERMISSION_MODE)
        .permission_rules(super::session::flow_rules(sh, &rec));
    let spec = super::spec_with(spec, instance, |s, v| s.instance(&v));
    let spec = super::spec_with(spec, project_root, |s, v| s.project_root(v));
    let spec = super::spec_with(spec, placement.model.clone(), |s, v| s.model(&v));
    let spec = super::spec_with(spec, placement.provider.clone(), |s, v| s.provider(&v));
    let spec = super::spec_with(spec, recording, |s, v| s.recording(&v));
    let s = cx
        .sessions()
        .create(spec)
        .await
        .map_err(|e| format!("no main session: {}", e.message))?;
    // A task started from where a project's sessions are listed is shown there too, to be talked
    // to: the session stays the flow's, in the flow's space. A space the application does not
    // have is the hub's to refuse; the run goes on, and the task opens from the flow's side.
    if let Some(shown) = rec.show_in_space.as_deref().filter(|shown| *shown != space) {
        if let Err(e) = cx.sessions().expose(&s, shown, Exposure::interactive()).await {
            tracing::warn!(run, space = shown, error = %e.message, "the main session is not shown in the space the start named");
        }
    }
    let _ = cx
        .sessions()
        .append_entry(&s, Entry::new(SESSION_SETTINGS, wanted))
        .await;
    let title = main_title(&rec);
    let _ = cx
        .call(
            "engine.session.rename",
            json!({"instance": s.instance, "session_id": s.session_id, "name": title}),
        )
        .await;
    let attached = engine.attach_main(run, &session_name(&s)).await;
    settle_quiet(sh, run, attached).await;
    Ok(s)
}

/// The run whose main session this is (`flow.runOf`): the one watching it while it runs, else
/// the newest run whose record names it; `None` for a session that is no run's main.
pub(crate) async fn run_of_main(sh: &Arc<Shared>, instance: &str, session_id: &str) -> Option<String> {
    let name = session_name(&SessionRef::new(instance, session_id));
    if let Some(run) = sh.watched_run(session_id) {
        return Some(run);
    }
    let engine = sh.engine().ok()?;
    for summary in engine.list(crate::kernel::run::RunQuery::default()).await.ok()? {
        if let Ok(rec) = engine.record(&summary.id).await {
            if rec.main_session() == Some(name.as_str()) {
                return Some(rec.id);
            }
        }
    }
    None
}

/// The main session's name in the session list: the run's title, else its flow's and its id.
fn main_title(rec: &RunRecord) -> String {
    let flow = if rec.def.title.is_empty() {
        &rec.flow
    } else {
        &rec.def.title
    };
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
/// The session is tapped before this returns, so nothing sent to it afterwards goes unseen.
pub(crate) async fn watch(sh: &Arc<Shared>, run: &str, s: &SessionRef, main: bool) {
    if !sh.watch(&s.session_id, run) {
        return;
    }
    let tap = match sh.cx.get() {
        Some(cx) => cx.sessions().tap(s).await.ok(),
        None => None,
    };
    let Some(tap) = tap else {
        sh.unwatch(&s.session_id);
        return;
    };
    let (sh2, run, s) = (sh.clone(), run.to_string(), s.clone());
    sh.spawn(async move {
        watch_loop(&sh2, &run, &s, main, tap).await;
        sh2.unwatch(&s.session_id);
    });
}

/// The tool calls a watch keeps the input of, to judge the question that may follow.
const CALLS_KEPT: usize = 64;

async fn watch_loop(sh: &Arc<Shared>, run: &str, s: &SessionRef, main: bool, mut tap: harnix_ext::Tap) {
    // What it asks about (the pinned flow's), and where its project is: a question about any other
    // tool is the engine's own, and a person's to answer; a path outside the project is a
    // person's to allow.
    let asked: Vec<String> = match sh.engine() {
        Ok(e) => match e.record(run).await {
            Ok(rec) => crate::kernel::parts::asked_tools(&rec.def, &rec.parts.policies, &sh.cfg.guarded),
            Err(_) => return,
        },
        Err(_) => return,
    };
    let root: Option<String> = match sh.cx.get() {
        Some(cx) => cx
            .call("engine.session.get", json!({"instance": s.instance, "session_id": s.session_id}))
            .await
            .ok()
            .and_then(|v| v.get("project_root").and_then(Value::as_str).map(String::from)),
        None => None,
    };
    // Turn → the task it was started for: read when the turn's first frame comes, while that
    // task is in hand (by its settlement it has often handed in and been let go).
    let mut turns: std::collections::HashMap<String, Option<Binding>> =
        std::collections::HashMap::new();
    // `Agent` calls of the main Agent: tool use id → what it named the sub-agent.
    let mut agent_calls: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    // Tool calls, newest last — (id, tool, input): what a question about a call is judged on.
    let mut inputs: std::collections::VecDeque<(String, String, Value)> = Default::default();
    // Questions held for a person: the call's id (the question's when it names none) → the task.
    let mut held: std::collections::HashMap<String, Binding> = Default::default();
    loop {
        let ev = tokio::select! {
            ev = tap.next() => ev,
            _ = sh.cancel.cancelled() => return,
            _ = tokio::time::sleep(std::time::Duration::from_secs(5)) => {
                // A run that is over stops being watched.
                if sh.stopping() || !run_active(sh, run).await {
                    return;
                }
                continue;
            }
        };
        let frame = match ev {
            None => return,
            Some(TapEvent::Lagged { .. }) => {
                let _ = tap.catch_up_settled().await;
                continue;
            }
            Some(TapEvent::Frame(f)) => f,
            Some(_) => continue,
        };
        let turn = frame
            .pointer("/params/turn_id")
            .and_then(Value::as_str)
            .filter(|t| !t.is_empty())
            .map(String::from);
        // A turn begins with its `user_message`: one the dispatcher sent for a task is that
        // task's, any other a person's.
        if let (Some(t), Some(text)) = (
            &turn,
            frame
                .pointer("/params/event")
                .filter(|e| e.get("kind").and_then(Value::as_str) == Some("user_message"))
                .and_then(|e| e.get("text"))
                .and_then(Value::as_str),
        ) {
            turns.insert(t.clone(), sh.take_sent(&s.session_id, text));
        }
        match frame.get("method").and_then(Value::as_str) {
            Some("harnix.turn_settled") => {
                let task = turn
                    .as_ref()
                    .and_then(|t| turns.remove(t))
                    .flatten()
                    .map(|b| (b.station, b.task));
                // Whatever was held is over with the turn.
                for (_, b) in held.drain() {
                    unhold(sh, &b).await;
                }
                let usage = frame
                    .pointer("/params/outcome/usage")
                    .filter(|u| u.is_object())
                    .map(Usage::of_turn);
                if let (true, Some(u)) = (main, usage) {
                    count(sh, run, task, u).await;
                }
            }
            Some("harnix.event") => {
                let Some(ev) = frame.pointer("/params/event") else {
                    continue;
                };
                // A sub-agent's calls and questions come wrapped in its progress.
                let inner = match ev.get("kind").and_then(Value::as_str) {
                    Some("subagent_progress") => ev.get("event").unwrap_or(ev),
                    _ => ev,
                };
                match inner.get("kind").and_then(Value::as_str) {
                    Some("tool_use") => {
                        if let (Some(id), Some(input)) = (inner.get("id").and_then(Value::as_str), inner.get("input")) {
                            let tool = inner.get("name").and_then(Value::as_str).unwrap_or_default();
                            inputs.push_back((id.to_string(), tool.to_string(), input.clone()));
                            if inputs.len() > CALLS_KEPT {
                                inputs.pop_front();
                            }
                        }
                    }
                    Some("tool_result") => {
                        // The engine names the call `id`; a hub may say `tool_use_id`.
                        for k in ["id", "tool_use_id"] {
                            if let Some(b) = inner.get(k).and_then(Value::as_str).and_then(|id| held.remove(id)) {
                                unhold(sh, &b).await;
                            }
                        }
                    }
                    Some("prompt") => {
                        let tool = inner.get("tool_name").and_then(Value::as_str).unwrap_or_default();
                        if !crate::kernel::parts::tool_matches(&asked, tool) {
                            continue;
                        }
                        // The turn's task; a turn begun before this watch saw it (a restart, a lag)
                        // is the session's bound task's.
                        let owner = match turn.as_ref().and_then(|t| turns.get(t)) {
                            Some(o) => o.clone(),
                            None => sh.binding(&s.session_id),
                        };
                        let call_id = inner.get("tool_use_id").and_then(Value::as_str);
                        // By the call; a sub-agent's question names none — its latest call of the tool.
                        let input = match call_id {
                            Some(id) => inputs.iter().rev().find(|(k, _, _)| k == id),
                            None => inputs.iter().rev().find(|(_, t, _)| t == tool),
                        }
                        .map(|(_, _, v)| v.clone());
                        if let Some(b) = owner {
                            let asked_person = answer(sh, s, inner, input.as_ref(), root.as_deref(), &b).await;
                            if asked_person {
                                let key = call_id
                                    .or_else(|| inner.get("prompt_id").and_then(Value::as_str))
                                    .unwrap_or_default();
                                held.insert(key.to_string(), b.clone());
                            }
                        }
                    }
                    _ => {}
                }
                if main {
                    note_agents(sh, run, s, ev, &mut agent_calls);
                }
            }
            _ => {}
        }
    }
}

/// The sub-agents a main session starts, as its events show them: the `Agent` call (its
/// description is the flow task's tag, `agent_tag`), then the sub-agent's own progress, which
/// names its session and the call it came from.
fn note_agents(
    sh: &Shared,
    run: &str,
    s: &SessionRef,
    ev: &Value,
    calls: &mut std::collections::HashMap<String, String>,
) {
    match ev.get("kind").and_then(Value::as_str) {
        Some("tool_use") if ev.get("name").and_then(Value::as_str) == Some("Agent") => {
            if let Some(id) = ev.get("id").and_then(Value::as_str) {
                let d = ev
                    .pointer("/input/description")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                calls.insert(id.to_string(), d.to_string());
            }
        }
        Some("subagent_progress") => {
            let Some(agent) = ev.get("agent_session_id").and_then(Value::as_str) else {
                return;
            };
            let call = ev.get("parent_tool_use_id").and_then(Value::as_str);
            let description = call.and_then(|c| calls.get(c)).cloned();
            let flow = description
                .as_deref()
                .and_then(|d| d.strip_prefix(&format!("flow:{run}/")))
                .and_then(|r| r.split_once('/'))
                .map(|(st, t)| json!({"station": st, "task": t}));
            let inner = ev.pointer("/event/kind").and_then(Value::as_str);
            let status = match inner {
                Some("turn_complete") => Some("done"),
                Some("error") => Some("failed"),
                _ => None,
            };
            sh.note_agent(
                run,
                json!({
                    "agent_session_id": agent,
                    "instance": s.instance,
                    "agent_type": ev.get("agent_type"),
                    "tool_use_id": call,
                    "description": description,
                    "flow": flow,
                    "status": status.map_or(Value::Null, |st| json!(st)),
                }),
            );
        }
        _ => {}
    }
}

async fn run_active(sh: &Shared, run: &str) -> bool {
    match sh.engine() {
        Ok(e) => e.record(run).await.is_ok_and(|r| r.is_active()),
        Err(_) => false,
    }
}

async fn count(sh: &Arc<Shared>, run: &str, task: Option<(String, String)>, u: Usage) {
    let Ok(engine) = sh.engine() else { return };
    let task = task.as_ref().map(|(st, t)| (st.as_str(), t.as_str()));
    let r = engine.account(run, task, u).await;
    settle_quiet(sh, run, r).await;
}

/// What becomes of a question about a call of a task's turn: its permissions judge it.
pub(crate) fn verdict(sh: &Shared, b: &Binding, prompt: &Value, input: Option<&Value>, root: Option<&str>) -> Verdict {
    let tool = prompt.get("tool_name").and_then(Value::as_str).unwrap_or("");
    let paths: Vec<String> = prompt
        .get("paths")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|p| p.as_str().map(String::from)).collect())
        .unwrap_or_default();
    let ws = sh.engine().ok().and_then(|e| e.workspace(&b.run));
    judge(
        &b.permissions,
        &sh.cfg.guarded,
        &Call {
            tool,
            input,
            paths: &paths,
            workspace: ws.as_ref().and_then(|p| p.to_str()),
            root,
        },
        &b.station,
    )
}

/// The answer to give the engine for a verdict; `None` for `ask` (a person answers).
pub(crate) fn decision(v: &Verdict) -> Option<Value> {
    match v.effect {
        Effect::Allow => Some(json!({"type": "permit"})),
        Effect::Deny => Some(json!({"type": "deny", "reason": v.reason})),
        Effect::Ask => None,
    }
}

/// Answer a question of a task's turn from its permissions. Returns whether it was left to a
/// person (the task is marked as waiting for them).
async fn answer(
    sh: &Arc<Shared>,
    s: &SessionRef,
    prompt: &Value,
    input: Option<&Value>,
    root: Option<&str>,
    b: &Binding,
) -> bool {
    let Some(cx) = sh.cx.get() else {
        return false;
    };
    let Some(id) = prompt.get("prompt_id").and_then(Value::as_str) else {
        return false;
    };
    let v = verdict(sh, b, prompt, input, root);
    let Some(d) = decision(&v) else {
        let tool = prompt.get("tool_name").and_then(Value::as_str).unwrap_or("").to_string();
        if let Ok(engine) = sh.engine() {
            let r = engine
                .asking(&b.run, &b.station, &b.task, &b.token, Some((tool, v.reason)))
                .await;
            settle_quiet(sh, &b.run, r).await;
        }
        return true;
    };
    if let Err(e) = cx
        .call(
            "harnix.session.respond",
            json!({
                "instance": s.instance, "session_id": s.session_id,
                "prompt_id": id, "prompt_type": "permission", "decision": d,
            }),
        )
        .await
    {
        let tool = prompt.get("tool_name").and_then(Value::as_str).unwrap_or("");
        tracing::warn!(tool, error = %e.message, "a session's question not answered");
    }
    false
}

/// A held call was answered (or its turn is over): the task works again.
async fn unhold(sh: &Arc<Shared>, b: &Binding) {
    if let Ok(engine) = sh.engine() {
        let r = engine.asking(&b.run, &b.station, &b.task, &b.token, None).await;
        settle_quiet(sh, &b.run, r).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_verdict_is_an_answer_unless_a_person_is_asked() {
        let v = |effect, reason: &str| Verdict { effect, reason: reason.into() };
        assert_eq!(decision(&v(Effect::Allow, "")), Some(json!({"type": "permit"})));
        assert_eq!(
            decision(&v(Effect::Deny, "station s does not allow Bash")),
            Some(json!({"type": "deny", "reason": "station s does not allow Bash"}))
        );
        assert_eq!(decision(&v(Effect::Ask, "a nod")), None);
    }
}
