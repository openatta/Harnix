//! Golden runs: a flow pack driven through a script of what the host and people do, and the run's
//! record and history it leaves compared with the files kept beside it. They pin the state format
//! (`RUN_FORMAT`): a change that moves a field moves the files, and the format goes up with it when
//! a host's stored runs would no longer read.
//!
//! ```text
//! tests/golden/<case>/script.yaml   pack, catalog, flow, subject, inputs, steps
//! tests/golden/<case>/run.json      the record when the steps are done (tokens left out)
//! tests/golden/<case>/history.jsonl its history, one entry a line (tokens left out)
//! tests/golden/packs/<name>/        a pack several cases share (`pack: /tests/golden/packs/<name>`)
//! ```
//!
//! `FLOW_GOLDEN=write cargo test -p harnix-flow --test golden` writes the files again.
//!
//! The steps stand in for the host's dispatcher and for the people: `main` binds the run's main
//! session; `submit` hands in a task's output (binding its session first when the step names one,
//! claiming an external task first); `gate` hands in a gate's result; `confirm`, `rework` and
//! `advance` do what they say.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use harnix_flow::*;
use serde::Deserialize;
use serde_json::{json, Value};

const T0: u64 = 1_794_787_200_000;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Script {
    /// The pack's directory, relative to the case (`pack`) or, starting `/`, to the crate.
    pack: String,
    #[serde(default)]
    catalog: Catalog,
    flow: String,
    subject: String,
    #[serde(default)]
    inputs: Value,
    #[serde(with = "serde_yaml::with::singleton_map_recursive")]
    steps: Vec<Step>,
    /// What the run is once the steps are done.
    status: RunStatus,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
enum Step {
    Main { session: String, #[serde(default)] opened_with: Value },
    Submit {
        station: String,
        #[serde(default)]
        task: Option<String>,
        #[serde(default)]
        session: Option<String>,
        #[serde(default)]
        opened_with: Value,
        output: Value,
    },
    Confirm { station: String, pass: bool, #[serde(default)] reason: Option<String> },
    Rework { to: String, reason: String },
    Advance { ms: u64 },
    Gate { station: String, gate: String, pass: bool, #[serde(default)] detail: String },
}

fn crate_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

fn by() -> Actor {
    Actor::device("golden")
}

/// What the store holds, without what is drawn at random (tokens).
fn steady(mut v: Value) -> Value {
    fn strip(v: &mut Value) {
        match v {
            Value::Object(m) => {
                m.remove("token");
                m.values_mut().for_each(strip);
            }
            Value::Array(a) => a.iter_mut().for_each(strip),
            _ => {}
        }
    }
    strip(&mut v);
    v
}

/// The token of the work in hand for `station` (`task`, else its first), claiming an external
/// task first.
async fn token_of(eng: &Engine, run: &str, station: &str, task: Option<&str>, now: u64) -> (String, String, Option<String>) {
    let rec = eng.record(run).await.unwrap();
    for w in eng.dispatched_work(&rec, now) {
        if let WorkItem::Task { station: s, task: t, token, session_key, .. } = w {
            if s == station && task.is_none_or(|want| want == t) {
                return (t, token, session_key);
            }
        }
    }
    let task = task.unwrap_or(station).to_string();
    let claimed = eng.claim(run, station, &task, by(), 60_000).await.unwrap();
    (task, claimed.value["token"].as_str().unwrap().to_string(), None)
}

async fn play(case: &Path) -> (Value, String) {
    let script: Script = serde_yaml::from_str(&std::fs::read_to_string(case.join("script.yaml")).unwrap()).unwrap();
    let pack = match script.pack.strip_prefix('/') {
        Some(in_crate) => crate_dir().join(in_crate),
        None => case.join(&script.pack),
    };
    let packs = Arc::new(DirPacks::new([pack]).with_catalog(script.catalog.clone()));
    let clock = Arc::new(ManualClock::new(T0));
    let eng = Engine::new(Arc::new(MemStore::new()), packs.clone()).with_clock(clock.clone());
    let flow = Packs::flow(packs.as_ref(), &script.flow).map(|(_, digest)| digest);
    assert!(flow.is_some(), "{}: the pack has no flow {}", case.display(), script.flow);
    let problems = validate(&Packs::flow(packs.as_ref(), &script.flow).unwrap().0, packs.as_ref());
    assert!(problems.is_empty(), "{}: {problems:?}", case.display());

    let input = CreateIn {
        flow: script.flow.clone(),
        subject: script.subject.clone(),
        inputs: script.inputs.clone(),
        ..Default::default()
    };
    let run = eng.create(input, by()).await.unwrap().value["run"].as_str().unwrap().to_string();
    for (i, step) in script.steps.iter().enumerate() {
        let at = format!("{}: step {}", case.display(), i + 1);
        match step {
            Step::Main { session, opened_with } => {
                eng.attach_main(&run, session, opened_with.clone()).await.expect(&at);
            }
            Step::Submit { station, task, session, opened_with, output } => {
                let (task, token, key) = token_of(&eng, &run, station, task.as_deref(), clock.now_ms()).await;
                if let Some(s) = session {
                    eng.attach_session(&run, station, &task, &token, s, key.as_deref(), opened_with, Actor::flow("dispatch"))
                        .await
                        .expect(&at);
                }
                eng.submit(&run, station, &task, &token, output.clone(), Actor::flow("dispatch")).await.expect(&at);
            }
            Step::Confirm { station, pass, reason } => {
                eng.confirm(&run, station, *pass, reason.as_deref(), by()).await.expect(&at);
            }
            Step::Rework { to, reason } => {
                eng.rework(&run, to, None, reason, by()).await.expect(&at);
            }
            Step::Advance { ms } => clock.advance(*ms),
            Step::Gate { station, gate, pass, detail } => {
                let round = eng.record(&run).await.unwrap().stations[station].round;
                eng.gate_result(&run, station, round, gate, GateResult { pass: *pass, detail: detail.clone() }).await.expect(&at);
            }
        }
    }
    let rec = eng.record(&run).await.unwrap();
    assert_eq!(rec.status, script.status, "{}", case.display());
    let record = steady(serde_json::to_value(&rec).unwrap());
    let history: Vec<String> = eng
        .history(&run)
        .await
        .unwrap()
        .into_iter()
        .map(|h| serde_json::to_string(&steady(serde_json::to_value(h).unwrap())).unwrap())
        .collect();
    (record, history.join("\n") + "\n")
}

#[tokio::test]
async fn each_case_leaves_the_record_and_history_kept_for_it() {
    let root = crate_dir().join("tests/golden");
    let mut cases: Vec<PathBuf> = std::fs::read_dir(&root)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.join("script.yaml").is_file())
        .collect();
    cases.sort();
    assert!(!cases.is_empty(), "no cases under {}", root.display());
    let write = std::env::var_os("FLOW_GOLDEN").is_some_and(|v| v == "write");
    let mut stale = vec![];
    for case in &cases {
        let (record, history) = play(case).await;
        assert_eq!(record["format"], json!(RUN_FORMAT), "{}: the record says its format", case.display());
        let record = serde_json::to_string_pretty(&record).unwrap() + "\n";
        let (run_file, history_file) = (case.join("run.json"), case.join("history.jsonl"));
        if write {
            std::fs::write(&run_file, &record).unwrap();
            std::fs::write(&history_file, &history).unwrap();
            continue;
        }
        let kept = |p: &Path| std::fs::read_to_string(p).unwrap_or_default();
        if kept(&run_file) != record || kept(&history_file) != history {
            stale.push(case.file_name().unwrap().to_string_lossy().into_owned());
        }
    }
    assert!(
        stale.is_empty(),
        "the runs of {stale:?} differ from their golden files: if the change is meant, \
         `FLOW_GOLDEN=write cargo test -p harnix-flow --test golden`, look at the diff, and raise \
         RUN_FORMAT when stored runs would no longer read"
    );
}
