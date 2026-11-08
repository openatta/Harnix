//! What the review of the flow branch found, one regression case each: tokens stay with their
//! claimants, push tasks are handed in by the flow alone, an unresolved fan-out never passes,
//! leases are bounded, expiry respects retries, runs close their shared sessions, recursive
//! subflows are refused and nesting is capped, a failing run does not stop the tick.

mod common;

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use common::*;
use harnix_flow::*;
use serde_json::json;

const RESUB: &str = "name: resub\nstations: [{id: s, executor: {kind: external}, output: {type: object}, exit: [{kind: output}, {kind: confirm}]}]";
const FAN: &str = r#"
name: fan
stations:
  - {id: a, executor: {kind: program, step: work}, output: {type: object}, exit: [{kind: output}]}
  - id: b
    foreach: ${stations.a.output.list}
    executor: {kind: program, step: work}
    output: {type: object}
    exit: [{kind: output}]
"#;
const SHARED: &str = "name: shared\nstations:\n  - {id: one, executor: {kind: session, scene: harnix, session: per_run}, skill: analyze, output: {type: object}, exit: [{kind: output}]}\n  - {id: two, exit: [{kind: confirm}]}";
const OWN: &str = "name: own\nstations: [{id: s, executor: {kind: session, scene: harnix}, skill: analyze, retry: {max: 1}, output: {type: object}, exit: [{kind: output}]}]";
const CHILD: &str = "name: leaf\nstations: [{id: s, exit: [{kind: confirm}]}]";

fn packs_all() -> MemPacks {
    packs()
        .flow(RESUB)
        .flow(FAN)
        .flow(SHARED)
        .flow(OWN)
        .flow(CHILD)
        .flow("name: ext\nstations: [{id: sign, executor: {kind: external, who: reviewer}, output: {type: object}, exit: [{kind: output}]}]")
        .flow("name: seq\nstations: [{id: a, executor: {kind: program, step: work}, output: {type: object}, exit: [{kind: output}]}]")
        .flow("name: parent\nstations: [{id: c, executor: {kind: subflow, flow: leaf}, exit: [{kind: children}]}]")
}

fn session_job(o: &Outcome) -> (String, String, String, String) {
    o.work
        .iter()
        .find_map(|w| match w {
            WorkItem::Task {
                run,
                station,
                task,
                token,
                ..
            } => Some((run.clone(), station.clone(), task.clone(), token.clone())),
            _ => None,
        })
        .expect("a task")
}

fn closes(o: &Outcome) -> Vec<String> {
    o.work
        .iter()
        .filter_map(|w| match w {
            WorkItem::CloseSession { session } => Some(session.clone()),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn readers_never_see_a_lease_token() {
    let h = harness(packs_all());
    let (run, o) = start(&h, "seq").await;
    let pushed = jobs(&o)[0].token.clone();
    let (ext, _) = start(&h, "ext").await;
    let claimed = h
        .eng
        .claim(&ext, "sign", "sign", me(), 60_000)
        .await
        .unwrap();
    let token = claimed.value["token"].as_str().unwrap().to_string();
    for (r, t) in [(&run, &pushed), (&ext, &token)] {
        let view = h.eng.get(r).await.unwrap();
        let json = serde_json::to_string(&view).unwrap();
        assert!(!json.contains(t.as_str()), "a view carries a token: {json}");
        let lease = view
            .record
            .stations
            .values()
            .flat_map(|s| &s.tasks)
            .find_map(|t| t.lease.clone())
            .unwrap();
        assert!(
            lease.token.is_empty() && lease.until > T0,
            "who and until stay: {lease:?}"
        );
        // The host's own record still has it.
        let full = h.eng.record(r).await.unwrap();
        assert!(serde_json::to_string(&full).unwrap().contains(t.as_str()));
    }
}

#[derive(Default)]
struct Seen(Mutex<Vec<String>>);

impl Authorizer for Seen {
    fn check(&self, op: &Op, by: &Actor, _: Option<&RunRecord>) -> std::result::Result<(), String> {
        let line = match op {
            Op::Submit { executor, .. } => format!("submit {executor} {by}"),
            Op::Fail { executor, .. } => format!("fail {executor} {by}"),
            Op::Heartbeat { executor, .. } => format!("heartbeat {executor} {by}"),
            other => other.name().to_string(),
        };
        self.0.lock().unwrap().push(line);
        Ok(())
    }
}

#[tokio::test]
async fn push_tasks_are_handed_in_by_the_flow_alone_even_with_the_token() {
    let seen = Arc::new(Seen::default());
    let store = Arc::new(MemStore::new());
    let mut h = harness_on(packs_all(), store);
    h.eng = Engine::new(h.store.clone(), h.packs.clone())
        .with_clock(h.clock.clone())
        .with_lease_ms(60_000)
        .with_authorizer(seen.clone());

    let (run, o) = start(&h, "seq").await;
    let j = jobs(&o)[0].clone();
    for by in [me(), Actor::agent("system/s")] {
        let s = h
            .eng
            .submit(&run, "a", "a", &j.token, json!({}), by.clone())
            .await;
        assert!(matches!(s, Err(FlowError::Denied(_))), "{by}: {s:?}");
        let f = h.eng.fail(&run, "a", "a", &j.token, "x", by.clone()).await;
        assert!(matches!(f, Err(FlowError::Denied(_))), "{by}: {f:?}");
        let b = h
            .eng
            .heartbeat(&run, "a", "a", &j.token, 1000, by.clone())
            .await;
        assert!(matches!(b, Err(FlowError::Denied(_))), "{by}: {b:?}");
    }
    h.eng
        .heartbeat(&run, "a", "a", &j.token, 1000, Actor::flow("dispatch"))
        .await
        .unwrap();

    // A session task: its own session may hand it in, a device may not.
    let (own, o) = start(&h, "own").await;
    let (_, st, task, token) = session_job(&o);
    let s = h
        .eng
        .submit(&own, &st, &task, &token, json!({}), me())
        .await;
    assert!(matches!(s, Err(FlowError::Denied(_))), "{s:?}");
    h.eng
        .submit(
            &own,
            &st,
            &task,
            &token,
            json!({}),
            Actor::agent("system/s-1"),
        )
        .await
        .unwrap();

    // A subflow task: nobody but the flow.
    let (parent, _) = start(&h, "parent").await;
    let rec = h.eng.record(&parent).await.unwrap();
    let child_token = rec.stations["c"].tasks[0]
        .lease
        .as_ref()
        .unwrap()
        .token
        .clone();
    let s = h
        .eng
        .submit(&parent, "c", "c", &child_token, json!({}), me())
        .await;
    assert!(matches!(s, Err(FlowError::Denied(_))), "{s:?}");

    let seen = seen.0.lock().unwrap().clone();
    assert!(
        seen.contains(&"heartbeat program flow:dispatch".to_string()),
        "{seen:?}"
    );
    assert!(
        seen.contains(&"submit session agent:system/s-1".to_string()),
        "{seen:?}"
    );
}

#[tokio::test]
async fn a_resubmission_needs_a_live_lease_and_a_heartbeat_extends_it() {
    let h = harness(packs_all());
    let (run, _) = start(&h, "resub").await;
    let c = h.eng.claim(&run, "s", "s", me(), 1000).await.unwrap();
    let token = c.value["token"].as_str().unwrap().to_string();
    h.eng
        .submit(&run, "s", "s", &token, json!({"v": 1}), me())
        .await
        .unwrap();
    h.eng
        .submit(&run, "s", "s", &token, json!({"v": 2}), me())
        .await
        .unwrap();
    let hb = h
        .eng
        .heartbeat(&run, "s", "s", &token, 5000, me())
        .await
        .unwrap();
    assert_eq!(hb.value["until"], json!(T0 + 5000));
    h.clock.advance(4000);
    h.eng
        .submit(&run, "s", "s", &token, json!({"v": 3}), me())
        .await
        .unwrap();
    h.clock.advance(2000);
    let late = h
        .eng
        .submit(&run, "s", "s", &token, json!({"v": 4}), me())
        .await;
    assert!(matches!(late, Err(FlowError::Lease(_))), "{late:?}");
    assert_eq!(task(&h, &run, "s", "s").await.output, Some(json!({"v": 3})));
}

#[tokio::test]
async fn leases_are_capped_and_never_overflow() {
    let h = harness(packs_all());
    let (run, _) = start(&h, "ext").await;
    let c = h
        .eng
        .claim(&run, "sign", "sign", me(), u64::MAX)
        .await
        .unwrap();
    assert_eq!(c.value["until"], json!(T0 + MAX_LEASE_MS));
    let token = c.value["token"].as_str().unwrap().to_string();
    let hb = h
        .eng
        .heartbeat(&run, "sign", "sign", &token, u64::MAX, me())
        .await
        .unwrap();
    assert_eq!(hb.value["until"], json!(T0 + MAX_LEASE_MS));
}

#[tokio::test]
async fn an_unresolved_fan_out_stays_unmet_and_an_empty_one_passes() {
    let h = harness(packs_all());
    let (run, o) = start(&h, "fan").await;
    submit(&h, &jobs(&o)[0], json!({})).await;
    assert_eq!(status(&h, &run, "b").await, StationStatus::Unmet);
    h.eng.evaluate(&run).await.unwrap();
    let view = h.eng.get(&run).await.unwrap();
    assert_eq!(
        view.record.stations["b"].status,
        StationStatus::Unmet,
        "evaluate does not pass it"
    );
    assert!(
        view.unmet["b"].iter().any(|c| c.kind == "foreach"),
        "{:?}",
        view.unmet
    );

    let (empty, o) = start(&h, "fan").await;
    submit(&h, &jobs(&o)[0], json!({"list": []})).await;
    assert_eq!(run_status(&h, &empty).await, RunStatus::Finished);
}

#[tokio::test]
async fn retry_needs_an_open_station() {
    let h = harness(packs_all());
    let (run, o) = start(&h, "fan").await;
    let j = jobs(&o)[0].clone();
    h.eng
        .fail(&run, "a", "a", &j.token, "boom", Actor::flow("dispatch"))
        .await
        .unwrap();
    assert_eq!(status(&h, &run, "a").await, StationStatus::Unmet);
    h.eng.force(&run, "a", "move on", me()).await.unwrap();
    assert_eq!(run_status(&h, &run).await, RunStatus::Active);
    assert_eq!(task(&h, &run, "a", "a").await.status, TaskStatus::Failed);
    let r = h.eng.retry(&run, "a", "a", me()).await;
    assert!(
        matches!(r, Err(FlowError::Invalid(ref m)) if m.contains("Passed")),
        "{r:?}"
    );
}

#[tokio::test]
async fn a_finished_or_cancelled_run_closes_its_shared_sessions() {
    let h = harness(packs_all());
    for cancel in [false, true] {
        let (run, o) = start(&h, "shared").await;
        let (_, st, task, token) = session_job(&o);
        h.eng
            .attach_session(
                &run,
                &st,
                &task,
                &token,
                "system/shared-1",
                Some("run"),
                Actor::flow("dispatch"),
            )
            .await
            .unwrap();
        let o = if cancel {
            h.eng.cancel(&run, "stop", me()).await.unwrap()
        } else {
            h.eng
                .submit(&run, &st, &task, &token, json!({}), Actor::flow("dispatch"))
                .await
                .unwrap();
            h.eng.confirm(&run, "two", true, None, me()).await.unwrap()
        };
        assert_eq!(closes(&o), ["system/shared-1"], "cancel={cancel}");
    }
}

#[tokio::test]
async fn an_expired_session_task_closes_its_own_session_before_it_is_dispatched_again() {
    let h = harness(packs_all());
    let (run, o) = start(&h, "own").await;
    let (_, st, tid, token) = session_job(&o);
    h.eng
        .attach_session(
            &run,
            &st,
            &tid,
            &token,
            "system/old",
            None,
            Actor::flow("dispatch"),
        )
        .await
        .unwrap();
    h.clock.advance(60_001);
    let ticked = h.eng.tick().await.unwrap();
    assert_eq!(closes(&ticked[0]), ["system/old"]);
    let again = jobs(&ticked[0]);
    assert_eq!(again.len(), 1);
    assert!(task(&h, &run, "s", "s").await.session.is_none());
}

#[tokio::test]
async fn recursive_subflows_are_refused_and_nesting_is_capped() {
    let set = packs()
        .flow("name: ping\nstations: [{id: s, executor: {kind: subflow, flow: pong}, exit: [{kind: children}]}]")
        .flow("name: pong\nstations: [{id: s, executor: {kind: subflow, flow: ping}, exit: [{kind: children}]}]")
        .flow("name: me-again\nstations: [{id: s, executor: {kind: subflow, flow: me-again}, exit: [{kind: children}]}]");
    for name in ["ping", "me-again"] {
        let (def, _) = Packs::flow(&set, name).unwrap();
        let problems = validate(&def, &set);
        assert!(
            problems.iter().any(|p| p.message.contains("leads back")),
            "{name}: {problems:?}"
        );
    }

    // A chain of distinct flows deeper than the cap: the deepest spawn fails its task.
    let mut chain = packs();
    let depth = MAX_SUBFLOW_DEPTH + 2;
    for i in 0..depth {
        let yaml = if i + 1 == depth {
            format!("name: d{i}\nstations: [{{id: s, exit: [{{kind: confirm}}]}}]")
        } else {
            format!(
                "name: d{i}\nstations: [{{id: s, executor: {{kind: subflow, flow: d{}}}, exit: [{{kind: children}}]}}]",
                i + 1
            )
        };
        chain = chain.flow(&yaml);
    }
    let h = harness(chain);
    let (_, o) = start(&h, "d0").await;
    let failed: Vec<&Event> = o
        .events
        .iter()
        .filter(|e| e.kind == "task.failed")
        .collect();
    assert_eq!(failed.len(), 1, "{:?}", kinds(&o));
    assert!(failed[0].data["error"].as_str().unwrap().contains("deeper"));
    let created = o.events.iter().filter(|e| e.kind == "run.created").count();
    assert_eq!(
        created,
        MAX_SUBFLOW_DEPTH + 1,
        "the root and {MAX_SUBFLOW_DEPTH} levels of children"
    );
}

/// A store whose commits fail for one run.
struct Flaky {
    inner: Arc<MemStore>,
    bad: Mutex<Option<String>>,
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
        let bad = self.bad.lock().unwrap().clone();
        if cs.records.iter().any(|(r, _)| Some(&r.id) == bad.as_ref()) {
            return Err(StoreError::Other("disk on fire".into()));
        }
        self.inner.commit(cs).await
    }
    async fn history(&self, run: &str) -> std::result::Result<Vec<History>, StoreError> {
        self.inner.history(run).await
    }
}

#[tokio::test]
async fn a_run_that_fails_to_tick_does_not_stop_the_others() {
    let flaky = Arc::new(Flaky {
        inner: Arc::new(MemStore::new()),
        bad: Mutex::new(None),
    });
    let clock = Arc::new(ManualClock::new(T0));
    let eng = Engine::new(flaky.clone(), Arc::new(packs_all())).with_clock(clock.clone());
    let mut runs = vec![];
    for _ in 0..2 {
        let o = eng
            .create(
                CreateIn {
                    flow: "ext".into(),
                    subject: "x".into(),
                    ..Default::default()
                },
                me(),
            )
            .await
            .unwrap();
        let run = o.value["run"].as_str().unwrap().to_string();
        eng.claim(&run, "sign", "sign", me(), 1000).await.unwrap();
        runs.push(run);
    }
    // Newest first: the first run ticked is the second created.
    *flaky.bad.lock().unwrap() = Some(runs[1].clone());
    clock.advance(5000);
    let ticked = eng
        .tick()
        .await
        .expect("a failing run is skipped, not an error");
    assert_eq!(ticked.len(), 1);
    assert_eq!(ticked[0].events[0].run, runs[0]);
}
