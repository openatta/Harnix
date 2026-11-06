//! Rules, policies and permissions, gates (xops-04): what a run pins, what a task is handed, how
//! a station is judged, and a call held for a person.

mod common;

use common::*;
use harnix_flow::*;
use serde_json::json;

const RULED: &str = r#"
name: ruled
inputs: {table: {schema: {type: string}, default: orders}}
rules: [no-secrets]
policies: [tables]
permissions: [{effect: deny, tools: [Bash], message: no shell in this flow}]
stations:
  - id: emit
    executor: {kind: session, scene: chat}
    skill: analyze
    rules: [house]
    permissions:
      - {effect: allow, tools: [Write], paths: ['${workspace}/out/**']}
      - {effect: ask, tools: [mcp__desk__emit_row], args: {table: ['${inputs.table}']}}
    output: {type: object}
    exit: [{kind: output}, {kind: gate, gate: rows}]
  - id: build
    exit: [{kind: gate, gate: tests}]
"#;

fn packs_with() -> MemPacks {
    packs()
        .command_gates()
        .rule("no-secrets", "# No secrets\n\nNever paste a key.")
        .rule("house", "# House style\n\nShort lines.")
        .policy("{name: tables, title: Declared tables, permissions: [{effect: allow, tools: ['mcp__desk__*'], args: {table: [orders, 'order_*']}}]}")
        .gate_def("{name: rows, kind: tool, tool: count, args: {run: '${run}'}, assert: [{path: total, eq: 0}]}")
        .gate_def("{name: tests, kind: command, command: 'true'}")
        .flow(RULED)
}

fn h() -> H {
    let mut h = harness(packs_with());
    h.eng = h.eng.with_workspaces("/data/ws").with_guarded(vec!["Bash".into(), "Write".into()]);
    h
}

#[tokio::test]
async fn a_task_is_handed_its_rules_and_its_permissions_resolved() {
    let h = h();
    assert_eq!(validate_yaml(RULED, h.packs.as_ref()), vec![]);
    let (run, o) = start(&h, "ruled").await;
    let rec = h.eng.record(&run).await.unwrap();
    assert_eq!(
        rec.parts.policies.keys().collect::<Vec<_>>(),
        ["tables"],
        "the policies are pinned with the definition"
    );
    assert_eq!(rec.parts.gates.keys().collect::<Vec<_>>(), ["rows", "tests"]);
    let WorkItem::Task {
        rules,
        permissions,
        protocol,
        ..
    } = o.work[0].clone()
    else {
        panic!("{:?}", o.work)
    };
    let names: Vec<&str> = rules.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(names, ["no-secrets", "house"], "the flow's, then the station's");
    assert!(rules.iter().all(|(_, d)| d.is_some()), "pinned on entry: {rules:?}");
    let from: Vec<&str> = permissions.iter().map(|g| g.from.as_str()).collect();
    assert_eq!(from, ["policy tables", "flow", "station emit", "station emit"]);
    assert_eq!(permissions[2].permission.paths, ["/data/ws/ruled-1/out/**"]);
    assert_eq!(permissions[3].permission.args["table"], ["orders"]);
    let protocol = protocol.unwrap();
    for want in ["no shell in this flow", "Write allowed", "a person is asked", "Gate `rows`", "Refused unless a permission allows them: Bash, Write"] {
        assert!(protocol.contains(want), "missing {want}: {protocol}");
    }

    // Judging calls with them: deny over ask over allow; a guarded tool nothing allows is refused.
    let call = |tool: &'static str, input: serde_json::Value| (tool, input);
    for ((tool, input), want) in [
        (call("Write", json!({"file_path": "/data/ws/ruled-1/out/a.json"})), Effect::Allow),
        (call("Write", json!({"file_path": "/etc/passwd"})), Effect::Deny),
        (call("Bash", json!({"command": "ls"})), Effect::Deny),
        (call("mcp__desk__emit_row", json!({"table": "orders"})), Effect::Ask),
        (call("mcp__desk__emit_row", json!({"table": "order_lines"})), Effect::Allow),
        (call("Read", json!({"file_path": "/etc/passwd"})), Effect::Allow),
    ] {
        let v = judge(
            &permissions,
            h.eng.guarded(),
            &Call { tool, input: Some(&input), paths: &[], workspace: Some("/data/ws/ruled-1") },
            "emit",
        );
        assert_eq!(v.effect, want, "{tool} {input}: {}", v.reason);
    }

    // A pack changed after the run started does not change it.
    h.packs.put_part(PartKind::Policy, "tables", "{name: tables, permissions: [{effect: deny, tools: ['mcp__desk__*']}]}");
    let again = h.eng.dispatched_work(&h.eng.record(&run).await.unwrap(), T0);
    let WorkItem::Task { permissions: p2, .. } = &again[0] else { panic!() };
    assert_eq!(p2, &permissions);
}

#[tokio::test]
async fn a_call_held_for_a_person_makes_the_run_wait_for_approval() {
    let h = h();
    let (run, o) = start(&h, "ruled").await;
    let j = &jobs(&o)[0];
    let o = h
        .eng
        .asking(&run, "emit", "emit", Some(("mcp__desk__emit_row".into(), "a nod first".into())))
        .await
        .unwrap();
    assert!(has(&o, "task.asking:emit/emit"));
    let phase = h.eng.record(&run).await.unwrap().phase();
    assert_eq!((phase.state.as_str(), phase.reason.as_deref()), ("waiting", Some("approval")));
    assert_eq!(task(&h, &run, "emit", "emit").await.asking.unwrap().reason, "a nod first");
    // Paused meanwhile: the turn is still in hand.
    h.eng.pause(&run, me()).await.unwrap();
    assert_eq!(h.eng.record(&run).await.unwrap().phase().state, "pausing");
    h.eng.resume(&run, me()).await.unwrap();
    h.eng.asking(&run, "emit", "emit", None).await.unwrap();
    assert_eq!(h.eng.record(&run).await.unwrap().phase().state, "running");
    // Handed in, nothing is held any more.
    h.tools.set("count", json!({"total": 0}));
    submit(&h, j, json!({})).await;
    assert!(task(&h, &run, "emit", "emit").await.asking.is_none());
}

#[tokio::test]
async fn a_tool_gate_is_judged_on_the_spot_and_a_command_gate_is_handed_out() {
    let h = h();
    let (run, o) = start(&h, "ruled").await;
    h.tools.set("count", json!({"total": 2}));
    let o2 = submit(&h, &jobs(&o)[0], json!({})).await;
    assert_eq!(status(&h, &run, "emit").await, StationStatus::Unmet);
    assert!(!o2.work.iter().any(|w| matches!(w, WorkItem::Gate { .. })), "a tool gate is not work");
    h.tools.set("count", json!({"total": 0}));
    let o3 = h.eng.evaluate(&run).await.unwrap();
    assert_eq!(status(&h, &run, "emit").await, StationStatus::Passed);
    let gates: Vec<&str> = o3
        .work
        .iter()
        .filter_map(|w| match w {
            WorkItem::Gate { gate, .. } => Some(gate.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(gates, ["tests"]);
    let rec = h.eng.record(&run).await.unwrap();
    assert_eq!(h.eng.dispatched_work(&rec, T0).len(), 1, "a command gate waiting is rebuilt after a restart");
}

#[tokio::test]
async fn a_gate_not_in_the_packs_is_a_problem() {
    let mut p = packs_with();
    p = p.flow("name: late\nstations: [{id: a, exit: [{kind: gate, gate: later}]}]");
    let h = harness(p);
    // Validation refuses it before a run can start.
    assert!(validate_yaml("name: late\nstations: [{id: a, exit: [{kind: gate, gate: later}]}]", h.packs.as_ref())
        .iter()
        .any(|x| x.message.contains("gate later is not in the packs")));
}

#[tokio::test]
async fn the_parts_of_a_pack_are_read_and_saved_by_kind() {
    let dir = tempfile::tempdir().unwrap();
    let ship = dir.path().join("ship");
    std::fs::create_dir_all(ship.join("rules")).unwrap();
    std::fs::create_dir_all(ship.join("gates")).unwrap();
    std::fs::write(ship.join("rules/house.md"), "# House\n\nShort.").unwrap();
    std::fs::write(ship.join("gates/lint.yaml"), "name: lint\nkind: host\n").unwrap();
    let p = DirPacks::new([ship]).with_writable(dir.path().join("mine"));
    assert_eq!(p.parts(PartKind::Rule), ["house"]);
    assert_eq!(p.gate("lint").unwrap().kind, GateKind::Host);
    p.save_part(PartKind::Rule, "house", "# House\n\nShorter.").unwrap();
    assert_eq!(p.part_origin(PartKind::Rule, "house"), Origin::Override);
    p.save_part(PartKind::Hook, "erp", "name: erp\nkind: webhook\naction: https://erp.example/h\n").unwrap();
    assert_eq!(p.hooks().len(), 1);
    assert!(dir.path().join("mine/hooks/erp.yaml").is_file());
    p.delete_part(PartKind::Rule, "house").unwrap();
    assert_eq!(p.rule("house").unwrap().0, "# House\n\nShort.");
    assert!(p.delete_part(PartKind::Rule, "house").is_err(), "a shipped part is not deleted");
    // A YAML part named otherwise inside is not offered as that name.
    p.save_part(PartKind::Policy, "x", "name: y\npermissions: [{effect: deny, tools: [Bash]}]\n").unwrap();
    assert!(p.policy("x").is_none());
}
