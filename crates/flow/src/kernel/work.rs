//! The work a run hands out, as a work item: what a claim of a push task is told, where a run's
//! main session goes, and — after a restart — what was handed out and not answered yet.

use std::collections::BTreeMap;

use serde_json::Value;

use super::def::{Executor, ExitCond, SessionBinding, TaskSpec, MAIN_SESSION_KEY};
use super::engine::Engine;
use super::host::{Delivery, Placement, WorkItem};
use super::parts::GateKind;
use super::protocol;
use super::refs::{context_in, resolve_str};
use super::run::{Actor, CheckStatus, RunRecord, StationStatus, TaskStatus};

/// The work item of a push task claimed under `token`: what [`M::claim_push`](super::txn::M::claim_push) hands out, and
/// what [`dispatched_work`] rebuilds after a restart.
#[allow(clippy::too_many_arguments)]
pub(crate) fn task_work(
    rec: &RunRecord,
    eng: &Engine,
    sid: &str,
    tid: &str,
    spec: &TaskSpec,
    token: String,
    round: u32,
    attempt: u32,
    item: Option<Value>,
    digest: Option<String>,
) -> WorkItem {
    let (guarded, submit_scenes, main_scene) = (&eng.guarded, &eng.submit_scenes, eng.main_scene.as_deref());
    let workspace = eng.workspace(&rec.id);
    let ctx = context_in(rec, item.as_ref(), Some(tid), workspace.as_ref().and_then(|p| p.to_str()));
    let resolve_params = |params: &BTreeMap<String, String>| -> BTreeMap<String, String> {
        params
            .iter()
            .map(|(k, v)| {
                let r = match resolve_str(v, &ctx) {
                    Value::String(s) => s,
                    Value::Null => String::new(),
                    v => v.to_string(),
                };
                (k.clone(), r)
            })
            .collect()
    };
    let session_key = match &spec.executor {
        Executor::Agent { .. } => Some(MAIN_SESSION_KEY.to_string()),
        e @ Executor::Session { .. } => match e.binding().unwrap_or(SessionBinding::New) {
            SessionBinding::Main => Some(MAIN_SESSION_KEY.to_string()),
            SessionBinding::New => None,
            SessionBinding::PerRun => Some("run".to_string()),
            SessionBinding::Key(k) => Some(match resolve_str(&k, &ctx) {
                Value::String(s) => s,
                v => v.to_string(),
            }),
        },
        _ => None,
    };
    let session = session_key
        .as_ref()
        .and_then(|k| rec.session_keys.get(k).cloned());
    let st = rec.def.station(sid);
    // What the task works under, fixed now (xops-04 §3).
    let permissions: Vec<super::parts::Granted> = match (&spec.executor, st) {
        (Executor::Session { .. } | Executor::Agent { .. }, Some(st)) => {
            super::parts::permissions_of(&rec.def, st, &rec.parts.policies)
                .into_iter()
                .map(|g| super::parts::Granted {
                    permission: super::parts::resolve_permission(&g.permission, &ctx),
                    from: g.from,
                })
                .collect()
        }
        _ => vec![],
    };
    let (placement, delivery, scene) = match &spec.executor {
        e if e.in_main() => {
            let (placement, scene) = main_placement(rec, main_scene, &resolve_params);
            (Some(placement), Some(Delivery::FlowSubmit), scene)
        }
        Executor::Session {
            scene,
            model,
            provider,
            instance,
            template,
            params,
            space,
            ..
        } => (
            Some(Placement {
                model: model.clone(),
                provider: provider.clone(),
                instance: instance.clone(),
                template: template.clone(),
                params: resolve_params(params),
                space: space.clone(),
            }),
            Some(if submit_scenes.contains(scene) {
                Delivery::FlowSubmit
            } else {
                Delivery::StructuredOutput
            }),
            Some(scene.clone()),
        ),
        _ => (None, None, None),
    };
    let protocol = match (&spec.executor, st) {
        (Executor::Session { .. } | Executor::Agent { .. }, Some(st)) => Some(protocol::render(&protocol::TaskContext {
            def: &rec.def,
            station: st,
            run: &rec.id,
            subject: &rec.subject,
            round,
            task: tid,
            item: item.as_ref(),
            guarded,
            permissions: &permissions,
            gates: &rec.parts.gates,
            delivery: delivery.unwrap_or(Delivery::StructuredOutput),
            executor: Some(&spec.executor),
        })),
        _ => None,
    };
    let rules: Vec<(String, Option<String>)> = match (&spec.executor, st) {
        (Executor::Session { .. } | Executor::Agent { .. }, Some(st)) => super::parts::rules_of(&rec.def, st)
            .into_iter()
            .map(|r| {
                let d = rec.stations.get(sid).and_then(|s| s.rule_digest.get(&r).cloned());
                (r, d)
            })
            .collect(),
        _ => vec![],
    };
    WorkItem::Task {
        run: rec.id.clone(),
        station: sid.to_string(),
        round,
        task: tid.to_string(),
        token,
        attempt,
        executor: spec.executor.clone(),
        skill: spec.skill.clone(),
        skill_digest: digest,
        rules,
        permissions,
        item,
        output_schema: st.and_then(|s| s.output.clone()),
        protocol,
        session_key,
        session,
        placement,
        delivery,
        scene,
    }
}

/// Template parameters, references resolved.
type ResolveParams<'a> = dyn Fn(&BTreeMap<String, String>) -> BTreeMap<String, String> + 'a;

/// [`main_placement`] with `params` resolved against the run (no item, no task).
pub(crate) fn main_of(rec: &RunRecord, main_scene: Option<&str>) -> (Placement, Option<String>) {
    let ctx = context_in(rec, None, None, None);
    let resolve_params = |params: &BTreeMap<String, String>| -> BTreeMap<String, String> {
        params
            .iter()
            .map(|(k, v)| {
                let r = match resolve_str(v, &ctx) {
                    Value::String(s) => s,
                    Value::Null => String::new(),
                    v => v.to_string(),
                };
                (k.clone(), r)
            })
            .collect()
    };
    main_placement(rec, main_scene, &resolve_params)
}

/// Where the run's main session goes and its scene: the flow's `main`, its `params` resolved,
/// and the module's main scene when it names none (`None`: there is none).
pub(crate) fn main_placement(
    rec: &RunRecord,
    main_scene: Option<&str>,
    resolve_params: &ResolveParams<'_>,
) -> (Placement, Option<String>) {
    let m = rec.def.main.clone().unwrap_or_default();
    (
        Placement {
            model: m.model,
            provider: m.provider,
            instance: m.instance,
            template: m.template,
            params: resolve_params(&m.params),
            space: m.space,
        },
        m.scene.or_else(|| main_scene.map(String::from)),
    )
}

/// The work handed out for a run and not answered yet, as it was handed out — for a module that
/// restarted and must carry on with what it had been given: every session and program task the
/// dispatcher holds a live claim on (`session` is the session the task reported with
/// `attach_session`, else its key's), and every
/// `command` or `host` gate of the current round still waiting for its result.
pub(crate) fn dispatched_work_with(rec: &RunRecord, eng: &Engine, now: u64) -> Vec<WorkItem> {
    let dispatcher = Actor::flow("dispatch");
    let mut out = vec![];
    if !rec.is_active() {
        return out;
    }
    for (sid, st) in &rec.stations {
        let Some(sdef) = rec.def.station(sid) else {
            continue;
        };
        if matches!(st.status, StationStatus::Working | StationStatus::Unmet) {
            for c in &st.checks {
                let gate = c.cond.and_then(|i| sdef.exit.get(i));
                if let (Some(ExitCond::Gate { gate, .. }), CheckStatus::Wait, true) =
                    (gate, c.status, c.round == st.round)
                {
                    if rec.parts.gates.get(gate).is_none_or(|g| g.kind == GateKind::Tool) {
                        continue;
                    }
                    out.push(WorkItem::Gate {
                        run: rec.id.clone(),
                        station: sid.clone(),
                        round: st.round,
                        gate: gate.clone(),
                    });
                }
            }
        }
        if st.status != StationStatus::Working {
            continue;
        }
        for t in &st.tasks {
            let Some(lease) = t.lease.as_ref() else {
                continue;
            };
            if t.status != TaskStatus::Claimed || lease.by != dispatcher || lease.until <= now {
                continue;
            }
            let Some(spec) = sdef.task_spec(&t.id) else {
                continue;
            };
            if !spec.executor.is_push() {
                continue;
            }
            let digest = spec
                .skill
                .as_ref()
                .and_then(|s| st.skill_digest.get(s).cloned());
            let mut w = task_work(
                rec,
                eng,
                sid,
                &t.id,
                &spec,
                lease.token.clone(),
                st.round,
                t.attempt,
                t.item.clone(),
                digest,
            );
            if let (WorkItem::Task { session, .. }, Some(own)) = (&mut w, &t.session) {
                *session = Some(own.clone());
            }
            out.push(w);
        }
    }
    out
}
