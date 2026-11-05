//! `flow`: the flow module with two demonstration flows (xops-01), for the LIVE suites and for
//! trying the flow space by hand.
//!
//! | flow | shows |
//! |---|---|
//! | `demo-linear` | program → session → confirm: the shortest run that touches each kind of work |
//! | `demo-review` | a program; a `foreach` session station over its list (two tasks); a gate-only checkpoint in parallel, joined later; a station `when` skips (input `mode`, `manual` by default); a `confirm` by `reviewer`; a closing station with a session task and an `external` one; `rework_to`, `on_unmet` |
//!
//! Session stations replay `flow-<station>` (`tests/live/fixtures/recordings/flow-analyze`,
//! `flow-report`): each calls `flow_submit` with a fixed output and ends its turn.

use std::sync::Arc;

use harnix_flow::{
    Event, FileStore, Flow, GateResult, Gates, Hooks, MemPacks, Programs, RunRecord, Tools,
    WorkItem,
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
    executor: {kind: session, scene: flow-chat}
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
hooks: [{on: run.finished, action: notify}]
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
    executor: {kind: session, scene: flow-chat}
    skill: analyze
    output: {type: object, required: [findings], properties: {findings: {type: array}}}
    exit: [{kind: output}]
    retry: {max: 1}
    writes: []
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
    exit: [{kind: output}, {kind: tool, tool: open_high, args: {run: "${run}"}, assert: [{path: total, eq: 0}]}]
    on_unmet: {rework_to: analyze, max: 2}
  - id: review
    title: Review
    needs: [self-check, lint]
    exit: [{kind: confirm, who: reviewer}]
    rework_to: [analyze, lint]
  - id: report
    title: Report and sign
    tasks:
      - {id: summary, executor: {kind: session, scene: flow-chat}, skill: report}
      - {id: sign, executor: {kind: external, who: reviewer}}
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

/// What the flow module is registered with in `harnix-testapp`.
pub fn module() -> Flow {
    Flow::builder()
        .store_with(|dir| FileStore::new(dir.join("runs")))
        .packs(
            MemPacks::new()
                .flow(DEMO_LINEAR)
                .flow(DEMO_REVIEW)
                .skill("analyze", SKILL_ANALYZE)
                .skill("report", SKILL_REPORT)
                .step("collect")
                .step("self-check")
                .gate("lint")
                .tool("open_high")
                .action("notify"),
        )
        .scene(Arc::new(scene::scene::chat::ChatScene))
        .programs(DemoPrograms)
        .gates(DemoGates)
        .tools(DemoTools)
        .hooks(DemoHooks)
        .recording(|w| match w {
            WorkItem::Task { station, .. } => Some(format!("flow-{station}")),
            _ => None,
        })
        .build()
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

struct DemoHooks;

#[harnix::async_trait]
impl Hooks for DemoHooks {
    async fn fire(&self, action: &str, event: &Event) {
        eprintln!("flow hook {action}: {} {}", event.kind, event.run);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use harnix_flow::{validate_yaml, Packs};

    #[test]
    fn the_demo_flows_are_valid() {
        let packs = MemPacks::new()
            .skill("analyze", SKILL_ANALYZE)
            .skill("report", SKILL_REPORT)
            .scene("flow-chat")
            .step("collect")
            .step("self-check")
            .gate("lint")
            .tool("open_high")
            .action("notify");
        for yaml in [DEMO_LINEAR, DEMO_REVIEW] {
            let problems = validate_yaml(yaml, &packs as &dyn Packs);
            assert!(problems.is_empty(), "{problems:?}");
        }
    }
}
