//! The run's main session — the flow's main Agent (xops-03 §1).
//!
//! It is opened when the run is created, where the flow's `main` places it (the module's main
//! scene in the module's space by default), and recorded as the run's `@main` session key, so
//! the session tasks bound to `main` and the sub-agent tasks take their turns in it (the shared
//! session's lock, `run#@main`).
//!
//! Its permission rules `ask` for every guarded tool. While a turn of it works on a task, the
//! module answers those questions itself from that task's station's `writes` — a station's
//! limit holds in a session that every station shares — and a turn that works on no task (a
//! person talking to the main Agent) is asked of the person as ever. The same watch counts each
//! turn's tokens on the task it worked on, or on the run (`Engine::account`).

use std::sync::Arc;

use harnix_module::{Entry, SessionRef, SessionSpec, TapEvent};
use serde_json::{json, Value};

use super::dispatch::{place_at, settle_quiet, SESSION_SETTINGS, TASK_PERMISSION_MODE};
use super::shared::{session_name, session_ref, Binding, Shared};
use crate::kernel::def::MAIN_SESSION_KEY;
use crate::kernel::protocol::{matches_any, refused};
use crate::kernel::run::{RunRecord, Usage};

/// The lock the main session's turns take (the shared-session lock of the `@main` key).
pub(crate) fn main_lock(run: &str) -> String {
    format!("{run}#{MAIN_SESSION_KEY}")
}

/// The main session's permission rules: `ask` for each guarded tool, so each use comes to the
/// module (or the person) to be decided.
pub fn ask_rules(guarded: &[String]) -> Value {
    Value::Array(
        refused(guarded, &[])
            .into_iter()
            .map(|(_, tool)| json!({"source": "session", "behavior": "ask", "tool_name": tool}))
            .collect(),
    )
}

/// What a main session is opened with: everything but the stations' writes, which it does not
/// keep (they are answered turn by turn).
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
        watch(sh, run, s);
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
    let (placement, scene) = crate::kernel::txn::main_of(&rec, engine.main_scene());
    let scene = scene.ok_or("the flow names no main scene and the module has none")?;
    let (space, instance) = place_at(sh, &placement, run).await?;
    let wanted = main_settings(&scene, &placement, placement.template_instance(run).as_deref());
    let recording = sh.cfg.main_recording.as_ref().and_then(|f| f(&rec));
    // A scene that needs a project works in a directory of the run's own.
    let project_root = if sh.cfg.project_scenes.contains(&scene) {
        let dir = cx.data_dir().join("workspaces").join(safe_dir(run));
        std::fs::create_dir_all(&dir).map_err(|e| format!("no workspace for the run: {e}"))?;
        write_agent_type(&dir, &scene);
        Some(dir)
    } else {
        None
    };
    let spec = SessionSpec {
        space: Some(space),
        instance,
        project_root,
        scene: Some(scene),
        model: placement.model.clone(),
        provider: placement.provider.clone(),
        permission_mode: Some(TASK_PERMISSION_MODE.into()),
        permission_rules: Some(ask_rules(&sh.cfg.guarded)),
        recording,
        ..SessionSpec::default()
    };
    let s = cx
        .sessions()
        .create(spec)
        .await
        .map_err(|e| format!("no main session: {}", e.message))?;
    let _ = cx
        .sessions()
        .append_entry(&s, Entry::new(SESSION_SETTINGS, wanted))
        .await;
    let title = main_title(&rec);
    let _ = cx
        .call(
            "session.rename",
            json!({"instance": s.instance, "session_id": s.session_id, "name": title}),
        )
        .await;
    let attached = engine.attach_main(run, &session_name(&s)).await;
    settle_quiet(sh, run, attached).await;
    Ok(s)
}

/// The agent type a sub-agent station's sub-agent is when the station names none
/// ([`crate::kernel::protocol::DEFAULT_AGENT_TYPE`]), written into the run's workspace
/// (`.agix/agents/`, where the engine reads a project's agent types).
///
/// It runs on the wrapped scene's own scene (`flow-coding` → `coding`): a sub-agent on the wrapped
/// one does not build — the engine registers the scene's tools again over the ones it inherited
/// (Agix issue draft 10). The inner scene grants nothing the wrapped one does not.
fn write_agent_type(workspace: &std::path::Path, main_scene: &str) {
    let inner = main_scene.strip_prefix("flow-").unwrap_or(main_scene);
    let dir = workspace.join(".agix").join("agents");
    let body = format!(
        "---\nname: {}\ndescription: Does one task of a flow run, handed over by the run's main \
         agent\nscene: {inner}\n---\nYou do one task of a flow run, handed to you by the run's \
         main agent. Do it, then reply with the result only.\n",
        crate::kernel::protocol::DEFAULT_AGENT_TYPE
    );
    let ok = std::fs::create_dir_all(&dir).and_then(|_| {
        std::fs::write(
            dir.join(format!("{}.md", crate::kernel::protocol::DEFAULT_AGENT_TYPE)),
            body,
        )
    });
    if let Err(e) = ok {
        tracing::warn!(error = %e, "flow: the sub-agent type not written");
    }
}

/// A run id as a directory name.
fn safe_dir(run: &str) -> String {
    run.chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '-' })
        .collect()
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

/// Watch a main session while the run lasts, once.
pub(crate) fn watch(sh: &Arc<Shared>, run: &str, s: &SessionRef) {
    if !sh.watch(&s.session_id, run) {
        return;
    }
    let (sh, run, s) = (sh.clone(), run.to_string(), s.clone());
    tokio::spawn(async move {
        watch_loop(&sh, &run, &s).await;
        sh.unwatch(&s.session_id);
    });
}

async fn watch_loop(sh: &Arc<Shared>, run: &str, s: &SessionRef) {
    let Some(cx) = sh.cx.get() else { return };
    let Ok(mut tap) = cx.sessions().tap(s).await else {
        return;
    };
    // Turn → the task it was started for: read when the turn's first frame comes, while that
    // task is in hand (by its settlement it has often handed in and been let go).
    let mut turns: std::collections::HashMap<String, Option<Binding>> =
        std::collections::HashMap::new();
    // `Agent` calls of the main Agent: tool use id → what it named the sub-agent.
    let mut agent_calls: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    loop {
        let ev = tokio::select! {
            ev = tap.next() => ev,
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
                let usage = frame
                    .pointer("/params/outcome/usage")
                    .filter(|u| u.is_object())
                    .map(Usage::of_turn);
                if let Some(u) = usage {
                    count(sh, run, task, u).await;
                }
            }
            Some("harnix.event") => {
                let Some(ev) = frame.pointer("/params/event") else {
                    continue;
                };
                // A sub-agent's question comes wrapped in its progress.
                let prompt = match ev.get("kind").and_then(Value::as_str) {
                    Some("prompt") => Some(ev),
                    Some("subagent_progress") => ev
                        .get("event")
                        .filter(|e| e.get("kind").and_then(Value::as_str) == Some("prompt")),
                    _ => None,
                };
                if let Some(p) = prompt {
                    let owner = turn.as_ref().and_then(|t| turns.get(t)).cloned().flatten();
                    answer(sh, s, p, owner).await;
                }
                note_agents(sh, run, s, ev, &mut agent_calls);
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

/// The decision on a guarded tool's use for the task a turn works on: what its station's
/// `writes` say. `None`: not the module's to decide — no task, or not a guarded tool.
///
/// Judged on the guarded patterns the station leaves refused, so a nested group (`mcp__desk__*`)
/// is the group as written, not its first namespace (review #1).
pub(crate) fn decision(guarded: &[String], writes: &[String], station: &str, tool: &str) -> Option<Value> {
    if !matches_any(guarded, tool) {
        return None;
    }
    let refused_patterns: Vec<String> = refused(guarded, writes)
        .into_iter()
        .map(|(g, _)| g)
        .collect();
    if matches_any(&refused_patterns, tool) {
        Some(json!({
            "type": "deny",
            "reason": format!("station {station} of the flow does not allow {tool} (its writes: {})",
                if writes.is_empty() { "none".to_string() } else { writes.join(", ") }),
        }))
    } else {
        Some(json!({"type": "permit"}))
    }
}

/// Answer a question of a task's turn from its station; a person's turn's is the person's.
async fn answer(sh: &Arc<Shared>, s: &SessionRef, prompt: &Value, owner: Option<Binding>) {
    let (Some(cx), Some(b)) = (sh.cx.get(), owner) else {
        return;
    };
    let tool = prompt.get("tool_name").and_then(Value::as_str).unwrap_or("");
    let Some(id) = prompt.get("prompt_id").and_then(Value::as_str) else {
        return;
    };
    let Some(d) = decision(&sh.cfg.guarded, &b.writes, &b.station, tool) else {
        return;
    };
    if let Err(e) = cx
        .call(
            "session.respondToPrompt",
            json!({
                "instance": s.instance, "session_id": s.session_id,
                "prompt_id": id, "prompt_type": "permission", "decision": d,
            }),
        )
        .await
    {
        tracing::warn!(tool, error = %e.message, "flow: a main session's question not answered");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn g(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn a_station_decides_the_guarded_tools_of_its_turns_and_nothing_else() {
        let guarded = g(&["Bash", "Write", "git__*"]);
        let permit = Some(json!({"type": "permit"}));
        assert_eq!(decision(&guarded, &g(&["Write"]), "s", "Write"), permit);
        assert_eq!(
            decision(&guarded, &g(&["Write"]), "s", "Bash").unwrap()["type"],
            "deny"
        );
        assert_eq!(decision(&guarded, &g(&["git__*"]), "s", "git__commit"), permit);
        assert_eq!(decision(&guarded, &g(&[]), "s", "git__commit").unwrap()["type"], "deny");
        // A nested group is judged as written: `mcp__desk__*` refused is not opened by `mcp`.
        let nested = g(&["mcp__desk__*", "mcp__wiki__*"]);
        assert_eq!(decision(&nested, &g(&[]), "s", "mcp__desk__ticket_create").unwrap()["type"], "deny");
        assert_eq!(decision(&nested, &g(&["mcp__wiki__*"]), "s", "mcp__desk__ticket_create").unwrap()["type"], "deny");
        assert_eq!(decision(&nested, &g(&["mcp__desk__*"]), "s", "mcp__desk__ticket_create"), permit);
        // Not guarded: the person's (or the mode's) to decide.
        assert_eq!(decision(&guarded, &g(&[]), "s", "Read"), None);
    }

    #[test]
    fn the_main_session_asks_for_every_guarded_tool() {
        let rules = ask_rules(&g(&["Bash", "git__*"]));
        let names: Vec<&str> = rules
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["tool_name"].as_str().unwrap())
            .collect();
        assert_eq!(names, ["Bash", "git"]);
        assert!(rules.as_array().unwrap().iter().all(|r| r["behavior"] == "ask"));
    }
}
