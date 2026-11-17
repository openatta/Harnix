//! `flow`: the flow module with two demonstration flows (xops-01), for the LIVE suites and for
//! trying the flow space by hand.
//!
//! | flow | shows |
//! |---|---|
//! | `demo-linear` | program → session → confirm: the shortest run that touches each kind of work |
//! | `demo-review` | a program; a `foreach` session station over its list (two tasks); a gate-only checkpoint in parallel, joined later; a station `when` skips (input `mode`, `manual` by default); a `confirm` by `reviewer`; a closing station with a session task and an `external` one; `rework_to`, `on_unmet` |
//!
//! | `demo-propose` | a session that tries a write its station does not allow (`guard: strict`), proposes a rework, hands in |
//! | `demo-shared` | two stations, one after the other, whose tasks share the run's session (`per_run`) |
//! | `demo-placed` | xops-02: a station on an in-process template's instance in the `fleet` space with a model of its own, one on a spawned template's instance — both handing in by structured output — and a confirmation |
//! | `demo-guarded` | xops-02: a session on a template's instance tries a write its station does not allow (`guard: strict`: refused by its permission rules), then hands in |
//! | `demo-misplaced` | xops-02: a station whose template does not exist; the task fails, saying why |
//! | `demo-bypassed` | xops-02: like `demo-guarded`, on an instance (`bypass-box`) whose settings a LIVE suite sets to `bypassPermissions` — the session asks for a mode that keeps its rules |
//! | `demo-main` | xops-03: an external task, then two session stations without a scene — turns of the run's main session; the first tries a write no permission allows (`guard: strict`), the second one its permission allows — then a confirmation |
//! | `demo-agent` | xops-03: a sub-agent station; the main session is on `flow-harnix` (it has the `Agent` tool), in a directory of the run's |
//! | `demo-asked` | xops-04: a main-session station whose policy asks a person before the write; a rule; a tool gate and a command gate |
//! | `demo-task` | a task in a project (`guard: auto`): a session writes `notes.txt` into the project with no one asked, then a command gate finds it there |
//! | `demo-outside` / `demo-outside-strict` | a write outside the project: refused under `auto`, asked of a person under `strict` |
//! | `demo-reserved` | a write into the project's `.harnix/` under an `allow` over the whole workspace: refused, the directory is the application's |
//!
//! The pack also has (xops-04) the rule `review-rules`, the policy `ask-todo`, and the gates `lint`
//! (the application's own check), `open-high` (a tool), `self-test` and `task-check` (commands:
//! command gates are on here).
//!
//! The flow module has two templates of its own for those (`flow.local` in process, `flow.remote`
//! spawned): their engines run Agix's harnix scene, and a session with an `output_schema` always
//! has `StructuredOutput` there.
//!
//! Session stations replay `flow-<station>` (`tests/live/fixtures/recordings/flow-analyze`,
//! `flow-report`, `flow-propose`, `flow-write`, `flow-outside`): each calls `flow_submit` with a
//! fixed output and ends its turn; `flow-propose` first calls `TodoWrite` — which this application
//! guards as a write — and `flow_propose`; `flow-write` first writes `notes.txt` (relative: into the
//! project), `flow-outside` first writes `/tmp/harnix-flow-outside-probe.txt`, `flow-reserved`
//! first writes `.harnix/notes.md` (relative: into the project's data directory). A task with a
//! session key replays `flow-shared` instead: one turn per task,
//! each handing in its own step. A run's main session replays `flow-main` (`demo-main`) or
//! `flow-agent` (`demo-agent`).


use harnix::prelude::*;
use harnix_ext_flow::{
    FileStore, Flow, GateResult, Gates, MemPacks, Programs, RunRecord, Tools, WorkItem,
};
use serde_json::{json, Value};

pub const DEMO_LINEAR: &str = r#"name: demo-linear
title: "Demo: linear"
description: A program collects, a session analyzes, a person approves.
stations:
  - id: collect
    title: Collect the modules
    executor: {kind: program, step: collect}
    output: {type: object, required: [modules], properties: {modules: {type: array}}}
    exit: [{kind: output}]
  - id: analyze
    title: Analyze
    executor: {kind: session, scene: flow-harnix}
    skill: analyze
    output: {type: object, required: [findings], properties: {findings: {type: array}}}
    exit: [{kind: output}]
  - id: approve
    title: Approve
    exit: [{kind: confirm}]
"#;

pub const DEMO_REVIEW: &str = r#"name: demo-review
title: "Demo: code review"
description: Every kind of station the flow module has, in one small review.
inputs:
  mode: {schema: {enum: [manual, auto]}, default: manual}
rework_limit: 5
rules: [review-rules]
stations:
  - id: collect
    title: Collect the modules
    executor: {kind: program, step: collect}
    output: {type: object, required: [modules], properties: {modules: {type: array}}}
    exit: [{kind: output}]
  - id: analyze
    title: Analyze each module
    foreach: ${stations.collect.output.modules}
    max_parallel: 2
    executor: {kind: session, scene: flow-harnix}
    skill: analyze
    output: {type: object, required: [findings], properties: {findings: {type: array}}}
    exit: [{kind: output}]
    retry: {max: 1}
  - id: lint
    title: Lint
    needs: [collect]
    exit: [{kind: gate, gate: lint}]
  - id: self-check
    title: Self-check
    needs: [analyze]
    when: [{path: inputs.mode, eq: auto}]
    executor: {kind: program, step: self-check}
    output: {type: object}
    exit: [{kind: output}, {kind: gate, gate: open-high}]
    on_unmet: {rework_to: analyze, max: 2}
  - id: review
    title: Review
    needs: [self-check, lint]
    exit: [{kind: confirm, who: reviewer}]
    rework_to: [analyze, lint]
  - id: report
    title: Report and sign
    tasks:
      - {id: summary, executor: {kind: session, scene: flow-harnix}, skill: report}
      - {id: sign, executor: {kind: external, who: reviewer}}
    output: {type: object}
    exit: [{kind: output}]
"#;

pub const DEMO_PROPOSE: &str = r#"name: demo-propose
title: "Demo: a session that proposes"
guard: strict
description: The session tries a write it may not make, proposes a rework, and hands in.
stations:
  - id: propose
    title: Work, and propose
    executor: {kind: session, scene: flow-harnix}
    skill: analyze
    output: {type: object}
    exit: [{kind: output}]
  - id: approve
    title: Approve
    exit: [{kind: confirm}]
    rework_to: [propose]
"#;

pub const DEMO_MAIN: &str = r#"name: demo-main
title: "Demo: the main Agent"
guard: strict
stations:
  - id: wait
    title: Wait for someone
    executor: {kind: external}
    output: {type: object}
    exit: [{kind: output}]
  - id: draft
    title: Draft, in the main session
    executor: {kind: session}
    skill: analyze
    output: {type: object, required: [text], properties: {text: {type: string}}}
    exit: [{kind: output}]
  - id: edit
    title: Edit, in the main session, writing allowed
    executor: {kind: session}
    skill: report
    permissions: [{effect: allow, tools: [TodoWrite]}]
    output: {type: object, required: [edited], properties: {edited: {const: true}}}
    exit: [{kind: output}]
  - id: sign
    title: Sign
    exit: [{kind: confirm}]
"#;

pub const DEMO_AGENT: &str = r#"name: demo-agent
title: "Demo: a sub-agent"
main: {scene: flow-harnix}
stations:
  - id: check
    title: Checked by a sub-agent
    executor: {kind: agent, description: lists the files}
    skill: analyze
    output: {type: object, required: [files], properties: {files: {type: array}}}
    exit: [{kind: output}]
"#;

pub const DEMO_ASKED: &str = r#"name: demo-asked
title: "Demo: a write a person is asked about"
rules: [review-rules]
stations:
  - id: edit
    title: Edit, asking first
    executor: {kind: session}
    skill: report
    policies: [ask-todo]
    output: {type: object, required: [edited], properties: {edited: {const: true}}}
    exit: [{kind: output}, {kind: gate, gate: open-high}]
  - id: test
    title: Self-test
    exit: [{kind: gate, gate: self-test}]
"#;

pub const DEMO_TASK: &str = r#"name: demo-task
title: "Demo: a task in a project"
description: Writes into the project with no one asked, then a command gate checks it there.
stations:
  - id: write
    title: Write the notes
    executor: {kind: session, scene: flow-harnix}
    skill: write-notes
    output: {type: object, required: [written], properties: {written: {const: true}}}
    exit: [{kind: output}]
  - id: check
    title: Checked in the project
    exit: [{kind: gate, gate: task-check}]
"#;

pub const DEMO_OUTSIDE: &str = r#"name: demo-outside
title: "Demo: a write outside the project"
permissions: [{effect: allow, tools: [Write]}]
stations:
  - id: outside
    title: Try outside, then hand in
    executor: {kind: session, scene: flow-harnix}
    skill: write-outside
    output: {type: object, required: [tried], properties: {tried: {const: true}}}
    exit: [{kind: output}]
"#;

pub const DEMO_OUTSIDE_STRICT: &str = r#"name: demo-outside-strict
title: "Demo: a write outside the project, under strict"
guard: strict
permissions: [{effect: allow, tools: [Write]}]
stations:
  - id: outside
    title: Try outside, then hand in
    executor: {kind: session, scene: flow-harnix}
    skill: write-outside
    output: {type: object, required: [tried], properties: {tried: {const: true}}}
    exit: [{kind: output}]
"#;

pub const DEMO_RESERVED: &str = r#"name: demo-reserved
title: "Demo: a write into the project's data directory"
permissions: [{effect: allow, tools: [Write], paths: ['${workspace}/**']}]
stations:
  - id: reserved
    title: Try the data directory, then hand in
    executor: {kind: session, scene: flow-harnix}
    skill: write-reserved
    output: {type: object, required: [tried], properties: {tried: {const: true}}}
    exit: [{kind: output}]
"#;

pub const SKILL_WRITE_NOTES: &str = "---
name: write-notes
description: Write the task's notes into the project.
---

Use the Write tool to create `notes.txt` in the working directory with exactly one line:
`harnix-task-notes`. Then hand in `{\"written\": true}`.
";

pub const SKILL_WRITE_OUTSIDE: &str = "---
name: write-outside
description: Try a write outside the project, then hand in.
---

Use the Write tool once to create `/tmp/harnix-flow-outside-probe.txt` with the line `outside`.
Whatever the answer, then hand in `{\"tried\": true}`.
";

pub const SKILL_WRITE_RESERVED: &str = "---
name: write-reserved
description: Try a write into the project's .harnix directory, then hand in.
---

Use the Write tool once to create `.harnix/notes.md` in the working directory with the line
`reserved`. Whatever the answer, then hand in `{\"tried\": true}`.
";

pub const GATE_TASK_CHECK: &str = "name: task-check
title: The notes are in the project
kind: command
command: 'test -f \"$FLOW_WORKSPACE/notes.txt\" && test -f \"$FLOW_CONTEXT_FILE\" && echo notes found'
";

pub const RULE_REVIEW: &str = "# Review rules

Name the module a finding is about. Never paste a secret into a finding.
";

pub const POLICY_ASK_TODO: &str = "name: ask-todo
title: Ask before the todo list
permissions:
  - {effect: ask, tools: [TodoWrite], message: the todo list is a person's call here}
";

pub const GATE_LINT: &str = "name: lint\ntitle: Lint\ndescription: The application's own lint.\nkind: host\n";
pub const GATE_OPEN_HIGH: &str = "name: open-high
title: No open high findings
kind: tool
tool: open_high
args: {run: \"${run}\"}
assert: [{path: total, eq: 0}]
";
pub const GATE_SELF_TEST: &str = "name: self-test\ntitle: Self-test\nkind: command\ncommand: echo self-test passed\n";

pub const DEMO_SHARED: &str = r#"name: demo-shared
title: "Demo: one session for the run"
stations:
  - id: first
    executor: {kind: session, scene: flow-harnix, session: per_run}
    skill: analyze
    output: {type: object, required: [step], properties: {step: {const: 1}}}
    exit: [{kind: output}]
  - id: second
    executor: {kind: session, scene: flow-harnix, session: per_run}
    skill: report
    output: {type: object, required: [step], properties: {step: {const: 2}}}
    exit: [{kind: output}]
"#;

pub const DEMO_PLACED: &str = r#"name: demo-placed
title: "Demo: sessions placed on template instances"
description: Two sessions on instances of the flow module's templates, handing in by structured output.
stations:
  - id: local
    title: In process, in the fleet space, on a model of its own
    executor: {kind: session, scene: harnix, template: flow.local, params: {name: "local-${run}"}, space: fleet, model: flow-demo-model}
    skill: analyze
    output: {type: object, required: [findings], properties: {findings: {type: array}}}
    exit: [{kind: output}]
  - id: remote
    title: On a spawned daemon
    executor: {kind: session, scene: harnix, template: flow.remote}
    skill: analyze
    output: {type: object, required: [findings], properties: {findings: {type: array}}}
    exit: [{kind: output}]
  - id: approve
    title: Approve
    exit: [{kind: confirm}]
"#;

pub const DEMO_GUARDED: &str = r#"name: demo-guarded
title: "Demo: a write refused on a template instance"
guard: strict
stations:
  - id: guarded
    title: Work without writing
    executor: {kind: session, scene: harnix, template: flow.local}
    skill: analyze
    output: {type: object, required: [done], properties: {done: {const: true}}}
    exit: [{kind: output}]
  - id: approve
    title: Approve
    exit: [{kind: confirm}]
"#;

pub const DEMO_BYPASSED: &str = r#"name: demo-bypassed
title: "Demo: a write refused on an instance that bypasses permissions"
guard: strict
stations:
  - id: bypassed
    title: Work without writing
    executor: {kind: session, scene: harnix, template: flow.local, params: {name: bypass-box}}
    skill: analyze
    output: {type: object, required: [done], properties: {done: {const: true}}}
    exit: [{kind: output}]
"#;

pub const DEMO_MISPLACED: &str = r#"name: demo-misplaced
title: "Demo: a station placed nowhere"
stations:
  - id: nowhere
    executor: {kind: session, scene: harnix, template: flow.nowhere}
    skill: analyze
    output: {type: object}
    exit: [{kind: output}]
"#;

pub const SKILL_ANALYZE: &str = "---
name: analyze
description: Look at one module and list what is wrong with it.
---

Read the module named in your item. List each problem you find as a finding with a `severity`
(`high`, `medium` or `low`) and a one-line `text`. No problems is a valid answer: an empty list.
";

pub const SKILL_REPORT: &str = "---
name: report
description: Summarise a review for the people who sign it.
---

Read the findings handed in at the `analyze` station (`flow_state` shows the run). Write a short
summary: how many findings of each severity, and the one that matters most.
";

/// The engine of the flow module's templates: Agix's harnix scene.
fn template_engine() -> EngineProfile {
    EngineProfile::new().scene("harnix")
}

/// What the flow module is registered with in `harnix-testapp`.
pub fn module() -> Flow {
    Flow::builder()
        .store_with(|dir| FileStore::new(dir.join("runs")))
        .template(
            InstanceTemplate::in_process("local", template_engine()),
        )
        .template(
            InstanceTemplate::spawn("remote", template_engine()),
        )
        .packs_with(|dir| harnix_ext_flow::DirPacks::writable(dir.join("packs")).with_builtin(packs()))
        .programs(DemoPrograms)
        .gates(DemoGates)
        .tools(DemoTools)
        .command_gates(true)
        .guarded_tools(
            harnix_ext_flow::module::DEFAULT_GUARDED
                .iter()
                .copied()
                .chain(["TodoWrite"]),
        )
        .recording(|w| match w {
            WorkItem::Task {
                session_key: Some(_),
                ..
            } => Some("flow-shared".into()),
            WorkItem::Task { station, .. } => Some(format!("flow-{station}")),
            _ => None,
        })
        .main_recording(|rec: &RunRecord| match rec.flow.as_str() {
            "demo-main" => Some("flow-main".into()),
            "demo-agent" => Some("flow-agent".into()),
            "demo-asked" => Some("flow-asked".into()),
            _ => None,
        })
        .build()
}

/// The flows and parts built in; saved ones go over them in the module's data directory.
fn packs() -> MemPacks {
    parts(MemPacks::new())
        .flow(DEMO_LINEAR)
        .flow(DEMO_REVIEW)
        .flow(DEMO_PROPOSE)
        .flow(DEMO_SHARED)
        .flow(DEMO_PLACED)
        .flow(DEMO_GUARDED)
        .flow(DEMO_MISPLACED)
        .flow(DEMO_BYPASSED)
        .flow(DEMO_MAIN)
        .flow(DEMO_AGENT)
        .flow(DEMO_ASKED)
        .flow(DEMO_TASK)
        .flow(DEMO_OUTSIDE)
        .flow(DEMO_OUTSIDE_STRICT)
        .flow(DEMO_RESERVED)
}

/// The skills, rules, policies and gates, and the catalog names they use.
fn parts(p: MemPacks) -> MemPacks {
    p.skill("analyze", SKILL_ANALYZE)
        .skill("report", SKILL_REPORT)
        .skill("write-notes", SKILL_WRITE_NOTES)
        .skill("write-outside", SKILL_WRITE_OUTSIDE)
        .skill("write-reserved", SKILL_WRITE_RESERVED)
        .rule("review-rules", RULE_REVIEW)
        .policy(POLICY_ASK_TODO)
        .gate_def(GATE_LINT)
        .gate_def(GATE_OPEN_HIGH)
        .gate_def(GATE_SELF_TEST)
        .gate_def(GATE_TASK_CHECK)
        .scene("harnix")
        .step("collect")
        .step("self-check")
        .check("lint")
        .tool("open_high")
}

struct DemoPrograms;

#[harnix::async_trait]
impl Programs for DemoPrograms {
    async fn run(
        &self,
        step: &str,
        _run: &RunRecord,
        _item: Option<&Value>,
    ) -> Result<Value, String> {
        match step {
            "collect" => Ok(json!({"modules": ["core", "ui"]})),
            "self-check" => Ok(json!({"checked": true})),
            other => Err(format!("no step {other}")),
        }
    }
}

struct DemoGates;

#[harnix::async_trait]
impl Gates for DemoGates {
    async fn run(&self, _run: &RunRecord, _station: &str, gate: &str) -> GateResult {
        GateResult {
            pass: gate == "lint",
            detail: format!("{gate}: clean"),
        }
    }
}

struct DemoTools;

#[harnix::async_trait]
impl Tools for DemoTools {
    async fn call(&self, tool: &str, _args: &Value) -> Result<Value, String> {
        match tool {
            "open_high" => Ok(json!({"total": 0})),
            other => Err(format!("no tool {other}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use harnix_ext_flow::{validate_yaml, Packs};

    #[test]
    fn the_demo_flows_are_valid() {
        let packs = parts(MemPacks::new())
            .main_scene("flow-harnix")
            .command_gates();
        for yaml in [
            DEMO_LINEAR,
            DEMO_REVIEW,
            DEMO_PROPOSE,
            DEMO_SHARED,
            DEMO_PLACED,
            DEMO_GUARDED,
            DEMO_MISPLACED,
            DEMO_BYPASSED,
            DEMO_MAIN,
            DEMO_AGENT,
            DEMO_ASKED,
            DEMO_TASK,
            DEMO_OUTSIDE,
            DEMO_OUTSIDE_STRICT,
            DEMO_RESERVED,
        ] {
            let problems = validate_yaml(yaml, &packs as &dyn Packs);
            assert!(problems.is_empty(), "{problems:?}");
        }
    }
}
