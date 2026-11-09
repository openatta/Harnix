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
        parts: Default::default(),
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
        paused: None,
        usage_other: Default::default(),
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

#[tokio::test]
async fn file_store_names_no_file_outside_its_directory() {
    let dir = tempfile::tempdir().unwrap();
    let runs = dir.path().join("runs");
    let s = FileStore::new(&runs);
    // A history file beside the directory, reachable with `..` if ids were not checked.
    std::fs::write(
        dir.path().join("secret.history.jsonl"),
        serde_json::to_string(&hist("secret", 1, "leak")).unwrap() + "\n",
    )
    .unwrap();
    for id in ["../secret", "..\\secret", ".hidden", ""] {
        assert!(s.history(id).await.unwrap().is_empty(), "{id:?}");
        assert_eq!(s.load(id).await.unwrap(), None, "{id:?}");
    }
    let bad = s
        .commit(&ChangeSet {
            records: vec![(record("../escape", "demo", "x", 1, 1), None)],
            history: vec![],
        })
        .await;
    assert!(bad.is_err());
    assert!(!dir.path().join("escape.json").exists());
}

#[tokio::test]
async fn file_store_lists_from_an_index_its_commits_keep_current() {
    let dir = tempfile::tempdir().unwrap();
    let s = FileStore::new(dir.path());
    s.commit(&ChangeSet {
        records: vec![(record("demo-1", "demo", "x", 1, 1), None)],
        history: vec![],
    })
    .await
    .unwrap();
    assert_eq!(s.list(&RunQuery::default()).await.unwrap().len(), 1);
    // Read once: a file appearing behind the store's back is not listed...
    std::fs::write(
        dir.path().join("demo-9.json"),
        serde_json::to_vec(&record("demo-9", "demo", "z", 1, 9)).unwrap(),
    )
    .unwrap();
    assert_eq!(s.list(&RunQuery::default()).await.unwrap().len(), 1);
    // ...while its own commits are, at once.
    let mut a = record("demo-1", "demo", "x", 2, 1);
    a.status = RunStatus::Finished;
    s.commit(&ChangeSet {
        records: vec![(a, Some(1)), (record("demo-2", "demo", "y", 1, 2), None)],
        history: vec![],
    })
    .await
    .unwrap();
    let listed = s.list(&RunQuery::default()).await.unwrap();
    assert_eq!(
        listed.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
        ["demo-2", "demo-1"]
    );
    assert_eq!(listed[1].status, RunStatus::Finished);
    // A fresh store reads the directory.
    assert_eq!(
        FileStore::new(dir.path())
            .list(&RunQuery::default())
            .await
            .unwrap()
            .len(),
        3
    );
}

/// A crash between the record's rename and its fsync could leave a zero-byte `<id>.json` behind.
/// That one run does not read; every other run is still listed — the engine's tick, recovery and
/// `app.flow.list` all start from `list()`.
#[tokio::test]
async fn a_record_of_zero_bytes_is_left_out_of_the_list_and_the_rest_are_listed() {
    let dir = tempfile::tempdir().unwrap();
    let s = FileStore::new(dir.path());
    let a = record("demo-1", "demo", "x", 1, 10);
    s.commit(&ChangeSet { records: vec![(a, None)], history: vec![] })
        .await
        .unwrap();
    std::fs::write(dir.path().join("demo-2.json"), b"").unwrap();

    // A fresh store reads the directory.
    let fresh = FileStore::new(dir.path());
    let listed = fresh.list(&RunQuery::default()).await.unwrap();
    assert_eq!(listed.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), ["demo-1"]);
    assert!(fresh.load("demo-2").await.is_err(), "the broken one is an error of its own");
}

/// The process stopped halfway through appending a history line. What came before still reads,
/// and the next commit's line does not run into the torn one.
#[tokio::test]
async fn a_history_whose_last_line_was_cut_short_still_reads() {
    let dir = tempfile::tempdir().unwrap();
    let s = FileStore::new(dir.path());
    let a = record("demo-1", "demo", "x", 1, 10);
    s.commit(&ChangeSet {
        records: vec![(a.clone(), None)],
        history: vec![hist("demo-1", 1, "create"), hist("demo-1", 2, "claim")],
    })
    .await
    .unwrap();
    let path = dir.path().join("demo-1.history.jsonl");
    let mut text = std::fs::read_to_string(&path).unwrap();
    let whole = serde_json::to_string(&hist("demo-1", 3, "submit")).unwrap();
    text.push_str(&whole[..whole.len() / 2]);
    std::fs::write(&path, &text).unwrap();

    let seqs = |h: Vec<History>| h.iter().map(|h| h.seq).collect::<Vec<_>>();
    assert_eq!(seqs(s.history("demo-1").await.unwrap()), [1, 2]);

    let mut a2 = a.clone();
    a2.rev = 2;
    s.commit(&ChangeSet { records: vec![(a2, Some(1))], history: vec![hist("demo-1", 4, "pass")] })
        .await
        .unwrap();
    assert_eq!(seqs(s.history("demo-1").await.unwrap()), [1, 2, 4]);
    assert!(std::fs::read_to_string(&path).unwrap().ends_with('\n'));

    // A line that does not read in the middle is still an error: that is not a torn append.
    std::fs::write(&path, format!("{{\"half\n{whole}\n")).unwrap();
    assert!(s.history("demo-1").await.is_err());
}
