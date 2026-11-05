//! Writing flows in conversation (xops-02 §4): the `flow-author` scene and its tools.
//!
//! The flow space's default scene. It wraps a scene the application gives (Agix's chat by
//! default) and adds the tools below and a system prompt block on how to write flows. A tool
//! that needs **the current flow** reads it from the conversation's focus: what the person has
//! selected in the panel beside it (device method `focus`), or the flow `flow_create` just made.
//!
//! | tool | read-only | does |
//! |---|---|---|
//! | `flow_list` | yes | the flows: name, title, origin, valid |
//! | `flow_show` | yes | the current flow: YAML, the skills it uses, problems, its stations |
//! | `flow_schema` | yes | the definition's JSON Schema and this application's catalog |
//! | `flow_validate {yaml}` | yes | a text's problems, nothing saved |
//! | `flow_create {yaml, skills?}` | no | a new flow, which becomes the current one |
//! | `flow_update {yaml?, skills?}` | no | change the current flow |
//! | `flow_delete` | no | delete the current flow (a built-in one: its override) |
//! | `flow_skill_set {name, body}` / `flow_skill_delete {name}` | no | change, delete a skill |
//! | `flow_dry_run {inputs?, mocks?}` | yes | dry-run the current flow, as far as it goes |
//! | `flow_start {subject, inputs?, title?}` | no | start the current flow for real |
//!
//! Tools that are not read-only are asked about through the engine's permission mechanism —
//! the person is in the conversation. None of them confirms, rejects, forces, reworks, releases
//! or cancels anything: an agent still approves nothing on a person's behalf.

use std::collections::BTreeMap;
use std::sync::Arc;

use base::interface::prompt::PromptBlock;
use base::interface::scene::{
    AgentScene, ExecutionParams, ReminderContext, ScenePromptContext, TokenBudget,
};
use base::tool::{PermissionDecision, ProgressSender, Tool, ToolContext, ToolResult};
use serde_json::{json, Value};

use super::dispatch;
use super::shared::Shared;
use crate::dryrun::StartIn;
use crate::kernel::def::{definition_schema, FlowDef, Work};
use crate::kernel::engine::CreateIn;
use crate::kernel::host::{Catalog, Packs};
use crate::kernel::run::Actor;
use crate::kernel::validate::validate_yaml;

/// The scene's id.
pub const AUTHOR_SCENE: &str = "flow-author";

/// What a tool needing the current flow answers when there is none.
pub const NO_FOCUS: &str = "No flow is selected yet: ask the user to select the flow to work on \
     in the panel on the right first (还没有选择流程：请用户先在右边栏选择要操作的流程). Creating a \
     new flow with `flow_create` needs no selection.";

pub const TOOL_LIST: &str = "flow_list";
pub const TOOL_SHOW: &str = "flow_show";
pub const TOOL_SCHEMA: &str = "flow_schema";
pub const TOOL_VALIDATE: &str = "flow_validate";
pub const TOOL_CREATE: &str = "flow_create";
pub const TOOL_UPDATE: &str = "flow_update";
pub const TOOL_DELETE: &str = "flow_delete";
pub const TOOL_SKILL_SET: &str = "flow_skill_set";
pub const TOOL_SKILL_DELETE: &str = "flow_skill_delete";
pub const TOOL_DRY_RUN: &str = "flow_dry_run";
pub const TOOL_START: &str = "flow_start";

/// Every authoring tool, and whether it is read-only.
pub const AUTHOR_TOOLS: [(&str, bool); 11] = [
    (TOOL_LIST, true),
    (TOOL_SHOW, true),
    (TOOL_SCHEMA, true),
    (TOOL_VALIDATE, true),
    (TOOL_CREATE, false),
    (TOOL_UPDATE, false),
    (TOOL_DELETE, false),
    (TOOL_SKILL_SET, false),
    (TOOL_SKILL_DELETE, false),
    (TOOL_DRY_RUN, true),
    (TOOL_START, false),
];

/// The system prompt block `flow-author` adds.
pub const AUTHOR_PROMPT: &str = "## Writing flows\n\n\
You help the user write and try out **flows**: YAML definitions of a graph of stations that a run \
moves one subject through. Each station has work (`executor`: a `session` working from a skill, a \
`program`, an `external` claimant, a `subflow`; or `tasks`, or `foreach` + `executor`; or none, a \
pure checkpoint) and exit conditions (`exit`: `output`, `tool`, `gate`, `confirm`, `children`) that \
must all hold for it to pass; `needs` orders stations (absent = the previous one), `rework_to` \
says where work may be sent back. A session station needs a `skill` and an `output` contract (a \
JSON Schema): the session hands its output in against it.\n\n\
How to work:\n\
- Read `flow_schema` before writing: the fields, and the names this application offers (scenes, \
  program steps, gates, tools, actions, templates…). Use only those names.\n\
- Before changing a flow, read it with `flow_show`. The flow you work on is the one the user \
  selected in the panel on the right; if none is selected, ask them to select one (creating a \
  new flow with `flow_create` needs no selection, and selects it).\n\
- Check every text with `flow_validate` before saving it with `flow_create` or `flow_update`; a \
  text with problems is not saved.\n\
- Try the path with `flow_dry_run` (nothing outside the flow is called; outputs, gates and tools \
  are mocked, `mocks` can set them). Start a real run with `flow_start` only when the user asks.\n\
- You cannot confirm, reject, force, rework, release or cancel anything in a run: those are the \
  user's, in the panel.";

/// An application's scene with the authoring tools and prompt added.
pub struct AuthorScene {
    inner: Arc<dyn AgentScene>,
    sh: Arc<Shared>,
}

impl AuthorScene {
    pub(crate) fn new(inner: Arc<dyn AgentScene>, sh: Arc<Shared>) -> Self {
        Self { inner, sh }
    }
}

impl AgentScene for AuthorScene {
    fn id(&self) -> &str {
        AUTHOR_SCENE
    }
    fn name(&self) -> &str {
        "Flow author"
    }
    fn description(&self) -> &str {
        "write, check and try out flows in conversation"
    }
    fn build_system_prompt(&self, ctx: &ScenePromptContext) -> Vec<PromptBlock> {
        let mut blocks = self.inner.build_system_prompt(ctx);
        blocks.push(PromptBlock::system_cached(AUTHOR_PROMPT.to_string()));
        blocks
    }
    /// The base whitelist plus ours; an empty one (everything) stays empty.
    fn tools(&self) -> Vec<String> {
        let mut tools = self.inner.tools();
        if !tools.is_empty() {
            tools.extend(AUTHOR_TOOLS.iter().map(|(n, _)| n.to_string()));
        }
        tools
    }
    fn requires_project(&self) -> bool {
        false
    }
    fn supports_team(&self) -> bool {
        self.inner.supports_team()
    }
    fn extra_tools(&self) -> Vec<Arc<dyn Tool>> {
        let mut tools = self.inner.extra_tools();
        for (name, read_only) in AUTHOR_TOOLS {
            tools.push(Arc::new(AuthorTool {
                name,
                read_only,
                sh: self.sh.clone(),
            }));
        }
        tools
    }
    fn token_budget(&self) -> TokenBudget {
        self.inner.token_budget()
    }
    fn build_system_reminder(&self, ctx: &ReminderContext) -> String {
        self.inner.build_system_reminder(ctx)
    }
    fn disallowed_tools(&self) -> Vec<String> {
        self.inner.disallowed_tools()
    }
    fn permits_tool(&self, name: &str) -> bool {
        AUTHOR_TOOLS.iter().any(|(n, _)| *n == name) || self.inner.permits_tool(name)
    }
    fn deferred_tools(&self) -> Vec<String> {
        self.inner.deferred_tools()
    }
    fn execution_params(&self) -> ExecutionParams {
        self.inner.execution_params()
    }
    /// No model call outside the conversation (under replay, one that is not recorded).
    fn auto_name_session(&self) -> bool {
        false
    }
    fn session_name_prompt(&self, _first_message: &str) -> Option<String> {
        None
    }
    fn memory_extraction_prompt(&self) -> Option<String> {
        self.inner.memory_extraction_prompt()
    }
}

/// The packs as they will be once `skills` are saved: what a text is checked against first.
struct WithSkills<'a> {
    base: &'a dyn Packs,
    skills: &'a BTreeMap<String, String>,
}

impl Packs for WithSkills<'_> {
    fn flows(&self) -> Vec<String> {
        self.base.flows()
    }
    fn flow(&self, name: &str) -> Option<(FlowDef, String)> {
        self.base.flow(name)
    }
    fn skill(&self, name: &str) -> Option<(String, String)> {
        match self.skills.get(name) {
            Some(b) => Some((b.clone(), crate::kernel::host::digest(b))),
            None => self.base.skill(name),
        }
    }
    fn catalog(&self) -> Catalog {
        self.base.catalog()
    }
}

/// The flow a tool that changes or runs one acts on: the `flow` it names, if that is still the
/// conversation's current flow.
pub(crate) fn target_flow(input: &Value, focus: Option<&str>) -> Result<String, String> {
    let Some(named) = input
        .get("flow")
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
    else {
        return Err(
            "name the flow this acts on in `flow`: the current one, as flow_show gave it".into(),
        );
    };
    match focus {
        None => Err(NO_FOCUS.into()),
        Some(f) if f == named => Ok(named.to_string()),
        Some(f) => Err(format!(
            "the current flow changed to `{f}` (you named `{named}`): nothing was done — ask the \
             user again which flow they mean"
        )),
    }
}

/// What is wrong with skills about to be saved.
fn skill_problems(skills: &BTreeMap<String, String>) -> Vec<String> {
    let mut out = vec![];
    for (k, body) in skills {
        if !crate::kernel::validate::is_skill_name(k) {
            out.push(format!("skill name {k:?} must match [a-z][a-z0-9_-]*"));
        }
        if body.trim().is_empty() {
            out.push(format!("skill {k} needs a body"));
        }
    }
    out
}

/// Put back skills this call saved: each as its body was before, or gone if it was not there.
fn restore_skills(packs: &dyn Packs, before: &[(String, Option<String>)], saved: &[String]) {
    for (k, old) in before.iter().filter(|(k, _)| saved.contains(k)) {
        let _ = packs.delete_skill(k);
        let now = packs.skill(k).map(|(b, _)| b);
        if now != *old {
            if let Some(body) = old {
                let _ = packs.save_skill(k, body);
            }
        }
    }
}

type ToolOut = Result<ToolResult, base::error::ToolError>;

/// One authoring tool: which one is its name.
struct AuthorTool {
    name: &'static str,
    read_only: bool,
    sh: Arc<Shared>,
}

fn pretty(v: &Value) -> String {
    serde_json::to_string_pretty(v).unwrap_or_else(|_| v.to_string())
}

fn text(v: Value) -> ToolOut {
    Ok(ToolResult::text(pretty(&v)))
}

fn error(m: impl Into<String>) -> ToolOut {
    Ok(ToolResult::error_text(m.into()))
}

fn str_of<'v>(input: &'v Value, k: &str) -> Option<&'v str> {
    input
        .get(k)
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
}

/// `skills: {name: body}` from a tool's input.
fn skills_of(input: &Value) -> Result<BTreeMap<String, String>, String> {
    match input.get("skills") {
        None | Some(Value::Null) => Ok(BTreeMap::new()),
        Some(Value::Object(m)) => m
            .iter()
            .map(|(k, v)| {
                v.as_str()
                    .map(|b| (k.clone(), b.to_string()))
                    .ok_or_else(|| format!("skill {k}: the body is a string"))
            })
            .collect(),
        Some(_) => Err("`skills` is an object of skill name → SKILL.md body".into()),
    }
}

fn catalog_json(packs: &dyn Packs) -> Value {
    let c: Catalog = packs.catalog();
    json!({
        "scenes": c.scenes, "steps": c.steps, "gates": c.gates, "tools": c.tools,
        "actions": c.actions, "templates": c.templates, "instances": c.instances,
        "spaces": c.spaces, "models": c.models, "skills": packs.skills(),
    })
}

/// A flow's stations, briefly: `id`, `title`, `needs`, its work, its exits.
fn stations_of(def: &FlowDef) -> Vec<Value> {
    def.stations
        .iter()
        .map(|s| {
            let work = match s.work() {
                Work::None => "checkpoint".to_string(),
                Work::One(t) => t.executor.kind().to_string(),
                Work::Tasks(ts) => format!(
                    "tasks: {}",
                    ts.iter()
                        .map(|t| format!("{} ({})", t.id, t.executor.kind()))
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
                Work::Foreach { items, task, .. } => {
                    format!("foreach {items}: {}", task.executor.kind())
                }
            };
            json!({
                "id": s.id, "title": s.title, "needs": def.needs_of(&s.id), "work": work,
                "exit": s.exit.iter().map(|e| e.label()).collect::<Vec<_>>(),
                "rework_to": s.rework_to,
            })
        })
        .collect()
}

#[async_trait::async_trait]
impl Tool for AuthorTool {
    fn name(&self) -> &str {
        self.name
    }
    fn description(&self) -> &str {
        match self.name {
            TOOL_LIST => "The flows on offer: name, title, origin (builtin, override, user) and whether each is valid; and which one is current.",
            TOOL_SHOW => "The current flow (the one the user selected): its YAML, the bodies of the skills it uses, its problems, and its stations in brief.",
            TOOL_SCHEMA => "The JSON Schema of a flow definition, and the names this application offers (scenes, steps, gates, tools, actions, templates, instances, spaces, models, skills).",
            TOOL_VALIDATE => "Every problem of a flow definition text (YAML), without saving anything. An empty list: it is valid.",
            TOOL_CREATE => "Create a new flow from its YAML (and, optionally, skills it uses: {name: SKILL.md body}). It is validated first and not saved if it has problems. It becomes the current flow.",
            TOOL_UPDATE => "Change the current flow, named in `flow`: its YAML (same name), and/or skills ({name: SKILL.md body}). Validated first; nothing is saved if anything has problems. Runs already started keep their definition.",
            TOOL_DELETE => "Delete the current flow, named in `flow`. For a built-in flow, this deletes the saved override and the built-in version is back.",
            TOOL_SKILL_SET => "Create or change a skill: its SKILL.md body.",
            TOOL_SKILL_DELETE => "Delete a saved skill.",
            TOOL_DRY_RUN => "Dry-run the current flow, named in `flow`: the real kernel, everything outside mocked (no session, no program, no model). Runs until it ends or waits for a person, and returns the path it took. `mocks`: {stations: {id: output}, gates: {gate: false}, tools: {tool: result}}.",
            TOOL_START => "Start a real run of the current flow, named in `flow`, for a subject. Only when the user asks for it.",
            _ => "",
        }
    }
    fn input_schema(&self) -> Value {
        let skills = json!({"type": "object", "additionalProperties": {"type": "string"},
                            "description": "skill name → SKILL.md body"});
        match self.name {
            TOOL_VALIDATE => {
                json!({"type": "object", "required": ["yaml"], "properties": {"yaml": {"type": "string"}}})
            }
            TOOL_CREATE => {
                json!({"type": "object", "required": ["yaml"], "properties": {"yaml": {"type": "string"}, "skills": skills}})
            }
            TOOL_UPDATE => {
                json!({"type": "object", "required": ["flow"], "properties": {"flow": {"type": "string", "description": "the flow this acts on, by name: the current one, as flow_show or flow_list gave it"}, "yaml": {"type": "string"}, "skills": skills}})
            }
            TOOL_DELETE => {
                json!({"type": "object", "required": ["flow"], "properties": {"flow": {"type": "string", "description": "the flow this acts on, by name: the current one, as flow_show or flow_list gave it"}}})
            }
            TOOL_SKILL_SET => {
                json!({"type": "object", "required": ["name", "body"], "properties": {"name": {"type": "string"}, "body": {"type": "string"}}})
            }
            TOOL_SKILL_DELETE => {
                json!({"type": "object", "required": ["name"], "properties": {"name": {"type": "string"}}})
            }
            TOOL_DRY_RUN => json!({"type": "object", "required": ["flow"], "properties": {
                "flow": {"type": "string", "description": "the flow this acts on, by name: the current one, as flow_show or flow_list gave it"},
                "inputs": {"type": "object"},
                "mocks": {"type": "object", "properties": {
                    "stations": {"type": "object"}, "gates": {"type": "object"}, "tools": {"type": "object"}}}}}),
            TOOL_START => json!({"type": "object", "required": ["flow", "subject"], "properties": {
                "flow": {"type": "string", "description": "the flow this acts on, by name: the current one, as flow_show or flow_list gave it"},
                "subject": {"type": "string"}, "inputs": {"type": "object"}, "title": {"type": "string"}}}),
            _ => json!({"type": "object", "properties": {}}),
        }
    }
    fn is_read_only(&self, _: &Value) -> bool {
        self.read_only
    }
    fn is_concurrency_safe(&self, _: &Value) -> bool {
        self.read_only
    }
    /// What changes something is asked about: the person is in the conversation.
    async fn check_permissions(&self, input: &Value, _: &ToolContext) -> PermissionDecision {
        if self.read_only {
            return PermissionDecision::allow();
        }
        let named = |k: &str| {
            input
                .get(k)
                .and_then(Value::as_str)
                .map(|n| format!(" `{n}`"))
                .unwrap_or_default()
        };
        let created = input
            .get("yaml")
            .and_then(Value::as_str)
            .and_then(|y| FlowDef::parse(y).ok())
            .map(|d| format!(" `{}`", d.name))
            .unwrap_or_default();
        let what = match self.name {
            TOOL_CREATE => format!("create the flow{created}"),
            TOOL_UPDATE => format!("change the flow{}", named("flow")),
            TOOL_DELETE => format!("delete the flow{}", named("flow")),
            TOOL_SKILL_SET => format!("save the skill{}", named("name")),
            TOOL_SKILL_DELETE => format!("delete the skill{}", named("name")),
            TOOL_START => format!(
                "start a run of the flow{} for{}",
                named("flow"),
                named("subject")
            ),
            other => other.to_string(),
        };
        PermissionDecision::ask(format!("Let the agent {what}?"))
    }
    async fn call(&self, input: Value, cx: ToolContext, _p: ProgressSender) -> ToolOut {
        let sh = &self.sh;
        let Ok(engine) = sh.engine() else {
            return error("the flow module has not started");
        };
        let instance = sh.cfg.instance.clone();
        let session = cx.session_id.clone();
        let by = Actor::agent(format!("{instance}/{session}"));
        let packs = engine.packs().clone();
        let focus = sh
            .focus(&instance, &session)
            .filter(|f| packs.flow(f).is_some());
        let need_focus = || focus.clone().ok_or(());
        // What changes or runs a flow names it, and acts only if it is still the current one:
        // the person approved that flow, not whatever is selected when the call runs.
        let target = || target_flow(&input, focus.as_deref());
        match self.name {
            TOOL_LIST => text(json!({
                "flows": dispatch::defs_list(packs.as_ref()),
                "current": focus,
                "writable": packs.writable(),
            })),
            TOOL_SCHEMA => text(json!({
                "definition": definition_schema(),
                "catalog": catalog_json(packs.as_ref()),
            })),
            TOOL_VALIDATE => {
                let Some(yaml) = str_of(&input, "yaml") else {
                    return error("give the definition as `yaml`");
                };
                text(json!({"problems": validate_yaml(yaml, packs.as_ref())}))
            }
            TOOL_SHOW => {
                let Ok(name) = need_focus() else {
                    return error(NO_FOCUS);
                };
                let Some((def, digest)) = packs.flow(&name) else {
                    return error(format!("flow {name} is gone"));
                };
                let mut skills = BTreeMap::new();
                for s in &def.stations {
                    let specs = match s.work() {
                        Work::None => vec![],
                        Work::One(t) | Work::Foreach { task: t, .. } => vec![t],
                        Work::Tasks(ts) => ts,
                    };
                    for t in specs {
                        if let Some(k) = t.skill {
                            let body = packs.skill(&k).map(|(b, _)| b);
                            skills.insert(k, body);
                        }
                    }
                }
                text(json!({
                    "name": name, "title": def.title, "origin": packs.origin(&name), "digest": digest,
                    "yaml": packs.flow_text(&name).unwrap_or_default(),
                    "problems": crate::kernel::validate::validate(&def, packs.as_ref()),
                    "stations": stations_of(&def),
                    "skills": skills,
                }))
            }
            TOOL_CREATE | TOOL_UPDATE => {
                let creating = self.name == TOOL_CREATE;
                let current = match (creating, target()) {
                    (true, _) => None,
                    (false, Ok(f)) => Some(f),
                    (false, Err(m)) => return error(m),
                };
                let skills = match skills_of(&input) {
                    Ok(s) => s,
                    Err(m) => return error(m),
                };
                let yaml = str_of(&input, "yaml");
                if creating && yaml.is_none() {
                    return error("give the new flow's definition as `yaml`");
                }
                if yaml.is_none() && skills.is_empty() {
                    return error("give `yaml`, `skills`, or both");
                }
                let name = match yaml.map(FlowDef::parse) {
                    Some(Ok(d)) => Some(d.name),
                    Some(Err(e)) => return error(format!("not a flow definition: {e}")),
                    None => None,
                };
                if let (Some(n), Some(cur)) = (&name, &current) {
                    if n != cur {
                        return error(format!(
                            "the text defines flow `{n}`, but the current flow is `{cur}`: keep \
                             the name, or create a new flow with {TOOL_CREATE}"
                        ));
                    }
                }
                if let (true, Some(n)) = (creating, &name) {
                    if packs.flow(n).is_some() {
                        return error(format!(
                            "there is a flow `{n}` already: ask the user to select it, then change \
                             it with {TOOL_UPDATE}, or choose another name"
                        ));
                    }
                }
                // Everything is checked — the skills, the text with the skills it brings — before
                // anything is written.
                let mut problems: Vec<Value> = skill_problems(&skills)
                    .into_iter()
                    .map(|m| json!({"message": m}))
                    .collect();
                if let Some(y) = yaml {
                    let with = WithSkills {
                        base: packs.as_ref(),
                        skills: &skills,
                    };
                    problems.extend(
                        validate_yaml(y, &with)
                            .into_iter()
                            .filter_map(|p| serde_json::to_value(p).ok()),
                    );
                }
                if !problems.is_empty() {
                    return text(json!({"saved": false, "problems": problems}));
                }
                // Then written; if the flow is not saved after all, the skills this call wrote
                // are put back as they were.
                let before: Vec<(String, Option<String>)> = skills
                    .keys()
                    .map(|k| (k.clone(), packs.skill(k).map(|(b, _)| b)))
                    .collect();
                let mut saved_skills = vec![];
                let mut failed = None;
                for (k, body) in &skills {
                    match engine.save_skill(k, body, by.clone()).await {
                        Ok(_) => saved_skills.push(k.clone()),
                        Err(e) => {
                            failed = Some(format!("skill {k} not saved: {e}"));
                            break;
                        }
                    }
                }
                let mut out = json!({"saved": true, "skills": saved_skills});
                if let (None, Some(y)) = (&failed, yaml) {
                    match engine.save_flow(y, by.clone()).await {
                        Ok(v) => out["flow"] = v,
                        Err(e) => failed = Some(format!("not saved: {e}")),
                    }
                }
                if let Some(m) = failed {
                    restore_skills(packs.as_ref(), &before, &saved_skills);
                    dispatch::publish_defs(sh).await;
                    return error(format!("{m} (nothing this call wrote was kept)"));
                }
                dispatch::publish_defs(sh).await;
                if let (true, Some(n)) = (creating, name) {
                    sh.set_focus(&instance, &session, Some(n.clone()));
                    dispatch::publish_focus(sh, &instance, &session).await;
                    out["current"] = json!(n);
                }
                text(out)
            }
            TOOL_DELETE => {
                let name = match target() {
                    Ok(n) => n,
                    Err(m) => return error(m),
                };
                match engine.delete_flow(&name, by).await {
                    Ok(v) => {
                        dispatch::publish_defs(sh).await;
                        for (i, s) in sh.focused_on(&name) {
                            if packs.flow(&name).is_none() {
                                sh.set_focus(&i, &s, None);
                            }
                            dispatch::publish_focus(sh, &i, &s).await;
                        }
                        text(v)
                    }
                    Err(e) => error(format!("not deleted: {e}")),
                }
            }
            TOOL_SKILL_SET => {
                let (Some(name), Some(body)) = (str_of(&input, "name"), str_of(&input, "body"))
                else {
                    return error("give the skill's `name` and `body`");
                };
                match engine.save_skill(name, body, by).await {
                    Ok(v) => {
                        dispatch::publish_defs(sh).await;
                        text(v)
                    }
                    Err(e) => error(format!("not saved: {e}")),
                }
            }
            TOOL_SKILL_DELETE => {
                let Some(name) = str_of(&input, "name") else {
                    return error("give the skill's `name`");
                };
                match engine.delete_skill(name, by).await {
                    Ok(v) => {
                        dispatch::publish_defs(sh).await;
                        text(v)
                    }
                    Err(e) => error(format!("not deleted: {e}")),
                }
            }
            TOOL_DRY_RUN => {
                let name = match target() {
                    Ok(n) => n,
                    Err(m) => return error(m),
                };
                let Ok(sandbox) = sh.sandbox() else {
                    return error("the flow module has not started");
                };
                let start = StartIn {
                    flow: Some(name),
                    inputs: input.get("inputs").cloned().unwrap_or(Value::Null),
                    mocks: match input.get("mocks") {
                        None | Some(Value::Null) => Default::default(),
                        Some(m) => match serde_json::from_value(m.clone()) {
                            Ok(m) => m,
                            Err(e) => return error(format!("`mocks`: {e}")),
                        },
                    },
                    ..Default::default()
                };
                let run = match sandbox.start(start, by).await {
                    Ok(r) => r,
                    Err(e) => return error(format!("the dry run did not start: {e}")),
                };
                let progress = sandbox.run(&run).await;
                dispatch::publish_dry(sh, Some(&run)).await;
                let progress = match progress {
                    Ok(p) => p,
                    Err(e) => return error(e.to_string()),
                };
                let view = sandbox.get(&run).await.unwrap_or(Value::Null);
                let stations: BTreeMap<String, Value> = view
                    .pointer("/record/stations")
                    .and_then(Value::as_object)
                    .map(|m| {
                        m.iter()
                            .map(|(k, v)| (k.clone(), v["status"].clone()))
                            .collect()
                    })
                    .unwrap_or_default();
                text(json!({
                    "run": run, "status": progress.status, "steps": progress.steps,
                    "stations": stations, "waiting": view.get("unmet"),
                }))
            }
            TOOL_START => {
                let name = match target() {
                    Ok(n) => n,
                    Err(m) => return error(m),
                };
                let Some(subject) = str_of(&input, "subject") else {
                    return error("give the run's `subject`");
                };
                let r = engine
                    .create(
                        CreateIn {
                            flow: name,
                            subject: subject.to_string(),
                            inputs: input.get("inputs").cloned().unwrap_or(Value::Null),
                            title: str_of(&input, "title").map(String::from),
                            parent: None,
                        },
                        by,
                    )
                    .await;
                match r {
                    Ok(o) => {
                        let value = o.value.clone();
                        dispatch::apply(sh, None, o).await;
                        text(value)
                    }
                    Err(e) => error(format!("not started: {e}")),
                }
            }
            other => error(format!("no tool {other}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tools_that_change_nothing_are_read_only_and_the_rest_are_not() {
        let read_only: Vec<&str> = AUTHOR_TOOLS
            .iter()
            .filter(|(_, r)| *r)
            .map(|(n, _)| *n)
            .collect();
        assert_eq!(
            read_only,
            [
                TOOL_LIST,
                TOOL_SHOW,
                TOOL_SCHEMA,
                TOOL_VALIDATE,
                TOOL_DRY_RUN
            ]
        );
        let writes: Vec<&str> = AUTHOR_TOOLS
            .iter()
            .filter(|(_, r)| !*r)
            .map(|(n, _)| *n)
            .collect();
        assert_eq!(
            writes,
            [
                TOOL_CREATE,
                TOOL_UPDATE,
                TOOL_DELETE,
                TOOL_SKILL_SET,
                TOOL_SKILL_DELETE,
                TOOL_START
            ]
        );
        // Nothing that decides for a person.
        for (n, _) in AUTHOR_TOOLS {
            for verb in ["confirm", "reject", "force", "rework", "release", "cancel"] {
                assert!(!n.contains(verb), "{n}");
            }
        }
    }

    #[test]
    fn a_change_acts_only_on_the_flow_it_names_while_that_is_current() {
        let named = json!({"flow": "a"});
        assert_eq!(target_flow(&named, Some("a")).unwrap(), "a");
        let changed = target_flow(&named, Some("b")).unwrap_err();
        assert!(
            changed.contains("changed to `b`") && changed.contains("ask the user again"),
            "{changed}"
        );
        assert_eq!(target_flow(&named, None).unwrap_err(), NO_FOCUS);
        assert!(target_flow(&json!({}), Some("a"))
            .unwrap_err()
            .contains("`flow`"));
    }

    #[test]
    fn skills_written_before_a_failed_save_are_put_back() {
        use crate::packs::MemPacks;
        let packs = MemPacks::new().skill("old", "Built in.").writable();
        packs.save_skill("mine", "Mine before.").unwrap();
        let before = vec![
            ("old".to_string(), Some("Built in.".to_string())),
            ("mine".to_string(), Some("Mine before.".to_string())),
            ("new".to_string(), None),
        ];
        for (k, b) in [("old", "Changed."), ("mine", "Changed."), ("new", "Fresh.")] {
            packs.save_skill(k, b).unwrap();
        }
        let saved = ["old", "mine", "new"].map(String::from).to_vec();
        restore_skills(&packs, &before, &saved);
        assert_eq!(Packs::skill(&packs, "old").unwrap().0, "Built in.");
        assert_eq!(Packs::skill(&packs, "mine").unwrap().0, "Mine before.");
        assert!(Packs::skill(&packs, "new").is_none());
        assert_eq!(
            skill_problems(&BTreeMap::from([("Bad".to_string(), " ".to_string())])).len(),
            2
        );
    }

    #[test]
    fn skills_come_as_a_table_of_bodies() {
        assert_eq!(skills_of(&json!({})).unwrap().len(), 0);
        assert_eq!(
            skills_of(&json!({"skills": {"a": "A."}})).unwrap()["a"],
            "A."
        );
        assert!(skills_of(&json!({"skills": {"a": 1}})).is_err());
        assert!(skills_of(&json!({"skills": []})).is_err());
    }
}
