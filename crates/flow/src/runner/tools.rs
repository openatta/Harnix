//! The flow's tools, which a host registers to every flow session: one set, answered here. A tool
//! finds the task it serves by the calling session's id; a session that serves none is told so.
//!
//! | tool | does |
//! |---|---|
//! | `flow_state` | the run as far as this task is concerned: the station, what is still unmet |
//! | `flow_submit {output}` | hands the output in under the session's claim; problems come back to fix |
//! | `flow_propose {kind, to?, reason}` | writes a `proposal` note; a person accepts it or not |

use std::sync::Weak;

use serde_json::{json, Value};

use super::host::{Note, SessionId, ToolSpec};
use super::state::Binding;
use super::Runner;
use crate::kernel::run::Actor;
use crate::kernel::FlowError;

pub const TOOL_STATE: &str = "flow_state";
pub const TOOL_SUBMIT: &str = "flow_submit";
pub const TOOL_PROPOSE: &str = "flow_propose";

/// What a tool answers: text for the model, and whether it is an error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolReply {
    pub text: String,
    pub is_error: bool,
}

impl ToolReply {
    pub(crate) fn text(text: impl Into<String>) -> Self {
        Self { text: text.into(), is_error: false }
    }
    pub(crate) fn error(text: impl Into<String>) -> Self {
        Self { text: text.into(), is_error: true }
    }
}

/// The flow's three tools: their specs, for the host to register; their calls, for the host to
/// route here.
pub struct FlowTools {
    runner: Weak<Runner>,
    specs: Vec<ToolSpec>,
}

impl FlowTools {
    pub(crate) fn new(runner: Weak<Runner>) -> Self {
        Self { runner, specs: tool_specs() }
    }

    /// `flow_state`, `flow_submit`, `flow_propose`. None of them changes anything outside the
    /// flow's own record, and a headless session has nobody to ask: all read-only.
    pub fn specs(&self) -> &[ToolSpec] {
        &self.specs
    }

    /// Answer a call of `tool` from the session `session` (the id the host got from `open`).
    pub async fn call(&self, session: &SessionId, tool: &str, input: Value) -> ToolReply {
        let Some(rn) = super::upgrade(&self.runner) else {
            return ToolReply::error("the flow runner has stopped");
        };
        let Some(b) = rn.state.binding(session) else {
            return ToolReply::error(
                "This session is not working on a flow task (or its task is over): there is nothing \
                 to read or hand in here.",
            );
        };
        match tool {
            TOOL_STATE => state(&rn, &b).await,
            TOOL_SUBMIT => submit(&rn, &b, session, input).await,
            TOOL_PROPOSE => propose(&rn, &b, session, input).await,
            other => ToolReply::error(format!("no flow tool `{other}`")),
        }
    }
}

/// The flow's three tools' specs, as a host registers them: the same whether or not a runner is
/// there yet.
pub fn tool_specs() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: TOOL_STATE.into(),
            description: "The flow task this session is working on: the run, the station and its round, your \
                 task and item, the output contract, what you handed in, and which exit conditions are \
                 still not met."
                .into(),
            input_schema: json!({"type": "object", "properties": {}}),
            read_only: true,
        },
        ToolSpec {
            name: TOOL_SUBMIT.into(),
            description: "Hand in this task's output: one JSON value satisfying the station's output contract. \
                 Problems come back for you to fix; submit again until it is accepted. You may submit \
                 again until the station passes; the last submission counts."
                .into(),
            input_schema: json!({
                "type": "object",
                "required": ["output"],
                "properties": {
                    "output": {"description": "The output, as the flow protocol's contract says."}
                }
            }),
            read_only: true,
        },
        ToolSpec {
            name: TOOL_PROPOSE.into(),
            description: "Propose that a person rework an upstream station (kind `rework`, `to` the station) or \
                 cancel the run (kind `cancel`), with your reason. It is shown to people as a card; \
                 nothing happens unless one of them accepts it."
                .into(),
            input_schema: json!({
                "type": "object",
                "required": ["kind", "reason"],
                "properties": {
                    "kind": {"type": "string", "enum": ["rework", "cancel"]},
                    "to": {"type": "string", "description": "With `rework`: the station to go back to."},
                    "reason": {"type": "string"}
                }
            }),
            read_only: true,
        },
    ]
}

fn pretty(v: &Value) -> String {
    serde_json::to_string_pretty(v).unwrap_or_else(|_| v.to_string())
}

async fn state(rn: &Runner, b: &Binding) -> ToolReply {
    // The whole record, to tell whether the claim is still this session's; only the fields below
    // leave here, never a token.
    let full = match rn.engine().record(&b.run).await {
        Ok(r) => r,
        Err(e) => return ToolReply::error(e.to_string()),
    };
    let current = full
        .stations
        .get(&b.station)
        .and_then(|s| s.task(&b.task))
        .is_some_and(|t| t.lease.as_ref().is_some_and(|l| l.token == b.token));
    let view = crate::kernel::engine::view(full);
    let rec = &view.record;
    let st = rec.stations.get(&b.station);
    let task = st.and_then(|s| s.task(&b.task));
    let sdef = rec.def.station(&b.station);
    let state = json!({
        "run": rec.id,
        "flow": rec.flow,
        "subject": rec.subject,
        "run_status": rec.status,
        "station": b.station,
        "round": st.map(|s| s.round),
        "station_status": st.map(|s| s.status),
        "task": b.task,
        "task_status": task.map(|t| t.status),
        "item": task.and_then(|t| t.item.clone()),
        "submitted": task.and_then(|t| t.output.clone()),
        "contract": sdef.and_then(|s| s.output.clone()),
        "exits": sdef.map(|s| s.exit.iter().map(|e| e.label()).collect::<Vec<_>>()),
        "unmet": view.unmet.get(&b.station),
        "permissions": b.permissions,
        "current": current,
    });
    ToolReply::text(pretty(&state))
}

async fn submit(rn: &std::sync::Arc<Runner>, b: &Binding, session: &SessionId, input: Value) -> ToolReply {
    let output = input.get("output").cloned().unwrap_or(Value::Null);
    let result = rn
        .engine()
        .submit(&b.run, &b.station, &b.task, &b.token, output, Actor::agent(session.clone()))
        .await;
    match result {
        Ok(o) => {
            let status = o.value.get("station").cloned().unwrap_or(Value::Null);
            rn.apply_for(&b.run, o).await;
            ToolReply::text(format!(
                "Accepted. Station `{}` is now {}. If it is still working, others' tasks or its checks \
                 are outstanding; your part is done unless you want to improve your output.",
                b.station,
                status.as_str().unwrap_or("unknown")
            ))
        }
        Err(FlowError::Contract(m)) => ToolReply::error(format!("Not accepted: {m}. Fix the output and call {TOOL_SUBMIT} again.")),
        Err(e @ FlowError::Invalid(_)) => ToolReply::error(format!("Not accepted: {e}. The station is not taking submissions now.")),
        Err(e) => ToolReply::error(format!("Not accepted: {e}. Your claim on this task is no longer current; stop working on it.")),
    }
}

async fn propose(rn: &Runner, b: &Binding, session: &SessionId, input: Value) -> ToolReply {
    let kind = input.get("kind").and_then(Value::as_str).unwrap_or("");
    let reason = input.get("reason").and_then(Value::as_str).unwrap_or("").trim().to_string();
    let to = input.get("to").and_then(Value::as_str).filter(|s| !s.is_empty()).map(String::from);
    if !matches!(kind, "rework" | "cancel") {
        return ToolReply::error("`kind` is `rework` or `cancel`");
    }
    if kind == "rework" && to.is_none() {
        return ToolReply::error("a rework proposal names the station to go back to in `to`");
    }
    if reason.is_empty() {
        return ToolReply::error("a proposal needs a reason");
    }
    let payload = json!({
        "run": b.run, "station": b.station, "task": b.task,
        "kind": kind, "to": to, "reason": reason,
    });
    match rn.host().note(session, Note { event: "proposal".into(), payload }).await {
        Ok(()) => ToolReply::text(
            "Proposed. A person will accept or dismiss it; carry on with your task meanwhile, or stop \
             if you cannot do it without the change.",
        ),
        Err(e) => ToolReply::error(format!("The proposal was not recorded: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::kernel::host::WorkItem;
    use crate::runner::{Runner, ScriptedHost, SessionHost};
    use crate::{CreateIn, Engine, MemPacks, MemStore, StationStatus};

    const FLOW: &str = "{name: f, stations: [\
        {id: a, executor: {kind: session, scene: flow-harnix}, skill: do, output: {type: object, required: [n]}, exit: [{kind: output}]},\
        {id: b, exit: [{kind: confirm}]}]}";

    async fn bound() -> (Arc<Runner>, Arc<ScriptedHost>, String, Binding) {
        let packs = MemPacks::new().scene("flow-harnix").skill("do", "Do it.").flow(FLOW);
        let engine = Arc::new(Engine::new(Arc::new(MemStore::new()), Arc::new(packs)).with_submit_scenes(["flow-harnix"]));
        let host = ScriptedHost::new();
        let rn = Runner::for_tests(engine.clone(), host.clone());
        let o = engine.create(CreateIn { flow: "f".into(), subject: "s".into(), ..Default::default() }, Actor::device("d")).await.unwrap();
        let WorkItem::Task { run, station, round, task, token, permissions, .. } = &o.work[0] else { panic!() };
        let b = Binding { run: run.clone(), station: station.clone(), round: *round, task: task.clone(), token: token.clone(), permissions: permissions.clone() };
        let id = host.open(crate::runner::OpenSpec::new("flow:r/a/a@t", run, "flow-harnix", crate::Delivery::FlowSubmit)).await.unwrap().id;
        rn.state.bind(&id, b.clone());
        (rn, host, id, b)
    }

    #[test]
    fn the_tools_touch_nothing() {
        let rn = Runner::for_tests(Arc::new(Engine::new(Arc::new(MemStore::new()), Arc::new(MemPacks::new()))), ScriptedHost::new());
        let names: Vec<&str> = rn.tools().specs().iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, [TOOL_STATE, TOOL_SUBMIT, TOOL_PROPOSE]);
        assert!(rn.tools().specs().iter().all(|t| t.read_only));
    }

    #[tokio::test]
    async fn a_session_that_serves_no_task_is_told_so() {
        let rn = Runner::for_tests(Arc::new(Engine::new(Arc::new(MemStore::new()), Arc::new(MemPacks::new()))), ScriptedHost::new());
        let out = rn.tools().call(&"nobody".to_string(), TOOL_STATE, json!({})).await;
        assert!(out.is_error && out.text.contains("not working on a flow task"));
        let out = rn.tools().call(&"nobody".to_string(), "flow_nope", json!({})).await;
        assert!(out.is_error);
    }

    #[tokio::test]
    async fn the_state_and_a_hand_in_are_the_tasks_and_a_proposal_is_a_note() {
        let (rn, host, id, b) = bound().await;
        let state = rn.tools().call(&id, TOOL_STATE, json!({})).await;
        assert!(!state.is_error);
        let v: Value = serde_json::from_str(&state.text).unwrap();
        assert_eq!((v["station"].as_str(), v["current"].as_bool()), (Some("a"), Some(true)));
        assert!(v.get("token").is_none());
        let refused = rn.tools().call(&id, TOOL_SUBMIT, json!({"output": {"m": 1}})).await;
        assert!(refused.is_error && refused.text.starts_with("Not accepted: "), "{}", refused.text);
        let bad = rn.tools().call(&id, TOOL_PROPOSE, json!({"kind": "rework", "reason": "x"})).await;
        assert!(bad.is_error && bad.text.contains("`to`"));
        let ok = rn.tools().call(&id, TOOL_PROPOSE, json!({"kind": "cancel", "reason": "no point"})).await;
        assert!(!ok.is_error, "{}", ok.text);
        let notes = host.notes();
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].1.event, "proposal");
        assert_eq!(notes[0].1.payload["reason"], "no point");
        assert_eq!(notes[0].1.payload["run"], b.run);
        let taken = rn.tools().call(&id, TOOL_SUBMIT, json!({"output": {"n": 1}})).await;
        assert!(!taken.is_error && taken.text.starts_with("Accepted. Station `a` is now passed"), "{}", taken.text);
        assert_eq!(rn.engine().record(&b.run).await.unwrap().stations["a"].status, StationStatus::Passed);
        // The station passed: the session serves no task any more.
        let over = rn.tools().call(&id, TOOL_STATE, json!({})).await;
        assert!(over.is_error && over.text.contains("not working on a flow task"), "{}", over.text);
    }
}
