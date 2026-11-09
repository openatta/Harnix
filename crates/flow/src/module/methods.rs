//! `app.flow.*`: what a device may do. Every method is declared for `Device` only; the actor
//! recorded is `{kind: "device", id: <the device>}`, and the application's authorizer has the
//! last word on each change.
//!
//! A run whose id starts `dry-` is a dry run's (xops-02 §5): `get`, `history` and every action on
//! a run reach the sandbox for it, and what the action produced is queued there, not carried
//! out. Writing flows (`save`, `delete`, `skill.save`, `skill.delete`) needs writable packs.

use std::collections::BTreeMap;
use std::future::Future;
use std::sync::Arc;

use harnix_module::{CallContext, MethodRegistry, Principal, RpcError};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{json, Value};

use super::{dispatch, publish};
use super::rpc_error;
use super::shared::Shared;
use crate::dryrun::{is_dry, StartIn as DryStartIn};
use crate::kernel::def::{definition_schema, Work};
use crate::kernel::engine::{CreateIn, Engine};
use crate::kernel::host::Packs;
use crate::kernel::protocol;
use crate::kernel::run::{run_schema, Actor, RunQuery};
use crate::kernel::validate::validate;
use crate::kernel::FlowError;

fn device(cx: &CallContext) -> Actor {
    Actor::new("device", cx.device().unwrap_or("unknown"))
}

#[derive(Deserialize)]
struct RunIn {
    run: String,
}

#[derive(Deserialize)]
struct NameIn {
    name: String,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct SchemaIn {
    of: Option<String>,
}

#[derive(Deserialize)]
struct YamlIn {
    yaml: String,
}

#[derive(Deserialize)]
struct PartIn {
    kind: crate::PartKind,
    name: String,
    #[serde(default)]
    text: String,
}

#[derive(Deserialize)]
struct KindIn {
    kind: crate::PartKind,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct GuideIn {
    lang: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct FlowSessionIn {
    flow: Option<String>,
}

#[derive(Deserialize)]
struct FocusIn {
    instance: String,
    session_id: String,
    #[serde(default)]
    flow: Option<String>,
}

#[derive(Deserialize)]
struct StartIn {
    flow: String,
    subject: String,
    #[serde(default)]
    inputs: Value,
    #[serde(default)]
    title: Option<String>,
}

#[derive(Deserialize)]
struct TaskIn {
    run: String,
    station: String,
    task: String,
}

#[derive(Deserialize)]
struct ClaimIn {
    run: String,
    station: String,
    task: String,
    #[serde(default)]
    lease_ms: Option<u64>,
}

#[derive(Deserialize)]
struct HeartbeatIn {
    run: String,
    station: String,
    task: String,
    token: String,
    #[serde(default)]
    lease_ms: Option<u64>,
}

#[derive(Deserialize)]
struct SubmitIn {
    run: String,
    station: String,
    task: String,
    token: String,
    output: Value,
}

#[derive(Deserialize)]
struct FailIn {
    run: String,
    station: String,
    task: String,
    token: String,
    error: String,
}

#[derive(Deserialize)]
struct ConfirmIn {
    run: String,
    station: String,
    pass: bool,
    #[serde(default)]
    reason: Option<String>,
}

#[derive(Deserialize)]
struct ReasonIn {
    run: String,
    station: String,
    reason: String,
}

/// A session's `proposal` entry a person is accepting.
#[derive(Deserialize, Clone)]
struct ProposalRef {
    instance: String,
    session_id: String,
    entry: String,
}

#[derive(Deserialize)]
struct ReworkIn {
    run: String,
    to: String,
    #[serde(default)]
    tasks: Option<Vec<String>>,
    reason: String,
    /// Accepting this proposal: done once, and recorded in its session.
    #[serde(default)]
    proposal: Option<ProposalRef>,
}

#[derive(Deserialize)]
struct StationIn {
    run: String,
    station: String,
}

#[derive(Deserialize)]
struct CancelIn {
    run: String,
    reason: String,
    #[serde(default)]
    proposal: Option<ProposalRef>,
}

/// The lease an external claimant gets when it names none: five minutes.
const CLAIM_MS: u64 = 5 * 60 * 1000;

type Answer = Result<Value, RpcError>;

/// Declare `name` for devices with a handler given the shared state and the packs.
fn method<P, F, Fut>(
    m: &mut MethodRegistry,
    sh: &Arc<Shared>,
    packs: &Arc<dyn Packs>,
    name: &str,
    describe: &str,
    f: F,
) where
    P: DeserializeOwned + Send + 'static,
    F: Fn(Arc<Shared>, Arc<dyn Packs>, CallContext, P) -> Fut + Send + Sync + Copy + 'static,
    Fut: Future<Output = Answer> + Send + 'static,
{
    let sh = sh.clone();
    let packs = packs.clone();
    m.method(name, Principal::Device)
        .describe(describe)
        .handler(move |cx: CallContext, p: P| f(sh.clone(), packs.clone(), cx, p));
}

/// Run a change, carry out what it produced (a dry run's: queue it in the sandbox), answer its
/// value.
async fn change(sh: &Arc<Shared>, run: Option<&str>, r: crate::Result<crate::Outcome>) -> Answer {
    let o = r.map_err(rpc_error)?;
    let value = o.value.clone();
    match run {
        Some(run) if in_sandbox(sh, run).await => {
            sh.sandbox()?.absorb(run, o).await.map_err(rpc_error)?;
            publish::publish_dry(sh, Some(run)).await;
        }
        _ => dispatch::apply(sh, run, o).await,
    }
    Ok(value)
}

/// Whether `run` is a dry run the sandbox holds: only then does a call go there. (No flow may
/// be named `dry-…`, so a real run's id never looks like one; this does not rely on it.)
async fn in_sandbox(sh: &Shared, run: &str) -> bool {
    match (is_dry(run), sh.sandbox()) {
        (true, Ok(sandbox)) => sandbox.holds(run).await,
        _ => false,
    }
}

fn engine(sh: &Shared) -> Result<Arc<Engine>, RpcError> {
    sh.engine()
}

/// The engine `run` is in: the sandbox's for a dry run, the module's otherwise.
async fn engine_of(sh: &Shared, run: &str) -> Result<Arc<Engine>, RpcError> {
    if in_sandbox(sh, run).await {
        sh.sandbox()?.engine_for(run).await.map_err(rpc_error)
    } else {
        engine(sh)
    }
}

/// After the packs changed: the `defs` topic, and the focus of conversations on a flow that is
/// gone.
async fn packs_changed(sh: &Arc<Shared>, flow: Option<&str>) {
    publish::publish_defs(sh).await;
    if let (Some(flow), Ok(engine)) = (flow, sh.engine()) {
        let gone = engine.packs().flow(flow).is_none();
        // A flow that is gone has no conversation any more (the session stays, as history).
        if gone {
            sh.set_flow_session(flow, None);
        }
        for (i, s) in sh.focused_on(flow) {
            if gone {
                sh.set_focus(&i, &s, None);
            }
            publish::publish_focus(sh, &i, &s).await;
        }
    }
}

/// The conversation of `flow` (`""`: the general one): the one recorded while it still exists,
/// else a new `flow-author` session in the flow space, named after the flow.
pub(crate) async fn flow_session(
    sh: &Arc<Shared>,
    packs: &Arc<dyn Packs>,
    flow: &str,
) -> Result<harnix_module::SessionRef, RpcError> {
    let cx = sh
        .cx
        .get()
        .ok_or_else(|| RpcError::new(harnix_contract::codes::INSTANCE_UNAVAILABLE, "the flow module has not started"))?;
    let _one = sh.making_session.lock().await;
    if let Some(name) = sh.flow_session(flow) {
        let s = super::session_ref(&name);
        if cx.sessions().open(&s.instance, &s.session_id).await.is_ok() {
            return Ok(s);
        }
    }
    let s = cx
        .sessions()
        .create(harnix_module::SessionSpec {
            space: Some(sh.cfg.space.clone()),
            instance: Some(super::FLOW_INSTANCE.to_string()),
            scene: Some(super::AUTHOR_SCENE.to_string()),
            // Every tool that changes something is asked of the person, every time: an `ask` rule
            // outranks an `allow` one ("always allow" included), so an approval stays a person's
            // decision (review #3).
            permission_rules: Some(super::author::ask_rules()),
            recording: sh.cfg.conversation_recording.as_ref().and_then(|f| f(flow)),
            ..Default::default()
        })
        .await?;
    sh.set_flow_session(flow, Some(super::session_name(&s)));
    let name = if flow.is_empty() {
        GENERAL_TITLE.to_string()
    } else {
        let title = packs.flow(flow).map(|(d, _)| d.title).unwrap_or_default();
        format!("{FLOW_TITLE}{}", if title.is_empty() { flow.to_string() } else { format!("{title} ({flow})") })
    };
    let _ = cx
        .call(
            "session.rename",
            json!({"instance": s.instance, "session_id": s.session_id, "name": name}),
        )
        .await;
    publish::publish_focus(sh, &s.instance, &s.session_id).await;
    Ok(s)
}

/// [`run_sessions`]' answer for one run.
async fn run_sessions(cx: &CallContext, rec: &crate::RunRecord) -> Answer {
    let own: Vec<Value> = rec
        .def
        .stations
        .iter()
        .filter_map(|d| rec.stations.get(&d.id).map(|s| (d, s)))
        .flat_map(|(d, s)| {
            s.tasks
                .iter()
                .filter(|t| t.session.is_some() && Some(t.session.as_deref()) != Some(rec.main_session()))
                .map(|t| {
                    let r = super::session_ref(t.session.as_deref().unwrap_or_default());
                    json!({"station": d.id, "task": t.id, "session": {"instance": r.instance, "session_id": r.session_id}})
                })
                .collect::<Vec<_>>()
        })
        .collect();
    let Some(main) = rec.main_session().map(super::session_ref) else {
        return Ok(json!({"main": null, "own": own, "tasks": [], "sidechains": []}));
    };
    let module = cx.module();
    let target = json!({"instance": main.instance, "session_id": main.session_id});
    let listed = module.call("session.tasks", target.clone()).await;

    if let Err(e) = &listed {
        tracing::warn!(run = rec.id, error = %e.message, "flow: a main session's tasks not listed");
    }
    let tasks: Vec<Value> = listed
        .ok()
        .and_then(|v| v.get("tasks").and_then(Value::as_array).cloned())
        .unwrap_or_default()
        .into_iter()
        .map(|mut t| {
            // A sub-agent a flow task asked for is named by its tag (`agent_tag`).
            let tag = t.pointer("/detail/description").and_then(Value::as_str).unwrap_or("");
            let flow_task = tag
                .strip_prefix(&format!("flow:{}/", rec.id))
                .and_then(|r| r.split_once('/'))
                .map(|(st, task)| json!({"station": st, "task": task}));
            if let (Some(obj), Some(ft)) = (t.as_object_mut(), flow_task) {
                obj.insert("flow".into(), ft);
            }
            t
        })
        .collect();
    let side = module
        .call(
            "session.list",
            json!({"instance": main.instance, "parent_session_id": main.session_id}),
        )
        .await;

    let sidechains: Vec<Value> = side
        .ok()
        .and_then(|v| match v {
            Value::Array(a) => Some(a),
            Value::Object(o) => o.get("sessions").and_then(Value::as_array).cloned(),
            _ => None,
        })
        .unwrap_or_default()
        .into_iter()
        .map(|mut row| {
            if let Some(o) = row.as_object_mut() {
                o.insert("instance".into(), json!(main.instance));
            }
            row
        })
        .collect();
    Ok(json!({
        "main": {"instance": main.instance, "session_id": main.session_id},
        "own": own, "tasks": tasks, "sidechains": sidechains,
    }))
}

/// The name of the general conversation, and the start of a flow's conversation's name.
pub const GENERAL_TITLE: &str = "Flows · general";
pub const FLOW_TITLE: &str = "Flow · ";

fn to_value(v: impl serde::Serialize) -> Answer {
    serde_json::to_value(v)
        .map_err(|e| RpcError::new(harnix_contract::codes::INTERNAL_ERROR, e.to_string()))
}

pub(crate) fn register(m: &mut MethodRegistry, sh: &Arc<Shared>, packs: &Arc<dyn Packs>) {
    // ── reads ────────────────────────────────────────────────────────────────
    method(
        m,
        sh,
        packs,
        "list",
        "runs, newest first: {flow?, status?, subject?, parent?, limit?}",
        |sh, _, _, q: RunQuery| async move { to_value(engine(&sh)?.list(q).await.map_err(rpc_error)?) },
    );
    method(
        m,
        sh,
        packs,
        "get",
        "a run with its pinned definition and what each open station lacks (a dry run's session tasks carry `preview`): {run}",
        |sh, _, _, p: RunIn| async move {
            if in_sandbox(&sh, &p.run).await {
                return sh.sandbox()?.get(&p.run).await.map_err(rpc_error);
            }
            to_value(engine(&sh)?.get(&p.run).await.map_err(rpc_error)?)
        },
    );
    method(
        m,
        sh,
        packs,
        "history",
        "a run's history, oldest first: {run}",
        |sh, _, _, p: RunIn| async move {
            to_value(
                engine_of(&sh, &p.run)
                    .await?
                    .history(&p.run)
                    .await
                    .map_err(rpc_error)?,
            )
        },
    );
    method(
        m,
        sh,
        packs,
        "defs",
        "the flows on offer, as the `defs` topic has them: [{name, title, digest, origin, valid}]",
        |_, packs, _, _: Value| async move { Ok(Value::Array(publish::defs_list(packs.as_ref()))) },
    );
    method(
        m,
        sh,
        packs,
        "def",
        "one flow: {def, yaml, digest, origin, valid, problems, writable, skills: {name: body}, protocol: {station: text}}",
        |sh, packs, _, p: NameIn| async move { def_info(&*engine(&sh)?, packs.as_ref(), &p.name) },
    );
    method(
        m,
        sh,
        packs,
        "schema",
        "the JSON Schema of a definition (of: definition) or a run record (of: run)",
        |_, _, _, p: SchemaIn| async move {
            match p.of.as_deref().unwrap_or("definition") {
                "definition" | "def" | "flow" => Ok(definition_schema()),
                "run" => Ok(run_schema()),
                other => Err(RpcError::invalid_params(format!(
                    "no schema of {other}: definition or run"
                ))),
            }
        },
    );
    method(
        m,
        sh,
        packs,
        "catalog",
        "the names definitions may use: {scenes, steps, checks, tools, command_gates, templates, instances, spaces, models, guarded, parts: {kind: [name]}, writable}",
        |_, packs, _, _: Value| async move {
            let mut v = super::author::catalog_json(packs.as_ref());
            v["writable"] = json!(packs.writable());
            Ok(v)
        },
    );
    method(
        m,
        sh,
        packs,
        "validate",
        "every structural problem of a definition text: [{station?, message}]",
        |sh, _, _, p: YamlIn| async move { to_value(engine(&sh)?.validate(&p.yaml).await) },
    );

    // ── writing flows (xops-02 §3) ───────────────────────────────────────────
    method(
        m,
        sh,
        packs,
        "save",
        "save a definition in the writable packs, once it validates: {yaml} → {name, digest, origin}",
        |sh, _, cx, p: YamlIn| async move {
            let v = engine(&sh)?.save_flow(&p.yaml, device(&cx)).await.map_err(rpc_error)?;
            packs_changed(&sh, v["name"].as_str()).await;
            Ok(v)
        },
    );
    method(
        m,
        sh,
        packs,
        "delete",
        "delete a saved flow (an override: the built-in one is back): {name} → {name, origin}",
        |sh, _, cx, p: NameIn| async move {
            let v = engine(&sh)?
                .delete_flow(&p.name, device(&cx))
                .await
                .map_err(rpc_error)?;
            packs_changed(&sh, Some(&p.name)).await;
            Ok(v)
        },
    );
    // ── the other parts of a pack (xops-04) ─────────────────────────────────
    method(
        m,
        sh,
        packs,
        "parts",
        "the parts of a kind on offer: {kind: skill|rule|policy|gate|hook} → [{name, title, description, origin, flows}] (flows: the flows that use it)",
        |_, packs, _, p: KindIn| async move { Ok(Value::Array(parts_list(packs.as_ref(), p.kind))) },
    );
    method(
        m,
        sh,
        packs,
        "part",
        "one part: {kind, name} → {kind, name, text, digest, origin, title, description, parsed?, problems, flows}",
        |_, packs, _, p: PartIn| async move { part_info(packs.as_ref(), p.kind, &p.name) },
    );
    method(
        m,
        sh,
        packs,
        "part.save",
        "save a part in the writable packs, once it is sound: {kind, name, text} → {kind, name, digest, origin, broken: [{flow, problems}]}",
        |sh, _, cx, p: PartIn| async move {
            let v = engine(&sh)?
                .save_part(p.kind, &p.name, &p.text, device(&cx))
                .await
                .map_err(rpc_error)?;
            packs_changed(&sh, None).await;
            Ok(v)
        },
    );
    method(
        m,
        sh,
        packs,
        "part.delete",
        "delete a saved part (an override: the built-in one is back): {kind, name} → {kind, name, origin, broken}",
        |sh, _, cx, p: PartIn| async move {
            let v = engine(&sh)?
                .delete_part(p.kind, &p.name, device(&cx))
                .await
                .map_err(rpc_error)?;
            packs_changed(&sh, None).await;
            Ok(v)
        },
    );
    method(
        m,
        sh,
        packs,
        "hooks",
        "the hooks of the pack, each with its last delivery: [{name, title, description, on, flows, kind, action, signed, last: {at, event, run, status, code?, error?, tries} | null}]",
        |sh, packs, _, _: Value| async move { Ok(Value::Array(super::hooks::listing(&sh, &packs.hooks()))) },
    );
    method(
        m,
        sh,
        packs,
        "focus",
        "the flow a conversation works on (the panel's selection): {instance, session_id, flow | null} → {flow}; the session must be a flow-author conversation",
        |sh, packs, cx, p: FocusIn| async move {
            // Only a conversation that can use it: an existing session in the flow-author scene.
            let session = cx
                .module()
                .call(
                    "session.get",
                    json!({"instance": p.instance, "session_id": p.session_id}),
                )
                .await
                .map_err(|e| {
                    RpcError::invalid_params(format!(
                        "no session {}/{}: {}",
                        p.instance, p.session_id, e.message
                    ))
                })?;
            if session.get("scene").and_then(Value::as_str) != Some(super::AUTHOR_SCENE) {
                return Err(RpcError::invalid_params(format!(
                    "session {}/{} is not a {} conversation",
                    p.instance,
                    p.session_id,
                    super::AUTHOR_SCENE
                )));
            }
            let flow = p.flow.filter(|f| !f.is_empty());
            if let Some(f) = &flow {
                if packs.flow(f).is_none() {
                    return Err(rpc_error(FlowError::NotFound(format!("flow {f}"))));
                }
            }
            // A flow's own conversation works on that flow for good; the general one on none.
            if let Some(own) = sh.flow_of_session(&p.instance, &p.session_id) {
                let own = (!own.is_empty()).then_some(own);
                if own != flow {
                    return Err(RpcError::invalid_params(format!(
                        "session {}/{} is {}: its flow does not change",
                        p.instance,
                        p.session_id,
                        own.map_or("the general conversation".to_string(), |f| format!("flow {f}'s conversation")),
                    )));
                }
                return Ok(json!({"flow": own}));
            }
            sh.set_focus(&p.instance, &p.session_id, flow.clone());
            publish::publish_focus(&sh, &p.instance, &p.session_id).await;
            Ok(json!({"flow": flow}))
        },
    );

    method(
        m,
        sh,
        packs,
        "session",
        "a flow's conversation, made when it has none (xops-03 §5): {flow?} → {instance, session_id, flow}; no flow: the general one (new flows, help)",
        |sh, packs, _, p: FlowSessionIn| async move {
            let flow = p.flow.filter(|f| !f.is_empty()).unwrap_or_default();
            if !flow.is_empty() && packs.flow(&flow).is_none() {
                return Err(rpc_error(FlowError::NotFound(format!("flow {flow}"))));
            }
            let s = flow_session(&sh, &packs, &flow).await?;
            Ok(json!({"instance": s.instance, "session_id": s.session_id, "flow": (!flow.is_empty()).then_some(flow)}))
        },
    );
    method(
        m,
        sh,
        packs,
        "sessions",
        "a run's sessions (xops-03 §2): {run} → {main, own: [{station, task, session}], agents: [the main session's sub-agents as its events showed them: {agent_session_id, instance, agent_type, tool_use_id, description, flow: {station, task}?, status}], tasks: [the engine's tasks of the main session], sidechains: [its sub-agents' sessions on disk]}",
        |sh, _, cx, p: RunIn| async move {
            if in_sandbox(&sh, &p.run).await {
                return Ok(json!({"main": null, "own": [], "tasks": [], "sidechains": []}));
            }
            let mut rec = engine(&sh)?.record(&p.run).await.map_err(rpc_error)?;
            // Every run has its main session; one that has none (made where the module was not
            // running) is given it now.
            if rec.main_session().is_none() {
                if let Err(why) = super::main::ensure_main(&sh, &p.run).await {
                    tracing::warn!(run = p.run, why, "flow: no main session");
                }
                rec = engine(&sh)?.record(&p.run).await.map_err(rpc_error)?;
            }
            let mut v = run_sessions(&cx, &rec).await?;
            v["agents"] = json!(sh.agents(&p.run));
            Ok(v)
        },
    );
    method(
        m,
        sh,
        packs,
        "guide",
        "the guide to flows (xops-03 §6), markdown: {lang?: en | zh-CN} → {text, lang}",
        |_, _, _, p: GuideIn| async move {
            let zh = p.lang.as_deref().is_some_and(|l| l.starts_with("zh"));
            Ok(json!({
                "lang": if zh { "zh-CN" } else { "en" },
                "text": if zh { super::author::GUIDE_ZH } else { super::author::GUIDE_EN },
            }))
        },
    );
    method(
        m,
        sh,
        packs,
        "pause",
        "pause a run: what is in hand finishes, nothing new starts (xops-03 §3): {run}",
        |sh, _, cx, p: RunIn| async move {
            let r = engine_of(&sh, &p.run)
                .await?
                .pause(&p.run, device(&cx))
                .await;
            change(&sh, Some(&p.run), r).await
        },
    );
    method(
        m,
        sh,
        packs,
        "resume",
        "carry on with a paused run: {run}",
        |sh, _, cx, p: RunIn| async move {
            let r = engine_of(&sh, &p.run)
                .await?
                .resume(&p.run, device(&cx))
                .await;
            change(&sh, Some(&p.run), r).await
        },
    );

    // ── dry runs (xops-02 §5) ────────────────────────────────────────────────
    method(
        m,
        sh,
        packs,
        "dry_run.start",
        "start a dry run, nothing outside the flow called: {flow | yaml, inputs?, mocks?, subject?, title?} → {run}",
        |sh, _, cx, p: DryStartIn| async move {
            let sandbox = sh.sandbox()?;
            let run = sandbox.start(p, device(&cx)).await.map_err(rpc_error)?;
            publish::publish_dry(&sh, Some(&run)).await;
            Ok(json!({"run": run}))
        },
    );
    method(
        m,
        sh,
        packs,
        "dry_run.step",
        "take a dry run's next work item: {run} → {run, steps, pending, status}",
        |sh, _, _, p: RunIn| async move {
            let progress = sh.sandbox()?.step(&p.run).await.map_err(rpc_error)?;
            publish::publish_dry(&sh, Some(&p.run)).await;
            to_value(progress)
        },
    );
    method(
        m,
        sh,
        packs,
        "dry_run.run",
        "take a dry run's work until it is over or waits for a person: {run} → {run, steps, pending, status}",
        |sh, _, _, p: RunIn| async move {
            let progress = sh.sandbox()?.run(&p.run).await.map_err(rpc_error)?;
            publish::publish_dry(&sh, Some(&p.run)).await;
            to_value(progress)
        },
    );
    method(
        m,
        sh,
        packs,
        "dry_run.list",
        "the dry runs kept, newest first: [{id, flow, status, created_at}]",
        |sh, _, _, _: Value| async move { to_value(sh.sandbox()?.list().await) },
    );

    // ── runs ─────────────────────────────────────────────────────────────────
    method(
        m,
        sh,
        packs,
        "start",
        "start a run: {flow, subject, inputs?, title?} → {run}",
        |sh, _, cx, p: StartIn| async move {
            let r = engine(&sh)?
                .create(
                    CreateIn {
                        flow: p.flow,
                        subject: p.subject,
                        inputs: p.inputs,
                        title: p.title,
                        parent: None,
                    },
                    device(&cx),
                )
                .await;
            change(&sh, None, r).await
        },
    );
    method(
        m,
        sh,
        packs,
        "cancel",
        "cancel a run, its tasks and its child runs: {run, reason}",
        |sh, _, cx, p: CancelIn| async move {
            let accepting = accept_once(&sh, &cx, p.proposal.as_ref()).await?;
            if let Accept::Already(done) = &accepting {
                return Ok(json!({"run": p.run, "already": done}));
            }
            let r = engine_of(&sh, &p.run)
                .await?
                .cancel(&p.run, &p.reason, device(&cx))
                .await;
            let value = change(&sh, Some(&p.run), r).await?;
            accepted(&cx, accepting, "cancel").await;
            Ok(value)
        },
    );
    method(
        m,
        sh,
        packs,
        "proposal",
        "whether a session's proposal was accepted: {instance, session_id, entry} → {handled}",
        |_, _, cx, p: ProposalRef| async move { Ok(json!({"handled": handled(&cx, &p).await?})) },
    );

    // ── external tasks ───────────────────────────────────────────────────────
    method(
        m,
        sh,
        packs,
        "claim",
        "claim an external task: {run, station, task, lease_ms?} → {token, until}",
        |sh, _, cx, p: ClaimIn| async move {
            let r = engine_of(&sh, &p.run)
                .await?
                .claim(
                    &p.run,
                    &p.station,
                    &p.task,
                    device(&cx),
                    p.lease_ms.unwrap_or(CLAIM_MS),
                )
                .await;
            change(&sh, Some(&p.run), r).await
        },
    );
    method(
        m,
        sh,
        packs,
        "heartbeat",
        "extend a claim: {run, station, task, token, lease_ms?} → {until}",
        |sh, _, cx, p: HeartbeatIn| async move {
            let r = engine_of(&sh, &p.run)
                .await?
                .heartbeat(
                    &p.run,
                    &p.station,
                    &p.task,
                    &p.token,
                    p.lease_ms.unwrap_or(CLAIM_MS),
                    device(&cx),
                )
                .await;
            change(&sh, Some(&p.run), r).await
        },
    );
    method(
        m,
        sh,
        packs,
        "submit",
        "hand in a claimed task's output: {run, station, task, token, output}",
        |sh, _, cx, p: SubmitIn| async move {
            let r = engine_of(&sh, &p.run)
                .await?
                .submit(&p.run, &p.station, &p.task, &p.token, p.output, device(&cx))
                .await;
            change(&sh, Some(&p.run), r).await
        },
    );
    method(
        m,
        sh,
        packs,
        "fail",
        "report a claimed task's attempt failed: {run, station, task, token, error}",
        |sh, _, cx, p: FailIn| async move {
            let r = engine_of(&sh, &p.run)
                .await?
                .fail(&p.run, &p.station, &p.task, &p.token, &p.error, device(&cx))
                .await;
            change(&sh, Some(&p.run), r).await
        },
    );
    method(
        m,
        sh,
        packs,
        "retry",
        "put a failed task back to ready: {run, station, task}",
        |sh, _, cx, p: TaskIn| async move {
            let r = engine_of(&sh, &p.run)
                .await?
                .retry(&p.run, &p.station, &p.task, device(&cx))
                .await;
            change(&sh, Some(&p.run), r).await
        },
    );

    // ── stations ─────────────────────────────────────────────────────────────
    method(
        m,
        sh,
        packs,
        "confirm",
        "confirm (pass: true) or reject a station: {run, station, pass, reason?}",
        |sh, _, cx, p: ConfirmIn| async move {
            let r = engine_of(&sh, &p.run)
                .await?
                .confirm(&p.run, &p.station, p.pass, p.reason.as_deref(), device(&cx))
                .await;
            change(&sh, Some(&p.run), r).await
        },
    );
    method(
        m,
        sh,
        packs,
        "force",
        "pass a station whose conditions are not met: {run, station, reason}",
        |sh, _, cx, p: ReasonIn| async move {
            let r = engine_of(&sh, &p.run)
                .await?
                .force(&p.run, &p.station, &p.reason, device(&cx))
                .await;
            change(&sh, Some(&p.run), r).await
        },
    );
    method(
        m,
        sh,
        packs,
        "rework",
        "send work back to a station: {run, to, tasks?, reason}",
        |sh, _, cx, p: ReworkIn| async move {
            let accepting = accept_once(&sh, &cx, p.proposal.as_ref()).await?;
            if let Accept::Already(done) = &accepting {
                return Ok(json!({"run": p.run, "already": done}));
            }
            let r = engine_of(&sh, &p.run)
                .await?
                .rework(&p.run, &p.to, p.tasks, &p.reason, device(&cx))
                .await;
            let value = change(&sh, Some(&p.run), r).await?;
            accepted(&cx, accepting, "rework").await;
            Ok(value)
        },
    );
    method(
        m,
        sh,
        packs,
        "release",
        "let a held station in: {run, station}",
        |sh, _, cx, p: StationIn| async move {
            let r = engine_of(&sh, &p.run)
                .await?
                .release(&p.run, &p.station, device(&cx))
                .await;
            change(&sh, Some(&p.run), r).await
        },
    );
}

/// The entry a proposal's acceptance is recorded as, in the proposal's own session.
pub const PROPOSAL_HANDLED: &str = "proposal_handled";

/// What accepting a proposal found.
enum Accept {
    /// No proposal named: an ordinary call.
    Plain,
    /// Not accepted before; the lock is held until it is recorded.
    First(ProposalRef, Actor, tokio::sync::OwnedMutexGuard<()>),
    /// Accepted already: what was recorded then.
    Already(Value),
}

/// The record of a proposal's acceptance, if there is one.
async fn handled(cx: &CallContext, p: &ProposalRef) -> Result<Option<Value>, RpcError> {
    let session = harnix_module::SessionRef {
        instance: p.instance.clone(),
        session_id: p.session_id.clone(),
    };
    let entries = cx.module().sessions().entries(&session).await?;
    Ok(entries.into_iter().find_map(|e| {
        (e.get("event").and_then(Value::as_str) == Some(PROPOSAL_HANDLED)
            && e.pointer("/payload/entry").and_then(Value::as_str) == Some(p.entry.as_str()))
        .then(|| e.get("payload").cloned().unwrap_or(Value::Null))
    }))
}

async fn accept_once(
    sh: &Arc<Shared>,
    cx: &CallContext,
    p: Option<&ProposalRef>,
) -> Result<Accept, RpcError> {
    let Some(p) = p else {
        return Ok(Accept::Plain);
    };
    let guard = sh.proposals.clone().lock_owned().await;
    match handled(cx, p).await? {
        Some(done) => Ok(Accept::Already(done)),
        None => Ok(Accept::First(p.clone(), device(cx), guard)),
    }
}

/// Record the acceptance in the proposal's session (the card reads it from there).
async fn accepted(cx: &CallContext, a: Accept, action: &str) {
    let Accept::First(p, by, _guard) = a else {
        return;
    };
    let session = harnix_module::SessionRef {
        instance: p.instance.clone(),
        session_id: p.session_id.clone(),
    };
    let entry = harnix_module::Entry::new(
        PROPOSAL_HANDLED,
        json!({"entry": p.entry, "action": action, "by": by}),
    );
    if let Err(e) = cx.module().sessions().append_entry(&session, entry).await {
        tracing::warn!(entry = p.entry, error = %e, "flow: a proposal's acceptance not recorded");
    }
}

/// `def`: the definition as parsed and as written, where it comes from and whether it is valid,
/// the parts it uses (skills, rules, policies, gates) and the hooks over it, each station's rules
/// and permissions, and the protocol layer each session station's tasks are given (with
/// placeholders for the run).
fn def_info(engine: &Engine, packs: &dyn Packs, name: &str) -> Answer {
    let (def, digest) = packs
        .flow(name)
        .ok_or_else(|| rpc_error(FlowError::NotFound(format!("flow {name}"))))?;
    let yaml = packs
        .flow_text(name)
        .or_else(|| serde_yaml::to_string(&def).ok())
        .unwrap_or_default();
    let mut proto = BTreeMap::new();
    let mut stations = BTreeMap::new();
    let pinned = crate::kernel::parts::pin(&def, packs);
    for s in &def.stations {
        let specs = match s.work() {
            Work::None => vec![],
            Work::One(t) => vec![t],
            Work::Tasks(ts) => ts,
            Work::Foreach { task, .. } => vec![task],
        };
        let permissions = crate::kernel::parts::permissions_of(&def, s, &pinned.policies);
        stations.insert(
            s.id.clone(),
            json!({"rules": crate::kernel::parts::rules_of(&def, s), "permissions": permissions}),
        );
        if let Some(t) = specs
            .iter()
            .find(|t| matches!(t.executor, crate::Executor::Session { .. } | crate::Executor::Agent { .. }))
        {
            let delivery = match &t.executor {
                crate::Executor::Session { scene, .. } if !t.executor.in_main() => {
                    engine.delivery_of(scene)
                }
                _ => crate::Delivery::FlowSubmit,
            };
            let text = protocol::render(&protocol::TaskContext {
                def: &def,
                station: s,
                run: "<run>",
                subject: "<subject>",
                round: 1,
                task: &t.id,
                item: None,
                guarded: engine.guarded(),
                permissions: &permissions,
                gates: &pinned.gates,
                delivery,
                executor: Some(&t.executor),
            });
            // What the session is told before its skill: the rules too.
            let rules: Vec<(String, String)> = crate::kernel::parts::rules_of(&def, s)
                .into_iter()
                .filter_map(|r| packs.rule(&r).map(|(t, _)| (r, t)))
                .collect();
            proto.insert(s.id.clone(), protocol::task_message(&text, &rules, None));
        }
    }
    let problems = validate(&def, packs);
    let used = super::author::parts_used(packs, &def);
    Ok(json!({
        "def": def, "yaml": yaml, "digest": digest, "origin": packs.origin(name),
        "valid": problems.is_empty(), "problems": problems, "writable": packs.writable(),
        "parts": used["parts"], "hooks": used["hooks"], "stations": stations, "protocol": proto,
    }))
}

/// A part's title and description: a YAML part's fields, a rule's heading, a skill's front matter.
fn part_heading(kind: crate::PartKind, name: &str, text: &str) -> (String, String) {
    use crate::PartKind as K;
    match kind {
        K::Rule => {
            let (title, body) = crate::kernel::parts::rule_parts(name, text);
            (title, body.lines().next().unwrap_or_default().to_string())
        }
        K::Skill => {
            let front = text
                .trim_start()
                .strip_prefix("---\n")
                .and_then(|r| r.split_once("\n---"))
                .and_then(|(f, _)| serde_yaml::from_str::<serde_yaml::Value>(f).ok());
            let get = |k: &str| {
                front
                    .as_ref()
                    .and_then(|f| f.get(k))
                    .and_then(|v| v.as_str())
                    .map(String::from)
            };
            (get("name").unwrap_or_else(|| name.to_string()), get("description").unwrap_or_default())
        }
        K::Policy | K::Gate | K::Hook => {
            let v = serde_yaml::from_str::<serde_yaml::Value>(text).ok();
            let get = |k: &str| {
                v.as_ref()
                    .and_then(|f| f.get(k))
                    .and_then(|v| v.as_str())
                    .map(String::from)
                    .unwrap_or_default()
            };
            (get("title"), get("description"))
        }
    }
}

/// The flows that use a part.
fn users_of(packs: &dyn Packs, kind: crate::PartKind, name: &str) -> Vec<String> {
    match kind {
        crate::PartKind::Hook => packs
            .hook(name)
            .map(|h| if h.flows.is_empty() { packs.flows() } else { h.flows })
            .unwrap_or_default(),
        _ => packs
            .flows()
            .into_iter()
            .filter(|f| packs.flow(f).is_some_and(|(d, _)| crate::kernel::parts::uses(&d, kind, name)))
            .collect(),
    }
}

fn parts_list(packs: &dyn Packs, kind: crate::PartKind) -> Vec<Value> {
    packs
        .parts(kind)
        .into_iter()
        .filter_map(|n| {
            let (text, _) = packs.part(kind, &n)?;
            let (title, description) = part_heading(kind, &n, &text);
            Some(json!({
                "name": n, "title": title, "description": description,
                "origin": packs.part_origin(kind, &n), "flows": users_of(packs, kind, &n),
            }))
        })
        .collect()
}

fn part_info(packs: &dyn Packs, kind: crate::PartKind, name: &str) -> Answer {
    let (text, digest) = packs
        .part(kind, name)
        .ok_or_else(|| rpc_error(FlowError::NotFound(format!("{kind} {name}"))))?;
    let (title, description) = part_heading(kind, name, &text);
    let parsed = match kind {
        crate::PartKind::Policy => packs.policy(name).and_then(|p| serde_json::to_value(p).ok()),
        crate::PartKind::Gate => packs.gate(name).and_then(|g| serde_json::to_value(g).ok()),
        crate::PartKind::Hook => packs.hook(name).and_then(|h| serde_json::to_value(h).ok()),
        _ => None,
    };
    Ok(json!({
        "kind": kind, "name": name, "text": text, "digest": digest,
        "origin": packs.part_origin(kind, name), "title": title, "description": description,
        "parsed": parsed, "problems": crate::kernel::validate::part_problems(kind, name, &text, packs),
        "flows": users_of(packs, kind, name), "writable": packs.writable(),
    }))
}
