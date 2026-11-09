//! xops-02 in the kernel: placement and delivery on session work, writable packs, saving
//! through the engine, dry runs.

mod common;

use std::sync::Arc;

use common::{packs, T0};
use harnix_flow::dryrun::{StartIn, KEEP};
use harnix_flow::kernel::host::default_instance;
use harnix_flow::*;
use serde_json::{json, Value};

fn me() -> Actor {
    Actor::device("d")
}

const PLACED: &str = r#"
name: placed
inputs: {box: {schema: {type: string}, default: b1}}
stations:
  - id: work
    executor: {kind: session, scene: harnix, model: m1, provider: acme, template: app.box, params: {name: "box-${inputs.box}", run: "${run}"}, space: dev}
    skill: analyze
    output: {type: object, required: [done], properties: {done: {const: true}}}
    exit: [{kind: output}]
  - id: wrapped
    executor: {kind: session, scene: flow-harnix, instance: side}
    skill: analyze
    output: {type: object}
    exit: [{kind: output}]
"#;

/// The station named its instance: the run does not own it.
fn p_named(_run: &str) -> bool {
    let named = Placement {
        template: Some("t".into()),
        params: [("name".to_string(), "box".to_string())].into(),
        ..Default::default()
    };
    named.names_its_instance()
}

fn engine(p: MemPacks) -> Engine {
    Engine::new(Arc::new(MemStore::new()), Arc::new(p))
        .with_clock(Arc::new(FixedClock(T0)))
        .with_submit_scenes(["flow-harnix"])
}

#[tokio::test]
async fn session_work_carries_its_placement_resolved_and_how_it_hands_in() {
    let eng = engine(packs().scene("flow-harnix").flow(PLACED));
    let o = eng
        .create(
            CreateIn {
                flow: "placed".into(),
                subject: "s".into(),
                ..Default::default()
            },
            me(),
        )
        .await
        .unwrap();
    let run = o.value["run"].as_str().unwrap().to_string();
    let WorkItem::Task {
        placement,
        delivery,
        protocol,
        output_schema,
        token,
        ..
    } = o.work[0].clone()
    else {
        panic!("{:?}", o.work)
    };
    let p = placement.unwrap();
    assert_eq!(p.model.as_deref(), Some("m1"));
    assert_eq!(p.provider.as_deref(), Some("acme"));
    assert_eq!(p.template.as_deref(), Some("app.box"));
    assert_eq!(p.space.as_deref(), Some("dev"));
    assert_eq!(p.params["name"], "box-b1");
    assert_eq!(p.params["run"], run);
    assert_eq!(p.template_instance(&run).as_deref(), Some("box-b1"));
    assert_eq!(delivery, Some(Delivery::StructuredOutput));
    assert!(protocol.unwrap().contains("StructuredOutput"));
    assert!(output_schema.is_some());

    let o = eng
        .submit(
            &run,
            "work",
            "work",
            &token,
            json!({"done": true}),
            Actor::flow("dispatch"),
        )
        .await
        .unwrap();
    let WorkItem::Task {
        placement,
        delivery,
        protocol,
        ..
    } = o.work[0].clone()
    else {
        panic!()
    };
    assert_eq!(placement.unwrap().instance.as_deref(), Some("side"));
    assert_eq!(
        delivery,
        Some(Delivery::FlowSubmit),
        "a wrapped scene hands in with flow_submit"
    );
    assert!(protocol.unwrap().contains("flow_submit"));
    // Rebuilt after a restart, the same.
    let rec = eng.record(&run).await.unwrap();
    let again = eng.dispatched_work(&rec, T0);
    assert!(matches!(
        &again[0],
        WorkItem::Task {
            delivery: Some(Delivery::FlowSubmit),
            ..
        }
    ));
    // A template without a name parameter gets one from the run and the template: it fits
    // the 64-character limit on instance names, and is the run's to stop.
    let bare = Placement {
        template: Some("t".into()),
        ..Default::default()
    };
    assert!(bare.names_its_instance() && !p_named(&run));
    let name = bare.template_instance("r-1").unwrap();
    assert!(
        name.starts_with("flow-r-1-") && name.len() == "flow-r-1-".len() + 8,
        "{name}"
    );
    assert_eq!(
        bare.template_instance("r-1"),
        Some(default_instance("r-1", "t"))
    );
    assert_ne!(default_instance("r-1", "t"), default_instance("r-1", "u"));
    let long = format!("{}-12", "a-very-long-flow-name".repeat(5));
    assert!(default_instance(&long, "app.box").len() <= 64);
}

// ── writable packs ───────────────────────────────────────────────────────────

const BUILT: &str = "name: built\ntitle: Built in\nstations: [{id: a, exit: [{kind: confirm}]}]";
const BUILT_2: &str = "name: built\ntitle: Changed\nstations: [{id: a, exit: [{kind: confirm}]}]";
const MINE: &str = "name: mine\nstations: [{id: a, exit: [{kind: confirm}]}]";

fn check_layers(p: &dyn Packs) {
    assert!(p.writable());
    assert_eq!(p.origin("built"), Origin::Builtin);
    let d = p.save_flow(MINE).unwrap();
    assert_eq!(d, digest(MINE));
    assert_eq!(p.origin("mine"), Origin::User);
    assert_eq!(p.flows(), ["built", "mine"]);
    // Saving a built-in flow is an override; deleting it brings the built-in back.
    p.save_flow(BUILT_2).unwrap();
    assert_eq!(p.origin("built"), Origin::Override);
    assert_eq!(p.flow("built").unwrap().0.title, "Changed");
    p.delete_flow("built").unwrap();
    assert_eq!(p.origin("built"), Origin::Builtin);
    assert_eq!(p.flow("built").unwrap().0.title, "Built in");
    assert!(
        p.delete_flow("built").is_err(),
        "a built-in flow is not deleted"
    );
    p.delete_flow("mine").unwrap();
    assert!(p.flow("mine").is_none());
    // Parts the same way.
    assert!(p.save_part(PartKind::Skill, "Bad Name", "x").is_err());
    p.save_part(PartKind::Skill, "fresh", "Do it.").unwrap();
    assert_eq!(p.skill("fresh").unwrap().0, "Do it.");
    assert!(p.parts(PartKind::Skill).contains(&"fresh".to_string()));
    p.delete_part(PartKind::Skill, "fresh").unwrap();
    assert!(p.skill("fresh").is_none());
    p.save_part(PartKind::Rule, "house", "# House style\n\nShort lines.").unwrap();
    assert_eq!(p.part_origin(PartKind::Rule, "house"), Origin::User);
    assert_eq!(p.parts(PartKind::Rule), ["house"]);
    p.delete_part(PartKind::Rule, "house").unwrap();
    assert!(p.rule("house").is_none());
    // What does not parse is not stored.
    assert!(p.save_flow("name: [").is_err());
    assert!(p.save_flow("name: Bad\nstations: []").is_err());
}

#[test]
fn mem_packs_take_a_writable_layer_over_what_was_given() {
    check_layers(&MemPacks::new().flow(BUILT).writable());
    assert!(!MemPacks::new().writable().save_flow("x").is_ok());
    assert!(
        MemPacks::new().save_flow(MINE).is_err(),
        "read-only unless made writable"
    );
}

#[test]
fn dir_packs_save_in_the_top_layer_over_the_shipped_ones() {
    let tmp = tempfile::tempdir().unwrap();
    let shipped = tmp.path().join("shipped");
    std::fs::create_dir_all(shipped.join("flows")).unwrap();
    std::fs::write(shipped.join("flows/built.yaml"), BUILT).unwrap();
    let top = tmp.path().join("mine");
    check_layers(&DirPacks::new([&shipped]).with_writable(&top));
    assert!(!top.join("flows/built.yaml").exists());

    // Built into the binary, under the directories.
    let embedded =
        DirPacks::writable(&top).with_builtin(MemPacks::new().flow(BUILT).skill("s", "S."));
    assert_eq!(embedded.flows(), ["built"]);
    assert_eq!(embedded.origin("built"), Origin::Builtin);
    assert_eq!(embedded.skill("s").unwrap().0, "S.");
    embedded.save_flow(BUILT_2).unwrap();
    assert_eq!(embedded.origin("built"), Origin::Override);
    assert!(top.join("flows/built.yaml").is_file());
    assert!(!DirPacks::new([&shipped]).writable());
}

#[tokio::test]
async fn the_engine_saves_only_what_validates_and_asks_the_authorizer() {
    struct NoEdits;
    impl Authorizer for NoEdits {
        fn check(
            &self,
            op: &Op,
            _: &Actor,
            _: Option<&RunRecord>,
        ) -> std::result::Result<(), String> {
            match op {
                Op::EditFlow { .. } | Op::EditPart { .. } => Err(format!("no {}", op.name())),
                _ => Ok(()),
            }
        }
    }
    let eng = engine(packs().flow(BUILT).writable());
    let bad = eng
        .save_flow("name: bad\nstations: [{id: a, executor: {kind: program, step: nowhere}, output: {type: object}, exit: [{kind: output}]}]", me())
        .await
        .unwrap_err();
    assert!(bad.to_string().contains("nowhere"), "{bad}");
    assert!(eng.packs().flow("bad").is_none(), "not saved");
    let saved = eng.save_flow(MINE, me()).await.unwrap();
    assert_eq!(saved["origin"], "user");
    // A run pins the definition it started with.
    let o = eng
        .create(
            CreateIn {
                flow: "mine".into(),
                subject: "s".into(),
                ..Default::default()
            },
            me(),
        )
        .await
        .unwrap();
    let run = o.value["run"].as_str().unwrap().to_string();
    eng.save_flow(
        &MINE.replace("name: mine", "name: mine\ntitle: Later"),
        me(),
    )
    .await
    .unwrap();
    assert_eq!(eng.get(&run).await.unwrap().record.def.title, "");
    let gone = eng.delete_flow("mine", me()).await.unwrap();
    assert_eq!(gone["origin"], Value::Null);
    assert!(eng.delete_flow("built", me()).await.is_err());
    assert_eq!(
        eng.delete_flow("nothing", me()).await.unwrap_err().code(),
        "not_found"
    );
    eng.save_part(PartKind::Skill, "extra", "Body.", me()).await.unwrap();
    eng.delete_part(PartKind::Skill, "extra", me()).await.unwrap();
    // A part with problems is not saved; one that breaks a flow says so.
    let bad = eng
        .save_part(PartKind::Policy, "p", "{name: other, permissions: []}", me())
        .await
        .unwrap_err()
        .to_string();
    assert!(bad.contains("declares the name") && bad.contains("no permissions"), "{bad}");
    eng.save_flow("name: uses\npolicies: [strict]\nstations: [{id: a, exit: [{kind: confirm}]}]", me())
        .await
        .unwrap_err();
    eng.save_part(PartKind::Policy, "strict", "{name: strict, permissions: [{effect: deny, tools: [Bash]}]}", me())
        .await
        .unwrap();
    eng.save_flow("name: uses\npolicies: [strict]\nstations: [{id: a, exit: [{kind: confirm}]}]", me())
        .await
        .unwrap();
    let gone = eng.delete_part(PartKind::Policy, "strict", me()).await.unwrap();
    assert_eq!(gone["broken"][0]["flow"], "uses", "{gone}");

    let read_only = engine(packs());
    assert!(read_only
        .save_flow(MINE, me())
        .await
        .unwrap_err()
        .to_string()
        .contains("read-only"));
    let denied = engine(packs().writable()).with_authorizer(Arc::new(NoEdits));
    assert_eq!(
        denied.save_flow(MINE, me()).await.unwrap_err().code(),
        "denied"
    );
    assert_eq!(
        denied.save_part(PartKind::Skill, "x", "y", me()).await.unwrap_err().code(),
        "denied"
    );
}

// ── dry runs ─────────────────────────────────────────────────────────────────

const TRIAL: &str = r#"
name: trial
stations:
  - id: collect
    executor: {kind: program, step: not-registered}
    output: {type: object, required: [modules], properties: {modules: {type: array, minItems: 2, items: {type: string}}}}
    exit: [{kind: output}]
  - id: analyze
    foreach: ${stations.collect.output.modules}
    executor: {kind: session, scene: harnix}
    skill: analyze
    output: {type: object, required: [findings], properties: {findings: {type: array}}}
    exit: [{kind: output}]
  - id: lint
    needs: [collect]
    exit: [{kind: gate, gate: not-registered-either}]
  - id: check
    needs: [analyze, lint]
    exit: [{kind: gate, gate: counted}]
  - id: approve
    exit: [{kind: confirm}]
"#;

fn sandbox(p: MemPacks) -> Sandbox {
    Sandbox::new(Arc::new(p), Default::default())
}

fn start(flow: &str, mocks: Value) -> StartIn {
    serde_json::from_value(json!({"flow": flow, "mocks": mocks})).unwrap()
}

fn station<'v>(v: &'v Value, id: &str) -> &'v Value {
    &v["record"]["stations"][id]
}

#[tokio::test]
async fn a_dry_run_mocks_the_outside_and_stops_where_a_person_is_needed() {
    let sb = sandbox(packs().flow(TRIAL));
    let run = sb
        .start(
            start("trial", json!({"tools": {"count": {"total": 0}}})),
            me(),
        )
        .await
        .unwrap();
    assert!(run.starts_with("dry-trial-"), "{run}");
    // Nothing is taken until asked: one step, one item.
    let p = sb.step(&run).await.unwrap();
    assert_eq!(p.steps.len(), 1);
    assert_eq!(p.steps[0].station.as_deref(), Some("collect"));
    let p = sb.run(&run).await.unwrap();
    assert_eq!(p.pending, 0);
    assert_eq!(p.status, Some(RunStatus::Active));
    let v = sb.get(&run).await.unwrap();
    // The program's output is a minimal value of its contract: two modules.
    assert_eq!(
        station(&v, "collect")["tasks"][0]["output"]["modules"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(station(&v, "analyze")["tasks"].as_array().unwrap().len(), 2);
    assert_eq!(station(&v, "lint")["status"], "passed");
    assert_eq!(station(&v, "check")["status"], "passed");
    assert_eq!(station(&v, "approve")["status"], "working");
    // A session task records what its session would have been sent.
    let preview = &station(&v, "analyze")["tasks"][0]["preview"];
    let message = preview["message"].as_str().unwrap();
    assert!(
        message.contains("Flow protocol") && message.contains("## Skill `analyze`"),
        "{message}"
    );
    assert_eq!(preview["delivery"], "structured_output");

    // The person's part goes through the sandbox's engine; a run being driven carries on.
    let eng = sb.engine_for(&run).await.unwrap();
    let o = eng
        .confirm(&run, "approve", true, None, me())
        .await
        .unwrap();
    let p = sb.absorb(&run, o).await.unwrap();
    assert_eq!(p.status, Some(RunStatus::Finished));
    assert_eq!(sb.list().await[0].status, RunStatus::Finished);
}

#[tokio::test]
async fn mocks_decide_outputs_gates_and_tools() {
    let sb = sandbox(packs().flow(TRIAL));
    let run = sb
        .start(
            start(
                "trial",
                json!({
                    "stations": {"collect": {"modules": ["x", "y", "z"]}, "analyze": {"y": {"findings": [1]}}},
                    "gates": {"not-registered-either": false},
                }),
            ),
            me(),
        )
        .await
        .unwrap();
    sb.run(&run).await.unwrap();
    let v = sb.get(&run).await.unwrap();
    let tasks = station(&v, "analyze")["tasks"].as_array().unwrap().clone();
    assert_eq!(tasks.len(), 3);
    let y = tasks.iter().find(|t| t["id"] == "y").unwrap();
    assert_eq!(y["output"], json!({"findings": [1]}));
    let x = tasks.iter().find(|t| t["id"] == "x").unwrap();
    assert_eq!(
        x["output"],
        json!({"findings": []}),
        "not in the table: the contract's minimal value"
    );
    // The gate mocked false stops the run at that station.
    assert_eq!(station(&v, "lint")["status"], "unmet");
    assert_eq!(station(&v, "check")["status"], "pending");
    // An unmocked tool answers {}: the assertion on `total` fails, the station stays unmet.
}

#[tokio::test]
async fn a_dry_run_takes_an_unsaved_text_and_the_sandbox_keeps_the_last_ones() {
    let sb = sandbox(packs());
    let yaml = "name: draft\nstations: [{id: a, executor: {kind: program, step: whatever}, output: {type: object, examples: [{ok: 1}]}, exit: [{kind: output}]}]";
    let input: StartIn = serde_json::from_value(json!({"yaml": yaml})).unwrap();
    let run = sb.start(input, me()).await.unwrap();
    let p = sb.run(&run).await.unwrap();
    assert_eq!(p.status, Some(RunStatus::Finished));
    assert_eq!(
        station(&sb.get(&run).await.unwrap(), "a")["tasks"][0]["output"],
        json!({"ok": 1})
    );
    // A contract the mock breaks fails the task, saying why.
    let bad: StartIn = serde_json::from_value(json!({
        "yaml": "name: strict\nstations: [{id: a, executor: {kind: program, step: s}, output: {type: object, required: [n]}, exit: [{kind: output}]}]",
        "mocks": {"stations": {"a": {"m": 1}}}
    }))
    .unwrap();
    let strict = sb.start(bad, me()).await.unwrap();
    let p = sb.run(&strict).await.unwrap();
    assert!(p.steps[0].result.starts_with("failed:"), "{:?}", p.steps);
    for _ in 0..KEEP {
        let input: StartIn = serde_json::from_value(json!({"yaml": yaml})).unwrap();
        sb.start(input, me()).await.unwrap();
    }
    assert_eq!(sb.list().await.len(), KEEP);
    assert_eq!(sb.get(&run).await.unwrap_err().code(), "not_found");
    assert!(!dryrun::is_dry("draft-1") && dryrun::is_dry(&strict));
}
