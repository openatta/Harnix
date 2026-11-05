//! Structural validation, rule by rule, and the YAML surface.

mod common;

use common::packs;
use harnix_flow::*;

/// The example of the issue (§3), with the `output` contract `report` needs.
const ISSUE_EXAMPLE: &str = r#"
name: demo-review
title: Demo review
inputs:
  mode: {schema: {enum: [manual, auto]}, default: manual}
rework_limit: 5
hooks: [{on: run.finished, action: notify}]
stations:
  - id: collect
    executor: {kind: program, step: collect}
    output: {type: object, required: [modules], properties: {modules: {type: array}}}
    exit: [{kind: output}]
  - id: analyze
    foreach: ${stations.collect.output.modules}
    max_parallel: 2
    executor: {kind: session, scene: chat, session: new}
    skill: analyze
    output: {type: object, required: [findings]}
    exit: [{kind: output}]
    retry: {max: 1}
    writes: []
  - id: lint
    needs: [collect]
    exit: [{kind: gate, gate: lint}]
  - id: self-check
    needs: [analyze]
    when: [{path: inputs.mode, eq: auto}]
    executor: {kind: program, step: check}
    exit: [{kind: tool, tool: count, args: {run: "${run}"}, assert: [{path: total, eq: 0}]}]
    on_unmet: {rework_to: analyze, max: 2}
  - id: review
    needs: [self-check, lint]
    exit: [{kind: confirm, who: reviewer}]
    rework_to: [analyze, lint]
  - id: report
    tasks:
      - {id: summary, executor: {kind: session, scene: chat}, skill: report}
      - {id: sign, executor: {kind: external, who: reviewer}}
    output: {type: object}
    exit: [{kind: output}]
"#;

fn problems(yaml: &str) -> Vec<String> {
    validate_yaml(yaml, &packs())
        .into_iter()
        .map(|p| p.to_string())
        .collect()
}

fn assert_problem(yaml: &str, needle: &str) {
    let ps = problems(yaml);
    assert!(
        ps.iter().any(|p| p.contains(needle)),
        "expected a problem containing {needle:?}, got {ps:#?}"
    );
}

#[test]
fn the_issue_example_is_valid() {
    assert_eq!(problems(ISSUE_EXAMPLE), Vec::<String>::new());
    let def = FlowDef::parse(ISSUE_EXAMPLE).unwrap();
    assert_eq!(def.needs_of("analyze"), vec!["collect"]);
    assert_eq!(def.needs_of("collect"), Vec::<String>::new());
    assert!(matches!(
        def.station("analyze").unwrap().work(),
        Work::Foreach {
            max_parallel: Some(2),
            ..
        }
    ));
    assert!(matches!(def.station("lint").unwrap().work(), Work::None));
    assert!(matches!(def.station("report").unwrap().work(), Work::Tasks(ref t) if t.len() == 2));
    assert_eq!(
        def.descendants("analyze"),
        vec!["self-check", "review", "report"]
    );
}

#[test]
fn without_the_contract_the_issue_example_reports_it() {
    let yaml = ISSUE_EXAMPLE.replace(
        "    output: {type: object}\n    exit: [{kind: output}]\n",
        "    exit: [{kind: output}]\n",
    );
    let ps = validate_yaml(&yaml, &packs());
    // The output exit needs it, and so does the session task (it is what the session hands in).
    assert_eq!(ps.len(), 2, "{ps:#?}");
    assert!(ps
        .iter()
        .all(|p| p.station.as_deref() == Some("report") && p.message.contains("output contract")));
}

// ── xops-02: placement and the relaxed session rule ─────────────────────────

fn session(executor_extra: &str, station_extra: &str) -> String {
    format!(
        "name: p\ninputs: {{who: {{schema: {{type: string}}, default: x}}}}\nstations:\n  - {{id: a, executor: {{kind: session, scene: chat{executor_extra}}}, skill: analyze{station_extra}, exit: [{{kind: confirm}}]}}\n"
    )
}

#[test]
fn a_session_station_needs_an_output_contract_and_any_catalog_scene() {
    assert_problem(&session("", ""), "needs an output contract");
    assert_eq!(
        problems(&session("", ", output: {type: object}")),
        Vec::<String>::new()
    );
}

#[test]
fn placement_fields_parse_and_are_checked_against_what_the_catalog_says() {
    let ok = session(
        ", model: m1, provider: acme, template: app.box, params: {name: 'box-${run}', who: '${inputs.who}'}, space: dev",
        ", output: {type: object}",
    );
    let def = FlowDef::parse(&ok).unwrap();
    match def.stations[0].executor.as_ref().unwrap() {
        Executor::Session {
            model,
            template,
            params,
            space,
            ..
        } => {
            assert_eq!(model.as_deref(), Some("m1"));
            assert_eq!(template.as_deref(), Some("app.box"));
            assert_eq!(params["name"], "box-${run}");
            assert_eq!(space.as_deref(), Some("dev"));
        }
        e => panic!("{e:?}"),
    }
    // An empty catalog part checks nothing.
    assert_eq!(problems(&ok), Vec::<String>::new());
    // A filled one checks its names.
    let strict = packs().template("app.other").space("prod").model("acme/m2");
    let ps: Vec<String> = validate_yaml(&ok, &strict)
        .iter()
        .map(|p| p.to_string())
        .collect();
    for want in ["template app.box", "space dev", "model acme/m1"] {
        assert!(ps.iter().any(|p| p.contains(want)), "{want}: {ps:#?}");
    }
    let known = packs().template("app.box").space("dev").model("acme/m1");
    assert!(validate_yaml(&ok, &known).is_empty());
    // A bare model name matches a `provider/model` entry when no provider is named.
    let bare = session(", model: m1", ", output: {type: object}");
    assert!(validate_yaml(&bare, &packs().model("acme/m1")).is_empty());
    let inst = session(", instance: side", ", output: {type: object}");
    assert!(validate_yaml(&inst, &packs().instance("side")).is_empty());
    assert!(!validate_yaml(&inst, &packs().instance("main")).is_empty());
}

#[test]
fn placement_combinations_and_references() {
    let o = ", output: {type: object}";
    assert_problem(&session(", instance: i, template: t", o), "not both");
    assert_problem(
        &session(", params: {name: x}", o),
        "params only go with a template",
    );
    assert_problem(
        &session(", template: t, params: {name: '${inputs.nope}'}", o),
        "no input nope",
    );
    assert_problem(&session(", model: ''", o), "model is empty");
}

#[test]
fn unknown_fields_are_rejected() {
    assert_problem(
        "name: x\nstations: [{id: a, exit: [{kind: confirm}], nope: 1}]",
        "not a flow definition",
    );
    assert_problem(
        "name: x\nstations: [{id: a, exit: [{kind: confirm, nope: 1}]}]",
        "not a flow definition",
    );
    assert_problem(
        "name: x\nstations: [{id: a, executor: {kind: program, step: work, x: 1}, exit: [{kind: confirm}]}]",
        "not a flow definition",
    );
}

#[test]
fn name_pattern() {
    assert_problem(
        "name: Bad_Name\nstations: [{id: a, exit: [{kind: confirm}]}]",
        "must match [a-z][a-z0-9-]*",
    );
    assert_problem(
        "name: x\nstations: [{id: A, exit: [{kind: confirm}]}]",
        "station id must match",
    );
    assert_problem("name: x\nstations: []", "at least one station");
}

#[test]
fn station_ids_are_unique() {
    assert_problem(
        "name: x\nstations: [{id: a, exit: [{kind: confirm}]}, {id: a, exit: [{kind: confirm}]}]",
        "not unique",
    );
}

#[test]
fn needs_exist_and_are_acyclic() {
    assert_problem(
        "name: x\nstations: [{id: a, needs: [zz], exit: [{kind: confirm}]}]",
        "needs zz, which is not a station",
    );
    assert_problem(
        "name: x\nstations: [{id: a, needs: [a], exit: [{kind: confirm}]}]",
        "needs itself",
    );
    assert_problem(
        "name: x\nstations: [{id: a, needs: [b], exit: [{kind: confirm}]}, {id: b, needs: [a], exit: [{kind: confirm}]}]",
        "cycle",
    );
}

#[test]
fn rework_targets_are_self_or_ancestors() {
    let base = |rw: &str| {
        format!(
            "name: x\nstations:\n  - {{id: a, exit: [{{kind: confirm}}]}}\n  - {{id: b, needs: [], exit: [{{kind: confirm}}]}}\n  - {{id: c, needs: [a], exit: [{{kind: confirm}}], {rw}}}\n"
        )
    };
    assert!(problems(&base("rework_to: [a, c]")).is_empty());
    assert_problem(&base("rework_to: [b]"), "rework_to b is neither");
    assert_problem(&base("rework_to: [zz]"), "rework_to zz is not a station");
    assert_problem(
        &base("on_unmet: {rework_to: b, max: 1}"),
        "on_unmet.rework_to b is neither",
    );
    assert_problem(&base("on_unmet: {rework_to: a, max: 0}"), "on_unmet.max");
}

#[test]
fn every_station_has_an_exit() {
    assert_problem("name: x\nstations: [{id: a}]", "no exit condition");
}

#[test]
fn output_exit_needs_a_contract_and_tasks() {
    assert_problem(
        "name: x\nstations: [{id: a, executor: {kind: program, step: work}, exit: [{kind: output}]}]",
        "needs an output contract",
    );
    assert_problem(
        "name: x\nstations: [{id: a, output: {type: object}, exit: [{kind: output}]}]",
        "a checkpoint has none",
    );
    assert_problem(
        "name: x\nstations: [{id: a, executor: {kind: program, step: work}, output: {type: 12}, exit: [{kind: output}]}]",
        "not a valid JSON Schema",
    );
}

#[test]
fn children_only_with_subflow() {
    assert_problem(
        "name: x\nstations: [{id: a, executor: {kind: program, step: work}, exit: [{kind: children}]}]",
        "children exit only goes with a subflow",
    );
}

#[test]
fn session_tasks_need_skill_and_scene() {
    assert_problem(
        "name: x\nstations: [{id: a, executor: {kind: session, scene: chat}, exit: [{kind: confirm}]}]",
        "needs a skill",
    );
    assert_problem(
        "name: x\nstations: [{id: a, executor: {kind: session, scene: nowhere}, skill: analyze, exit: [{kind: confirm}]}]",
        "scene nowhere is not in the catalog",
    );
}

#[test]
fn work_shapes() {
    assert_problem(
        "name: x\nstations: [{id: a, foreach: '${inputs.l}', exit: [{kind: confirm}]}]",
        "foreach needs an executor",
    );
    assert_problem(
        "name: x\nstations: [{id: a, executor: {kind: program, step: work}, tasks: [{id: t, executor: {kind: external}}], exit: [{kind: confirm}]}]",
        "either executor or tasks",
    );
    assert_problem(
        "name: x\nstations: [{id: a, executor: {kind: program, step: work}, max_parallel: 2, exit: [{kind: confirm}]}]",
        "max_parallel only goes with foreach",
    );
    assert_problem(
        "name: x\nstations: [{id: a, tasks: [{id: t, executor: {kind: external}}, {id: t, executor: {kind: external}}], exit: [{kind: confirm}]}]",
        "task id t is not unique",
    );
}

#[test]
fn catalog_references() {
    assert_problem(
        "name: x\nstations: [{id: a, executor: {kind: program, step: nope}, exit: [{kind: confirm}]}]",
        "step nope is not in the catalog",
    );
    assert_problem(
        "name: x\nstations: [{id: a, exit: [{kind: gate, gate: nope}]}]",
        "gate nope is not in the catalog",
    );
    assert_problem(
        "name: x\nstations: [{id: a, exit: [{kind: tool, tool: nope, assert: [{path: x, eq: 1}]}]}]",
        "tool nope is not in the catalog",
    );
    assert_problem(
        "name: x\nstations: [{id: a, executor: {kind: session, scene: chat}, skill: nope, exit: [{kind: confirm}]}]",
        "skill nope is not in the packs",
    );
    assert_problem(
        "name: x\nhooks: [{on: run.finished, action: nope}]\nstations: [{id: a, exit: [{kind: confirm}]}]",
        "hook action nope is not in the catalog",
    );
    assert_problem(
        "name: x\nstations: [{id: a, hooks: [{on: run.exploded, action: notify}], exit: [{kind: confirm}]}]",
        "not an event kind",
    );
    assert_problem(
        "name: x\nstations: [{id: a, executor: {kind: subflow, flow: nope}, exit: [{kind: children}]}]",
        "subflow nope is not a flow",
    );
}

#[test]
fn assertions_have_exactly_one_operator() {
    assert_problem(
        "name: x\ninputs: {m: {}}\nstations: [{id: a, when: [{path: inputs.m}], exit: [{kind: confirm}]}]",
        "exactly one of",
    );
    assert_problem(
        "name: x\ninputs: {m: {}}\nstations: [{id: a, when: [{path: inputs.m, eq: 1, ne: 2}], exit: [{kind: confirm}]}]",
        "exactly one of",
    );
    assert_problem(
        "name: x\nstations: [{id: a, exit: [{kind: tool, tool: count, assert: [{path: total, len: 1, min_len: 1}]}]}]",
        "exactly one of",
    );
    assert_problem(
        "name: x\nstations: [{id: a, exit: [{kind: tool, tool: count, assert: []}]}]",
        "has no assertion",
    );
}

#[test]
fn references_are_checked() {
    assert_problem(
        "name: x\nstations: [{id: a, when: [{path: inputs.nope, eq: 1}], exit: [{kind: confirm}]}]",
        "no input nope",
    );
    assert_problem(
        "name: x\nstations: [{id: a, exit: [{kind: confirm}]}, {id: b, needs: [], when: [{path: stations.a.output, exists: true}], exit: [{kind: confirm}]}]",
        "station a is not an ancestor",
    );
    assert_problem(
        "name: x\nstations: [{id: a, exit: [{kind: tool, tool: count, args: {x: '${bogus}'}, assert: [{path: t, eq: 0}]}]}]",
        "unknown root",
    );
}

#[test]
fn rework_limit_is_bounded() {
    assert_problem(
        "name: x\nrework_limit: 21\nstations: [{id: a, exit: [{kind: confirm}]}]",
        "rework_limit 21",
    );
    assert!(
        problems("name: x\nrework_limit: 20\nstations: [{id: a, exit: [{kind: confirm}]}]")
            .is_empty()
    );
}

#[test]
fn input_defaults_satisfy_their_schema() {
    assert_problem(
        "name: x\ninputs: {mode: {schema: {enum: [a, b]}, default: c}}\nstations: [{id: a, exit: [{kind: confirm}]}]",
        "default does not satisfy",
    );
}

#[test]
fn all_problems_are_reported_at_once() {
    let ps = problems("name: X\nrework_limit: 99\nstations: [{id: a}, {id: a, needs: [zz]}]");
    assert!(ps.len() >= 5, "{ps:#?}");
}

#[test]
fn schemas_are_produced() {
    let d = definition_schema();
    assert!(d["properties"]["stations"].is_object(), "{d}");
    let r = run_schema();
    assert!(r["properties"]["stations"].is_object(), "{r}");
    assert!(r.to_string().contains("rounds_total"));
}

// ── xops-02 review ───────────────────────────────────────────────────────────

#[test]
fn the_dry_prefix_is_kept_for_dry_runs() {
    assert_problem(
        "name: dry-x\nstations: [{id: a, exit: [{kind: confirm}]}]",
        "kept for dry runs",
    );
    assert!(MemPacks::new()
        .writable()
        .save_flow("name: dry-x\nstations: [{id: a, exit: [{kind: confirm}]}]")
        .is_err());
}

#[test]
fn a_write_may_open_a_guarded_group_whole_only() {
    let mut cat = Packs::catalog(&packs());
    cat.guarded = vec!["Bash".into(), "mcp__db__*".into()];
    struct P(MemPacks, Catalog);
    impl Packs for P {
        fn flows(&self) -> Vec<String> {
            Packs::flows(&self.0)
        }
        fn flow(&self, n: &str) -> Option<(FlowDef, String)> {
            Packs::flow(&self.0, n)
        }
        fn skill(&self, n: &str) -> Option<(String, String)> {
            Packs::skill(&self.0, n)
        }
        fn catalog(&self) -> Catalog {
            self.1.clone()
        }
    }
    let p = P(packs(), cat);
    let yaml = |w: &str| {
        format!(
        "name: w\nstations: [{{id: a, executor: {{kind: session, scene: chat}}, skill: analyze, output: {{type: object}}, writes: [{w}], exit: [{{kind: output}}]}}]"
    )
    };
    let ps = validate_yaml(&yaml("mcp__db__insert"), &p);
    assert!(
        ps.iter().any(|x| x
            .message
            .contains("opens only part of the guarded group mcp__db__*")),
        "{ps:?}"
    );
    assert!(validate_yaml(&yaml("'mcp__db__*'"), &p).is_empty());
    assert!(validate_yaml(&yaml("Bash"), &p).is_empty());
}

#[test]
fn tasks_sharing_a_session_must_be_opened_alike() {
    let two = |a: &str, b: &str, extra_b: &str| {
        format!(
        "name: s\nstations:\n  - {{id: one, executor: {{kind: session, scene: chat, session: per_run{a}}}, skill: analyze, output: {{type: object}}, exit: [{{kind: output}}]}}\n  - {{id: two, executor: {{kind: session, scene: chat, session: per_run{b}}}, skill: analyze, output: {{type: object}}{extra_b}, exit: [{{kind: output}}]}}\n"
    )
    };
    assert_eq!(problems(&two("", "", "")), Vec::<String>::new());
    let ps = problems(&two("", "", ", writes: [Bash]"));
    assert!(
        ps.iter().any(|p| p.contains("station two")
            && p.contains("shares its session")
            && p.contains("station one")
            && p.contains("writes")),
        "{ps:#?}"
    );
    let ps = problems(&two("", ", model: m2", ""));
    assert!(ps.iter().any(|p| p.contains("model")), "{ps:#?}");
    let ps = problems(&two(", template: t", "", ""));
    assert!(ps.iter().any(|p| p.contains("placement")), "{ps:#?}");
    let contract = two("", "", "").replacen(
        "output: {type: object}, exit",
        "output: {type: object, required: [x]}, exit",
        1,
    );
    assert!(problems(&contract)
        .iter()
        .any(|p| p.contains("output contract")));
    // Handing in with flow_submit, the contract is the flow's to check, not the session's.
    let wrapped = contract.replace("scene: chat", "scene: flow-chat");
    let mut cat = Packs::catalog(&packs().scene("flow-chat"));
    cat.submit_scenes.insert("flow-chat".into());
    struct Q(MemPacks, Catalog);
    impl Packs for Q {
        fn flows(&self) -> Vec<String> {
            Packs::flows(&self.0)
        }
        fn flow(&self, n: &str) -> Option<(FlowDef, String)> {
            Packs::flow(&self.0, n)
        }
        fn skill(&self, n: &str) -> Option<(String, String)> {
            Packs::skill(&self.0, n)
        }
        fn catalog(&self) -> Catalog {
            self.1.clone()
        }
    }
    assert!(validate_yaml(&wrapped, &Q(packs(), cat)).is_empty());
    // A session of its own each: nothing to agree on.
    let apart = two("", "", ", writes: [Bash]").replace("session: per_run", "session: new");
    assert_eq!(problems(&apart), Vec::<String>::new());
    // Tasks of one station sharing by key.
    let keyed = "name: k\nstations:\n  - id: a\n    tasks:\n      - {id: x, executor: {kind: session, scene: chat, session: {key: '${run}'}}, skill: analyze}\n      - {id: y, executor: {kind: session, scene: other, session: {key: '${run}'}}, skill: analyze}\n    output: {type: object}\n    exit: [{kind: output}]\n";
    let ps: Vec<String> = validate_yaml(keyed, &packs().scene("other"))
        .iter()
        .map(|p| p.to_string())
        .collect();
    assert!(
        ps.iter().any(|p| p.contains("task y")
            && p.contains("task x of this station")
            && p.contains("scene")),
        "{ps:#?}"
    );
}
