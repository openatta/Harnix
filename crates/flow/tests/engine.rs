//! The engine's behaviour, one case per acceptance item.

mod common;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use common::*;
use harnix_flow::kernel::refs::station_output;
use harnix_flow::*;
use serde_json::{json, Value};

const SEQ: &str = r#"
name: seq
hooks: [{on: run.finished, action: notify}]
stations:
  - {id: a, executor: {kind: program, step: work}, output: {type: object, required: [n]}, exit: [{kind: output}]}
  - id: b
    executor: {kind: program, step: work}
    output: {type: object}
    exit: [{kind: output}]
    hooks: [{on: station.passed, action: notify}]
"#;

const PAR: &str = r#"
name: par
stations:
  - {id: collect, executor: {kind: program, step: collect}, output: {type: object}, exit: [{kind: output}]}
  - {id: analyze, executor: {kind: program, step: work}, output: {type: object}, exit: [{kind: output}]}
  - {id: scan, needs: [collect], executor: {kind: program, step: work}, output: {type: object}, exit: [{kind: output}]}
  - {id: review, needs: [analyze, scan], exit: [{kind: confirm, who: reviewer}], rework_to: [analyze]}
  - {id: report, executor: {kind: program, step: work}, output: {type: object}, exit: [{kind: output}]}
"#;

const COND: &str = r#"
name: cond
inputs:
  mode: {schema: {enum: [manual, auto]}, default: manual}
stations:
  - {id: a, executor: {kind: program, step: work}, output: {type: object}, exit: [{kind: output}]}
  - {id: check, needs: [a], when: [{path: inputs.mode, eq: auto}], executor: {kind: program, step: check}, output: {type: object}, exit: [{kind: output}]}
  - {id: side, needs: [a], executor: {kind: program, step: work}, output: {type: object}, exit: [{kind: output}]}
  - {id: join, needs: [check, side], exit: [{kind: confirm}]}
"#;

const MULTI: &str = r#"
name: multi
stations:
  - {id: collect, executor: {kind: program, step: collect}, output: {type: object, required: [modules]}, exit: [{kind: output}]}
  - id: analyze
    foreach: ${stations.collect.output.modules}
    max_parallel: 2
    executor: {kind: session, scene: chat}
    skill: analyze
    output: {type: object, required: [findings]}
    exit: [{kind: output}]
  - id: report
    tasks:
      - {id: summary, executor: {kind: program, step: work}}
      - {id: sign, executor: {kind: external, who: reviewer}}
    output: {type: object}
    exit: [{kind: output}]
    rework_to: [analyze]
"#;

const AUTO: &str = r#"
name: auto
stations:
  - {id: work, executor: {kind: program, step: work}, output: {type: object}, exit: [{kind: output}]}
  - id: check
    exit: [{kind: tool, tool: count, args: {run: "${run}", what: high}, assert: [{path: total, eq: 0}]}]
    on_unmet: {rework_to: work, max: 2}
"#;

const EXT: &str = r#"
name: ext
stations:
  - {id: sign, executor: {kind: external, who: reviewer}, output: {type: object}, exit: [{kind: output}]}
"#;

fn all_packs() -> MemPacks {
    packs()
        .flow(SEQ)
        .flow(PAR)
        .flow(COND)
        .flow(MULTI)
        .flow(AUTO)
        .flow(EXT)
        .flow("name: held\nstations: [{id: a, hold: true, executor: {kind: program, step: work}, output: {type: object}, exit: [{kind: output}]}]")
        .flow("name: ok\nstations: [{id: a, exit: [{kind: confirm, who: reviewer}]}]")
        .flow("name: limited\nrework_limit: 2\nstations: [{id: a, exit: [{kind: confirm}], rework_to: [a]}]")
        .flow("name: gated\nstations: [{id: lint, exit: [{kind: gate, gate: lint}]}]")
        .flow("name: retry\nstations: [{id: a, executor: {kind: program, step: work}, retry: {max: 1}, output: {type: object}, exit: [{kind: output}]}]")
        .flow("name: child\nstations: [{id: do, executor: {kind: program, step: work}, output: {type: object}, exit: [{kind: output}]}]")
        .flow(
            "name: parent\ninputs: {items: {schema: {type: array}}}\nstations: [{id: split, foreach: '${inputs.items}', executor: {kind: subflow, flow: child}, exit: [{kind: children}]}]",
        )
        .flow(
            "name: chat\nstations:\n  - {id: one, executor: {kind: session, scene: chat, session: per_run}, skill: analyze, output: {type: object}, exit: [{kind: confirm}]}\n  - {id: two, executor: {kind: session, scene: chat, session: per_run}, skill: report, output: {type: object}, exit: [{kind: confirm}]}",
        )
}

fn h() -> H {
    harness(all_packs())
}

fn stations_of(o: &Outcome) -> Vec<String> {
    jobs(o)
        .into_iter()
        .map(|j| format!("{}/{}", j.station, j.task))
        .collect()
}

// ── shape ────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn sequence_runs_to_the_end() {
    let h = h();
    let (run, o) = start(&h, "seq").await;
    assert_eq!(stations_of(&o), ["a/a"]);
    assert!(has(&o, "run.created") && has(&o, "station.entered:a") && has(&o, "task.claimed:a/a"));
    assert_eq!(status(&h, &run, "b").await, StationStatus::Pending);

    let j = &jobs(&o)[0];
    let bad = h
        .eng
        .submit(&run, "a", "a", &j.token, json!({}), Actor::flow("dispatch"))
        .await;
    assert!(
        matches!(bad, Err(FlowError::Contract(ref m)) if m.contains("n")),
        "{bad:?}"
    );

    let o = submit(&h, j, json!({"n": 1})).await;
    assert!(has(&o, "station.passed:a"));
    assert_eq!(stations_of(&o), ["b/b"]);
    let o = submit(&h, &jobs(&o)[0], json!({})).await;
    assert!(has(&o, "run.finished"));
    // hooks: the run-level one and the station-level one
    let hooks: Vec<&str> = o
        .work
        .iter()
        .filter_map(|w| match w {
            WorkItem::Hook { event, .. } => Some(event.kind.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(hooks, ["station.passed", "run.finished"]);
    assert_eq!(run_status(&h, &run).await, RunStatus::Finished);

    let hist = h.eng.history(&run).await.unwrap();
    let ops: Vec<&str> = hist.iter().map(|x| x.op.as_str()).collect();
    assert_eq!(ops.first(), Some(&"create"));
    assert_eq!(ops.last(), Some(&"finish"));
    assert!(ops.contains(&"submit") && ops.contains(&"claim") && ops.contains(&"pass"));
    assert!(hist.windows(2).all(|w| w[1].seq == w[0].seq + 1));
}

#[tokio::test]
async fn parallel_branches_join() {
    let h = h();
    let (run, o) = start(&h, "par").await;
    let o = submit(&h, &jobs(&o)[0], json!({})).await;
    let mut js = stations_of(&o);
    js.sort();
    assert_eq!(js, ["analyze/analyze", "scan/scan"]);
    let both = jobs(&o);
    let o = submit(&h, &both[0], json!({})).await;
    assert!(jobs(&o).is_empty());
    assert_eq!(status(&h, &run, "review").await, StationStatus::Pending);
    let o = submit(&h, &both[1], json!({})).await;
    assert!(has(&o, "station.entered:review"));
    assert_eq!(status(&h, &run, "review").await, StationStatus::Working);
    let o = h
        .eng
        .confirm(&run, "review", true, None, me())
        .await
        .unwrap();
    assert_eq!(stations_of(&o), ["report/report"]);
}

#[tokio::test]
async fn skipped_station_counts_as_finished_for_a_join() {
    let h = h();
    let (run, o) = start(&h, "cond").await;
    let o = submit(&h, &jobs(&o)[0], json!({})).await;
    assert!(has(&o, "station.skipped:check"));
    assert_eq!(stations_of(&o), ["side/side"]);
    let o = submit(&h, &jobs(&o)[0], json!({})).await;
    assert!(has(&o, "station.entered:join"));
    h.eng.confirm(&run, "join", true, None, me()).await.unwrap();
    assert_eq!(run_status(&h, &run).await, RunStatus::Finished);
    assert_eq!(status(&h, &run, "check").await, StationStatus::Skipped);

    // with the input set, the station runs
    let (run2, o) = start_with(&h, "cond", json!({"mode": "auto"})).await;
    let o = submit(&h, &jobs(&o)[0], json!({})).await;
    let mut js = stations_of(&o);
    js.sort();
    assert_eq!(js, ["check/check", "side/side"]);
    assert_eq!(status(&h, &run2, "check").await, StationStatus::Working);

    // inputs are checked against their schema
    let bad = h
        .eng
        .create(
            CreateIn {
                flow: "cond".into(),
                subject: "s".into(),
                inputs: json!({"mode": "sideways"}),
                ..Default::default()
            },
            me(),
        )
        .await;
    assert!(matches!(bad, Err(FlowError::Invalid(_))), "{bad:?}");
}

#[tokio::test]
async fn foreach_and_tasks_complete_all_before_judging() {
    let h = h();
    let (run, o) = start(&h, "multi").await;
    let o = submit(&h, &jobs(&o)[0], json!({"modules": ["m1", "m2", "m3"]})).await;
    // max_parallel 2
    let first = jobs(&o);
    assert_eq!(stations_of(&o), ["analyze/m1", "analyze/m2"]);
    assert_eq!(first[0].item, Some(json!("m1")));
    let p = first[0].protocol.as_deref().unwrap();
    assert!(
        p.contains(&run) && p.contains("\"m1\"") && p.contains("findings"),
        "{p}"
    );
    let o = submit(&h, &first[0], json!({"findings": [1]})).await;
    assert_eq!(stations_of(&o), ["analyze/m3"]);
    let third = jobs(&o)[0].clone();
    submit(&h, &first[1], json!({"findings": [2]})).await;
    assert_eq!(status(&h, &run, "analyze").await, StationStatus::Working);
    let o = submit(&h, &third, json!({"findings": [3]})).await;
    assert!(has(&o, "station.passed:analyze"));
    let rec = h.eng.record(&run).await.unwrap();
    assert_eq!(
        station_output(&rec.def, &rec, "analyze"),
        json!([{"findings": [1]}, {"findings": [2]}, {"findings": [3]}])
    );

    // static tasks: the program one is dispatched, the external one waits to be claimed
    assert_eq!(stations_of(&o), ["report/summary"]);
    assert!(has(&o, "task.ready:report/sign"));
    let o = submit(&h, &jobs(&o)[0], json!({"text": "ok"})).await;
    assert!(!has(&o, "station.passed:report"));
    let push_claim = h.eng.claim(&run, "report", "summary", me(), 1000).await;
    assert!(push_claim.is_err());
    let c = h
        .eng
        .claim(&run, "report", "sign", me(), 60_000)
        .await
        .unwrap();
    let token = c.value["token"].as_str().unwrap().to_string();
    let o = h
        .eng
        .submit(
            &run,
            "report",
            "sign",
            &token,
            json!({"signed": true}),
            me(),
        )
        .await
        .unwrap();
    assert!(has(&o, "run.finished"), "{:?}", kinds(&o));
    let rec = h.eng.record(&run).await.unwrap();
    assert_eq!(
        station_output(&rec.def, &rec, "report"),
        json!({"summary": {"text": "ok"}, "sign": {"signed": true}})
    );
}

// ── rework ───────────────────────────────────────────────────────────────────

#[tokio::test]
async fn rework_makes_downstream_stale_and_keeps_unrelated_branches() {
    let h = h();
    let (run, o) = start(&h, "par").await;
    let o = submit(&h, &jobs(&o)[0], json!({})).await;
    for j in jobs(&o) {
        submit(&h, &j, json!({"from": j.station})).await;
    }
    assert_eq!(status(&h, &run, "review").await, StationStatus::Working);

    // only what the open station lists
    let no = h.eng.rework(&run, "scan", None, "why", me()).await;
    assert!(matches!(no, Err(FlowError::Invalid(_))), "{no:?}");

    let o = h
        .eng
        .rework(&run, "analyze", None, "redo", me())
        .await
        .unwrap();
    assert!(has(&o, "station.stale:review") && has(&o, "station.reworked:analyze"));
    assert_eq!(stations_of(&o), ["analyze/analyze"]);
    let v = h.eng.get(&run).await.unwrap().record;
    assert_eq!(v.stations["scan"].status, StationStatus::Passed);
    assert_eq!(v.stations["collect"].status, StationStatus::Passed);
    assert_eq!(v.stations["review"].status, StationStatus::Stale);
    assert_eq!(v.stations["report"].status, StationStatus::Pending);
    assert_eq!(v.stations["analyze"].round, 2);
    assert_eq!(
        v.stations["scan"].tasks[0].output,
        Some(json!({"from": "scan"}))
    );

    let o = submit(&h, &jobs(&o)[0], json!({})).await;
    assert!(has(&o, "station.entered:review"));
    assert_eq!(
        h.eng.get(&run).await.unwrap().record.stations["review"].round,
        2
    );
}

#[tokio::test]
async fn partial_rework_redoes_only_the_named_tasks() {
    let h = h();
    let (run, o) = start(&h, "multi").await;
    let all = drive(&h, o, |j| match j.station.as_str() {
        "collect" => json!({"modules": ["m1", "m2", "m3"]}),
        _ => json!({"findings": [j.task]}),
    })
    .await;
    assert!(all.iter().any(|o| has(o, "station.entered:report")));
    let o = h
        .eng
        .rework(
            &run,
            "analyze",
            Some(vec!["m2".into()]),
            "m2 is wrong",
            me(),
        )
        .await
        .unwrap();
    assert_eq!(stations_of(&o), ["analyze/m2"]);
    assert!(has(&o, "station.stale:report"));
    let rec = h.eng.record(&run).await.unwrap();
    let a = &rec.stations["analyze"];
    assert_eq!(
        a.task("m1").unwrap().output,
        Some(json!({"findings": ["m1"]}))
    );
    assert_eq!(a.task("m3").unwrap().status, TaskStatus::Submitted);
    assert_eq!(a.task("m2").unwrap().status, TaskStatus::Claimed);
    assert_eq!(a.round, 2);

    let o = submit(&h, &jobs(&o)[0], json!({"findings": ["again"]})).await;
    assert!(has(&o, "station.passed:analyze") && has(&o, "station.entered:report"));
    let rec = h.eng.record(&run).await.unwrap();
    assert_eq!(
        station_output(&rec.def, &rec, "analyze"),
        json!([{"findings": ["m1"]}, {"findings": ["again"]}, {"findings": ["m3"]}])
    );

    let missing = h
        .eng
        .rework(&run, "analyze", Some(vec!["m9".into()]), "x", me())
        .await;
    assert!(
        matches!(missing, Err(FlowError::NotFound(_))),
        "{missing:?}"
    );
}

#[tokio::test]
async fn rework_closes_the_sessions_of_cancelled_tasks() {
    let h = h();
    let (run, o) = start(&h, "multi").await;
    let o = submit(&h, &jobs(&o)[0], json!({"modules": ["m1", "m2"]})).await;
    let m1 = jobs(&o)[0].clone();
    h.eng
        .attach_session(
            &run,
            "analyze",
            "m1",
            &m1.token,
            "sess-1",
            None,
            Actor::flow("dispatch"),
        )
        .await
        .unwrap();
    let o = h
        .eng
        .rework(&run, "analyze", None, "start over", me())
        .await
        .unwrap();
    assert!(
        o.work.contains(&WorkItem::CloseSession {
            session: "sess-1".into()
        }),
        "{:?}",
        o.work
    );
    assert!(has(&o, "task.cancelled:analyze/m1"));
    // the old token is dead
    let late = submit_err(&h, &m1).await;
    assert!(matches!(late, FlowError::Lease(_)), "{late:?}");
}

async fn submit_err(h: &H, j: &Job) -> FlowError {
    h.eng
        .submit(
            &j.run,
            &j.station,
            &j.task,
            &j.token,
            json!({"findings": []}),
            Actor::flow("dispatch"),
        )
        .await
        .unwrap_err()
}

#[tokio::test]
async fn rework_limit_is_enforced() {
    let h = h();
    let (run, _) = start(&h, "limited").await;
    h.eng.rework(&run, "a", None, "1", me()).await.unwrap();
    let no = h.eng.rework(&run, "a", None, "2", me()).await;
    assert!(
        matches!(no, Err(FlowError::Invalid(ref m)) if m.contains("limit")),
        "{no:?}"
    );
    assert_eq!(
        h.eng.get(&run).await.unwrap().record.stations["a"].rounds_total,
        2
    );
}

#[tokio::test]
async fn on_unmet_reworks_automatically_up_to_max() {
    let h = h();
    h.tools.set("count", json!({"total": 1}));
    let (run, o) = start(&h, "auto").await;
    let o = submit(&h, &jobs(&o)[0], json!({})).await;
    assert!(
        has(&o, "station.unmet:check") && has(&o, "station.reworked:work"),
        "{:?}",
        kinds(&o)
    );
    assert_eq!(stations_of(&o), ["work/work"]);
    let o = submit(&h, &jobs(&o)[0], json!({})).await;
    assert_eq!(stations_of(&o), ["work/work"]);
    let o = submit(&h, &jobs(&o)[0], json!({})).await;
    // max 2 reached: stays unmet
    assert!(jobs(&o).is_empty());
    assert_eq!(status(&h, &run, "check").await, StationStatus::Unmet);
    let rec = h.eng.get(&run).await.unwrap();
    assert_eq!(rec.record.stations["check"].auto_reworks, 2);
    assert_eq!(rec.record.stations["work"].round, 3);
    let unmet = &rec.unmet["check"];
    assert!(unmet[0].detail.contains("total"), "{unmet:?}");
    let calls = h.tools.calls.lock().unwrap().clone();
    assert_eq!(
        calls[0],
        ("count".to_string(), json!({"run": run, "what": "high"}))
    );
    let hist = h.eng.history(&run).await.unwrap();
    assert!(hist
        .iter()
        .any(|x| x.op == "rework" && x.by == Actor::flow("auto")));

    // a re-check that still fails does not rework again
    let o = h.eng.evaluate(&run).await.unwrap();
    assert!(jobs(&o).is_empty() && !has(&o, "station.unmet:check"));
    // the condition comes true: evaluate passes it
    h.tools.set("count", json!({"total": 0}));
    let o = h.eng.evaluate(&run).await.unwrap();
    assert!(has(&o, "run.finished"), "{:?}", kinds(&o));
}

// ── people ───────────────────────────────────────────────────────────────────

#[tokio::test]
async fn force_is_recorded() {
    let h = h();
    h.tools.set("count", json!({"total": 5}));
    let (run, o) = start(&h, "auto").await;
    drive(&h, o, |_| json!({})).await;
    assert_eq!(status(&h, &run, "check").await, StationStatus::Unmet);
    let empty = h.eng.force(&run, "check", "  ", me()).await;
    assert!(matches!(empty, Err(FlowError::Invalid(_))));
    let o = h
        .eng
        .force(&run, "check", "known false positive", me())
        .await
        .unwrap();
    assert!(has(&o, "station.forced:check") && has(&o, "run.finished"));
    let rec = h.eng.record(&run).await.unwrap();
    let f = rec.stations["check"].forced.clone().unwrap();
    assert_eq!(f.by, me());
    assert_eq!(f.reason, "known false positive");
    assert_eq!(f.unmet.len(), 1);
    assert_eq!(f.unmet[0].kind, "tool");
    assert!(h
        .eng
        .history(&run)
        .await
        .unwrap()
        .iter()
        .any(|x| x.op == "force" && x.by == me()));
}

#[tokio::test]
async fn confirm_passes_and_reject_keeps_unmet() {
    let h = h();
    let (run, o) = start(&h, "ok").await;
    assert!(has(&o, "station.entered:a"));
    let v = h.eng.get(&run).await.unwrap();
    assert_eq!(v.unmet["a"][0].detail, "awaiting confirmation");
    let o = h
        .eng
        .confirm(&run, "a", false, Some("not yet"), me())
        .await
        .unwrap();
    assert!(has(&o, "station.unmet:a"));
    let v = h.eng.get(&run).await.unwrap();
    assert_eq!(v.record.stations["a"].status, StationStatus::Unmet);
    assert!(v.unmet["a"][0].detail.contains("not yet"));
    let o = h.eng.confirm(&run, "a", true, None, me()).await.unwrap();
    assert!(has(&o, "run.finished"));
    assert_eq!(
        h.eng.get(&run).await.unwrap().record.stations["a"]
            .confirms
            .len(),
        2
    );
    // a finished run takes no more confirmations
    assert!(h.eng.confirm(&run, "a", true, None, me()).await.is_err());
}

#[tokio::test]
async fn hold_waits_for_release() {
    let h = h();
    let (run, o) = start(&h, "held").await;
    assert!(has(&o, "station.held:a") && jobs(&o).is_empty());
    assert_eq!(status(&h, &run, "a").await, StationStatus::Held);
    let o = h.eng.release(&run, "a", me()).await.unwrap();
    assert!(has(&o, "station.released:a"));
    assert_eq!(stations_of(&o), ["a/a"]);
    assert!(h.eng.release(&run, "a", me()).await.is_err());
}

// ── leases and retries ───────────────────────────────────────────────────────

#[tokio::test]
async fn expired_lease_rejects_the_old_token_and_allows_a_new_claim() {
    let h = h();
    let (run, o) = start(&h, "ext").await;
    assert!(jobs(&o).is_empty() && has(&o, "task.ready:sign/sign"));
    let bob = Actor::new("user", "bob");
    let c = h
        .eng
        .claim(&run, "sign", "sign", bob.clone(), 1000)
        .await
        .unwrap();
    let old = c.value["token"].as_str().unwrap().to_string();
    assert_eq!(c.value["until"], json!(T0 + 1000));
    assert!(h.eng.claim(&run, "sign", "sign", me(), 1000).await.is_err());
    let hb = h
        .eng
        .heartbeat(&run, "sign", "sign", &old, 1000, bob.clone())
        .await
        .unwrap();
    assert_eq!(hb.value["until"], json!(T0 + 1000));

    h.clock.advance(5000);
    let late = h
        .eng
        .submit(&run, "sign", "sign", &old, json!({}), bob.clone())
        .await;
    assert!(matches!(late, Err(FlowError::Lease(_))), "{late:?}");
    let ticked = h.eng.tick().await.unwrap();
    assert_eq!(ticked.len(), 1);
    assert!(has(&ticked[0], "task.expired:sign/sign"));
    assert_eq!(
        task(&h, &run, "sign", "sign").await.status,
        TaskStatus::Ready
    );
    assert!(h.eng.tick().await.unwrap().is_empty());

    let c = h.eng.claim(&run, "sign", "sign", me(), 1000).await.unwrap();
    let new = c.value["token"].as_str().unwrap().to_string();
    assert_ne!(old, new);
    let stale = h
        .eng
        .submit(&run, "sign", "sign", &old, json!({}), bob)
        .await;
    assert!(matches!(stale, Err(FlowError::Lease(_))), "{stale:?}");
    assert!(h
        .eng
        .heartbeat(&run, "sign", "sign", &old, 1000, me())
        .await
        .is_err());
    let o = h
        .eng
        .submit(&run, "sign", "sign", &new, json!({}), me())
        .await
        .unwrap();
    assert!(has(&o, "run.finished"));
    assert_eq!(task(&h, &run, "sign", "sign").await.attempt, 2);
}

#[tokio::test]
async fn expired_push_task_is_dispatched_again_within_its_retries() {
    let h = h();
    let (run, o) = start(&h, "retry").await;
    let first = jobs(&o)[0].clone();
    h.clock.advance(60_001);
    let ticked = h.eng.tick().await.unwrap();
    assert!(has(&ticked[0], "task.expired:a/a") && has(&ticked[0], "task.failed:a/a"));
    let again = jobs(&ticked[0]);
    assert_eq!(again.len(), 1);
    assert_eq!(again[0].attempt, 2);
    assert_ne!(again[0].token, first.token);
    let late = h
        .eng
        .submit(
            &run,
            "a",
            "a",
            &first.token,
            json!({"n": 1}),
            Actor::flow("dispatch"),
        )
        .await;
    assert!(matches!(late, Err(FlowError::Lease(_))), "{late:?}");
    submit(&h, &again[0], json!({"n": 1})).await;
    assert_eq!(status(&h, &run, "a").await, StationStatus::Passed);
}

#[tokio::test]
async fn an_expired_push_task_out_of_retries_fails_instead_of_looping() {
    let h = h();
    let (run, _) = start(&h, "retry").await;
    for _ in 0..2 {
        h.clock.advance(60_001);
        h.eng.tick().await.unwrap();
    }
    let t = task(&h, &run, "a", "a").await;
    assert_eq!((t.status, t.attempt), (TaskStatus::Failed, 2));
    assert_eq!(status(&h, &run, "a").await, StationStatus::Unmet);
    h.clock.advance(60_001);
    assert!(
        h.eng.tick().await.unwrap().is_empty(),
        "nothing left to expire"
    );
}

#[tokio::test]
async fn failed_task_is_retried_then_fails_the_station() {
    let h = h();
    let (run, o) = start(&h, "retry").await;
    let j = jobs(&o)[0].clone();
    assert_eq!(j.attempt, 1);
    let o = h
        .eng
        .fail(&run, "a", "a", &j.token, "boom", Actor::flow("dispatch"))
        .await
        .unwrap();
    let j = jobs(&o)[0].clone();
    assert_eq!(j.attempt, 2);
    let o = h
        .eng
        .fail(
            &run,
            "a",
            "a",
            &j.token,
            "boom again",
            Actor::flow("dispatch"),
        )
        .await
        .unwrap();
    assert!(jobs(&o).is_empty());
    assert!(has(&o, "task.failed:a/a") && has(&o, "station.unmet:a"));
    let v = h.eng.get(&run).await.unwrap();
    assert_eq!(v.record.stations["a"].tasks[0].status, TaskStatus::Failed);
    assert_eq!(v.unmet["a"][0].detail, "boom again");

    let o = h.eng.retry(&run, "a", "a", me()).await.unwrap();
    let j = jobs(&o)[0].clone();
    assert_eq!(j.attempt, 1);
    assert_eq!(status(&h, &run, "a").await, StationStatus::Working);
    let o = submit(&h, &j, json!({})).await;
    assert!(has(&o, "run.finished"));
}

// ── children ─────────────────────────────────────────────────────────────────

#[tokio::test]
async fn subflow_station_passes_when_every_child_finished() {
    let h = h();
    let (run, o) = start_with(&h, "parent", json!({"items": ["x", "y"]})).await;
    let created: Vec<&str> = o
        .events
        .iter()
        .filter(|e| e.kind == "run.created")
        .map(|e| e.run.as_str())
        .collect();
    assert_eq!(created, [run.as_str(), "child-1", "child-2"]);
    let child = h.eng.get("child-1").await.unwrap().record;
    assert_eq!(child.subject, "x");
    assert_eq!(child.parent.as_ref().unwrap().run, run);
    assert_eq!(
        task(&h, &run, "split", "x").await.child.as_deref(),
        Some("child-1")
    );

    let js = jobs(&o);
    assert_eq!(js.len(), 2);
    let o = submit(&h, &js[0], json!({"done": "x"})).await;
    assert!(o
        .events
        .iter()
        .any(|e| e.kind == "run.finished" && e.run == "child-1"));
    assert_eq!(status(&h, &run, "split").await, StationStatus::Working);
    let o = submit(&h, &js[1], json!({"done": "y"})).await;
    assert!(
        o.events
            .iter()
            .any(|e| e.kind == "run.finished" && e.run == run),
        "{:?}",
        kinds(&o)
    );
    let out = task(&h, &run, "split", "y").await.output.unwrap();
    assert_eq!(out["run"], json!("child-2"));
    assert_eq!(out["stations"]["do"], json!({"done": "y"}));
}

#[tokio::test]
async fn cancelling_a_parent_cancels_its_children() {
    let h = h();
    let (run, _) = start_with(&h, "parent", json!({"items": ["x"]})).await;
    let o = h.eng.cancel(&run, "no longer needed", me()).await.unwrap();
    assert!(o
        .events
        .iter()
        .any(|e| e.kind == "run.cancelled" && e.run == "child-1"));
    assert_eq!(run_status(&h, "child-1").await, RunStatus::Cancelled);
    assert_eq!(run_status(&h, &run).await, RunStatus::Cancelled);
    assert!(h.eng.cancel(&run, "again", me()).await.is_err());
}

#[tokio::test]
async fn cancelled_child_fails_the_parent_task() {
    let h = h();
    let (run, _) = start_with(&h, "parent", json!({"items": ["x"]})).await;
    let o = h.eng.cancel("child-1", "abandon", me()).await.unwrap();
    assert!(o
        .events
        .iter()
        .any(|e| e.kind == "task.failed" && e.run == run));
    assert_eq!(status(&h, &run, "split").await, StationStatus::Unmet);
}

// ── sessions, gates ──────────────────────────────────────────────────────────

#[tokio::test]
async fn session_tasks_carry_protocol_and_share_a_per_run_session() {
    let h = h();
    let (run, o) = start(&h, "chat").await;
    let j = jobs(&o)[0].clone();
    assert_eq!(j.session_key.as_deref(), Some("run"));
    assert!(j.protocol.as_deref().unwrap().contains("StructuredOutput"));
    let skill_digest = o.work.iter().find_map(|w| match w {
        WorkItem::Task { skill_digest, .. } => skill_digest.clone(),
        _ => None,
    });
    assert_eq!(
        skill_digest,
        Packs::skill(h.packs.as_ref(), "analyze").map(|(_, d)| d)
    );
    h.eng
        .attach_session(
            &run,
            "one",
            "one",
            &j.token,
            "s-9",
            Some("run"),
            Actor::flow("dispatch"),
        )
        .await
        .unwrap();
    submit(&h, &j, json!({})).await;
    let o = h.eng.confirm(&run, "one", true, None, me()).await.unwrap();
    let session = o.work.iter().find_map(|w| match w {
        WorkItem::Task { session, .. } => session.clone(),
        _ => None,
    });
    assert_eq!(session.as_deref(), Some("s-9"));
}

#[tokio::test]
async fn gate_results_are_per_round() {
    let h = h();
    let (run, o) = start(&h, "gated").await;
    assert!(o.work.contains(&WorkItem::Gate {
        run: run.clone(),
        station: "lint".into(),
        round: 1,
        gate: "lint".into()
    }));
    assert_eq!(status(&h, &run, "lint").await, StationStatus::Working);
    let fail = GateResult {
        pass: false,
        detail: "3 warnings".into(),
    };
    let wrong = h
        .eng
        .gate_result(&run, "lint", 7, "lint", fail.clone())
        .await
        .unwrap();
    assert_eq!(wrong.value["applied"], json!(false));
    let o = h
        .eng
        .gate_result(&run, "lint", 1, "lint", fail.clone())
        .await
        .unwrap();
    assert!(has(&o, "station.unmet:lint"));
    let o = h
        .eng
        .rework(&run, "lint", None, "fixed", me())
        .await
        .unwrap();
    assert!(o
        .work
        .iter()
        .any(|w| matches!(w, WorkItem::Gate { round: 2, .. })));
    // the old round's result arrives late: ignored
    let late = h
        .eng
        .gate_result(&run, "lint", 1, "lint", fail)
        .await
        .unwrap();
    assert_eq!(late.value["applied"], json!(false));
    let o = h
        .eng
        .gate_result(
            &run,
            "lint",
            2,
            "lint",
            GateResult {
                pass: true,
                detail: String::new(),
            },
        )
        .await
        .unwrap();
    assert!(has(&o, "run.finished"));
}

// ── commit, authorization, pinning ───────────────────────────────────────────

/// A store that reports a conflict for the next `n` commits.
struct Flaky {
    inner: MemStore,
    conflicts: AtomicUsize,
}

#[async_trait]
impl RunStore for Flaky {
    async fn alloc_id(&self, flow: &str) -> std::result::Result<String, StoreError> {
        self.inner.alloc_id(flow).await
    }
    async fn load(&self, run: &str) -> std::result::Result<Option<RunRecord>, StoreError> {
        self.inner.load(run).await
    }
    async fn list(&self, q: &RunQuery) -> std::result::Result<Vec<RunSummary>, StoreError> {
        self.inner.list(q).await
    }
    async fn commit(&self, cs: &ChangeSet) -> std::result::Result<(), StoreError> {
        if self
            .conflicts
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
            .is_ok()
        {
            return Err(StoreError::Conflict("injected".into()));
        }
        self.inner.commit(cs).await
    }
    async fn history(&self, run: &str) -> std::result::Result<Vec<History>, StoreError> {
        self.inner.history(run).await
    }
}

#[tokio::test]
async fn a_conflict_yields_no_events_and_a_retry_yields_them_once() {
    let store = Arc::new(Flaky {
        inner: MemStore::new(),
        conflicts: AtomicUsize::new(0),
    });
    let eng =
        Engine::new(store.clone(), Arc::new(all_packs())).with_clock(Arc::new(FixedClock(T0)));
    let o = eng
        .create(
            CreateIn {
                flow: "ok".into(),
                subject: "s".into(),
                ..Default::default()
            },
            me(),
        )
        .await
        .unwrap();
    let run = o.value["run"].as_str().unwrap().to_string();

    store.conflicts.store(100, Ordering::SeqCst);
    let r = eng.confirm(&run, "a", true, None, me()).await;
    assert!(matches!(r, Err(FlowError::Conflict(_))), "{r:?}");
    assert_eq!(
        eng.get(&run).await.unwrap().record.status,
        RunStatus::Active
    );
    let before = eng.history(&run).await.unwrap().len();

    store.conflicts.store(2, Ordering::SeqCst);
    let o = eng.confirm(&run, "a", true, None, me()).await.unwrap();
    assert_eq!(
        kinds(&o)
            .iter()
            .filter(|k| *k == "station.passed:a")
            .count(),
        1
    );
    assert_eq!(kinds(&o).iter().filter(|k| *k == "run.finished").count(), 1);
    let hist = eng.history(&run).await.unwrap();
    assert_eq!(hist.iter().filter(|x| x.op == "confirm").count(), 1);
    assert!(hist.len() > before);
    assert!(hist.windows(2).all(|w| w[1].seq == w[0].seq + 1));
}

/// Only carol may confirm for `reviewer`; nobody may force; subject `blocked` cannot start.
struct Rules;

impl Authorizer for Rules {
    fn check(
        &self,
        op: &Op,
        by: &Actor,
        _run: Option<&RunRecord>,
    ) -> std::result::Result<(), String> {
        match op {
            Op::Create { subject, .. } if subject == "blocked" => Err("blocked subject".into()),
            Op::Confirm { who: Some(w), .. } if w == "reviewer" && by.id != "carol" => {
                Err(format!("{by} is not a reviewer"))
            }
            Op::Force { .. } => Err("forcing is off".into()),
            _ => Ok(()),
        }
    }
}

#[tokio::test]
async fn the_authorizer_can_say_no() {
    let store = Arc::new(MemStore::new());
    let eng = Engine::new(store, Arc::new(all_packs())).with_authorizer(Arc::new(Rules));
    let blocked = eng
        .create(
            CreateIn {
                flow: "ok".into(),
                subject: "blocked".into(),
                ..Default::default()
            },
            me(),
        )
        .await;
    assert!(matches!(blocked, Err(FlowError::Denied(_))), "{blocked:?}");
    let o = eng
        .create(
            CreateIn {
                flow: "ok".into(),
                subject: "fine".into(),
                ..Default::default()
            },
            me(),
        )
        .await
        .unwrap();
    let run = o.value["run"].as_str().unwrap().to_string();
    let r = eng.confirm(&run, "a", true, None, me()).await;
    assert!(
        matches!(r, Err(FlowError::Denied(ref m)) if m.contains("not a reviewer")),
        "{r:?}"
    );
    let r = eng.force(&run, "a", "because", me()).await;
    assert!(matches!(r, Err(FlowError::Denied(_))), "{r:?}");
    let o = eng
        .confirm(&run, "a", true, None, Actor::new("user", "carol"))
        .await
        .unwrap();
    assert!(has(&o, "run.finished"));
}

#[tokio::test]
async fn a_running_run_keeps_its_pinned_definition() {
    let h = h();
    let (run, o) = start(&h, "seq").await;
    let digest = h.eng.get(&run).await.unwrap().record.def_digest;
    assert_eq!(
        Some(digest.clone()),
        Packs::flow(h.packs.as_ref(), "seq").map(|(_, d)| d)
    );

    h.packs.put_flow(
        "name: seq\nstations:\n  - {id: a, executor: {kind: program, step: work}, output: {type: object}, exit: [{kind: output}]}\n  - {id: z, exit: [{kind: confirm}]}",
    );
    let o = submit(&h, &jobs(&o)[0], json!({"n": 1})).await;
    assert_eq!(stations_of(&o), ["b/b"]);
    let rec = h.eng.record(&run).await.unwrap();
    assert_eq!(rec.def_digest, digest);
    assert!(rec.def.station("b").is_some() && rec.def.station("z").is_none());

    let (run2, _) = start(&h, "seq").await;
    let rec2 = h.eng.get(&run2).await.unwrap().record;
    assert_ne!(rec2.def_digest, digest);
    assert!(rec2.stations.contains_key("z"));
}

#[tokio::test]
async fn list_and_views() {
    let h = h();
    let (a, _) = start(&h, "ok").await;
    h.clock.advance(1);
    let (b, _) = start(&h, "seq").await;
    h.eng.confirm(&a, "a", true, None, me()).await.unwrap();
    let all = h.eng.list(RunQuery::default()).await.unwrap();
    assert_eq!(
        all.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(),
        [b.as_str(), a.as_str()]
    );
    let active = h
        .eng
        .list(RunQuery {
            status: Some(RunStatus::Active),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(active.len(), 1);
    assert_eq!(active[0].open, ["a"]);
    assert_eq!(active[0].next_expiry, Some(T0 + 1 + 60_000));
    let v = h.eng.get(&b).await.unwrap();
    assert_eq!(v.unmet["a"][0].detail, "0 of 1 tasks submitted");
    let missing = h.eng.get("nope-1").await;
    assert!(matches!(missing, Err(FlowError::NotFound(_))));
    let problems = h.eng.validate("name: x\nstations: [{id: a}]").await;
    assert_eq!(problems.len(), 1);
    let _: Value = serde_json::to_value(&v).unwrap();
}

/// The module spawns these on tokio: every operation's future must be `Send`.
#[allow(dead_code)]
fn operations_are_send(e: &Engine) {
    fn send<T: Send>(_: T) {}
    send(e.create(CreateIn::default(), me()));
    send(e.submit("r", "s", "t", "k", Value::Null, me()));
    send(e.claim("r", "s", "t", me(), 1));
    send(e.rework("r", "s", None, "x", me()));
    send(e.evaluate("r"));
    send(e.tick());
    send(e.gate_result(
        "r",
        "s",
        1,
        "g",
        GateResult {
            pass: true,
            detail: String::new(),
        },
    ));
}

#[tokio::test]
async fn dispatched_work_rebuilds_what_the_dispatcher_holds() {
    let h = h();
    let (run, o) = start(&h, "multi").await;
    let o = submit(&h, &jobs(&o)[0], json!({"modules": ["m1", "m2"]})).await;
    let handed: Vec<WorkItem> = o
        .work
        .iter()
        .filter(|w| matches!(w, WorkItem::Task { .. }))
        .cloned()
        .collect();
    assert_eq!(handed.len(), 2);
    let m1 = jobs(&o)[0].clone();
    h.eng
        .attach_session(
            &run,
            "analyze",
            "m1",
            &m1.token,
            "system/s-1",
            None,
            Actor::flow("dispatch"),
        )
        .await
        .unwrap();
    let rec = h.eng.record(&run).await.unwrap();
    let again = dispatched_work(&rec, &[], h.clock.now_ms());
    assert_eq!(again.len(), 2, "{again:?}");
    for (w, was) in again.iter().zip(&handed) {
        match (w, was) {
            (
                WorkItem::Task {
                    token,
                    protocol,
                    task,
                    session,
                    ..
                },
                WorkItem::Task {
                    token: t0,
                    protocol: p0,
                    ..
                },
            ) => {
                assert_eq!(token, t0, "the same claim, not a new one");
                assert_eq!(protocol, p0);
                if task == "m1" {
                    assert_eq!(session.as_deref(), Some("system/s-1"));
                }
            }
            _ => unreachable!(),
        }
    }
    // A lease that ran out is not the dispatcher's any more: `tick` claims it again.
    h.clock.advance(120_000);
    assert!(dispatched_work(&rec, &[], h.clock.now_ms()).is_empty());
}

#[tokio::test]
async fn dispatched_work_includes_gates_still_waiting() {
    let h = h();
    let (run, o) = start(&h, "gated").await;
    assert!(o.work.iter().any(|w| matches!(w, WorkItem::Gate { .. })));
    let rec = h.eng.record(&run).await.unwrap();
    let again = dispatched_work(&rec, &[], h.clock.now_ms());
    assert_eq!(
        again,
        vec![WorkItem::Gate {
            run: run.clone(),
            station: "lint".into(),
            round: 1,
            gate: "lint".into()
        }]
    );
    h.eng
        .gate_result(
            &run,
            "lint",
            1,
            "lint",
            GateResult {
                pass: true,
                detail: String::new(),
            },
        )
        .await
        .unwrap();
    let rec = h.eng.record(&run).await.unwrap();
    assert!(dispatched_work(&rec, &[], h.clock.now_ms()).is_empty());
}
