//! Writing flows in conversation (xops-02 §4): the `flow-author` scene, with the library's
//! authoring tools (`harnix_flow::author`) and a system prompt block on how to write flows.
//!
//! The flow space's default scene. It wraps a scene the application gives (Agix's harnix by
//! default). What is Harnix's is here: a conversation's **current flow** is its focus — what the
//! person has selected in the panel beside it (device method `focus`), or the flow `flow_create`
//! just made — and a conversation acts on that flow only (`pinned`); the project it works in; and
//! the topics told of what changed (`defs`, `focus`, `dry_runs`).
//!
//! No tool asks the person (harnix-app.md §11): a change is checked, not approved — a
//! definition with problems is not saved. A run tool acts only when the user asks for it, and the
//! run's history records the conversation as who did it.

use std::sync::Arc;

use harnix_ext::engine::{Permission, PromptText, SceneBase, SceneLayer, Tool, ToolCx, ToolError, ToolOutput, ToolSpec as Spec};
use serde_json::Value;

use super::publish;
use super::state::{session_ref, Shared};
use crate::author::{AuthorHost, Notice, CONVERSATION_KIND, RUN_TOOLS};
use crate::kernel::run::Actor;
use crate::runner::{SessionId, ToolSpec as FlowToolSpec};

pub use crate::author::{
    TOOL_CANCEL, TOOL_CONFIRM, TOOL_CREATE, TOOL_DELETE, TOOL_DRY_RUN, TOOL_FORCE, TOOL_GUIDE, TOOL_LIST, TOOL_PART_DELETE,
    TOOL_PART_SET, TOOL_PAUSE, TOOL_RELEASE, TOOL_RESUME, TOOL_RETRY, TOOL_REWORK, TOOL_RUN, TOOL_RUNS, TOOL_SCHEMA,
    TOOL_SHOW, TOOL_START, TOOL_UPDATE, TOOL_VALIDATE,
};

/// The scene's id.
pub const AUTHOR_SCENE: &str = "flow-author";

/// What a tool needing the current flow answers when there is none.
pub const NO_FOCUS: &str = "This conversation works on no flow: ask the user to select the flow \
     in the panel on the right first — its own conversation opens (还没有选择流程：请用户先在右边栏\
     选择要操作的流程，会打开这个流程自己的会话). Creating a new flow with `flow_create` needs no \
     selection.";

/// Every authoring tool, and whether it is read-only, in the order the model is offered them
/// (`harnix_flow::author::tool_specs`).
pub const AUTHOR_TOOLS: [(&str, bool); 22] = [
    (TOOL_LIST, true),
    (TOOL_SHOW, true),
    (TOOL_SCHEMA, true),
    (TOOL_VALIDATE, true),
    (TOOL_CREATE, false),
    (TOOL_UPDATE, false),
    (TOOL_DELETE, false),
    (TOOL_PART_SET, false),
    (TOOL_PART_DELETE, false),
    (TOOL_DRY_RUN, true),
    (TOOL_START, false),
    (TOOL_GUIDE, true),
    (TOOL_RUNS, true),
    (TOOL_RUN, true),
    (TOOL_PAUSE, false),
    (TOOL_RESUME, false),
    (TOOL_CANCEL, false),
    (TOOL_CONFIRM, false),
    (TOOL_FORCE, false),
    (TOOL_REWORK, false),
    (TOOL_RETRY, false),
    (TOOL_RELEASE, false),
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
    for spec in crate::author::tool_specs() {
        layer = layer.tool(Arc::new(AuthorTool::new(spec, sh.clone())));
    }
    layer
        .requires_project(false)
        .interactive(true)
        .auto_name(false)
}

/// A conversation as the authoring tools see it, in Harnix: its focus, its project, the topics.
/// A conversation is named `<instance>/<session_id>`.
pub(crate) struct HarnixAuthorHost {
    sh: Arc<Shared>,
}

impl HarnixAuthorHost {
    pub(crate) fn new(sh: Arc<Shared>) -> Arc<Self> {
        Arc::new(Self { sh })
    }
}

#[async_trait::async_trait]
impl AuthorHost for HarnixAuthorHost {
    async fn current(&self, session: &SessionId) -> Option<String> {
        let s = session_ref(session);
        self.sh.focus(&s.instance, &s.session_id)
    }

    async fn project(&self, session: &SessionId) -> Option<String> {
        let s = session_ref(session);
        self.sh.conversation_project(&s.instance, &s.session_id)
    }

    /// What changes or runs a flow names it, and acts only if it is still the current one: the
    /// call means that flow, not whatever is selected when it runs.
    fn pinned(&self, _: &SessionId) -> bool {
        true
    }

    fn no_current(&self, _: &SessionId) -> String {
        NO_FOCUS.to_string()
    }

    async fn set_project(&self, session: &SessionId, root: &str) {
        let s = session_ref(session);
        self.sh.set_conversation_project(&s.instance, &s.session_id, Some(root.to_string()));
    }

    async fn created(&self, session: &SessionId, flow: &str) -> Option<String> {
        let s = session_ref(session);
        let (instance, id) = (s.instance.as_str(), s.session_id.as_str());
        let note = match self.sh.flow_of_session(instance, id) {
            // The general conversation becomes the new flow's own; the next one asked for is a new
            // general conversation (xops-03 §5).
            Some(own) if own.is_empty() => {
                self.sh.set_flow_session("", None);
                self.sh.set_flow_session(flow, Some(session.clone()));
                None
            }
            // Another flow's conversation stays that flow's.
            Some(own) => Some(format!(
                "this is flow {own}'s conversation: the new flow has its own — the user selects it on the right"
            )),
            None => {
                self.sh.set_focus(instance, id, Some(flow.to_string()));
                None
            }
        };
        publish::publish_focus(&self.sh, instance, id).await;
        note
    }

    async fn notice(&self, notice: Notice) {
        match notice {
            Notice::Packs => publish::publish_defs(&self.sh).await,
            Notice::Deleted { flow, gone } => {
                if gone {
                    self.sh.set_flow_session(&flow, None);
                }
                for (i, s) in self.sh.focused_on(&flow) {
                    if gone {
                        self.sh.set_focus(&i, &s, None);
                    }
                    publish::publish_focus(&self.sh, &i, &s).await;
                }
            }
            Notice::DryRun { run } => publish::publish_dry(&self.sh, Some(&run)).await,
            _ => {}
        }
    }
}

/// One authoring tool as the engine sees it: its spec from the library, its calls routed to the
/// library's `AuthorTools` under the calling conversation's name.
struct AuthorTool {
    told: Spec,
    name: String,
    sh: Arc<Shared>,
}

impl AuthorTool {
    /// What changes nothing is read-only and may run beside other calls.
    fn new(spec: FlowToolSpec, sh: Arc<Shared>) -> Self {
        let mut told = Spec::new(&spec.name, &spec.description, spec.input_schema);
        if spec.read_only {
            told = told.read_only().concurrency_safe();
        }
        Self { told, name: spec.name, sh }
    }
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
    async fn call(&self, cx: ToolCx, input: Value) -> Result<ToolOutput, ToolError> {
        let Some(tools) = self.sh.author.get() else {
            return Ok(ToolOutput::error("the flow module has not started"));
        };
        // The session the call is made in, on the instance its engine carries (the flow space's
        // when an engine does not say).
        let me = cx.session();
        let instance = if me.instance.is_empty() { self.sh.cfg.instance.as_str() } else { me.instance.as_str() };
        let session = format!("{instance}/{}", me.session_id);
        // A run's history records the conversation acting on it; what it saves or starts, the
        // conversation's session.
        let by = if RUN_TOOLS.contains(&self.name.as_str()) {
            Actor::new(CONVERSATION_KIND, session.clone())
        } else {
            Actor::agent(session.clone())
        };
        let reply = tools.call(&session, &self.name, input, &by).await;
        Ok(if reply.is_error { ToolOutput::error(reply.text) } else { ToolOutput::text(reply.text) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_scene_offers_the_librarys_tools_and_none_is_asked_about() {
        let sh = Arc::new(Shared::for_tests());
        let layer = author_scene(SceneBase::builtin("harnix"), sh);
        let p = layer.parts();
        let told: Vec<(&str, bool)> = p.tools.iter().map(|t| (t.spec().name(), t.spec().is_read_only())).collect();
        assert_eq!(told, AUTHOR_TOOLS, "the library's tools, in its order");
        assert_eq!((p.interactive, p.auto_name, p.requires_project), (Some(true), Some(false), Some(false)));
        // None of them asks the person: a change is checked as it runs, not approved.
        for tool in p.tools {
            assert_eq!(tool.permission(&json!({"flow": "f", "run": "r-1"})), Permission::Allow, "{}", tool.spec().name());
        }
    }

    #[tokio::test]
    async fn before_the_extension_starts_a_tool_says_so() {
        let layer = author_scene(SceneBase::builtin("harnix"), Arc::new(Shared::for_tests()));
        let out = layer.parts().tools[0].call(ToolCx::new("nobody", "t"), json!({})).await.unwrap();
        assert!(out.is_error() && out.content().contains("has not started"));
    }

    /// A module with an engine over `MemPacks` and its project packs, a runner and the authoring
    /// tools, before start (no context: a project's trust is what the test sets).
    fn started() -> Arc<Shared> {
        let sh = Arc::new(Shared::for_tests());
        let cfg = crate::runner::RunnerConfig {
            profiles: vec!["flow-harnix".into()],
            guarded: vec![],
            trust: sh.projects.clone(),
            ..Default::default()
        };
        let packs = Arc::new(crate::runner::CatalogPacks::over(
            &cfg,
            vec![],
            Arc::new(crate::MemPacks::new().skill("look", "Look.").writable()),
        ));
        packs.set_layout(super::super::layout(std::path::Path::new("/state/ext/flow/data")));
        let engine = Arc::new(crate::Engine::new(Arc::new(crate::MemStore::new()), packs.clone()));
        let _ = sh.engine.set(engine.clone());
        let runner = crate::runner::Runner::new(engine, super::super::host::HarnixSessionHost::new(sh.clone()), cfg);
        let sandbox = Arc::new(crate::Sandbox::new(packs, Default::default()));
        let _ = sh.author.set(Arc::new(crate::author::AuthorTools::new(&runner, sandbox, HarnixAuthorHost::new(sh.clone()))));
        let _ = sh.runner.set(runner);
        sh
    }

    async fn call(sh: &Arc<Shared>, tool: &str, session: &str, input: Value) -> ToolOutput {
        let spec = crate::author::tool_specs().into_iter().find(|t| t.name == tool).unwrap();
        let cx = ToolCx::new(session, "t1").with_instance("main");
        AuthorTool::new(spec, sh.clone()).call(cx, input).await.unwrap()
    }

    #[tokio::test]
    async fn a_change_acts_only_on_the_selected_flow_and_creating_one_selects_it() {
        let sh = started();
        let yaml = "name: fresh\nstations: [{id: a, exit: [{kind: confirm}]}]";
        // Nothing selected: the person is asked to select one.
        let out = call(&sh, TOOL_UPDATE, "s1", json!({"flow": "fresh", "yaml": yaml})).await;
        assert!(out.is_error() && out.content() == NO_FOCUS, "{}", out.content());
        let out = call(&sh, TOOL_CREATE, "s1", json!({"yaml": yaml})).await;
        assert!(out.content().contains("\"current\": \"fresh\""), "{}", out.content());
        assert_eq!(sh.focus("main", "s1").as_deref(), Some("fresh"));
        // Another flow named than the selected one: nothing done.
        sh.engine().unwrap().packs().save_flow("name: other\nstations: [{id: a, exit: [{kind: confirm}]}]").unwrap();
        let out = call(&sh, TOOL_DELETE, "s1", json!({"flow": "other"})).await;
        assert!(out.is_error() && out.content().contains("changed to `fresh`"), "{}", out.content());
        // Deleted: no longer selected.
        let out = call(&sh, TOOL_DELETE, "s1", json!({"flow": "fresh"})).await;
        assert!(!out.is_error(), "{}", out.content());
        assert_eq!(sh.focus("main", "s1"), None);
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
