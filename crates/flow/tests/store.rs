//! One contract, two stores: whatever MemStore promises, FileStore promises too.

mod common;

use harnix_flow::*;
use serde_json::{json, Value};

fn record(id: &str, flow: &str, subject: &str, rev: u64, at: u64) -> RunRecord {
    let def = FlowDef::parse(&format!(
        "name: {flow}\nstations: [{{id: a, exit: [{{kind: confirm}}]}}]"
    ))
    .unwrap();
    RunRecord {
        id: id.into(),
        flow: flow.into(),
        def_digest: "d".into(),
        title: String::new(),
        subject: subject.into(),
        inputs: json!({}),
        status: RunStatus::Active,
        rev,
        seq: 0,
        created_at: at,
        created_by: Actor::new("user", "u"),
        finished_at: None,
        parent: None,
        stations: [("a".to_string(), StationRec::new())].into(),
        session_keys: Default::default(),
        def,
    }
}

fn hist(run: &str, seq: u64, op: &str) -> History {
    History {
        run: run.into(),
        seq,
        at: seq,
        by: Actor::new("user", "u"),
        op: op.into(),
        station: None,
        task: None,
        detail: Value::Null,
    }
}

async fn contract(s: &dyn RunStore) {
    // ids: per flow, sequential
    assert_eq!(s.alloc_id("demo").await.unwrap(), "demo-1");
    assert_eq!(s.alloc_id("demo").await.unwrap(), "demo-2");
    assert_eq!(s.alloc_id("other").await.unwrap(), "other-1");

    // missing
    assert_eq!(s.load("demo-1").await.unwrap(), None);
    assert!(s.history("demo-1").await.unwrap().is_empty());

    // create: expected None
    let a = record("demo-1", "demo", "x", 1, 10);
    s.commit(&ChangeSet {
        records: vec![(a.clone(), None)],
        history: vec![hist("demo-1", 1, "create")],
    })
    .await
    .unwrap();
    assert_eq!(s.load("demo-1").await.unwrap(), Some(a.clone()));

    // creating again conflicts
    let again = s
        .commit(&ChangeSet {
            records: vec![(a.clone(), None)],
            history: vec![],
        })
        .await;
    assert!(matches!(again, Err(StoreError::Conflict(_))), "{again:?}");

    // update with the right rev
    let mut a2 = a.clone();
    a2.rev = 2;
    a2.title = "two".into();
    s.commit(&ChangeSet {
        records: vec![(a2.clone(), Some(1))],
        history: vec![hist("demo-1", 2, "edit")],
    })
    .await
    .unwrap();
    assert_eq!(s.load("demo-1").await.unwrap().unwrap().title, "two");

    // a stale rev conflicts, and a change set is all or nothing
    let b = record("demo-2", "demo", "y", 1, 20);
    let mut a3 = a2.clone();
    a3.rev = 3;
    a3.title = "three".into();
    let r = s
        .commit(&ChangeSet {
            records: vec![(b.clone(), None), (a3, Some(1))],
            history: vec![hist("demo-2", 1, "create"), hist("demo-1", 3, "lost")],
        })
        .await;
    assert!(matches!(r, Err(StoreError::Conflict(_))), "{r:?}");
    assert_eq!(s.load("demo-2").await.unwrap(), None);
    assert_eq!(s.load("demo-1").await.unwrap().unwrap().title, "two");
    assert_eq!(s.history("demo-1").await.unwrap().len(), 2);

    // two records at once
    let mut a4 = a2.clone();
    a4.rev = 3;
    a4.status = RunStatus::Finished;
    s.commit(&ChangeSet {
        records: vec![(b.clone(), None), (a4, Some(2))],
        history: vec![hist("demo-2", 1, "create"), hist("demo-1", 3, "finish")],
    })
    .await
    .unwrap();
    let h = s.history("demo-1").await.unwrap();
    assert_eq!(
        h.iter().map(|x| x.op.as_str()).collect::<Vec<_>>(),
        ["create", "edit", "finish"]
    );
    assert_eq!(h.iter().map(|x| x.seq).collect::<Vec<_>>(), [1, 2, 3]);

    let o = record("other-1", "other", "x", 1, 30);
    s.commit(&ChangeSet {
        records: vec![(o, None)],
        history: vec![],
    })
    .await
    .unwrap();

    // list: newest first, filters, limit
    let ids = |v: Vec<RunSummary>| v.into_iter().map(|x| x.id).collect::<Vec<_>>();
    assert_eq!(
        ids(s.list(&RunQuery::default()).await.unwrap()),
        ["other-1", "demo-2", "demo-1"]
    );
    let q = RunQuery {
        flow: Some("demo".into()),
        ..Default::default()
    };
    assert_eq!(ids(s.list(&q).await.unwrap()), ["demo-2", "demo-1"]);
    let q = RunQuery {
        status: Some(RunStatus::Active),
        subject: Some("x".into()),
        ..Default::default()
    };
    assert_eq!(ids(s.list(&q).await.unwrap()), ["other-1"]);
    let q = RunQuery {
        limit: Some(1),
        ..Default::default()
    };
    assert_eq!(ids(s.list(&q).await.unwrap()), ["other-1"]);
}

#[tokio::test]
async fn mem_store_meets_the_contract() {
    contract(&MemStore::new()).await;
}

#[tokio::test]
async fn file_store_meets_the_contract() {
    let dir = tempfile::tempdir().unwrap();
    contract(&FileStore::new(dir.path().join("runs"))).await;
    // and a second store on the same directory sees the same thing
    let again = FileStore::new(dir.path().join("runs"));
    assert_eq!(again.alloc_id("demo").await.unwrap(), "demo-3");
    assert_eq!(
        again.load("demo-1").await.unwrap().unwrap().status,
        RunStatus::Finished
    );
}

/// The engine runs the same on a FileStore.
#[tokio::test]
async fn engine_on_a_file_store() {
    use std::sync::Arc;
    let dir = tempfile::tempdir().unwrap();
    let packs = Arc::new(common::packs().flow(
        "name: seq\nstations:\n  - {id: a, executor: {kind: program, step: work}, output: {type: object}, exit: [{kind: output}]}\n  - {id: b, exit: [{kind: confirm}]}",
    ));
    let eng = Engine::new(Arc::new(FileStore::new(dir.path())), packs);
    let o = eng
        .create(
            CreateIn {
                flow: "seq".into(),
                subject: "s".into(),
                ..Default::default()
            },
            common::me(),
        )
        .await
        .unwrap();
    let j = &common::jobs(&o)[0];
    eng.submit(
        &j.run,
        &j.station,
        &j.task,
        &j.token,
        json!({}),
        Actor::flow("dispatch"),
    )
    .await
    .unwrap();
    let o = eng
        .confirm(&j.run, "b", true, None, common::me())
        .await
        .unwrap();
    assert!(common::has(&o, "run.finished"), "{:?}", common::kinds(&o));
    let h = eng.history(&j.run).await.unwrap();
    assert!(h.windows(2).all(|w| w[1].seq == w[0].seq + 1), "{h:#?}");
    assert_eq!(
        eng.get(&j.run).await.unwrap().record.status,
        RunStatus::Finished
    );
}
