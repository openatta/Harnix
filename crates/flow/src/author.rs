//! Writing flows and working their runs from a conversation: the authoring tools, declared and
//! answered here, for a host to register to the conversations it lets write flows.
//!
//! A host registers [`tool_specs`] (or [`AuthorTools::specs`]) to a conversation and routes each
//! call to [`AuthorTools::call`] with the calling session and who acts. What is the host's — the
//! conversation's current flow, the project it works in, what to tell its own people when
//! something changed — comes through [`AuthorHost`]; who may do what is the engine's
//! [`Authorizer`](crate::Authorizer), asked before every tool acts.
//!
//! | tool | read-only | does |
//! |---|---|---|
//! | `flow_list` | yes | the flows: name, title, origin, valid; the current one; the project |
//! | `flow_show {flow?}` | yes | a flow: YAML, the parts it uses, the hooks over it, problems, its stations |
//! | `flow_schema` | yes | the JSON Schemas of a definition and of the parts, and the catalog |
//! | `flow_validate {yaml}` | yes | a text's problems, nothing saved |
//! | `flow_create {yaml, parts?, project_root?}` | no | a new flow, which the host may make current |
//! | `flow_update {flow?, yaml?, parts?}` | no | change a flow |
//! | `flow_delete {flow?}` | no | delete a flow (a built-in one: its override) |
//! | `flow_part_set {kind, name, text}` / `flow_part_delete {kind, name}` | no | change, delete a skill, rule, policy, gate or hook |
//! | `flow_dry_run {flow?, inputs?, mocks?}` | yes | dry-run a flow, as far as it goes |
//! | `flow_start {flow?, subject, inputs?, title?}` | no | start a flow for real |
//! | `flow_guide {lang?}` | yes | the flow guide |
//! | `flow_runs {flow?}` / `flow_run {run?, flow?}` | yes | a flow's runs; one run |
//! | `flow_pause` … `flow_release` `{run?, flow?, …}` | no | act on a run |
//!
//! **Targets.** A tool acting on a flow takes it in `flow`; one acting on a run, in `run`. Absent,
//! the host's current one ([`AuthorHost::current`], [`AuthorHost::current_run`]); neither, the
//! call is refused and says so. A host that binds a conversation to one flow
//! ([`AuthorHost::pinned`]) refuses a flow named that is not its current one, and a run of
//! another flow.
//!
//! No tool asks a person: a change is checked, not approved — a definition with problems is not
//! saved. A run tool acts only when the user asks for it (each one's description says so), and the
//! run's history records who acted as the host said.

use std::collections::BTreeMap;
use std::sync::{Arc, Weak};

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::dryrun::{is_dry, Sandbox, StartIn};
use crate::kernel::def::{definition_schema, ExitCond, FlowDef, Work};
use crate::kernel::engine::{view, CreateIn, Engine};
use crate::kernel::host::{digest, Catalog, Op, Packs};
use crate::kernel::parts::{part_schema, rules_of, PartKind};
use crate::kernel::run::{Actor, RunQuery, RunRecord, RunSummary};
use crate::kernel::validate::{part_problems, validate, validate_yaml};
use crate::runner::{redact_url, Runner, SessionId, ToolReply, ToolSpec};
use crate::wire::defs_list;

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

/// The tools that act on a run: what a host records under a conversation's own name, if it tells
/// them apart.
pub const RUN_TOOLS: [&str; 8] =
    [TOOL_PAUSE, TOOL_RESUME, TOOL_CANCEL, TOOL_CONFIRM, TOOL_FORCE, TOOL_REWORK, TOOL_RETRY, TOOL_RELEASE];

/// The kind of an [`Actor`] that is a conversation acting for the person in it.
pub const CONVERSATION_KIND: &str = "conversation";

/// What a call needing a flow is told when it names none and the conversation has no current one
/// ([`AuthorHost::no_current`] when the host says nothing else).
pub const NO_CURRENT: &str =
    "This conversation works on no flow: name the flow in `flow` (flow_list lists them).";

/// What the host keeps for a conversation, and what it is told. Only [`AuthorHost::current`] and
/// [`AuthorHost::project`] are required; a host with neither a current flow nor projects answers
/// `None` to both.
#[async_trait]
pub trait AuthorHost: Send + Sync {
    /// The flow the conversation works on now, if the host keeps one: what a call naming no flow
    /// acts on.
    async fn current(&self, session: &SessionId) -> Option<String>;

    /// The project the conversation works in: flows and parts are read and saved there (while it
    /// is trusted), and runs started in it. `None`: the application's packs.
    async fn project(&self, session: &SessionId) -> Option<String>;

    /// Whether the conversation acts only on its current flow: a flow named that is not the
    /// current one, a run of another flow, and every call while there is none are refused.
    fn pinned(&self, _session: &SessionId) -> bool {
        false
    }

    /// What a call needing a flow is told when there is none to act on.
    fn no_current(&self, _session: &SessionId) -> String {
        NO_CURRENT.to_string()
    }

    /// The run a run tool naming none acts on (a session working in a run, say).
    async fn current_run(&self, _session: &SessionId) -> Option<String> {
        None
    }

    /// `flow_create` with a `project_root`: the conversation works in that project from now on.
    async fn set_project(&self, _session: &SessionId, _root: &str) {}

    /// The conversation created `flow`. The host may make it the current one; what it returns is
    /// told to the model as the call's `note`.
    async fn created(&self, _session: &SessionId, _flow: &str) -> Option<String> {
        None
    }

    /// Something the host may tell its own people about changed.
    async fn notice(&self, _notice: Notice) {}
}

/// What changed, for [`AuthorHost::notice`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Notice {
    /// A flow or a part was saved or deleted (or put back after a save that failed).
    Packs,
    /// `flow` was deleted; `gone`: no version of it is left (a built-in one is back otherwise).
    Deleted { flow: String, gone: bool },
    /// A dry run moved.
    DryRun { run: String },
}

/// The authoring tools: their specs, for the host to register; their calls, for the host to route
/// here. Made once the runner is, with the dry runs' sandbox and the host.
pub struct AuthorTools {
    runner: Weak<Runner>,
    sandbox: Arc<Sandbox>,
    host: Arc<dyn AuthorHost>,
    specs: Vec<ToolSpec>,
}

/// Which arm of [`AuthorTools::call`] a tool is.
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

/// One tool, everything about it but its code: the one place a tool is listed.
struct Row {
    name: &'static str,
    description: &'static str,
    schema: fn() -> Value,
    read_only: bool,
    does: Does,
}

fn flow_named() -> Value {
    json!({"type": "string", "description": "the flow this acts on, by name, as flow_list or flow_show gave it; absent: the conversation's current flow"})
}

fn run_named() -> Value {
    json!({"type": "string", "description": "the run this acts on, as flow_runs gave it; absent: the conversation's current run, where it has one"})
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

fn flow_only() -> Value {
    json!({"type": "object", "properties": {"flow": flow_named()}})
}

/// A run tool's input: `run`, `flow`, and what else it takes.
fn run_input(required: &[&str], more: Value) -> Value {
    let mut props = json!({"run": run_named(), "flow": flow_named()});
    if let (Some(p), Value::Object(m)) = (props.as_object_mut(), more) {
        p.extend(m);
    }
    json!({"type": "object", "required": required, "properties": props})
}

/// Every tool, in the order the model is offered them.
const TOOLS: &[Row] = &[
    Row {
        name: TOOL_LIST,
        description: "The flows on offer: name, title, origin (builtin, override, user, project, plugin — with the plugin's name as source) and whether each is valid; which one is current; and the project the conversation works in, if any.",
        schema: no_input,
        read_only: true,
        does: Does::List,
    },
    Row {
        name: TOOL_SHOW,
        description: "A flow, named in `flow` (absent: the current one, the one the user selected): its YAML, the texts of the parts it uses (skills, rules, policies, gates), the hooks over it, its problems, and its stations in brief.",
        schema: flow_only,
        read_only: true,
        does: Does::Show,
    },
    Row {
        name: TOOL_SCHEMA,
        description: "The JSON Schemas of a flow definition and of policies, gates and hooks, and the names this application offers (scenes, steps, checks, tools, templates, instances, spaces, models, guarded tools, and the parts on offer by kind).",
        schema: no_input,
        read_only: true,
        does: Does::Schema,
    },
    Row {
        name: TOOL_VALIDATE,
        description: "Every problem of a flow definition text (YAML), without saving anything. An empty list: it is valid.",
        schema: || json!({"type": "object", "required": ["yaml"], "properties": {"yaml": {"type": "string"}}}),
        read_only: true,
        does: Does::Validate,
    },
    Row {
        name: TOOL_CREATE,
        description: "Create a new flow from its YAML (and, optionally, parts it uses: [{kind: skill|rule|policy|gate, name, text}]). Everything is validated first and nothing is saved if anything has problems. It becomes the current flow. With `project_root`, it is saved in that project (with the project's own flows, committed with it; the project must be trusted) and the conversation works in that project from then on.",
        schema: || json!({"type": "object", "required": ["yaml"], "properties": {"yaml": {"type": "string"}, "parts": parts_given(), "project_root": {"type": "string", "description": "the project to save it in, when the user asks for that: its absolute directory"}}}),
        read_only: false,
        does: Does::Save { creating: true },
    },
    Row {
        name: TOOL_UPDATE,
        description: "Change a flow, named in `flow` (absent: the current one): its YAML (same name), and/or parts ([{kind, name, text}]). Validated first; nothing is saved if anything has problems. Runs already started keep their definition.",
        schema: || json!({"type": "object", "properties": {"flow": flow_named(), "yaml": {"type": "string"}, "parts": parts_given()}}),
        read_only: false,
        does: Does::Save { creating: false },
    },
    Row {
        name: TOOL_DELETE,
        description: "Delete a flow, named in `flow` (absent: the current one). For a built-in flow, this deletes the saved override and the built-in version is back.",
        schema: flow_only,
        read_only: false,
        does: Does::Delete,
    },
    Row {
        name: TOOL_PART_SET,
        description: "Create or change a part of the pack: a skill (its SKILL.md), a rule (markdown, first `# ` heading its title), a policy, a gate or a hook (YAML; its `name` the same). Checked first; the flows it breaks are listed.",
        schema: || json!({"type": "object", "required": ["kind", "name", "text"], "properties": {"kind": {"enum": ["skill", "rule", "policy", "gate", "hook"]}, "name": {"type": "string"}, "text": {"type": "string"}}}),
        read_only: false,
        does: Does::PartSet,
    },
    Row {
        name: TOOL_PART_DELETE,
        description: "Delete a saved part (a built-in one: its override, and the built-in one is back).",
        schema: || json!({"type": "object", "required": ["kind", "name"], "properties": {"kind": {"enum": ["skill", "rule", "policy", "gate", "hook"]}, "name": {"type": "string"}}}),
        read_only: false,
        does: Does::PartDelete,
    },
    Row {
        name: TOOL_DRY_RUN,
        description: "Dry-run a flow, named in `flow` (absent: the current one): the real kernel, everything outside mocked (no session, no program, no model, no command). Runs until it ends or waits for a person, and returns the path it took. `mocks`: {stations: {id: output}, gates: {gate: false} (command and host gates), tools: {tool: result} (what tool gates get)}.",
        schema: || json!({"type": "object", "properties": {
            "flow": flow_named(),
            "inputs": {"type": "object"},
            "mocks": {"type": "object", "properties": {
                "stations": {"type": "object"}, "gates": {"type": "object"}, "tools": {"type": "object"}}}}}),
        read_only: true,
        does: Does::DryRun,
    },
    Row {
        name: TOOL_START,
        description: "Start a real run of a flow, named in `flow` (absent: the current one), for a subject. Only when the user asks for it.",
        schema: || json!({"type": "object", "required": ["subject"], "properties": {
            "flow": flow_named(),
            "subject": {"type": "string"}, "inputs": {"type": "object"}, "title": {"type": "string"}}}),
        read_only: false,
        does: Does::Start,
    },
    Row {
        name: TOOL_GUIDE,
        description: "The guide to flows: the objects (flow, station, work, task, executor, exit condition, rule, policy and permission, gate, hook, run, main session, sub-agent, lease, rework, dry run), what they mean, and how to define a flow, with examples. `lang`: en or zh-CN.",
        schema: || json!({"type": "object", "properties": {"lang": {"enum": ["en", "zh-CN"]}}}),
        read_only: true,
        does: Does::Guide,
    },
    Row {
        name: TOOL_RUNS,
        description: "The runs of a flow, named in `flow` (absent: the current one), newest first: id, title, what each is doing (phase), when it started, its tokens.",
        schema: flow_only,
        read_only: true,
        does: Does::Runs,
    },
    Row {
        name: TOOL_RUN,
        description: "One run, named in `run` (with `flow`, it must be a run of that flow): each station's status, time and tokens, what it waits for, its tasks.",
        schema: || run_input(&[], json!({})),
        read_only: true,
        does: Does::Run,
    },
    Row {
        name: TOOL_PAUSE,
        description: "Pause a run: what is in hand finishes, nothing new starts. Only when the user asks.",
        schema: || run_input(&[], json!({})),
        read_only: false,
        does: Does::Act(Act::Pause),
    },
    Row {
        name: TOOL_RESUME,
        description: "Resume a paused run. Only when the user asks.",
        schema: || run_input(&[], json!({})),
        read_only: false,
        does: Does::Act(Act::Resume),
    },
    Row {
        name: TOOL_CANCEL,
        description: "Cancel a run, with a reason. Only when the user asks.",
        schema: || run_input(&["reason"], json!({"reason": {"type": "string"}})),
        read_only: false,
        does: Does::Act(Act::Cancel),
    },
    Row {
        name: TOOL_CONFIRM,
        description: "Confirm a station of a run (`pass`: true passes, false rejects), with an optional reason. Only when the user asks for it.",
        schema: || run_input(&["station", "pass"], json!({"station": {"type": "string"}, "pass": {"type": "boolean"}, "reason": {"type": "string"}})),
        read_only: false,
        does: Does::Act(Act::Confirm),
    },
    Row {
        name: TOOL_FORCE,
        description: "Pass a station of a run by force, with a reason, whatever is not met. Only when the user asks.",
        schema: || run_input(&["station", "reason"], json!({"station": {"type": "string"}, "reason": {"type": "string"}})),
        read_only: false,
        does: Does::Act(Act::Force),
    },
    Row {
        name: TOOL_REWORK,
        description: "Send a run back to station `to` (optionally only `tasks`), with a reason. Only when the user asks.",
        schema: || run_input(&["to", "reason"], json!({"to": {"type": "string"}, "tasks": {"type": "array", "items": {"type": "string"}}, "reason": {"type": "string"}})),
        read_only: false,
        does: Does::Act(Act::Rework),
    },
    Row {
        name: TOOL_RETRY,
        description: "Put a failed task of a run back to work with a fresh retry budget. Only when the user asks.",
        schema: || run_input(&["station", "task"], json!({"station": {"type": "string"}, "task": {"type": "string"}})),
        read_only: false,
        does: Does::Act(Act::Retry),
    },
    Row {
        name: TOOL_RELEASE,
        description: "Release a held station of a run. Only when the user asks.",
        schema: || run_input(&["station"], json!({"station": {"type": "string"}})),
        read_only: false,
        does: Does::Act(Act::Release),
    },
];

/// The authoring tools' specs, as a host registers them: the same whether or not a runner is there
/// yet. Those that change nothing are read-only.
pub fn tool_specs() -> Vec<ToolSpec> {
    TOOLS
        .iter()
        .map(|t| ToolSpec {
            name: t.name.into(),
            description: t.description.into(),
            input_schema: (t.schema)(),
            read_only: t.read_only,
        })
        .collect()
}

impl AuthorTools {
    pub fn new(runner: &Arc<Runner>, sandbox: Arc<Sandbox>, host: Arc<dyn AuthorHost>) -> Self {
        Self { runner: Arc::downgrade(runner), sandbox, host, specs: tool_specs() }
    }

    /// Every authoring tool, in the order the model is offered them.
    pub fn specs(&self) -> &[ToolSpec] {
        &self.specs
    }

    /// Answer a call of `tool` from the conversation `session`, acting as `by`: the engine's
    /// authorizer is asked first, as `by`, about what the call would do.
    pub async fn call(&self, session: &SessionId, tool: &str, input: Value, by: &Actor) -> ToolReply {
        let Some(row) = TOOLS.iter().find(|t| t.name == tool) else {
            return ToolReply::error(format!("no authoring tool `{tool}`"));
        };
        if row.does == Does::Guide {
            let zh = str_of(&input, "lang").is_some_and(|l| l.starts_with("zh"));
            return ToolReply::text(if zh { crate::GUIDE_ZH } else { crate::GUIDE_EN });
        }
        let Some(rn) = self.runner.upgrade() else {
            return ToolReply::error("the flow runner has stopped");
        };
        let cx = Cx { rn: &rn, engine: rn.engine(), host: self.host.as_ref(), sandbox: &self.sandbox, session, by };
        match cx.call(row.does, &input).await {
            Ok(v) => ToolReply::text(pretty(&v)),
            Err(m) => ToolReply::error(m),
        }
    }
}

/// A call's answer: a value shown to the model, or why nothing was done.
type Answer = Result<Value, String>;

/// One call, with what it works with.
struct Cx<'a> {
    rn: &'a Arc<Runner>,
    engine: &'a Arc<Engine>,
    host: &'a dyn AuthorHost,
    sandbox: &'a Sandbox,
    session: &'a SessionId,
    by: &'a Actor,
}

impl Cx<'_> {
    fn authorize(&self, op: &Op, run: Option<&RunRecord>) -> Result<(), String> {
        self.engine.authorize(op, self.by, run).map_err(|e| e.to_string())
    }

    /// The flow a tool acting on one acts on: the one named, else the current one. A pinned
    /// conversation acts only on its current flow.
    fn target(&self, input: &Value, current: Option<&str>) -> Result<String, String> {
        let pinned = self.host.pinned(self.session);
        match (str_of(input, "flow"), current) {
            (Some(named), Some(c)) if pinned && named != c => Err(format!(
                "the current flow changed to `{c}` (you named `{named}`): nothing was done — ask the user \
                 again which flow they mean"
            )),
            (Some(_), None) if pinned => Err(self.host.no_current(self.session)),
            (Some(named), _) => Ok(named.to_string()),
            (None, Some(c)) => Ok(c.to_string()),
            (None, None) => Err(self.host.no_current(self.session)),
        }
    }

    async fn call(&self, does: Does, input: &Value) -> Answer {
        let engine = self.engine;
        // A conversation in a project reads and saves the project's packs (its own flows);
        // `flow_create` with a project moves it into that one.
        let asked = match (does, str_of(input, "project_root")) {
            (Does::Save { creating: true }, Some(root)) => Some(project_root(root)?),
            _ => None,
        };
        let project = match asked.clone() {
            Some(root) => Some(root),
            None => self.host.project(self.session).await,
        };
        let packs = match &project {
            Some(root) => {
                let writes = matches!(does, Does::Save { .. } | Does::Delete | Does::PartSet | Does::PartDelete);
                if !self.rn.hear(root).await && writes {
                    return Err(format!(
                        "the project {root} is not trusted: nothing is saved in it until the user trusts it"
                    ));
                }
                engine.packs_for(Some(root))
            }
            None => engine.packs().clone(),
        };
        if let Some(root) = &asked {
            self.host.set_project(self.session, root).await;
        }
        // The current flow, if it is still one of these packs'.
        let current = self.host.current(self.session).await.filter(|f| packs.flow(f).is_some());
        let packs = packs.as_ref();
        match does {
            Does::List => {
                self.authorize(&Op::ReadFlows, None)?;
                Ok(json!({
                    // What the conversation needs of each: its inputs are in the definition `flow_show` gives.
                    "flows": defs_list(packs).into_iter().map(|d| {
                        let mut row = json!({
                            "name": d.name, "title": d.title, "digest": d.digest,
                            "origin": d.origin, "valid": d.valid,
                        });
                        if let Some(source) = d.source {
                            row["source"] = json!(source);
                        }
                        row
                    }).collect::<Vec<_>>(),
                    "current": current,
                    "writable": packs.writable(),
                    "project": project,
                }))
            }
            Does::Schema => {
                self.authorize(&Op::ReadFlows, None)?;
                Ok(json!({"definition": definition_schema(), "parts": part_schemas(), "catalog": catalog_json(packs)}))
            }
            Does::Validate => {
                let yaml = str_of(input, "yaml").ok_or("give the definition as `yaml`")?;
                self.authorize(&Op::ReadFlows, None)?;
                Ok(json!({"problems": validate_yaml(yaml, packs)}))
            }
            Does::Show => {
                let name = self.target(input, current.as_deref())?;
                self.authorize(&Op::ReadFlow { flow: name.clone() }, None)?;
                let (def, digest) = packs.flow(&name).ok_or_else(|| format!("flow {name} is gone"))?;
                let used = parts_used(packs, &def);
                let mut shown = json!({
                    "name": name, "title": def.title, "origin": packs.origin(&name), "digest": digest,
                    "yaml": packs.flow_text(&name).unwrap_or_default(),
                    "problems": validate(&def, packs),
                    "stations": stations_of(&def),
                    "parts": used["parts"], "hooks": used["hooks"],
                });
                if let Some(source) = packs.source(&name) {
                    shown["source"] = json!(source);
                }
                Ok(shown)
            }
            Does::Save { creating } => self.save(creating, input, packs, current.as_deref()).await,
            Does::Delete => {
                let name = self.target(input, current.as_deref())?;
                let v = engine
                    .delete_flow_in(packs, &name, self.by.clone())
                    .await
                    .map_err(|e| format!("not deleted: {e}"))?;
                self.host.notice(Notice::Packs).await;
                let gone = packs.flow(&name).is_none();
                self.host.notice(Notice::Deleted { flow: name, gone }).await;
                Ok(v)
            }
            Does::PartSet => {
                let kind = str_of(input, "kind").and_then(PartKind::parse);
                let (Some(kind), Some(name), Some(body)) = (kind, str_of(input, "name"), str_of(input, "text")) else {
                    return Err("give the part's `kind` (skill, rule, policy, gate, hook), `name` and `text`".into());
                };
                let v = engine
                    .save_part_in(packs, kind, name, body, self.by.clone())
                    .await
                    .map_err(|e| format!("not saved: {e}"))?;
                self.host.notice(Notice::Packs).await;
                Ok(v)
            }
            Does::PartDelete => {
                let kind = str_of(input, "kind").and_then(PartKind::parse);
                let (Some(kind), Some(name)) = (kind, str_of(input, "name")) else {
                    return Err("give the part's `kind` and `name`".into());
                };
                let v = engine
                    .delete_part_in(packs, kind, name, self.by.clone())
                    .await
                    .map_err(|e| format!("not deleted: {e}"))?;
                self.host.notice(Notice::Packs).await;
                Ok(v)
            }
            Does::DryRun => self.dry_run(input, current.as_deref()).await,
            Does::Start => {
                let name = self.target(input, current.as_deref())?;
                let subject = str_of(input, "subject").ok_or("give the run's `subject`")?;
                let start = CreateIn {
                    flow: name,
                    subject: subject.to_string(),
                    inputs: input.get("inputs").cloned().unwrap_or(Value::Null),
                    title: str_of(input, "title").map(String::from),
                    parent: None,
                    project_root: project.clone(),
                    ..Default::default()
                };
                // Authorized as `create` before anything is made for the run.
                let o = self.rn.start_run(start, self.by.clone()).await.map_err(|e| format!("not started: {e}"))?;
                let value = o.value.clone();
                self.rn.apply(o).await;
                Ok(value)
            }
            // Answered before a runner is needed.
            Does::Guide => Err("the guide is not read here".into()),
            Does::Runs => {
                let name = self.target(input, current.as_deref())?;
                self.authorize(&Op::ListRuns { flow: Some(name.clone()) }, None)?;
                let q = RunQuery { flow: Some(name), project: project.clone(), limit: Some(50), ..Default::default() };
                let list = engine.list(q).await.map_err(|e| e.to_string())?;
                Ok(json!({"runs": list.iter().map(run_line).collect::<Vec<_>>()}))
            }
            Does::Run => {
                let rec = self.run(input, current.as_deref()).await?;
                self.authorize(&Op::ReadRun, Some(&rec))?;
                Ok(run_detail(&rec, engine.clock().now_ms()))
            }
            Does::Act(act) => self.act(act, input, current.as_deref()).await,
        }
    }

    async fn save(&self, creating: bool, input: &Value, packs: &dyn Packs, current: Option<&str>) -> Answer {
        let engine = self.engine;
        let target = if creating { None } else { Some(self.target(input, current)?) };
        if let Some(t) = target.as_ref().filter(|t| packs.flow(t).is_none()) {
            return Err(format!("there is no flow `{t}` (flow_list lists them)"));
        }
        let parts = parts_of(input)?;
        let yaml = str_of(input, "yaml");
        if creating && yaml.is_none() {
            return Err("give the new flow's definition as `yaml`".into());
        }
        if yaml.is_none() && parts.is_empty() {
            return Err("give `yaml`, `parts`, or both".into());
        }
        let name = match yaml.map(FlowDef::parse) {
            Some(Ok(d)) => Some(d.name),
            Some(Err(e)) => return Err(format!("not a flow definition: {e}")),
            None => None,
        };
        if let (Some(n), Some(t)) = (&name, &target) {
            if n != t {
                return Err(format!(
                    "the text defines flow `{n}`, but this changes flow `{t}`: keep the name, or create a new \
                     flow with {TOOL_CREATE}"
                ));
            }
        }
        if let (true, Some(n)) = (creating, &name) {
            if packs.flow(n).is_some() {
                let how = if self.host.pinned(self.session) { "ask the user to select it, then change it" } else { "change it" };
                return Err(format!("there is a flow `{n}` already: {how} with {TOOL_UPDATE}, or choose another name"));
            }
        }
        // Asked first, as one: nothing is written that the rest would not be allowed after.
        if let Some(n) = name.as_ref().or(target.as_ref()) {
            self.authorize(&Op::EditFlow { flow: n.clone() }, None)?;
        }
        for (k, n) in parts.keys() {
            self.authorize(&Op::EditPart { kind: *k, name: n.clone() }, None)?;
        }
        // Everything is checked — the parts, the text with the parts it brings — before anything
        // is written.
        let with = WithParts { base: packs, parts: &parts };
        let mut problems: Vec<Value> = parts
            .iter()
            .flat_map(|((k, n), t)| {
                part_problems(*k, n, t, &with).into_iter().map(move |m| json!({"message": format!("{k} {n}: {m}")}))
            })
            .collect();
        if let Some(y) = yaml {
            problems.extend(validate_yaml(y, &with).into_iter().filter_map(|p| serde_json::to_value(p).ok()));
        }
        if !problems.is_empty() {
            return Ok(json!({"saved": false, "problems": problems}));
        }
        // Then written; if the flow is not saved after all, the parts this call wrote are put back
        // as they were.
        let before: Vec<((PartKind, String), Option<String>)> =
            parts.keys().map(|(k, n)| ((*k, n.clone()), packs.part(*k, n).map(|(b, _)| b))).collect();
        let mut saved_parts: Vec<(PartKind, String)> = vec![];
        let mut failed = None;
        for ((k, n), t) in &parts {
            match engine.save_part_in(packs, *k, n, t, self.by.clone()).await {
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
            match engine.save_flow_in(packs, y, self.by.clone()).await {
                Ok(v) => out["flow"] = v,
                Err(e) => failed = Some(format!("not saved: {e}")),
            }
        }
        if let Some(m) = failed {
            restore_parts(packs, &before, &saved_parts);
            self.host.notice(Notice::Packs).await;
            return Err(format!("{m} (nothing this call wrote was kept)"));
        }
        self.host.notice(Notice::Packs).await;
        if let (true, Some(n)) = (creating, name) {
            let note = self.host.created(self.session, &n).await;
            if self.host.current(self.session).await.as_deref() == Some(n.as_str()) {
                out["current"] = json!(n);
            }
            if let Some(note) = note {
                out["note"] = json!(note);
            }
        }
        Ok(out)
    }

    async fn dry_run(&self, input: &Value, current: Option<&str>) -> Answer {
        let name = self.target(input, current)?;
        self.authorize(&Op::DryRun { flow: name.clone() }, None)?;
        let start = StartIn {
            flow: Some(name),
            inputs: input.get("inputs").cloned().unwrap_or(Value::Null),
            mocks: match input.get("mocks") {
                None | Some(Value::Null) => Default::default(),
                Some(m) => serde_json::from_value(m.clone()).map_err(|e| format!("`mocks`: {e}"))?,
            },
            ..Default::default()
        };
        let sandbox = self.sandbox;
        let run = sandbox.start(start, self.by.clone()).await.map_err(|e| format!("the dry run did not start: {e}"))?;
        let progress = sandbox.run(&run).await;
        self.host.notice(Notice::DryRun { run: run.clone() }).await;
        let progress = progress.map_err(|e| e.to_string())?;
        let view = sandbox.get(&run).await.unwrap_or(Value::Null);
        let stations: BTreeMap<String, Value> = view
            .pointer("/record/stations")
            .and_then(Value::as_object)
            .map(|m| m.iter().map(|(k, v)| (k.clone(), v["status"].clone())).collect())
            .unwrap_or_default();
        Ok(json!({
            "run": run, "status": progress.status, "steps": progress.steps,
            "stations": stations, "waiting": view.get("unmet"),
        }))
    }

    /// The run a run tool acts on: the one named, else the current one; of the flow named, or —
    /// pinned — of the current flow.
    async fn run(&self, input: &Value, current: Option<&str>) -> Result<RunRecord, String> {
        let flow = match (self.host.pinned(self.session), str_of(input, "flow")) {
            (true, _) => Some(self.target(input, current)?),
            (false, named) => named.map(String::from),
        };
        let run = match str_of(input, "run") {
            Some(r) => r.to_string(),
            None => self.host.current_run(self.session).await.ok_or("name the run in `run` (flow_runs lists them)")?,
        };
        if is_dry(&run) {
            return Err("a dry run is driven with flow_dry_run, not the run tools".into());
        }
        let rec = self.engine.record(&run).await.map_err(|e| e.to_string())?;
        if let Some(flow) = flow.filter(|f| *f != rec.flow) {
            return Err(format!("run {run} is a run of flow {}, not of {flow}: nothing was done", rec.flow));
        }
        Ok(rec)
    }

    async fn act(&self, act: Act, input: &Value, current: Option<&str>) -> Answer {
        let engine = self.engine;
        let rec = self.run(input, current).await?;
        let run = rec.id.clone();
        let who = self.by.clone();
        let station = str_of(input, "station").unwrap_or_default().to_string();
        let reason = str_of(input, "reason").map(String::from);
        // Each authorized by the engine, as `by`, against the run as it is when it acts.
        let r = match act {
            Act::Pause => engine.pause(&run, who).await,
            Act::Resume => engine.resume(&run, who).await,
            Act::Cancel => engine.cancel(&run, reason.as_deref().unwrap_or("cancelled in conversation"), who).await,
            Act::Confirm => {
                let pass = input.get("pass").and_then(Value::as_bool).unwrap_or(true);
                engine.confirm(&run, &station, pass, reason.as_deref(), who).await
            }
            Act::Force => {
                engine.force(&run, &station, reason.as_deref().unwrap_or("forced in conversation"), who).await
            }
            Act::Rework => {
                let to = str_of(input, "to").unwrap_or_default();
                let tasks: Option<Vec<String>> = input
                    .get("tasks")
                    .and_then(Value::as_array)
                    .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect());
                engine.rework(&run, to, tasks, reason.as_deref().unwrap_or("sent back in conversation"), who).await
            }
            Act::Retry => {
                let task = str_of(input, "task").unwrap_or_default();
                engine.retry(&run, &station, task, who).await
            }
            Act::Release => engine.release(&run, &station, who).await,
        };
        let o = r.map_err(|e| format!("not done: {e}"))?;
        let value = o.value.clone();
        self.rn.apply_for(&run, o).await;
        let after = engine.record(&run).await.ok();
        Ok(json!({"done": value, "phase": after.map(|r| r.phase())}))
    }
}

fn pretty(v: &Value) -> String {
    serde_json::to_string_pretty(v).unwrap_or_else(|_| v.to_string())
}

fn str_of<'v>(input: &'v Value, k: &str) -> Option<&'v str> {
    input.get(k).and_then(Value::as_str).filter(|s| !s.trim().is_empty())
}

/// A project root a conversation gives: an existing absolute directory, as given but for
/// surrounding space and a trailing `/`.
pub fn project_root(root: &str) -> Result<String, String> {
    let root = root.trim();
    let trimmed = root.trim_end_matches('/');
    let root = if trimmed.is_empty() { root } else { trimmed };
    if !std::path::Path::new(root).is_absolute() {
        return Err(format!("project_root {root:?} is not an absolute path"));
    }
    match std::fs::metadata(root) {
        Ok(m) if m.is_dir() => Ok(root.to_string()),
        _ => Err(format!("project_root {root:?} is not an existing directory")),
    }
}

/// Parts about to be saved: (kind, name) → text.
type PartTexts = BTreeMap<(PartKind, String), String>;

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
            Some(b) => Some((b.clone(), digest(b))),
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

/// Put back parts a call saved: each as its text was before, or gone if it was not there.
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

/// The names the packs' catalog offers and the parts on offer, by kind.
pub fn catalog_json(packs: &dyn Packs) -> Value {
    let c: Catalog = packs.catalog();
    let parts: BTreeMap<&str, Vec<String>> = PartKind::ALL.into_iter().map(|k| (k.as_str(), packs.parts(k))).collect();
    json!({
        "scenes": c.scenes, "steps": c.steps, "checks": c.gates, "tools": c.tools,
        "command_gates": c.command_gates, "templates": c.templates, "instances": c.instances,
        "spaces": c.spaces, "models": c.models, "guarded": c.guarded, "parts": parts,
    })
}

/// The JSON Schemas of the parts that are YAML, by kind.
pub fn part_schemas() -> Value {
    let m: serde_json::Map<String, Value> = [PartKind::Policy, PartKind::Gate, PartKind::Hook]
        .into_iter()
        .filter_map(|k| part_schema(k).map(|s| (k.as_str().to_string(), s)))
        .collect();
    Value::Object(m)
}

/// The parts a flow uses, with their texts (`null`: not in the packs), and the hooks over it (their
/// URLs without credentials): `{parts: {<dir>: {name: text}}, hooks: [..]}`.
pub fn parts_used(packs: &dyn Packs, def: &FlowDef) -> Value {
    let mut used: BTreeMap<&str, BTreeMap<String, Option<String>>> = BTreeMap::new();
    let mut put = |kind: PartKind, name: &str| {
        used.entry(kind.dir()).or_default().insert(name.to_string(), packs.part(kind, name).map(|(t, _)| t));
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
            if let ExitCond::Gate { gate, .. } = e {
                put(PartKind::Gate, gate);
            }
        }
    }
    let hooks: Vec<Value> = packs
        .hooks()
        .into_iter()
        .filter(|h| h.flows.is_empty() || h.flows.contains(&def.name))
        .map(|h| json!({"name": h.name, "title": h.title, "on": h.on, "kind": h.kind, "action": redact_url(&h.action)}))
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
                    ts.iter().map(|t| format!("{} ({})", t.id, t.executor.kind())).collect::<Vec<_>>().join(", ")
                ),
                Work::Foreach { items, task, .. } => format!("foreach {items}: {}", task.executor.kind()),
            };
            json!({
                "id": s.id, "title": s.title, "needs": def.needs_of(&s.id), "work": work,
                "exit": s.exit.iter().map(|e| e.label()).collect::<Vec<_>>(),
                "rework_to": s.rework_to,
            })
        })
        .collect()
}

/// A run in a list, for the conversation.
fn run_line(s: &RunSummary) -> Value {
    json!({
        "run": s.id, "title": s.title, "subject": s.subject,
        "phase": s.phase, "created_at": s.created_at, "finished_at": s.finished_at,
        "tokens": s.usage.total(),
    })
}

/// A run's stations, for the conversation: status, time, tokens, what each waits for.
fn run_detail(rec: &RunRecord, now: u64) -> Value {
    let v = view(rec.clone());
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
                "waiting": v.unmet.get(&d.id),
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
    use crate::packs::MemPacks;

    #[test]
    fn every_tool_is_one_row_and_those_that_change_nothing_are_read_only() {
        let specs = tool_specs();
        let mut names: Vec<&str> = specs.iter().map(|t| t.name.as_str()).collect();
        let read_only: Vec<&str> = specs.iter().filter(|t| t.read_only).map(|t| t.name.as_str()).collect();
        assert_eq!(
            read_only,
            [TOOL_LIST, TOOL_SHOW, TOOL_SCHEMA, TOOL_VALIDATE, TOOL_DRY_RUN, TOOL_GUIDE, TOOL_RUNS, TOOL_RUN]
        );
        let acts: Vec<&str> = TOOLS.iter().filter(|t| matches!(t.does, Does::Act(_))).map(|t| t.name).collect();
        assert_eq!(acts, RUN_TOOLS, "the run tools are the ones that act on a run");
        assert_eq!(names.len(), 22);
        names.sort();
        names.dedup();
        assert_eq!(names.len(), 22, "no name twice");
        for t in &specs {
            assert!(t.name.starts_with("flow_") && !t.description.is_empty(), "{}", t.name);
            assert_eq!(t.input_schema["type"], "object", "{}", t.name);
        }
        // What acts on a flow or a run takes it, and needs it only where there is no current one.
        for t in TOOLS.iter().filter(|t| matches!(t.does, Does::Show | Does::Save { creating: false } | Does::Delete | Does::DryRun | Does::Start | Does::Runs | Does::Run | Does::Act(_))) {
            let s = (t.schema)();
            assert!(s["properties"]["flow"].is_object(), "{}", t.name);
            let required = s["required"].as_array().cloned().unwrap_or_default();
            assert!(!required.contains(&json!("flow")) && !required.contains(&json!("run")), "{}", t.name);
        }
    }

    #[test]
    fn parts_written_before_a_failed_save_are_put_back() {
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

    #[test]
    fn a_project_root_is_an_existing_absolute_directory() {
        let root = tempfile::tempdir().unwrap();
        let at = root.path().to_str().unwrap();
        assert_eq!(project_root(&format!(" {at}/ ")).unwrap(), at);
        assert!(project_root("relative").is_err());
        assert!(project_root(&format!("{at}/missing")).is_err());
    }
}
