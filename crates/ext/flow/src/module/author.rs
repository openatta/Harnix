//! Writing flows in conversation (xops-02 §4): the `flow-author` scene and its tools.
//!
//! The flow space's default scene. It wraps a scene the application gives (Agix's harnix by
//! default) and adds the tools below and a system prompt block on how to write flows. A tool
//! that needs **the current flow** reads it from the conversation's focus: what the person has
//! selected in the panel beside it (device method `focus`), or the flow `flow_create` just made.
//!
//! | tool | read-only | does |
//! |---|---|---|
//! | `flow_list` | yes | the flows: name, title, origin, valid |
//! | `flow_show` | yes | the current flow: YAML, the parts it uses, the hooks over it, problems, its stations |
//! | `flow_schema` | yes | the JSON Schemas of a definition and of the parts, and this application's catalog |
//! | `flow_validate {yaml}` | yes | a text's problems, nothing saved |
//! | `flow_create {yaml, parts?}` | no | a new flow, which becomes the current one |
//! | `flow_update {yaml?, parts?}` | no | change the current flow |
//! | `flow_delete` | no | delete the current flow (a built-in one: its override) |
//! | `flow_part_set {kind, name, text}` / `flow_part_delete {kind, name}` | no | change, delete a skill, rule, policy, gate or hook |
//! | `flow_dry_run {inputs?, mocks?}` | yes | dry-run the current flow, as far as it goes |
//! | `flow_start {subject, inputs?, title?}` | no | start the current flow for real |
//!
//! No tool asks the person (docs/system-space.md §5): a change is checked, not approved — a
//! definition with problems is not saved. A run tool acts only when the user asks for it, and the
//! run's history records the conversation as who did it.

use std::collections::BTreeMap;
use std::sync::Arc;

use harnix_ext::engine::{
    Permission, PromptText, SceneBase, SceneLayer, Tool, ToolCx, ToolError, ToolOutput, ToolSpec as Spec,
};
use serde_json::{json, Value};

use super::publish;
use super::state::Shared;
use crate::dryrun::StartIn;
use crate::kernel::def::{definition_schema, FlowDef, Work};
use crate::kernel::engine::CreateIn;
use crate::kernel::host::{Catalog, Packs};
use crate::kernel::parts::{part_schema, rules_of, PartKind};
use crate::kernel::run::Actor;
use crate::kernel::validate::{part_problems, validate_yaml};

/// The scene's id.
pub const AUTHOR_SCENE: &str = "flow-author";

/// What a tool needing the current flow answers when there is none.
pub const NO_FOCUS: &str = "This conversation works on no flow: ask the user to select the flow \
     in the panel on the right first — its own conversation opens (还没有选择流程：请用户先在右边栏\
     选择要操作的流程，会打开这个流程自己的会话). Creating a new flow with `flow_create` needs no \
     selection.";

pub const TOOL_LIST: &str = "flow_list";
pub const TOOL_SHOW: &str = "flow_show";
pub const TOOL_SCHEMA: &str = "flow_schema";
pub const TOOL_VALIDATE: &str = "flow_validate";
pub const TOOL_CREATE: &str = "flow_create";
pub const TOOL_UPDATE: &str = "flow_update";
pub const TOOL_DELETE: &str = "flow_delete";
pub const TOOL_PART_SET: &str = "flow_part_set";
pub const TOOL_PART_DELETE: &str = "flow_part_delete";
pub const TOOL_DRY_RUN: &str = "flow_dry_run";
pub const TOOL_START: &str = "flow_start";
pub const TOOL_GUIDE: &str = "flow_guide";
pub const TOOL_RUNS: &str = "flow_runs";
pub const TOOL_RUN: &str = "flow_run";
pub const TOOL_PAUSE: &str = "flow_pause";
pub const TOOL_RESUME: &str = "flow_resume";
pub const TOOL_CANCEL: &str = "flow_cancel";
pub const TOOL_CONFIRM: &str = "flow_confirm";
pub const TOOL_FORCE: &str = "flow_force";
pub const TOOL_REWORK: &str = "flow_rework";
pub const TOOL_RETRY: &str = "flow_retry";
pub const TOOL_RELEASE: &str = "flow_release";

/// The flow guide (xops-03 §6): what `flow_guide` gives and the reference tab shows.
pub const GUIDE_EN: &str = harnix_flow::GUIDE_EN;
pub const GUIDE_ZH: &str = harnix_flow::GUIDE_ZH;

/// Who acted when a run tool acts in a conversation: the conversation, `<instance>/<session>`.
pub const CONVERSATION_KIND: &str = "conversation";

/// Every authoring tool, and whether it is read-only: [`TOOLS`], as names.
pub const AUTHOR_TOOLS: [(&str, bool); TOOLS.len()] = {
    let mut out = [("", false); TOOLS.len()];
    let mut i = 0;
    while i < TOOLS.len() {
        out[i] = (TOOLS[i].name, TOOLS[i].read_only);
        i += 1;
    }
    out
};

/// One authoring tool, everything about it but its code. [`TOOLS`] is the one place a tool is
/// listed: its name, what the model is told of it, its input, whether it changes anything, and
/// which arm of [`AuthorTool::call`] runs it.
pub(crate) struct ToolSpec {
    pub name: &'static str,
    pub description: &'static str,
    /// Its input's JSON Schema.
    pub schema: fn() -> Value,
    /// It changes nothing: it may run beside other calls.
    pub read_only: bool,
    does: Does,
}

/// Which arm of [`AuthorTool::call`] a tool is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Does {
    List,
    Show,
    Schema,
    Validate,
    Save { creating: bool },
    Delete,
    PartSet,
    PartDelete,
    DryRun,
    Start,
    Guide,
    Runs,
    Run,
    /// A run tool: it acts on a run of the current flow (xops-03 §5).
    Act(Act),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Act {
    Pause,
    Resume,
    Cancel,
    Confirm,
    Force,
    Rework,
    Retry,
    Release,
}

fn flow_named() -> Value {
    json!({"type": "string", "description": "the flow this acts on, by name: the current one, as flow_show or flow_list gave it"})
}

fn parts_given() -> Value {
    json!({"type": "array", "description": "parts saved with the flow", "items": {
        "type": "object", "required": ["kind", "name", "text"], "properties": {
            "kind": {"enum": ["skill", "rule", "policy", "gate", "hook"]},
            "name": {"type": "string"}, "text": {"type": "string"}}}})
}

fn no_input() -> Value {
    json!({"type": "object", "properties": {}})
}

fn run_named() -> Value {
    json!({"type": "object", "required": ["run"], "properties": {"run": {"type": "string"}}})
}

/// Every authoring tool, in the order the model is offered them.
pub(crate) const TOOLS: &[ToolSpec] = &[
    ToolSpec {
        name: TOOL_LIST,
        description: "The flows on offer: name, title, origin (builtin, override, user, project) and whether each is valid; which one is current; and the project the conversation works in, if any.",
        schema: no_input,
        read_only: true,
        does: Does::List,
    },
    ToolSpec {
        name: TOOL_SHOW,
        description: "The current flow (the one the user selected): its YAML, the texts of the parts it uses (skills, rules, policies, gates), the hooks over it, its problems, and its stations in brief.",
        schema: no_input,
        read_only: true,
        does: Does::Show,
    },
    ToolSpec {
        name: TOOL_SCHEMA,
        description: "The JSON Schemas of a flow definition and of policies, gates and hooks, and the names this application offers (scenes, steps, checks, tools, templates, instances, spaces, models, guarded tools, and the parts on offer by kind).",
        schema: no_input,
        read_only: true,
        does: Does::Schema,
    },
    ToolSpec {
        name: TOOL_VALIDATE,
        description: "Every problem of a flow definition text (YAML), without saving anything. An empty list: it is valid.",
        schema: || json!({"type": "object", "required": ["yaml"], "properties": {"yaml": {"type": "string"}}}),
        read_only: true,
        does: Does::Validate,
    },
    ToolSpec {
        name: TOOL_CREATE,
        description: "Create a new flow from its YAML (and, optionally, parts it uses: [{kind: skill|rule|policy|gate, name, text}]). Everything is validated first and nothing is saved if anything has problems. It becomes the current flow. With `project_root`, it is saved in that project (its `.harnix/flows/`, committed with it; the project must be trusted) and the conversation works in that project from then on.",
        schema: || json!({"type": "object", "required": ["yaml"], "properties": {"yaml": {"type": "string"}, "parts": parts_given(), "project_root": {"type": "string", "description": "the project to save it in, when the user asks for that: its absolute directory"}}}),
        read_only: false,
        does: Does::Save { creating: true },
    },
    ToolSpec {
        name: TOOL_UPDATE,
        description: "Change the current flow, named in `flow`: its YAML (same name), and/or parts ([{kind, name, text}]). Validated first; nothing is saved if anything has problems. Runs already started keep their definition.",
        schema: || json!({"type": "object", "required": ["flow"], "properties": {"flow": flow_named(), "yaml": {"type": "string"}, "parts": parts_given()}}),
        read_only: false,
        does: Does::Save { creating: false },
    },
    ToolSpec {
        name: TOOL_DELETE,
        description: "Delete the current flow, named in `flow`. For a built-in flow, this deletes the saved override and the built-in version is back.",
        schema: || json!({"type": "object", "required": ["flow"], "properties": {"flow": flow_named()}}),
        read_only: false,
        does: Does::Delete,
    },
    ToolSpec {
        name: TOOL_PART_SET,
        description: "Create or change a part of the pack: a skill (its SKILL.md), a rule (markdown, first `# ` heading its title), a policy, a gate or a hook (YAML; its `name` the same). Checked first; the flows it breaks are listed.",
        schema: || json!({"type": "object", "required": ["kind", "name", "text"], "properties": {"kind": {"enum": ["skill", "rule", "policy", "gate", "hook"]}, "name": {"type": "string"}, "text": {"type": "string"}}}),
        read_only: false,
        does: Does::PartSet,
    },
    ToolSpec {
        name: TOOL_PART_DELETE,
        description: "Delete a saved part (a built-in one: its override, and the built-in one is back).",
        schema: || json!({"type": "object", "required": ["kind", "name"], "properties": {"kind": {"enum": ["skill", "rule", "policy", "gate", "hook"]}, "name": {"type": "string"}}}),
        read_only: false,
        does: Does::PartDelete,
    },
    ToolSpec {
        name: TOOL_DRY_RUN,
        description: "Dry-run the current flow, named in `flow`: the real kernel, everything outside mocked (no session, no program, no model, no command). Runs until it ends or waits for a person, and returns the path it took. `mocks`: {stations: {id: output}, gates: {gate: false} (command and host gates), tools: {tool: result} (what tool gates get)}.",
        schema: || json!({"type": "object", "required": ["flow"], "properties": {
            "flow": flow_named(),
            "inputs": {"type": "object"},
            "mocks": {"type": "object", "properties": {
                "stations": {"type": "object"}, "gates": {"type": "object"}, "tools": {"type": "object"}}}}}),
        read_only: true,
        does: Does::DryRun,
    },
    ToolSpec {
        name: TOOL_START,
        description: "Start a real run of the current flow, named in `flow`, for a subject. Only when the user asks for it.",
        schema: || json!({"type": "object", "required": ["flow", "subject"], "properties": {
            "flow": flow_named(),
            "subject": {"type": "string"}, "inputs": {"type": "object"}, "title": {"type": "string"}}}),
        read_only: false,
        does: Does::Start,
    },
    ToolSpec {
        name: TOOL_GUIDE,
        description: "The guide to flows: the objects (flow, station, work, task, executor, exit condition, rule, policy and permission, gate, hook, run, main session, sub-agent, lease, rework, dry run), what they mean, and how to define a flow, with examples. `lang`: en or zh-CN.",
        schema: || json!({"type": "object", "properties": {"lang": {"enum": ["en", "zh-CN"]}}}),
        read_only: true,
        does: Does::Guide,
    },
    ToolSpec {
        name: TOOL_RUNS,
        description: "The runs of the current flow, newest first: id, title, what each is doing (phase), when it started, its tokens.",
        schema: no_input,
        read_only: true,
        does: Does::Runs,
    },
    ToolSpec {
        name: TOOL_RUN,
        description: "One run of the current flow: each station's status, time and tokens, what it waits for, its tasks.",
        schema: run_named,
        read_only: true,
        does: Does::Run,
    },
    ToolSpec {
        name: TOOL_PAUSE,
        description: "Pause a run of the current flow: what is in hand finishes, nothing new starts. Only when the user asks.",
        schema: run_named,
        read_only: false,
        does: Does::Act(Act::Pause),
    },
    ToolSpec {
        name: TOOL_RESUME,
        description: "Resume a paused run of the current flow. Only when the user asks.",
        schema: run_named,
        read_only: false,
        does: Does::Act(Act::Resume),
    },
    ToolSpec {
        name: TOOL_CANCEL,
        description: "Cancel a run of the current flow, with a reason. Only when the user asks.",
        schema: || json!({"type": "object", "required": ["run", "reason"], "properties": {"run": {"type": "string"}, "reason": {"type": "string"}}}),
        read_only: false,
        does: Does::Act(Act::Cancel),
    },
    ToolSpec {
        name: TOOL_CONFIRM,
        description: "Confirm a station of a run of the current flow (`pass`: true passes, false rejects), with an optional reason. Only when the user asks for it.",
        schema: || json!({"type": "object", "required": ["run", "station", "pass"], "properties": {"run": {"type": "string"}, "station": {"type": "string"}, "pass": {"type": "boolean"}, "reason": {"type": "string"}}}),
        read_only: false,
        does: Does::Act(Act::Confirm),
    },
    ToolSpec {
        name: TOOL_FORCE,
        description: "Pass a station of a run by force, with a reason, whatever is not met. Only when the user asks.",
        schema: || json!({"type": "object", "required": ["run", "station", "reason"], "properties": {"run": {"type": "string"}, "station": {"type": "string"}, "reason": {"type": "string"}}}),
        read_only: false,
        does: Does::Act(Act::Force),
    },
    ToolSpec {
        name: TOOL_REWORK,
        description: "Send a run of the current flow back to station `to` (optionally only `tasks`), with a reason. Only when the user asks.",
        schema: || json!({"type": "object", "required": ["run", "to", "reason"], "properties": {"run": {"type": "string"}, "to": {"type": "string"}, "tasks": {"type": "array", "items": {"type": "string"}}, "reason": {"type": "string"}}}),
        read_only: false,
        does: Does::Act(Act::Rework),
    },
    ToolSpec {
        name: TOOL_RETRY,
        description: "Put a failed task of a run back to work with a fresh retry budget. Only when the user asks.",
        schema: || json!({"type": "object", "required": ["run", "station", "task"], "properties": {"run": {"type": "string"}, "station": {"type": "string"}, "task": {"type": "string"}}}),
        read_only: false,
        does: Does::Act(Act::Retry),
    },
    ToolSpec {
        name: TOOL_RELEASE,
        description: "Release a held station of a run. Only when the user asks.",
        schema: || json!({"type": "object", "required": ["run", "station"], "properties": {"run": {"type": "string"}, "station": {"type": "string"}}}),
        read_only: false,
        does: Does::Act(Act::Release),
    },
];

/// The system prompt block `flow-author` adds.
pub const AUTHOR_PROMPT: &str = "## Writing flows\n\n\
You help the user write and try out **flows**: YAML definitions of a graph of stations that a run \
moves one subject through. Each station has work (`executor`: a `session` working from a skill, an \
`agent` (a sub-agent), a `program`, an `external` claimant, a `subflow`; or `tasks`, or `foreach` + \
`executor`; or none, a pure checkpoint) and exit conditions (`exit`: `output`, `gate`, `confirm`, \
`children`) that must all hold for it to pass; `needs` orders stations (absent = the previous one), \
`rework_to` says where work may be sent back. A session station needs a `skill` and an `output` \
contract (a JSON Schema): the session hands its output in against it.\n\n\
Besides flows and skills, a pack holds **rules** (`rules/<name>.md`: prose every station of a flow \
that names them is told to keep — never enforced), **policies** (`policies/<name>.yaml`: a set of \
permissions), **gates** (`gates/<name>.yaml`: a check a `gate` exit names — `tool`, `command` or \
`host`) and **hooks** (`hooks/<name>.yaml`: tell a URL about runs' events — no flow names them). \
Flows and stations name `rules` and `policies` and may write `permissions` of their own: each tool \
call of a task is judged against them, deny over ask over allow; `ask` waits for a person. Where \
none speaks, the flow's `guard` decides: `auto` (the default) lets a task work freely inside its \
project and refuses any path outside it (an `allow` whose `paths` name the place lets it through); \
`strict` refuses a guarded tool nothing allows and asks a person about a path outside. A task asks \
no questions: where a person must look, write a `confirm` station. A run's deliverables go to its \
`output_dir` (a template relative to the project — `${flow}`, `${date}`, `${seq}`, `${title}`, \
`${inputs.<k>}`; default `flows/${flow}-${date}${seq}`), made when it starts; skills, permissions \
and gates name it `${output_dir}`.\n\n\
How to work:\n\
- Read `flow_schema` before writing: the fields, the parts' shapes, and the names this application \
  offers (scenes, program steps, checks, tools, templates, and the parts on offer). Use only those.\n\
- Before changing a flow, read it with `flow_show`. The flow you work on is the one the user \
  selected in the panel on the right; if none is selected, ask them to select one (creating a \
  new flow with `flow_create` needs no selection, and selects it).\n\
- Check every text with `flow_validate` before saving it with `flow_create` or `flow_update`; a \
  text with problems is not saved.\n\
- Try the path with `flow_dry_run` (nothing outside the flow is called; outputs, gates and tools \
  are mocked, `mocks` can set them). Start a real run with `flow_start` only when the user asks.\n\
- Each flow has a conversation of its own (this one, when one is current); the general \
  conversation creates flows and answers questions. `flow_guide` is the guide to flows: what each \
  object means and how to define one — answer questions about flows from it.\n\
- A task without a `scene` works in the run's **main session** (the flow's main Agent, placed by \
  the flow's `main`); `kind: agent` hands a task to a sub-agent of it; a `scene` gives a task a \
  session of its own.\n\
- Saving, deleting and starting take effect at once, without asking the user: validate first, \
  and do only what the user asked for.\n\
- A flow for one project is saved in it (`flow_create` with `project_root`): it is committed \
  with the project and offered only there. A conversation in a project saves everything there \
  and starts its runs as tasks of that project. Save with the flow tools only, never by writing \
  files into the project's `.harnix/` yourself: that directory is the application's, and a task's \
  tools are kept out of it too (only a permission whose `paths` point into it opens it).\n\
- The runs of the current flow: `flow_runs`, `flow_run`. When the user asks, you may pause, resume, \
  cancel, confirm (pass or reject), force, rework, retry or release with the run tools. Never do \
  one the user did not ask for.";

/// An application's scene with the authoring tools and prompt added, served as `flow-author`.
///
/// Attended whatever the base scene says: the module answers this session's permission asks (from
/// its task's permissions, or by asking a person in the flow space), so they must reach the host
/// rather than be refused as no one's to answer. No model call outside the conversation (under
/// replay, one that is not recorded), and no project to choose.
pub(crate) fn author_scene(base: SceneBase, sh: Arc<Shared>) -> SceneLayer {
    let mut layer = SceneLayer::new(AUTHOR_SCENE, base)
        .name("Flow author")
        .description("write, check and try out flows in conversation")
        .prompt(PromptText::cached(AUTHOR_PROMPT));
    for row in TOOLS {
        layer = layer.tool(Arc::new(AuthorTool::new(row, sh.clone())));
    }
    layer
        .requires_project(false)
        .interactive(true)
        .auto_name(false)
}

/// Parts about to be saved: (kind, name) → text.
type PartTexts = BTreeMap<(PartKind, String), String>;

/// The packs as they will be once `parts` are saved: what a text is checked against first.
struct WithParts<'a> {
    base: &'a dyn Packs,
    parts: &'a PartTexts,
}

impl Packs for WithParts<'_> {
    fn flows(&self) -> Vec<String> {
        self.base.flows()
    }
    fn flow(&self, name: &str) -> Option<(FlowDef, String)> {
        self.base.flow(name)
    }
    fn part(&self, kind: PartKind, name: &str) -> Option<(String, String)> {
        match self.parts.get(&(kind, name.to_string())) {
            Some(b) => Some((b.clone(), crate::kernel::host::digest(b))),
            None => self.base.part(kind, name),
        }
    }
    fn parts(&self, kind: PartKind) -> Vec<String> {
        let mut names = self.base.parts(kind);
        for (k, n) in self.parts.keys() {
            if *k == kind && !names.contains(n) {
                names.push(n.clone());
            }
        }
        names
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

/// Put back parts this call saved: each as its text was before, or gone if it was not there.
fn restore_parts(packs: &dyn Packs, before: &[((PartKind, String), Option<String>)], saved: &[(PartKind, String)]) {
    for ((kind, name), old) in before.iter().filter(|(k, _)| saved.contains(k)) {
        let _ = packs.delete_part(*kind, name);
        let now = packs.part(*kind, name).map(|(b, _)| b);
        if now != *old {
            if let Some(text) = old {
                let _ = packs.save_part(*kind, name, text);
            }
        }
    }
}

type ToolOut = Result<ToolOutput, ToolError>;

/// One authoring tool: its row of [`TOOLS`].
struct AuthorTool {
    spec: &'static ToolSpec,
    /// What the model is told of it: the row's name, description and input; what changes nothing
    /// is read-only and may run beside other calls.
    told: Spec,
    sh: Arc<Shared>,
}

impl AuthorTool {
    fn new(spec: &'static ToolSpec, sh: Arc<Shared>) -> Self {
        let mut told = Spec::new(spec.name, spec.description, (spec.schema)());
        if spec.read_only {
            told = told.read_only().concurrency_safe();
        }
        Self { spec, told, sh }
    }
}

fn pretty(v: &Value) -> String {
    serde_json::to_string_pretty(v).unwrap_or_else(|_| v.to_string())
}

fn text(v: Value) -> ToolOut {
    Ok(ToolOutput::text(pretty(&v)))
}

fn error(m: impl Into<String>) -> ToolOut {
    Ok(ToolOutput::error(m.into()))
}

fn str_of<'v>(input: &'v Value, k: &str) -> Option<&'v str> {
    input
        .get(k)
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
}

/// `parts: [{kind, name, text}]` from a tool's input.
fn parts_of(input: &Value) -> Result<PartTexts, String> {
    match input.get("parts") {
        None | Some(Value::Null) => Ok(BTreeMap::new()),
        Some(Value::Array(a)) => a
            .iter()
            .map(|p| {
                let kind = p
                    .get("kind")
                    .and_then(Value::as_str)
                    .and_then(PartKind::parse)
                    .ok_or("each part has a `kind`: skill, rule, policy, gate or hook")?;
                let name = p.get("name").and_then(Value::as_str).ok_or("each part has a `name`")?;
                let text = p.get("text").and_then(Value::as_str).ok_or("each part has its `text`")?;
                Ok(((kind, name.to_string()), text.to_string()))
            })
            .collect(),
        Some(_) => Err("`parts` is a list of {kind, name, text}".into()),
    }
}

/// The names this application offers and the parts on offer, by kind.
pub(crate) fn catalog_json(packs: &dyn Packs) -> Value {
    let c: Catalog = packs.catalog();
    let parts: BTreeMap<&str, Vec<String>> = PartKind::ALL
        .into_iter()
        .map(|k| (k.as_str(), packs.parts(k)))
        .collect();
    json!({
        "scenes": c.scenes, "steps": c.steps, "checks": c.gates, "tools": c.tools,
        "command_gates": c.command_gates, "templates": c.templates, "instances": c.instances,
        "spaces": c.spaces, "models": c.models, "guarded": c.guarded, "parts": parts,
    })
}

/// The JSON Schemas of the parts that are YAML.
pub(crate) fn part_schemas() -> Value {
    let m: serde_json::Map<String, Value> = [PartKind::Policy, PartKind::Gate, PartKind::Hook]
        .into_iter()
        .filter_map(|k| part_schema(k).map(|s| (k.as_str().to_string(), s)))
        .collect();
    Value::Object(m)
}

/// The parts a flow uses, with their texts (`null`: not in the packs), and the hooks over it.
pub(crate) fn parts_used(packs: &dyn Packs, def: &FlowDef) -> Value {
    let mut used: BTreeMap<&str, BTreeMap<String, Option<String>>> = BTreeMap::new();
    let mut put = |kind: PartKind, name: &str| {
        used.entry(kind.dir())
            .or_default()
            .insert(name.to_string(), packs.part(kind, name).map(|(t, _)| t));
    };
    for s in &def.stations {
        for t in s.task_specs() {
            if let Some(k) = t.skill {
                put(PartKind::Skill, &k);
            }
        }
        for r in rules_of(def, s) {
            put(PartKind::Rule, &r);
        }
        for p in def.policies.iter().chain(s.policies.iter()) {
            put(PartKind::Policy, p);
        }
        for e in &s.exit {
            if let crate::ExitCond::Gate { gate, .. } = e {
                put(PartKind::Gate, gate);
            }
        }
    }
    let hooks: Vec<Value> = packs
        .hooks()
        .into_iter()
        .filter(|h| h.flows.is_empty() || h.flows.contains(&def.name))
        .map(|h| json!({"name": h.name, "title": h.title, "on": h.on, "kind": h.kind, "action": crate::runner::redact_url(&h.action)}))
        .collect();
    json!({"parts": used, "hooks": hooks})
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
    fn spec(&self) -> &Spec {
        &self.told
    }
    /// Never asked about: what changes something is checked as it runs.
    fn permission(&self, _: &Value) -> Permission {
        Permission::Allow
    }
    async fn call(&self, cx: ToolCx, input: Value) -> ToolOut {
        let sh = &self.sh;
        let Ok(engine) = sh.engine() else {
            return error("the flow module has not started");
        };
        // The session the call is made in, on the instance its engine carries (the flow space's
        // when an engine does not say).
        let me = cx.session();
        let instance = if me.instance.is_empty() { sh.cfg.instance.clone() } else { me.instance };
        let session = me.session_id;
        let by = Actor::agent(format!("{instance}/{session}"));
        // A conversation in a project reads and saves the project's packs
        // (`<project>/.harnix/flows/`); `flow_create` with a project moves it into that one.
        let asked = match (self.spec.does, str_of(&input, "project_root")) {
            (Does::Save { creating: true }, Some(root)) => match super::state::project_root(root) {
                Ok(root) => Some(root),
                Err(m) => return error(m),
            },
            _ => None,
        };
        let project = asked.clone().or_else(|| sh.conversation_project(&instance, &session));
        let packs = match &project {
            Some(root) => {
                if !sh.hear(root).await
                    && matches!(self.spec.does, Does::Save { .. } | Does::Delete | Does::PartSet | Does::PartDelete)
                {
                    return error(format!(
                        "the project {root} is not trusted: nothing is saved in it until the user trusts it (in the projects space)"
                    ));
                }
                engine.packs_for(Some(root))
            }
            None => engine.packs().clone(),
        };
        if asked.is_some() {
            sh.set_conversation_project(&instance, &session, asked);
        }
        let focus = sh
            .focus(&instance, &session)
            .filter(|f| packs.flow(f).is_some());
        let need_focus = || focus.clone().ok_or(());
        // What changes or runs a flow names it, and acts only if it is still the current one:
        // the call means that flow, not whatever is selected when it runs.
        let target = || target_flow(&input, focus.as_deref());
        match self.spec.does {
            Does::List => text(json!({
                // What the conversation needs of each: its inputs are in the definition `flow_show` gives.
                "flows": publish::defs_list(packs.as_ref()).into_iter().map(|d| json!({
                    "name": d.name, "title": d.title, "digest": d.digest,
                    "origin": d.origin, "valid": d.valid,
                })).collect::<Vec<_>>(),
                "current": focus,
                "writable": packs.writable(),
                "project": project,
            })),
            Does::Schema => text(json!({
                "definition": definition_schema(),
                "parts": part_schemas(),
                "catalog": catalog_json(packs.as_ref()),
            })),
            Does::Validate => {
                let Some(yaml) = str_of(&input, "yaml") else {
                    return error("give the definition as `yaml`");
                };
                text(json!({"problems": validate_yaml(yaml, packs.as_ref())}))
            }
            Does::Show => {
                let Ok(name) = need_focus() else {
                    return error(NO_FOCUS);
                };
                let Some((def, digest)) = packs.flow(&name) else {
                    return error(format!("flow {name} is gone"));
                };
                let used = parts_used(packs.as_ref(), &def);
                text(json!({
                    "name": name, "title": def.title, "origin": packs.origin(&name), "digest": digest,
                    "yaml": packs.flow_text(&name).unwrap_or_default(),
                    "problems": crate::kernel::validate::validate(&def, packs.as_ref()),
                    "stations": stations_of(&def),
                    "parts": used["parts"], "hooks": used["hooks"],
                }))
            }
            Does::Save { creating } => {
                let current = match (creating, target()) {
                    (true, _) => None,
                    (false, Ok(f)) => Some(f),
                    (false, Err(m)) => return error(m),
                };
                let parts = match parts_of(&input) {
                    Ok(s) => s,
                    Err(m) => return error(m),
                };
                let yaml = str_of(&input, "yaml");
                if creating && yaml.is_none() {
                    return error("give the new flow's definition as `yaml`");
                }
                if yaml.is_none() && parts.is_empty() {
                    return error("give `yaml`, `parts`, or both");
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
                // Everything is checked — the parts, the text with the parts it brings — before
                // anything is written.
                let with = WithParts {
                    base: packs.as_ref(),
                    parts: &parts,
                };
                let mut problems: Vec<Value> = parts
                    .iter()
                    .flat_map(|((k, n), t)| {
                        part_problems(*k, n, t, &with)
                            .into_iter()
                            .map(move |m| json!({"message": format!("{k} {n}: {m}")}))
                    })
                    .collect();
                if let Some(y) = yaml {
                    problems.extend(
                        validate_yaml(y, &with)
                            .into_iter()
                            .filter_map(|p| serde_json::to_value(p).ok()),
                    );
                }
                if !problems.is_empty() {
                    return text(json!({"saved": false, "problems": problems}));
                }
                // Then written; if the flow is not saved after all, the parts this call wrote
                // are put back as they were.
                let before: Vec<((PartKind, String), Option<String>)> = parts
                    .keys()
                    .map(|(k, n)| ((*k, n.clone()), packs.part(*k, n).map(|(b, _)| b)))
                    .collect();
                let mut saved_parts: Vec<(PartKind, String)> = vec![];
                let mut failed = None;
                for ((k, n), t) in &parts {
                    match engine.save_part_in(packs.as_ref(), *k, n, t, by.clone()).await {
                        Ok(_) => saved_parts.push((*k, n.clone())),
                        Err(e) => {
                            failed = Some(format!("{k} {n} not saved: {e}"));
                            break;
                        }
                    }
                }
                let mut out = json!({
                    "saved": true,
                    "parts": saved_parts.iter().map(|(k, n)| json!({"kind": k, "name": n})).collect::<Vec<_>>(),
                });
                if let (None, Some(y)) = (&failed, yaml) {
                    match engine.save_flow_in(packs.as_ref(), y, by.clone()).await {
                        Ok(v) => out["flow"] = v,
                        Err(e) => failed = Some(format!("not saved: {e}")),
                    }
                }
                if let Some(m) = failed {
                    restore_parts(packs.as_ref(), &before, &saved_parts);
                    publish::publish_defs(sh).await;
                    return error(format!("{m} (nothing this call wrote was kept)"));
                }
                publish::publish_defs(sh).await;
                if let (true, Some(n)) = (creating, name) {
                    match sh.flow_of_session(&instance, &session) {
                        // The general conversation becomes the new flow's own; the next one
                        // asked for is a new general conversation (xops-03 §5).
                        Some(own) if own.is_empty() => {
                            sh.set_flow_session("", None);
                            sh.set_flow_session(&n, Some(format!("{instance}/{session}")));
                            out["current"] = json!(n);
                        }
                        // Another flow's conversation stays that flow's.
                        Some(own) => {
                            out["note"] = json!(format!(
                                "this is flow {own}'s conversation: the new flow has its own — \
                                 the user selects it on the right"
                            ));
                        }
                        None => {
                            sh.set_focus(&instance, &session, Some(n.clone()));
                            out["current"] = json!(n);
                        }
                    }
                    publish::publish_focus(sh, &instance, &session).await;
                }
                text(out)
            }
            Does::Delete => {
                let name = match target() {
                    Ok(n) => n,
                    Err(m) => return error(m),
                };
                match engine.delete_flow_in(packs.as_ref(), &name, by).await {
                    Ok(v) => {
                        publish::publish_defs(sh).await;
                        if packs.flow(&name).is_none() {
                            sh.set_flow_session(&name, None);
                        }
                        for (i, s) in sh.focused_on(&name) {
                            if packs.flow(&name).is_none() {
                                sh.set_focus(&i, &s, None);
                            }
                            publish::publish_focus(sh, &i, &s).await;
                        }
                        text(v)
                    }
                    Err(e) => error(format!("not deleted: {e}")),
                }
            }
            Does::PartSet => {
                let kind = str_of(&input, "kind").and_then(PartKind::parse);
                let (Some(kind), Some(name), Some(body)) = (kind, str_of(&input, "name"), str_of(&input, "text"))
                else {
                    return error("give the part's `kind` (skill, rule, policy, gate, hook), `name` and `text`");
                };
                match engine.save_part_in(packs.as_ref(), kind, name, body, by).await {
                    Ok(v) => {
                        publish::publish_defs(sh).await;
                        text(v)
                    }
                    Err(e) => error(format!("not saved: {e}")),
                }
            }
            Does::PartDelete => {
                let kind = str_of(&input, "kind").and_then(PartKind::parse);
                let (Some(kind), Some(name)) = (kind, str_of(&input, "name")) else {
                    return error("give the part's `kind` and `name`");
                };
                match engine.delete_part_in(packs.as_ref(), kind, name, by).await {
                    Ok(v) => {
                        publish::publish_defs(sh).await;
                        text(v)
                    }
                    Err(e) => error(format!("not deleted: {e}")),
                }
            }
            Does::DryRun => {
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
                publish::publish_dry(sh, Some(&run)).await;
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
            Does::Start => {
                let name = match target() {
                    Ok(n) => n,
                    Err(m) => return error(m),
                };
                let Some(subject) = str_of(&input, "subject") else {
                    return error("give the run's `subject`");
                };
                let start = CreateIn {
                    flow: name,
                    subject: subject.to_string(),
                    inputs: input.get("inputs").cloned().unwrap_or(Value::Null),
                    title: str_of(&input, "title").map(String::from),
                    parent: None,
                    project_root: project.clone(),
                    ..Default::default()
                };
                let Ok(rn) = sh.runner() else {
                    return error("the flow module has not started");
                };
                match rn.start_run(start, by).await {
                    Ok(o) => {
                        let value = o.value.clone();
                        rn.apply(o).await;
                        text(value)
                    }
                    Err(e) => error(format!("not started: {e}")),
                }
            }
            Does::Guide => {
                let zh = str_of(&input, "lang").is_some_and(|l| l.starts_with("zh"));
                Ok(ToolOutput::text(if zh { GUIDE_ZH } else { GUIDE_EN }.to_string()))
            }
            Does::Runs => {
                let Ok(name) = need_focus() else {
                    return error(NO_FOCUS);
                };
                match engine
                    .list(crate::RunQuery {
                        flow: Some(name),
                        project: project.clone(),
                        limit: Some(50),
                        ..Default::default()
                    })
                    .await
                {
                    Ok(list) => text(json!({"runs": list.iter().map(run_line).collect::<Vec<_>>()})),
                    Err(e) => error(e.to_string()),
                }
            }
            Does::Run => {
                let run = match run_of_focus(&engine, &input, focus.as_deref()).await {
                    Ok(r) => r,
                    Err(m) => return error(m),
                };
                text(run_detail(&run, engine.clock().now_ms()))
            }
            Does::Act(act) => {
                let rec = match run_of_focus(&engine, &input, focus.as_deref()).await {
                    Ok(r) => r,
                    Err(m) => return error(m),
                };
                let run = rec.id.clone();
                let who = Actor::new(CONVERSATION_KIND, format!("{instance}/{session}"));
                let station = str_of(&input, "station").unwrap_or_default().to_string();
                let reason = str_of(&input, "reason").map(String::from);
                let r = match act {
                    Act::Pause => engine.pause(&run, who).await,
                    Act::Resume => engine.resume(&run, who).await,
                    Act::Cancel => {
                        engine
                            .cancel(&run, reason.as_deref().unwrap_or("cancelled in conversation"), who)
                            .await
                    }
                    Act::Confirm => {
                        let pass = input.get("pass").and_then(Value::as_bool).unwrap_or(true);
                        engine.confirm(&run, &station, pass, reason.as_deref(), who).await
                    }
                    Act::Force => {
                        engine
                            .force(&run, &station, reason.as_deref().unwrap_or("forced in conversation"), who)
                            .await
                    }
                    Act::Rework => {
                        let to = str_of(&input, "to").unwrap_or_default();
                        let tasks: Option<Vec<String>> = input
                            .get("tasks")
                            .and_then(Value::as_array)
                            .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect());
                        engine
                            .rework(&run, to, tasks, reason.as_deref().unwrap_or("sent back in conversation"), who)
                            .await
                    }
                    Act::Retry => {
                        let task = str_of(&input, "task").unwrap_or_default();
                        engine.retry(&run, &station, task, who).await
                    }
                    Act::Release => engine.release(&run, &station, who).await,
                };
                match r {
                    Ok(o) => {
                        let value = o.value.clone();
                        match sh.runner() {
                            Ok(rn) => rn.apply_for(&run, o).await,
                            Err(_) => return error("the flow module has not started"),
                        }
                        let after = engine.record(&run).await.ok();
                        text(json!({"done": value, "phase": after.map(|r| r.phase())}))
                    }
                    Err(e) => error(format!("not done: {e}")),
                }
            }
        }
    }
}

/// The run a run tool names, if it is a run of the conversation's current flow.
async fn run_of_focus(
    engine: &crate::Engine,
    input: &Value,
    focus: Option<&str>,
) -> Result<crate::RunRecord, String> {
    let Some(flow) = focus else {
        return Err(NO_FOCUS.into());
    };
    let Some(run) = str_of(input, "run") else {
        return Err("name the run in `run` (flow_runs lists them)".into());
    };
    if crate::dryrun::is_dry(run) {
        return Err("a dry run is driven with flow_dry_run, not the run tools".into());
    }
    let rec = engine.record(run).await.map_err(|e| e.to_string())?;
    if rec.flow != flow {
        return Err(format!(
            "run {run} is a run of flow {}, not of {flow}, this conversation's flow: nothing was done",
            rec.flow
        ));
    }
    Ok(rec)
}

/// A run in a list, for the conversation.
fn run_line(s: &crate::RunSummary) -> Value {
    json!({
        "run": s.id, "title": s.title, "subject": s.subject,
        "phase": s.phase, "created_at": s.created_at, "finished_at": s.finished_at,
        "tokens": s.usage.total(),
    })
}

/// A run's stations, for the conversation: status, time, tokens, what each waits for.
fn run_detail(rec: &crate::RunRecord, now: u64) -> Value {
    let view = crate::view(rec.clone());
    let stations: Vec<Value> = rec
        .def
        .stations
        .iter()
        .filter_map(|d| {
            let s = rec.stations.get(&d.id)?;
            let elapsed = s.entered_at.map(|e| s.left_at.unwrap_or(now).saturating_sub(e));
            Some(json!({
                "station": d.id, "status": s.status, "round": s.round,
                "elapsed_ms": elapsed, "tokens": s.usage().total(),
                "waiting": view.unmet.get(&d.id),
                "tasks": s.tasks.iter().map(|t| json!({
                    "task": t.id, "status": t.status, "attempt": t.attempt,
                    "error": t.error, "tokens": t.usage.total(),
                })).collect::<Vec<_>>(),
            }))
        })
        .collect();
    json!({
        "run": rec.id, "flow": rec.flow, "subject": rec.subject, "phase": rec.phase(),
        "tokens": rec.usage().total(), "main_session": rec.main_session(), "stations": stations,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tools_that_change_nothing_are_read_only_and_none_is_asked_about() {
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
                TOOL_DRY_RUN,
                TOOL_GUIDE,
                TOOL_RUNS,
                TOOL_RUN
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
                TOOL_PART_SET,
                TOOL_PART_DELETE,
                TOOL_START,
                TOOL_PAUSE,
                TOOL_RESUME,
                TOOL_CANCEL,
                TOOL_CONFIRM,
                TOOL_FORCE,
                TOOL_REWORK,
                TOOL_RETRY,
                TOOL_RELEASE
            ]
        );
        // None of them asks the person: a change is checked as it runs, not approved.
        let sh = Arc::new(Shared::for_tests());
        for t in TOOLS {
            let tool = AuthorTool::new(t, sh.clone());
            assert_eq!(tool.permission(&json!({"flow": "f", "run": "r-1"})), Permission::Allow, "{}", t.name);
        }
    }

    /// The table is the one list: every name once, each with its words and an input schema.
    #[test]
    fn every_tool_is_one_row_of_the_table() {
        let mut names: Vec<&str> = TOOLS.iter().map(|t| t.name).collect();
        assert_eq!(AUTHOR_TOOLS.map(|(n, _)| n).to_vec(), names, "the public list is the table's");
        names.sort();
        names.dedup();
        assert_eq!(names.len(), TOOLS.len(), "no name twice");
        for t in TOOLS {
            assert!(t.name.starts_with("flow_") && !t.description.is_empty(), "{}", t.name);
            assert_eq!((t.schema)()["type"], "object", "{}", t.name);
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
    fn parts_written_before_a_failed_save_are_put_back() {
        use crate::packs::MemPacks;
        let packs = MemPacks::new().skill("old", "Built in.").writable();
        let sk = |n: &str| (PartKind::Skill, n.to_string());
        packs.save_part(PartKind::Skill, "mine", "Mine before.").unwrap();
        let before = vec![
            (sk("old"), Some("Built in.".to_string())),
            (sk("mine"), Some("Mine before.".to_string())),
            ((PartKind::Rule, "new".to_string()), None),
        ];
        packs.save_part(PartKind::Skill, "old", "Changed.").unwrap();
        packs.save_part(PartKind::Skill, "mine", "Changed.").unwrap();
        packs.save_part(PartKind::Rule, "new", "Fresh.").unwrap();
        let saved = vec![sk("old"), sk("mine"), (PartKind::Rule, "new".to_string())];
        restore_parts(&packs, &before, &saved);
        assert_eq!(Packs::skill(&packs, "old").unwrap().0, "Built in.");
        assert_eq!(Packs::skill(&packs, "mine").unwrap().0, "Mine before.");
        assert!(Packs::rule(&packs, "new").is_none());
    }

    #[test]
    fn parts_come_as_a_list() {
        assert_eq!(parts_of(&json!({})).unwrap().len(), 0);
        let p = parts_of(&json!({"parts": [{"kind": "rule", "name": "a", "text": "A."}]})).unwrap();
        assert_eq!(p[&(PartKind::Rule, "a".to_string())], "A.");
        assert!(parts_of(&json!({"parts": [{"kind": "nope", "name": "a", "text": "A."}]})).is_err());
        assert!(parts_of(&json!({"parts": {"a": "A."}})).is_err());
    }

    /// A module with an engine over `MemPacks` and its project packs, before start (no context:
    /// a project's trust is what the test sets).
    fn started() -> Arc<Shared> {
        let sh = Arc::new(Shared::for_tests());
        let cfg = crate::runner::RunnerConfig {
            profiles: vec!["flow-harnix".into()],
            guarded: vec![],
            trust: sh.projects.clone(),
            ..Default::default()
        };
        let packs = crate::runner::CatalogPacks::over(&cfg, vec![], Arc::new(crate::MemPacks::new().skill("look", "Look.").writable()));
        packs.set_layout(super::super::layout(std::path::Path::new("/state/ext/flow/data")));
        let engine = crate::Engine::new(Arc::new(crate::MemStore::new()), Arc::new(packs));
        let _ = sh.engine.set(Arc::new(engine));
        sh
    }

    async fn call(sh: &Arc<Shared>, tool: &str, session: &str, input: Value) -> ToolOutput {
        let row = TOOLS.iter().find(|t| t.name == tool).unwrap();
        let cx = ToolCx::new(session, "t1").with_instance("main");
        AuthorTool::new(row, sh.clone()).call(cx, input).await.unwrap()
    }

    #[tokio::test]
    async fn a_flow_created_for_a_project_is_saved_in_it_and_the_conversation_stays_there() {
        let sh = started();
        let root = tempfile::tempdir().unwrap();
        let at = root.path().to_str().unwrap().to_string();
        let yaml = "name: fresh\nstations: [{id: a, executor: {kind: session, scene: flow-harnix}, skill: house, output: {type: object}, exit: [{kind: output}]}]";
        let parts = json!([{"kind": "skill", "name": "house", "text": "The house way."}]);
        // The project's own flows, in its `.harnix/`. Tasks' tools are kept out of that directory
        // (`judge_under`'s reserved); `flow_create` is the module's own tool, which nothing judges.
        let dir = crate::project_pack_dir(root.path(), &super::super::layout(std::path::Path::new("/state/ext/flow/data")));
        assert_eq!(dir, root.path().join(".harnix/flows"));

        // Not trusted: nothing is written in it.
        let out = call(&sh, TOOL_CREATE, "s1", json!({"yaml": yaml, "parts": parts, "project_root": at})).await;
        assert!(out.is_error() && out.content().contains("not trusted"), "{}", out.content());
        assert!(!dir.exists());
        assert!(sh.conversation_project("main", "s1").is_none(), "refused: the conversation is not moved");

        sh.projects.set(&at, true);
        let out = call(&sh, TOOL_CREATE, "s1", json!({"yaml": yaml, "parts": parts, "project_root": at})).await;
        assert!(!out.is_error() && out.content().contains("\"saved\": true"), "{}", out.content());
        assert!(dir.join("fresh.yaml").is_file() && dir.join("skills/house/SKILL.md").is_file());
        let engine = sh.engine().unwrap();
        assert!(engine.packs().flow("fresh").is_none() && engine.packs().skill("house").is_none(), "only the project has them");
        assert_eq!(sh.conversation_project("main", "s1").as_deref(), Some(at.as_str()));

        // The conversation lists and changes the project's, and saves there.
        let list: Value = serde_json::from_str(call(&sh, TOOL_LIST, "s1", json!({})).await.content()).unwrap();
        assert_eq!(list["project"], at);
        assert!(list["flows"].as_array().unwrap().iter().any(|f| f["name"] == "fresh" && f["origin"] == "project"));
        let out = call(&sh, TOOL_PART_SET, "s1", json!({"kind": "rule", "name": "tone", "text": "# Tone"})).await;
        assert!(!out.is_error(), "{}", out.content());
        assert!(dir.join("rules/tone.md").is_file());
        // Validation as ever; a name that would leave the directory is not a name.
        let out = call(&sh, TOOL_PART_SET, "s1", json!({"kind": "rule", "name": "../../x", "text": "# X"})).await;
        assert!(out.is_error(), "{}", out.content());
        assert!(!root.path().join("x.md").exists() && !root.path().join(".harnix/x.md").exists());
        let bad = "name: worse\nstations: [{id: a, executor: {kind: session, scene: flow-harnix}, skill: nowhere, output: {type: object}, exit: [{kind: output}]}]";
        let out = call(&sh, TOOL_CREATE, "s1", json!({"yaml": bad})).await;
        assert!(out.content().contains("\"saved\": false"), "{}", out.content());
        assert!(!dir.join("worse.yaml").exists());

        // Another conversation is not in the project.
        let list: Value = serde_json::from_str(call(&sh, TOOL_LIST, "s2", json!({})).await.content()).unwrap();
        assert!(list["project"].is_null());
        assert!(!list["flows"].as_array().unwrap().iter().any(|f| f["name"] == "fresh"));
    }
}
