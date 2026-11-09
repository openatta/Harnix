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
stations:
  - id: collect
    executor: {kind: program, step: collect}
    output: {type: object, required: [modules], properties: {modules: {type: array}}}
    exit: [{kind: output}]
  - id: analyze
    foreach: ${stations.collect.output.modules}
    max_parallel: 2
    executor: {kind: session, scene: harnix, session: new}
    skill: analyze
    output: {type: object, required: [findings]}
    exit: [{kind: output}]
    retry: {max: 1}
    permissions: [{effect: deny, tools: [Bash], message: no shell here}]
  - id: lint
    needs: [collect]
    exit: [{kind: gate, gate: lint}]
  - id: self-check
    needs: [analyze]
    when: [{path: inputs.mode, eq: auto}]
    executor: {kind: program, step: check}
    exit: [{kind: gate, gate: counted}]
    on_unmet: {rework_to: analyze, max: 2}
  - id: review
    needs: [self-check, lint]
    exit: [{kind: confirm, who: reviewer}]
    rework_to: [analyze, lint]
  - id: report
    tasks:
      - {id: summary, executor: {kind: session, scene: harnix}, skill: report}
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
        "name: p\ninputs: {{who: {{schema: {{type: string}}, default: x}}}}\nstations:\n  - {{id: a, executor: {{kind: session, scene: harnix{executor_extra}}}, skill: analyze{station_extra}, exit: [{{kind: confirm}}]}}\n"
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
        "name: x\nstations: [{id: a, executor: {kind: session, scene: harnix}, exit: [{kind: confirm}]}]",
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
        "gate nope is not in the packs",
    );
    assert_problem(
        "name: x\nrules: [nope]\nstations: [{id: a, policies: [none], exit: [{kind: confirm}]}]",
        "rule nope is not in the packs",
    );
    assert_problem(
        "name: x\nstations: [{id: a, policies: [none], exit: [{kind: confirm}]}]",
        "policy none is not in the packs",
    );
    assert_problem(
        "name: x\nstations: [{id: a, executor: {kind: session, scene: harnix}, skill: nope, exit: [{kind: confirm}]}]",
        "skill nope is not in the packs",
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
    let gate = |text: &str| part_problems(PartKind::Gate, "g", text, &packs()).join("; ");
    assert!(gate("{name: g, kind: tool, tool: count, assert: [{path: total, len: 1, min_len: 1}]}").contains("exactly one of"));
    assert!(gate("{name: g, kind: tool, tool: count}").contains("needs an assertion"));
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
        "name: x\nstations: [{id: a, permissions: [{effect: allow, tools: [Write], paths: ['${bogus}/**']}], exit: [{kind: confirm}]}]",
        "unknown root",
    );
    assert_problem(
        "name: x\nstations: [{id: a, permissions: [{effect: allow, tools: [Write], paths: ['${stations.b.output.dir}/**']}], exit: [{kind: confirm}]}, {id: b, exit: [{kind: confirm}]}]",
        "station b is not an ancestor",
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
fn parts_are_checked_on_their_own() {
    let p = packs();
    let problems = |kind: PartKind, name: &str, text: &str| part_problems(kind, name, text, &p).join("; ");
    assert_eq!(problems(PartKind::Rule, "house", "# House\n\nShort."), "");
    assert!(problems(PartKind::Rule, "House", "x").contains("must match"));
    assert!(problems(PartKind::Rule, "house", "  ").contains("empty"));
    assert_eq!(
        problems(PartKind::Policy, "p", "{name: p, permissions: [{effect: ask, tools: ['mcp__db__*'], args: {table: [orders]}}]}"),
        ""
    );
    assert!(problems(PartKind::Policy, "p", "{name: p, permissions: [{effect: allow, tools: ['Note*']}]}").contains("tool pattern"));
    assert!(problems(PartKind::Policy, "p", "{name: p, permissions: [{effect: allow, tools: []}]}").contains("names no tool"));
    assert!(problems(PartKind::Policy, "p", "{name: p, permissions: [{effect: maybe, tools: [Bash]}]}").contains("not a policy"));
    // Command gates only where the application turned them on; host gates by the checks it has.
    assert!(problems(PartKind::Gate, "t", "{name: t, kind: command, command: make test}").contains("not turned on"));
    let on = packs().command_gates();
    assert!(part_problems(PartKind::Gate, "t", "{name: t, kind: command, command: make test}", &on).is_empty());
    assert!(problems(PartKind::Gate, "t", "{name: t, kind: command, command: make, tool: count}").contains("go with a tool gate"));
    assert!(problems(PartKind::Gate, "nope", "{name: nope, kind: host}").contains("no check named nope"));
    assert_eq!(problems(PartKind::Gate, "lint", "{name: lint, kind: host}"), "");
    assert!(problems(PartKind::Gate, "g", "{name: g, kind: tool, tool: nope, assert: [{path: x, eq: 1}]}").contains("tool nope is not in the catalog"));
    // Hooks: known events, a URL.
    assert_eq!(problems(PartKind::Hook, "h", "{name: h, on: [run.created, 'station.*'], kind: webhook, action: 'https://x.example/h'}"), "");
    assert!(problems(PartKind::Hook, "h", "{name: h, on: [run.exploded], kind: webhook, action: 'https://x.example/h'}").contains("not an event kind"));
    assert!(problems(PartKind::Hook, "h", "{name: h, kind: webhook, action: 'ftp://x'}").contains("http(s) URL"));
    assert!(problems(PartKind::Hook, "h", "{name: h, kind: mail, action: 'https://x.example/h'}").contains("not a hook"));
}

#[test]
fn a_flow_names_rules_and_policies_the_packs_have_and_its_permissions_are_sound() {
    let p = packs()
        .rule("house", "# House style\n\nShort lines.")
        .policy("{name: ro, permissions: [{effect: deny, tools: [Write, Edit]}]}");
    let ok = "name: r\nrules: [house]\npolicies: [ro]\nstations: [{id: a, rules: [house], permissions: [{effect: allow, tools: [Write], paths: ['${workspace}/out/**']}], exit: [{kind: confirm}]}]";
    assert_eq!(validate_yaml(ok, &p), vec![]);
    let twice = "name: r\nstations: [{id: a, policies: [ro, ro], exit: [{kind: confirm}]}]";
    assert!(validate_yaml(twice, &p).iter().any(|x| x.message.contains("named twice")));
    let bad = "name: r\npermissions: [{effect: deny, tools: ['a*b']}]\nstations: [{id: a, exit: [{kind: confirm}]}]";
    assert!(validate_yaml(bad, &p).iter().any(|x| x.message.contains("permissions[0]")));
}

#[test]
fn tasks_sharing_a_session_must_be_opened_alike() {
    let two = |a: &str, b: &str, extra_b: &str| {
        format!(
        "name: s\nstations:\n  - {{id: one, executor: {{kind: session, scene: harnix, session: per_run{a}}}, skill: analyze, output: {{type: object}}, exit: [{{kind: output}}]}}\n  - {{id: two, executor: {{kind: session, scene: harnix, session: per_run{b}}}, skill: analyze, output: {{type: object}}{extra_b}, exit: [{{kind: output}}]}}\n"
    )
    };
    assert_eq!(problems(&two("", "", "")), Vec::<String>::new());
    // What a task may do is judged call by call, not kept by the session: permissions may differ.
    assert_eq!(
        problems(&two("", "", ", permissions: [{effect: allow, tools: [Bash]}]")),
        Vec::<String>::new()
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
    let wrapped = contract.replace("scene: harnix", "scene: flow-harnix");
    let mut cat = Packs::catalog(&packs().scene("flow-harnix"));
    cat.submit_scenes.insert("flow-harnix".into());
    struct Q(MemPacks, Catalog);
    impl Packs for Q {
        fn flows(&self) -> Vec<String> {
            Packs::flows(&self.0)
        }
        fn flow(&self, n: &str) -> Option<(FlowDef, String)> {
            Packs::flow(&self.0, n)
        }
        fn part(&self, k: PartKind, n: &str) -> Option<(String, String)> {
            Packs::part(&self.0, k, n)
        }
        fn catalog(&self) -> Catalog {
            self.1.clone()
        }
    }
    assert!(validate_yaml(&wrapped, &Q(packs(), cat)).is_empty());
    // A session of its own each: nothing to agree on.
    let apart = two("", ", model: m2", "").replace("session: per_run", "session: new");
    assert_eq!(problems(&apart), Vec::<String>::new());
    // Tasks of one station sharing by key.
    let keyed = "name: k\nstations:\n  - id: a\n    tasks:\n      - {id: x, executor: {kind: session, scene: harnix, session: {key: '${run}'}}, skill: analyze}\n      - {id: y, executor: {kind: session, scene: other, session: {key: '${run}'}}, skill: analyze}\n    output: {type: object}\n    exit: [{kind: output}]\n";
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

/// What a station's work is made of, whatever its shape: the one place that says it.
#[test]
fn a_stations_task_specs_are_its_work_whatever_its_shape() {
    let def = FlowDef::parse(
        "{name: f, stations: [\
           {id: one, executor: {kind: program, step: collect}, skill: s},\
           {id: many, tasks: [{id: x, executor: {kind: external}}, {id: y, executor: {kind: program, step: work}}]},\
           {id: each, foreach: '${inputs.items}', executor: {kind: program, step: work}},\
           {id: check, exit: [{kind: confirm}]}]}",
    )
    .unwrap();
    let ids = |s: &str| -> Vec<String> {
        def.station(s).unwrap().task_specs().into_iter().map(|t| t.id).collect()
    };
    assert_eq!(ids("one"), ["one"]);
    assert_eq!(def.station("one").unwrap().task_specs()[0].skill.as_deref(), Some("s"));
    assert_eq!(ids("many"), ["x", "y"]);
    assert_eq!(ids("each"), ["each"], "a foreach station's tasks share its one template");
    assert!(ids("check").is_empty(), "a checkpoint has none");
}
