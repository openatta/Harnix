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

use super::dispatch;
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
struct SkillIn {
    name: String,
    body: String,
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
            dispatch::publish_dry(sh, Some(run)).await;
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
    dispatch::publish_defs(sh).await;
    if let (Some(flow), Ok(engine)) = (flow, sh.engine()) {
        let gone = engine.packs().flow(flow).is_none();
        for (i, s) in sh.focused_on(flow) {
            if gone {
                sh.set_focus(&i, &s, None);
            }
            dispatch::publish_focus(sh, &i, &s).await;
        }
    }
}

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
        |_, packs, _, _: Value| async move { Ok(Value::Array(dispatch::defs_list(packs.as_ref()))) },
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
        "the names definitions may use: {scenes, steps, gates, tools, actions, templates, instances, spaces, models, skills, writable}",
        |_, packs, _, _: Value| async move {
            let c = packs.catalog();
            Ok(json!({
                "scenes": c.scenes, "steps": c.steps, "gates": c.gates, "tools": c.tools,
                "actions": c.actions, "templates": c.templates, "instances": c.instances,
                "spaces": c.spaces, "models": c.models, "skills": packs.skills(),
                "writable": packs.writable(),
            }))
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
    method(
        m,
        sh,
        packs,
        "skill.save",
        "save a skill's SKILL.md body: {name, body} → {name, digest}",
        |sh, _, cx, p: SkillIn| async move {
            let v = engine(&sh)?
                .save_skill(&p.name, &p.body, device(&cx))
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
        "skill.delete",
        "delete a saved skill: {name} → {name}",
        |sh, _, cx, p: NameIn| async move {
            let v = engine(&sh)?
                .delete_skill(&p.name, device(&cx))
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
            sh.set_focus(&p.instance, &p.session_id, flow.clone());
            dispatch::publish_focus(&sh, &p.instance, &p.session_id).await;
            Ok(json!({"flow": flow}))
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
            dispatch::publish_dry(&sh, Some(&run)).await;
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
            dispatch::publish_dry(&sh, Some(&p.run)).await;
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
            dispatch::publish_dry(&sh, Some(&p.run)).await;
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
/// the skills it names, and the protocol layer each session station's tasks are given (with
/// placeholders for the run).
fn def_info(engine: &Engine, packs: &dyn Packs, name: &str) -> Answer {
    let (def, digest) = packs
        .flow(name)
        .ok_or_else(|| rpc_error(FlowError::NotFound(format!("flow {name}"))))?;
    let yaml = packs
        .flow_text(name)
        .or_else(|| serde_yaml::to_string(&def).ok())
        .unwrap_or_default();
    let mut skills = BTreeMap::new();
    let mut proto = BTreeMap::new();
    for s in &def.stations {
        let specs = match s.work() {
            Work::None => vec![],
            Work::One(t) => vec![t],
            Work::Tasks(ts) => ts,
            Work::Foreach { task, .. } => vec![task],
        };
        for t in &specs {
            if let Some(skill) = &t.skill {
                if let Some((body, _)) = packs.skill(skill) {
                    skills.insert(skill.clone(), body);
                }
            }
        }
        if let Some((t, scene)) = specs.iter().find_map(|t| match &t.executor {
            crate::Executor::Session { scene, .. } => Some((t, scene)),
            _ => None,
        }) {
            let text = protocol::render(&protocol::TaskContext {
                def: &def,
                station: s,
                run: "<run>",
                subject: "<subject>",
                round: 1,
                task: &t.id,
                item: None,
                guarded: engine.guarded(),
                delivery: engine.delivery_of(scene),
            });
            proto.insert(s.id.clone(), text);
        }
    }
    let problems = validate(&def, packs);
    Ok(json!({
        "def": def, "yaml": yaml, "digest": digest, "origin": packs.origin(name),
        "valid": problems.is_empty(), "problems": problems, "writable": packs.writable(),
        "skills": skills, "protocol": proto,
    }))
}
