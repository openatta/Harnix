//! The run's main session, sub-agent tasks, pausing, and what a run spent (xops-03).

mod common;

use common::*;
use harnix_flow::*;
use serde_json::json;

/// A session station without a scene, a sub-agent station, a person confirming.
const MAIN: &str = r#"
name: main
main: {model: big, params: {}}
stations:
  - {id: draft, executor: {kind: session}, skill: analyze, output: {type: object}, exit: [{kind: output}]}
  - {id: check, executor: {kind: agent, agent_type: reviewer, model: small}, skill: report, output: {type: object}, writes: [Write], exit: [{kind: output}]}
  - {id: sign, exit: [{kind: confirm}]}
"#;

/// Three items, one at a time, then a confirmation.
const EACH: &str = r#"
name: each
inputs: {items: {schema: {type: array}}}
stations:
  - {id: work, foreach: '${inputs.items}', max_parallel: 1, executor: {kind: program, step: work}, output: {type: object}, exit: [{kind: output}]}
  - {id: sign, exit: [{kind: confirm}]}
"#;

fn h() -> H {
    let mut h = harness(
        packs()
            .main_scene("flow-chat")
            .flow(MAIN)
            .flow(EACH)
            .flow("name: ext\nstations: [{id: x, executor: {kind: external}, output: {type: object}, exit: [{kind: output}]}, {id: y, exit: [{kind: confirm}]}]"),
    );
    h.eng = h.eng.with_main_scene("flow-chat").with_guarded(vec!["Write".to_string(), "Bash".to_string()]);
    h
}

fn work_of(o: &Outcome) -> Vec<WorkItem> {
    o.work
        .iter()
        .filter(|w| matches!(w, WorkItem::Task { .. }))
        .cloned()
        .collect()
}

#[tokio::test]
async fn a_station_without_a_scene_and_a_sub_agent_take_turns_in_the_main_session() {
    let h = h();
    assert!(h.eng.validate(MAIN).await.is_empty(), "{:?}", h.eng.validate(MAIN).await);
    let (run, o) = start(&h, "main").await;
    let WorkItem::Task {
        session_key,
        scene,
        delivery,
        placement,
        protocol,
        ..
    } = &work_of(&o)[0]
    else {
        unreachable!()
    };
    assert_eq!(session_key.as_deref(), Some(MAIN_SESSION_KEY));
    assert_eq!(scene.as_deref(), Some("flow-chat"));
    assert_eq!(*delivery, Some(Delivery::FlowSubmit));
    assert_eq!(placement.as_ref().unwrap().model.as_deref(), Some("big"));
    assert!(protocol.as_ref().unwrap().contains("main agent"));

    let o = submit(&h, &jobs(&o)[0], json!({})).await;
    let WorkItem::Task {
        session_key,
        protocol,
        executor,
        ..
    } = &work_of(&o)[0]
    else {
        unreachable!()
    };
    assert_eq!(session_key.as_deref(), Some(MAIN_SESSION_KEY));
    assert_eq!(executor.kind(), "agent");
    let p = protocol.as_ref().unwrap();
    for want in [
        "`Agent` tool",
        &format!("flow:{run}/check/check"),
        "subagent_type: \"reviewer\"",
        "model: \"small\"",
        "flow_submit",
    ] {
        assert!(p.contains(want), "{want} not in {p}");
    }
}

#[tokio::test]
async fn a_task_in_the_main_session_takes_its_place_from_the_flow() {
    let h = h();
    for (yaml, why) in [
        (
            "name: a\nstations: [{id: s, executor: {kind: session, model: m}, skill: analyze, output: {type: object}, exit: [{kind: output}]}]",
            "from the flow's `main`",
        ),
        (
            "name: b\nstations: [{id: s, executor: {kind: session, scene: chat, session: main}, skill: analyze, output: {type: object}, exit: [{kind: output}]}]",
            "from the flow's `main`",
        ),
        (
            "name: c\nstations: [{id: s, executor: {kind: agent}, exit: [{kind: output}]}]",
            "needs an output contract",
        ),
        (
            "name: d\nmain: {scene: chat}\nstations: [{id: s, executor: {kind: session}, skill: analyze, output: {type: object}, exit: [{kind: output}]}]",
            "does not hand in with flow_submit",
        ),
        (
            "name: e\nmain: {instance: i, template: t}\nstations: [{id: s, exit: [{kind: confirm}]}]",
            "an instance or a template",
        ),
        (
            "name: f\nstations: [{id: s, tasks: [{id: x, executor: {kind: session, scene: chat, session: {key: '@${run}'}}, skill: analyze}], output: {type: object}, exit: [{kind: output}]}]",
            "cannot start with `@`",
        ),
    ] {
        let problems = h.eng.validate(yaml).await;
        assert!(
            problems.iter().any(|p| p.message.contains(why)),
            "{yaml}: {problems:?}"
        );
    }
    // Without a main scene anywhere, tasks in the main session have nowhere to go.
    let bare = harness(packs().submit_scene("flow-chat"));
    let problems = bare.eng.validate("name: g\nstations: [{id: s, executor: {kind: session}, skill: analyze, output: {type: object}, exit: [{kind: output}]}]").await;
    assert!(problems.iter().any(|p| p.message.contains("no main scene")), "{problems:?}");
    // A scene written: a session of its own, as before.
    let own = h.eng.validate("name: h\nstations: [{id: s, executor: {kind: session, scene: chat}, skill: analyze, output: {type: object}, exit: [{kind: output}]}]").await;
    assert!(own.is_empty(), "{own:?}");
}

#[tokio::test]
async fn a_pause_lets_what_is_in_hand_finish_and_takes_nothing_new() {
    let h = h();
    let (run, o) = start_with(&h, "each", json!({"items": ["a", "b", "c"]})).await;
    let first = jobs(&o);
    assert_eq!(first.len(), 1);
    assert_eq!(h.eng.get(&run).await.unwrap().record.phase().state, "running");

    h.eng.pause(&run, me()).await.unwrap();
    assert!(h.eng.pause(&run, me()).await.is_err(), "paused twice");
    let p = h.eng.get(&run).await.unwrap().record.phase();
    assert_eq!((p.state.as_str(), p.stations.as_slice()), ("pausing", &["work".to_string()][..]));

    // The task in hand hands in; the next item is not dispatched.
    let o = submit(&h, &first[0], json!({})).await;
    assert!(jobs(&o).is_empty(), "{:?}", stations(&o));
    let rec = h.eng.record(&run).await.unwrap();
    assert_eq!(rec.phase().state, "paused");
    let t = &rec.stations["work"].tasks[0];
    assert!(t.started_at.is_some() && t.ended_at.is_some());

    let o = h.eng.resume(&run, me()).await.unwrap();
    assert!(has(&o, "run.resumed"));
    assert_eq!(jobs(&o).len(), 1, "the next item goes on");
    assert!(h.eng.resume(&run, me()).await.is_err(), "not paused");
}

fn stations(o: &Outcome) -> Vec<String> {
    jobs(o).into_iter().map(|j| format!("{}/{}", j.station, j.task)).collect()
}

#[tokio::test]
async fn a_station_that_passes_while_paused_does_not_lead_on() {
    let h = h();
    let (run, o) = start_with(&h, "each", json!({"items": ["a"]})).await;
    h.eng.pause(&run, me()).await.unwrap();
    let o = submit(&h, &jobs(&o)[0], json!({})).await;
    assert!(has(&o, "station.passed:work"));
    assert_eq!(status(&h, &run, "sign").await, StationStatus::Pending);
    assert_eq!(h.eng.record(&run).await.unwrap().phase().state, "paused");

    h.eng.resume(&run, me()).await.unwrap();
    assert_eq!(status(&h, &run, "sign").await, StationStatus::Working);
    let p = h.eng.record(&run).await.unwrap().phase();
    assert_eq!((p.state.as_str(), p.reason.as_deref()), ("waiting", Some("confirm")));
}

#[tokio::test]
async fn nobody_takes_on_an_external_task_while_paused() {
    let h = h();
    let (run, _) = start(&h, "ext").await;
    let p = h.eng.record(&run).await.unwrap().phase();
    assert_eq!((p.state.as_str(), p.reason.as_deref()), ("waiting", Some("external")));
    // Claimed before the pause: in hand, and timed.
    let rec = h.eng.record(&run).await.unwrap();
    assert!(rec.stations["x"].tasks[0].started_at.is_none());
    h.eng.pause(&run, me()).await.unwrap();
    let r = h.eng.claim(&run, "x", "x", me(), 60_000).await;
    assert!(matches!(r, Err(FlowError::Invalid(ref m)) if m.contains("paused")), "{r:?}");
    h.eng.resume(&run, me()).await.unwrap();
    h.eng.claim(&run, "x", "x", me(), 60_000).await.unwrap();
    let t = &h.eng.record(&run).await.unwrap().stations["x"].tasks[0];
    assert_eq!(t.started_at, Some(T0));
    // Cancelling a paused run is fine.
    h.eng.pause(&run, me()).await.unwrap();
    h.eng.cancel(&run, "enough", me()).await.unwrap();
    assert_eq!(h.eng.record(&run).await.unwrap().phase().state, "cancelled");
}

#[tokio::test]
async fn tokens_count_on_the_task_or_on_the_run() {
    let h = h();
    let (run, o) = start(&h, "main").await;
    let turn = Usage::of_turn(&json!({"input_tokens": 100, "output_tokens": 20}));
    h.eng.account(&run, Some(("draft", "draft")), turn).await.unwrap();
    h.eng.account(&run, Some(("draft", "draft")), turn).await.unwrap();
    // No task, or one that is not there: the run's.
    h.eng.account(&run, None, turn).await.unwrap();
    h.eng.account(&run, Some(("nope", "x")), turn).await.unwrap();
    let rec = h.eng.record(&run).await.unwrap();
    assert_eq!(rec.stations["draft"].usage().input_tokens, 200);
    assert_eq!(rec.stations["draft"].usage().turns, 2);
    assert_eq!(rec.usage_other.turns, 2);
    assert_eq!(rec.usage().total(), 480);
    assert_eq!(rec.summary().usage.total(), 480);
    // Bookkeeping is not history.
    assert!(h.eng.history(&run).await.unwrap().iter().all(|e| e.op != "account"));
    let _ = o;
}

#[tokio::test]
async fn the_main_session_is_recorded_once() {
    let h = h();
    let (run, _) = start(&h, "main").await;
    let o = h.eng.attach_main(&run, "system/s1").await.unwrap();
    assert_eq!(o.value["changed"], true);
    let o = h.eng.attach_main(&run, "system/s1").await.unwrap();
    assert_eq!(o.value["changed"], false);
    assert_eq!(h.eng.record(&run).await.unwrap().main_session(), Some("system/s1"));
}

/// Every whole flow in the guide (a YAML block that starts `name:`) is valid: the guide is what
/// the reference tab shows and the Agent answers from.
#[test]
fn the_guides_examples_are_valid_flows() {
    let packs = packs()
        .main_scene("flow-chat")
        .submit_scene("flow-coding")
        .gate("ci")
        .skill("draft", "Draft it.")
        .skill("collect", "Collect.")
        .skill("review-code", "Review the code.")
        .skill("review-docs", "Review the docs.")
        .skill("list-modules", "List the modules.")
        .skill("build", "Build it.");
    for (lang, text) in [
        ("en", include_str!("../docs/guide.en.md")),
        ("zh", include_str!("../docs/guide.zh-CN.md")),
    ] {
        let mut whole = 0;
        for block in text.split("```yaml\n").skip(1) {
            let yaml = block.split("```").next().unwrap();
            if !yaml.starts_with("name:") {
                continue;
            }
            whole += 1;
            let problems = validate_yaml(yaml, &packs);
            assert!(problems.is_empty(), "{lang}: {yaml}\n{problems:?}");
        }
        assert_eq!(whole, 3, "{lang}: three whole examples");
    }
}
