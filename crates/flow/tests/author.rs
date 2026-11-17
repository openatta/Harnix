//! The authoring tools (`harnix_flow::author`) over the real kernel, a memory store, writable memory
//! packs and a runner whose sessions `ScriptedHost` plays: each tool, the flow or run a call acts
//! on (named, or the host's current one), a conversation bound to its flow, a project's own packs,
//! and the authorizer asked before each kind of thing a tool does.

use std::collections::{BTreeSet, HashMap};
use std::path::Path;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use harnix_flow::author::*;
use harnix_flow::dryrun::{Sandbox, SandboxConfig};
use harnix_flow::runner::{CatalogPacks, Runner, RunnerConfig, ScriptedHost, SessionId, ToolReply};
use harnix_flow::*;
use serde_json::{json, Value};

const TWO: &str = "name: two\ntitle: Two stops\nstations:\n  - {id: a, exit: [{kind: confirm}]}\n  - {id: b, rework_to: [a], exit: [{kind: confirm}]}\n";
const OTHER: &str = "name: other\nstations:\n  - {id: x, exit: [{kind: confirm}]}\n";

/// A host keeping each conversation's current flow and project in memory; what it is told, kept.
#[derive(Default)]
struct Convo {
    current: Mutex<HashMap<String, String>>,
    projects: Mutex<HashMap<String, String>>,
    runs: Mutex<HashMap<String, String>>,
    pinned: bool,
    notices: Mutex<Vec<Notice>>,
}

#[async_trait]
impl AuthorHost for Convo {
    async fn current(&self, session: &SessionId) -> Option<String> {
        self.current.lock().unwrap().get(session).cloned()
    }
    async fn project(&self, session: &SessionId) -> Option<String> {
        self.projects.lock().unwrap().get(session).cloned()
    }
    fn pinned(&self, _: &SessionId) -> bool {
        self.pinned
    }
    async fn current_run(&self, session: &SessionId) -> Option<String> {
        self.runs.lock().unwrap().get(session).cloned()
    }
    async fn set_project(&self, session: &SessionId, root: &str) {
        self.projects.lock().unwrap().insert(session.clone(), root.to_string());
    }
    async fn created(&self, session: &SessionId, flow: &str) -> Option<String> {
        if session == "busy" {
            return Some("this conversation keeps its flow".into());
        }
        self.current.lock().unwrap().insert(session.clone(), flow.to_string());
        None
    }
    async fn notice(&self, n: Notice) {
        self.notices.lock().unwrap().push(n);
    }
}

impl Convo {
    fn set(&self, session: &str, flow: &str) {
        self.current.lock().unwrap().insert(session.into(), flow.into());
    }
    fn notices(&self) -> Vec<Notice> {
        std::mem::take(&mut *self.notices.lock().unwrap())
    }
}

/// Refuses the operations named.
struct Refuse(BTreeSet<&'static str>);

impl Authorizer for Refuse {
    fn check(&self, op: &Op, _: &Actor, _: Option<&RunRecord>) -> std::result::Result<(), String> {
        if self.0.contains(op.name()) {
            Err(format!("no {}", op.name()))
        } else {
            Ok(())
        }
    }
}

struct Bench {
    tools: AuthorTools,
    /// Held as a host holds it: the tools hold it weakly.
    _rn: Arc<Runner>,
    host: Arc<Convo>,
    scripted: Arc<ScriptedHost>,
    engine: Arc<Engine>,
    layout: ProjectLayout,
    _data: tempfile::TempDir,
}

fn bench(pinned: bool, refuse: &[&'static str]) -> Bench {
    let data = tempfile::tempdir().unwrap();
    let cfg = RunnerConfig { lease_ms: 60_000, guarded: vec![], ..Default::default() };
    let base = MemPacks::new().flow(TWO).flow(OTHER).skill("look", "Look.").writable();
    let packs = Arc::new(CatalogPacks::over(&cfg, vec![], Arc::new(base)));
    let layout = ProjectLayout::new(".harnix", data.path().join("worktrees"), "harnix/flow");
    packs.set_layout(layout.clone());
    let mut engine = Engine::new(Arc::new(MemStore::new()), packs.clone()).with_layout(layout.clone());
    if !refuse.is_empty() {
        engine = engine.with_authorizer(Arc::new(Refuse(refuse.iter().copied().collect())));
    }
    let engine = Arc::new(engine);
    let scripted = ScriptedHost::new();
    let rn = Runner::new(engine.clone(), scripted.clone(), cfg);
    let sandbox = Arc::new(Sandbox::new(packs, SandboxConfig { guarded: vec![], submit_scenes: vec![], main_scene: None }));
    let host = Arc::new(Convo { pinned, ..Default::default() });
    let tools = AuthorTools::new(&rn, sandbox, host.clone());
    Bench { tools, _rn: rn, host, scripted, engine, layout, _data: data }
}

fn me() -> Actor {
    Actor::new(CONVERSATION_KIND, "main/s1")
}

impl Bench {
    async fn call(&self, session: &str, tool: &str, input: Value) -> ToolReply {
        self.tools.call(&session.to_string(), tool, input, &me()).await
    }
    async fn ok(&self, session: &str, tool: &str, input: Value) -> Value {
        let r = self.call(session, tool, input).await;
        assert!(!r.is_error, "{tool}: {}", r.text);
        serde_json::from_str(&r.text).unwrap_or(Value::String(r.text))
    }
    async fn err(&self, session: &str, tool: &str, input: Value) -> String {
        let r = self.call(session, tool, input).await;
        assert!(r.is_error, "{tool} should have been refused: {}", r.text);
        r.text
    }
}

#[test]
fn the_specs_are_the_tables() {
    let b = bench(false, &[]);
    assert_eq!(b.tools.specs(), tool_specs().as_slice());
    let names: Vec<&str> = b.tools.specs().iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names.len(), 22);
    assert!(RUN_TOOLS.iter().all(|t| names.contains(t)));
    assert_eq!(names[0], TOOL_LIST);
}

#[tokio::test]
async fn reading_the_flows_the_schemas_and_a_text() {
    let b = bench(false, &[]);
    let list = b.ok("s1", TOOL_LIST, json!({})).await;
    let names: Vec<&str> = list["flows"].as_array().unwrap().iter().map(|f| f["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["other", "two"]);
    assert!(list["current"].is_null() && list["project"].is_null() && list["writable"] == true);
    b.host.set("s1", "two");
    assert_eq!(b.ok("s1", TOOL_LIST, json!({})).await["current"], "two");
    // A current flow that is gone is no current flow.
    b.host.set("s2", "gone");
    assert!(b.ok("s2", TOOL_LIST, json!({})).await["current"].is_null());

    let schema = b.ok("s1", TOOL_SCHEMA, json!({})).await;
    assert!(schema["definition"].is_object() && schema["parts"]["policy"].is_object());
    assert_eq!(schema["catalog"]["parts"]["skill"], json!(["look"]));
    let v = b.ok("s1", TOOL_VALIDATE, json!({"yaml": TWO})).await;
    assert_eq!(v["problems"], json!([]));
    let v = b.ok("s1", TOOL_VALIDATE, json!({"yaml": "name: bad\nstations: [{id: a, skill: nowhere, exit: [{kind: output}]}]"})).await;
    assert!(!v["problems"].as_array().unwrap().is_empty());
    assert!(b.err("s1", TOOL_VALIDATE, json!({})).await.contains("`yaml`"));

    let guide = b.call("s1", TOOL_GUIDE, json!({"lang": "zh-CN"})).await;
    assert!(!guide.is_error && guide.text == GUIDE_ZH);
    assert_eq!(b.call("s1", TOOL_GUIDE, json!({})).await.text, GUIDE_EN);
    assert!(b.err("s1", "flow_nope", json!({})).await.contains("no authoring tool"));
}

#[tokio::test]
async fn a_tool_acts_on_the_flow_named_and_else_on_the_current_one() {
    let b = bench(false, &[]);
    // Neither: refused, saying how to name one.
    assert_eq!(b.err("s1", TOOL_SHOW, json!({})).await, NO_CURRENT);
    // Named: that one, whatever is current.
    assert_eq!(b.ok("s1", TOOL_SHOW, json!({"flow": "other"})).await["name"], "other");
    b.host.set("s1", "two");
    assert_eq!(b.ok("s1", TOOL_SHOW, json!({"flow": "other"})).await["name"], "other");
    // Absent: the current one.
    let shown = b.ok("s1", TOOL_SHOW, json!({})).await;
    assert_eq!((shown["name"].as_str(), shown["title"].as_str()), (Some("two"), Some("Two stops")));
    assert_eq!(shown["stations"][1]["needs"], json!(["a"]));
    assert!(shown["yaml"].as_str().unwrap().contains("Two stops"));
    // A flow that is not there.
    assert!(b.err("s1", TOOL_SHOW, json!({"flow": "nope"})).await.contains("gone"));
    assert!(b.err("s1", TOOL_UPDATE, json!({"flow": "nope", "yaml": OTHER})).await.contains("no flow `nope`"));
}

#[tokio::test]
async fn a_pinned_conversation_acts_only_on_its_current_flow() {
    let b = bench(true, &[]);
    assert_eq!(b.err("s1", TOOL_SHOW, json!({"flow": "two"})).await, NO_CURRENT, "nothing current, nothing named works");
    b.host.set("s1", "two");
    assert_eq!(b.ok("s1", TOOL_SHOW, json!({"flow": "two"})).await["name"], "two");
    assert_eq!(b.ok("s1", TOOL_SHOW, json!({})).await["name"], "two");
    let changed = b.err("s1", TOOL_DELETE, json!({"flow": "other"})).await;
    assert!(changed.contains("changed to `two`") && changed.contains("ask the user again"), "{changed}");
    assert!(b.engine.packs().flow("other").is_some(), "nothing was deleted");
    // A run of another flow is out of its reach.
    let o = b.engine.create(CreateIn { flow: "other".into(), subject: "s".into(), ..Default::default() }, Actor::device("d")).await.unwrap();
    let run = o.value["run"].as_str().unwrap().to_string();
    let refused = b.err("s1", TOOL_PAUSE, json!({"run": run})).await;
    assert!(refused.contains("not of two"), "{refused}");
    // Creating names its own flow: no selection needed; one already there is told how to change it.
    let exists = b.err("s3", TOOL_CREATE, json!({"yaml": OTHER})).await;
    assert!(exists.contains("ask the user to select it"), "{exists}");
}

#[tokio::test]
async fn creating_updating_and_deleting_a_flow_with_its_parts() {
    let b = bench(false, &[]);
    let yaml = "name: fresh\nstations: [{id: a, exit: [{kind: confirm}]}, {id: b, rules: [house], exit: [{kind: confirm}]}]";
    // A text with problems (a rule not there) saves nothing.
    let bad = b.ok("s1", TOOL_CREATE, json!({"yaml": yaml})).await;
    assert_eq!(bad["saved"], false, "{bad}");
    assert!(b.engine.packs().flow("fresh").is_none());
    // With the part it needs, both are saved.
    let parts = json!([{"kind": "rule", "name": "house", "text": "# House\nKeep it tidy."}]);
    let made = b.ok("s1", TOOL_CREATE, json!({"yaml": yaml, "parts": parts})).await;
    assert_eq!((made["saved"].as_bool(), made["current"].as_str()), (Some(true), Some("fresh")), "{made}");
    assert_eq!(made["parts"], json!([{"kind": "rule", "name": "house"}]));
    assert!(b.engine.packs().rule("house").is_some());
    assert!(b.host.notices().contains(&Notice::Packs));
    // The host may keep its conversation where it was, and says why.
    let kept = b.ok("busy", TOOL_CREATE, json!({"yaml": "name: kept\nstations: [{id: a, exit: [{kind: confirm}]}]"})).await;
    assert!(kept["current"].is_null() && kept["note"] == "this conversation keeps its flow", "{kept}");

    // Updated: the current one, by default; a text naming another flow is refused.
    let renamed = b.err("s1", TOOL_UPDATE, json!({"yaml": OTHER})).await;
    assert!(renamed.contains("defines flow `other`"), "{renamed}");
    assert!(b.err("s1", TOOL_UPDATE, json!({})).await.contains("give `yaml`, `parts`, or both"));
    let up = b.ok("s1", TOOL_UPDATE, json!({"parts": [{"kind": "rule", "name": "house", "text": "# House\nTidier."}]})).await;
    assert_eq!(up["saved"], true);
    assert!(b.engine.packs().rule("house").unwrap().0.contains("Tidier"));

    let set = b.ok("s1", TOOL_PART_SET, json!({"kind": "skill", "name": "peek", "text": "Peek."})).await;
    assert_eq!(set["name"], "peek");
    assert!(b.err("s1", TOOL_PART_SET, json!({"kind": "rule", "name": "x"})).await.contains("`kind`"));
    b.ok("s1", TOOL_PART_DELETE, json!({"kind": "skill", "name": "peek"})).await;
    assert!(b.engine.packs().skill("peek").is_none());

    b.host.notices();
    b.ok("s1", TOOL_DELETE, json!({})).await;
    assert!(b.engine.packs().flow("fresh").is_none());
    assert_eq!(b.host.notices(), [Notice::Packs, Notice::Deleted { flow: "fresh".into(), gone: true }]);
}

#[tokio::test]
async fn a_dry_run_and_a_real_run_of_the_flow_named() {
    let b = bench(false, &[]);
    let dry = b.ok("s1", TOOL_DRY_RUN, json!({"flow": "two"})).await;
    let run = dry["run"].as_str().unwrap().to_string();
    assert!(run.starts_with("dry-two-"), "{dry}");
    assert_eq!(dry["stations"]["a"], "working");
    assert_eq!(b.host.notices(), [Notice::DryRun { run: run.clone() }]);
    assert!(b.err("s1", TOOL_DRY_RUN, json!({"flow": "two", "mocks": 3})).await.contains("`mocks`"));
    // A dry run is not worked with the run tools.
    assert!(b.err("s1", TOOL_PAUSE, json!({"run": run})).await.contains("flow_dry_run"));

    assert!(b.err("s1", TOOL_START, json!({"flow": "two"})).await.contains("`subject`"));
    let started = b.ok("s1", TOOL_START, json!({"flow": "two", "subject": "the repo", "title": "First"})).await;
    let run = started["run"].as_str().unwrap().to_string();
    let rec = b.engine.record(&run).await.unwrap();
    assert_eq!((rec.flow.as_str(), rec.title.as_str()), ("two", "First"));
    let runs = b.ok("s1", TOOL_RUNS, json!({"flow": "two"})).await;
    assert_eq!(runs["runs"][0]["run"], run.as_str());
    assert!(b.ok("s1", TOOL_RUNS, json!({"flow": "other"})).await["runs"].as_array().unwrap().is_empty());
    assert!(b.scripted.opened().is_empty(), "confirm stations open no session");
}

#[tokio::test]
async fn working_a_run_from_the_conversation() {
    let b = bench(false, &[]);
    let run = b.ok("s1", TOOL_START, json!({"flow": "two", "subject": "s"})).await["run"].as_str().unwrap().to_string();
    // Named, or the host's current run; neither, refused.
    assert!(b.err("s1", TOOL_RUN, json!({})).await.contains("name the run in `run`"));
    let one = b.ok("s1", TOOL_RUN, json!({"run": run})).await;
    assert_eq!((one["flow"].as_str(), one["stations"][0]["status"].as_str()), (Some("two"), Some("working")));
    b.host.runs.lock().unwrap().insert("in-run".into(), run.clone());
    assert_eq!(b.ok("in-run", TOOL_RUN, json!({})).await["run"], run.as_str());
    // With a flow named, it must be a run of it.
    assert!(b.err("s1", TOOL_RUN, json!({"run": run, "flow": "other"})).await.contains("not of other"));

    let phase = |v: &Value| v["phase"]["state"].as_str().map(String::from);
    assert_eq!(phase(&b.ok("s1", TOOL_PAUSE, json!({"run": run})).await).as_deref(), Some("paused"));
    assert_eq!(phase(&b.ok("s1", TOOL_RESUME, json!({"run": run})).await).as_deref(), Some("waiting"));
    b.ok("s1", TOOL_CONFIRM, json!({"run": run, "station": "a", "pass": true})).await;
    b.ok("s1", TOOL_REWORK, json!({"run": run, "to": "a", "reason": "again"})).await;
    b.ok("s1", TOOL_FORCE, json!({"run": run, "station": "a", "reason": "fine"})).await;
    assert!(b.err("s1", TOOL_RETRY, json!({"run": run, "station": "b", "task": "b"})).await.starts_with("not done: "));
    assert!(b.err("s1", TOOL_RELEASE, json!({"run": run, "station": "b"})).await.starts_with("not done: "));
    b.ok("in-run", TOOL_CANCEL, json!({"reason": "enough"})).await;
    let rec = b.engine.record(&run).await.unwrap();
    assert_eq!(rec.status, RunStatus::Cancelled);
    // Recorded as who the host said acted.
    let history = b.engine.store().history(&run).await.unwrap();
    assert!(history.iter().any(|h| h.by == me()), "{history:?}");
}

#[tokio::test]
async fn a_flow_written_for_a_project_is_saved_in_it_once_it_is_trusted() {
    let b = bench(false, &[]);
    let root = tempfile::tempdir().unwrap();
    let at = root.path().to_str().unwrap().to_string();
    let dir = project_pack_dir(root.path(), &b.layout);
    let yaml = "name: here\nstations: [{id: a, exit: [{kind: confirm}]}]";
    let refused = b.err("s1", TOOL_CREATE, json!({"yaml": yaml, "project_root": at})).await;
    assert!(refused.contains("not trusted"), "{refused}");
    assert!(!dir.exists());
    assert!(b.host.projects.lock().unwrap().is_empty(), "refused: the conversation is not moved");
    assert!(b.err("s1", TOOL_CREATE, json!({"yaml": yaml, "project_root": "relative"})).await.contains("absolute"));

    b.scripted.trust(Path::new(&at), true);
    let made = b.ok("s1", TOOL_CREATE, json!({"yaml": yaml, "project_root": at})).await;
    assert_eq!(made["saved"], true);
    assert!(dir.join("here.yaml").is_file());
    assert!(b.engine.packs().flow("here").is_none(), "only the project has it");
    let list = b.ok("s1", TOOL_LIST, json!({})).await;
    assert_eq!(list["project"], at.as_str());
    assert!(list["flows"].as_array().unwrap().iter().any(|f| f["name"] == "here" && f["origin"] == "project"));
    // Its runs start in the project.
    let run = b.ok("s1", TOOL_START, json!({"subject": "s"})).await["run"].as_str().unwrap().to_string();
    assert_eq!(b.engine.record(&run).await.unwrap().project_root.as_deref(), Some(at.as_str()));
}

#[tokio::test]
async fn the_authorizer_is_asked_before_each_kind_of_thing_a_tool_does() {
    let reads = bench(false, &["read_flows", "read_flow", "list_runs", "dry_run"]);
    for tool in [TOOL_LIST, TOOL_SCHEMA] {
        assert_eq!(reads.err("s1", tool, json!({})).await, "denied: no read_flows");
    }
    assert_eq!(reads.err("s1", TOOL_VALIDATE, json!({"yaml": TWO})).await, "denied: no read_flows");
    assert_eq!(reads.err("s1", TOOL_SHOW, json!({"flow": "two"})).await, "denied: no read_flow");
    assert_eq!(reads.err("s1", TOOL_RUNS, json!({"flow": "two"})).await, "denied: no list_runs");
    assert_eq!(reads.err("s1", TOOL_DRY_RUN, json!({"flow": "two"})).await, "denied: no dry_run");
    assert!(reads.host.notices().is_empty(), "no dry run was made");
    // The guide is the library's text, not anyone's data.
    assert!(!reads.call("s1", TOOL_GUIDE, json!({})).await.is_error);

    let runs = bench(false, &["read_run", "pause", "create"]);
    assert_eq!(runs.err("s1", TOOL_START, json!({"flow": "two", "subject": "s"})).await, "not started: denied: no create");
    let o = runs.engine.create(CreateIn { flow: "two".into(), subject: "s".into(), ..Default::default() }, Actor::device("d")).await;
    assert!(o.is_err(), "the engine asks the same authorizer");

    let edits = bench(false, &["edit_flow", "edit_part"]);
    let parts = json!([{"kind": "rule", "name": "house", "text": "# House"}]);
    let yaml = "name: fresh\nstations: [{id: a, rules: [house], exit: [{kind: confirm}]}]";
    assert_eq!(edits.err("s1", TOOL_CREATE, json!({"yaml": yaml, "parts": parts})).await, "denied: no edit_flow");
    assert!(edits.engine.packs().rule("house").is_none(), "no part was written before the flow was refused");
    assert!(edits.err("s1", TOOL_PART_SET, json!({"kind": "skill", "name": "x", "text": "X."})).await.contains("no edit_part"));
    assert!(edits.err("s1", TOOL_DELETE, json!({"flow": "two"})).await.contains("no edit_flow"));
    assert!(edits.engine.packs().flow("two").is_some());
}

#[tokio::test]
async fn reading_and_acting_on_a_run_are_asked_with_the_run() {
    struct OwnRuns;
    impl Authorizer for OwnRuns {
        fn check(&self, op: &Op, by: &Actor, run: Option<&RunRecord>) -> std::result::Result<(), String> {
            match (op, run) {
                // The kind of check a host makes of a session working in a run: its own run only.
                (Op::ReadRun | Op::Pause, Some(r)) if r.subject != by.id => Err(format!("{} is not {}'s", r.id, by.id)),
                _ => Ok(()),
            }
        }
    }
    let b = bench(false, &[]);
    let engine = Arc::new(Engine::new(b.engine.store().clone(), b.engine.packs().clone()).with_authorizer(Arc::new(OwnRuns)));
    let rn = Runner::new(engine.clone(), ScriptedHost::new(), RunnerConfig::default());
    let sandbox = Arc::new(Sandbox::new(engine.packs().clone(), SandboxConfig { guarded: vec![], submit_scenes: vec![], main_scene: None }));
    let tools = AuthorTools::new(&rn, sandbox, b.host.clone());
    let mine = engine.create(CreateIn { flow: "two".into(), subject: "main/s1".into(), ..Default::default() }, Actor::device("d")).await.unwrap();
    let theirs = engine.create(CreateIn { flow: "two".into(), subject: "else".into(), ..Default::default() }, Actor::device("d")).await.unwrap();
    let id = |o: &Outcome| o.value["run"].as_str().unwrap().to_string();
    let call = |tool: &'static str, run: String| {
        let tools = &tools;
        async move { tools.call(&"s1".to_string(), tool, json!({"run": run}), &me()).await }
    };
    assert!(!call(TOOL_RUN, id(&mine)).await.is_error);
    let r = call(TOOL_RUN, id(&theirs)).await;
    assert!(r.is_error && r.text.contains("is not main/s1's"), "{}", r.text);
    assert!(!call(TOOL_PAUSE, id(&mine)).await.is_error);
    let r = call(TOOL_PAUSE, id(&theirs)).await;
    assert!(r.is_error && r.text.starts_with("not done: denied"), "{}", r.text);
}
