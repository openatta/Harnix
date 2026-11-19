//! The runner end to end: a flow pack driven from start to finish by the real kernel over a memory
//! store, its sessions played by `ScriptedHost` — no model, no Harnix, no network. What the LIVE
//! flow specs see through a whole backend, here in a second.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use harnix_flow::runner::{CatalogPacks, Decision, Runner, RunnerConfig, ScriptedHost, SessionHost, Turn, TOOL_SUBMIT};
use harnix_flow::*;
use serde_json::{json, Value};

const T0: u64 = 1_794_787_200_000;

/// The runner over the example pack (`packs/examples`), with `packs` over it when given.
fn runner(host: Arc<ScriptedHost>, extra: Option<MemPacks>, data: &Path) -> (Arc<Runner>, Arc<Engine>) {
    let cfg = RunnerConfig {
        profiles: vec!["flow-harnix".into(), "flow-code".into()],
        submit_profiles: ["flow-harnix".to_string(), "flow-code".to_string()].into(),
        main_profile: Some("flow-harnix".into()),
        project_profiles: ["flow-code".to_string()].into(),
        lease_ms: 60_000,
        tick_ms: 1000,
        ..Default::default()
    };
    let examples = Path::new(env!("CARGO_MANIFEST_DIR")).join("packs/examples");
    let mut dir = DirPacks::new([examples]);
    if let Some(extra) = extra {
        dir = dir.with_builtin(extra);
    }
    let packs = CatalogPacks::over(&cfg, vec![], Arc::new(dir));
    let layout = ProjectLayout::new(".harnix", data.join("worktrees"), "harnix/flow");
    packs.set_layout(layout.clone());
    let engine = Arc::new(
        Engine::new(Arc::new(MemStore::new()), Arc::new(packs))
            .with_clock(Arc::new(ManualClock::new(T0)))
            .with_lease_ms(60_000)
            .with_submit_scenes(cfg.profiles.clone())
            .with_main_scene("flow-harnix")
            .with_guarded(cfg.guarded.clone())
            .with_workspaces(data.join("workspaces"))
            .with_layout(layout),
    );
    (Runner::new(engine.clone(), host, cfg), engine)
}

/// Wait until `run` is over or waits for a person, and nothing of the runner's is in hand.
async fn settled(engine: &Engine, run: &str, status: RunStatus) -> RunRecord {
    let deadline = Duration::from_secs(600);
    let waited = tokio::time::timeout(deadline, async {
        loop {
            tokio::task::yield_now().await;
            tokio::time::sleep(Duration::from_millis(50)).await;
            let rec = engine.record(run).await.unwrap();
            if rec.status == status {
                return rec;
            }
        }
    })
    .await;
    match waited {
        Ok(rec) => rec,
        Err(_) => panic!("run {run} did not reach {status:?}: {:?}", engine.get(run).await.map(|v| v.record.phase())),
    }
}

async fn start(rn: &Arc<Runner>, flow: &str, project: Option<&Path>) -> String {
    let input = CreateIn {
        flow: flow.into(),
        subject: "the repo".into(),
        project_root: project.map(|p| p.to_string_lossy().into_owned()),
        ..Default::default()
    };
    let o = rn.start_run(input, Actor::device("d")).await.unwrap();
    let run = o.value["run"].as_str().unwrap().to_string();
    rn.apply(o).await;
    run
}

/// The example pack from start to finish: the main Agent reviews in the run's main session, a
/// person sends it back once and then approves. What the host is asked for is in the flow's own
/// words; the run is told over.
#[tokio::test(start_paused = true)]
async fn the_example_pack_runs_from_start_to_finish_in_the_main_session() {
    let data = tempfile::tempdir().unwrap();
    let host = ScriptedHost::new();
    host.main(vec![
        Turn::Ends { output: Some(json!({"summary": "one thing", "findings": [{"severity": "low", "text": "a typo"}]})) },
        Turn::Ends { output: Some(json!({"summary": "two things", "findings": []})) },
    ]);
    let (rn, engine) = runner(host.clone(), None, data.path());
    rn.start().await;
    let run = start(&rn, "example-review", None).await;

    // The main session: opened under the run's main key, titled after the run, asking about the
    // guarded tools and what the policy names.
    let rec = settled(&engine, &run, RunStatus::Active).await;
    let waiting = tokio::time::timeout(Duration::from_secs(60), async {
        loop {
            let v = engine.get(&run).await.unwrap();
            if v.record.stations.get("approve").is_some_and(|s| s.status == StationStatus::Working) {
                return v.record;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("the review handed in and approve waits");
    let main = host.opened().into_iter().find(|s| s.main).expect("the main session was opened");
    assert_eq!(main.key, format!("flow:{run}#@main"));
    assert_eq!(main.profile, "flow-harnix");
    assert_eq!(main.delivery, Delivery::FlowSubmit);
    assert!(main.title.as_deref().unwrap().contains(&run), "{:?}", main.title);
    assert!(main.asks.iter().any(|a| a == "Write") && main.asks.iter().any(|a| a == "Bash"), "{:?}", main.asks);
    assert_eq!(main.tools.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(), ["flow_state", TOOL_SUBMIT, "flow_propose"]);
    let main_id = host.session_of(&main.key).unwrap();
    assert_eq!(rec.main_session(), Some(main_id.as_str()));
    // The task's message: the protocol layer, the rule, the skill.
    let sent = host.sent_to(&main_id);
    assert_eq!(sent.len(), 1, "{sent:#?}");
    assert!(sent[0].contains("review-honest") && sent[0].contains("flow_submit"), "{}", sent[0]);
    assert_eq!(waiting.stations["review"].status, StationStatus::Passed);
    assert_eq!(waiting.stations["review"].task("review").unwrap().output.as_ref().unwrap()["summary"], "one thing");

    // A person sends it back: the main session is told again, then approves.
    let by = Actor::device("d");
    rn.settle(engine.confirm(&run, "approve", false, Some("look again"), by.clone()).await).await.unwrap();
    rn.settle(engine.rework(&run, "review", None, "look again", by.clone()).await).await.unwrap();
    let again = tokio::time::timeout(Duration::from_secs(60), async {
        loop {
            if host.sent_to(&main_id).len() == 2 && engine.record(&run).await.unwrap().stations["review"].status == StationStatus::Passed {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await;
    assert!(again.is_ok(), "the second round handed in");
    rn.settle(engine.confirm(&run, "approve", true, None, by).await).await.unwrap();
    let done = settled(&engine, &run, RunStatus::Finished).await;
    assert_eq!(done.stations["review"].round, 2);
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(host.over(), [run.as_str()], "the host hears the run is over");
    assert!(rn.stop(Duration::from_secs(5)).await);
}

/// A station with a session of its own, judged call by call: a write the policy denies is refused
/// with the policy's reason, the turn ends with nothing, the session is nudged and hands in.
#[tokio::test(start_paused = true)]
async fn a_stations_own_session_is_judged_call_by_call_and_nudged() {
    let data = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let host = ScriptedHost::new();
    host.station(
        "fix",
        vec![
            Turn::Asks {
                tool: "Write".into(),
                paths: vec![project.path().join("src/a.rs").to_string_lossy().into_owned()],
                input: Value::Null,
                then: Box::new(Turn::Ends { output: Some(json!({"changed": ["src/a.rs"]})) }),
            },
            Turn::Ends { output: Some(json!({"changed": []})) },
        ],
    );
    host.trust(project.path(), true);
    let extra = MemPacks::new().skill("fix", "Fix it.").flow(
        "{name: own, stations: [{id: fix, executor: {kind: session, scene: flow-code, session: new}, skill: fix, \
         policies: [read-only], output: {type: object, required: [changed]}, exit: [{kind: output}]}, {id: done, exit: [{kind: confirm}]}]}",
    );
    let (rn, engine) = runner(host.clone(), Some(extra), data.path());
    rn.start().await;
    let run = start(&rn, "own", Some(project.path())).await;
    let waiting = tokio::time::timeout(Duration::from_secs(120), async {
        loop {
            let rec = engine.record(&run).await.unwrap();
            if rec.stations["fix"].status == StationStatus::Passed {
                return rec;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("the station passed");
    let own = host.opened().into_iter().find(|s| !s.main).expect("a session of the station's own");
    assert!(own.key.starts_with(&format!("flow:{run}/fix/fix@")), "{}", own.key);
    // A submit profile: the session hands in with `flow_submit`, and is given no output schema.
    assert_eq!((own.profile.as_str(), own.delivery), ("flow-code", Delivery::FlowSubmit));
    assert_eq!(own.project_root.as_deref(), Some(project.path()));
    assert!(own.output_schema.is_none());
    // The write was refused with the policy's words; the turn ended with nothing; the nudge.
    let answers = host.answers();
    assert_eq!(answers.len(), 1, "{answers:?}");
    let Decision::Deny { reason } = &answers[0].2 else { panic!("{answers:?}") };
    assert!(reason.contains("changes nothing"), "{reason}");
    let id = host.session_of(&own.key).unwrap();
    let sent = host.sent_to(&id);
    assert_eq!(sent.len(), 2, "{sent:#?}");
    assert!(sent[1].starts_with("Your flow task is not handed in yet"), "{}", sent[1]);
    assert_eq!(waiting.stations["fix"].task("fix").unwrap().output, Some(json!({"changed": []})));
    // The task's session is recorded on the run, and the process directory is in the project.
    assert_eq!(waiting.stations["fix"].task("fix").unwrap().session.as_deref(), Some(id.as_str()));
    assert!(project.path().join(".harnix/runs").join(&run).join("task.json").is_file());
    assert!(rn.stop(Duration::from_secs(5)).await);
}

/// The runner restarted while a task's session was idle: the next runner finds the session the
/// record names, takes the task up under the same claim, and nudges it rather than telling it
/// everything again.
#[tokio::test(start_paused = true)]
async fn a_restart_takes_up_the_work_in_the_sessions_the_records_name() {
    let data = tempfile::tempdir().unwrap();
    let host = ScriptedHost::new();
    host.station("a", vec![Turn::Runs, Turn::Ends { output: Some(json!({"n": 1})) }]);
    let extra = MemPacks::new().skill("do", "Do it.").flow(
        "{name: f, stations: [{id: a, executor: {kind: session, scene: flow-harnix, session: new}, skill: do, \
         output: {type: object, required: [n]}, exit: [{kind: output}]}, {id: b, exit: [{kind: confirm}]}]}",
    );
    let (rn, engine) = runner(host.clone(), Some(extra), data.path());
    rn.start().await;
    let run = start(&rn, "f", None).await;
    let own_key = tokio::time::timeout(Duration::from_secs(60), async {
        loop {
            if let Some(s) = host.opened().into_iter().find(|s| !s.main) {
                if host.sent_to(&host.session_of(&s.key).unwrap()).len() == 1 {
                    return s.key;
                }
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("the task was sent");
    let id = host.session_of(&own_key).unwrap();
    // The first runner stops while the turn runs; the turn ends while nobody watches.
    assert!(rn.stop(Duration::from_secs(5)).await);
    host.interrupt(&id).await.unwrap();
    assert!(host.interrupts().len() == 1);
    assert_eq!(engine.record(&run).await.unwrap().stations["a"].task("a").unwrap().status, TaskStatus::Claimed);

    // A second runner over the same store and host: the claim is still the dispatcher's.
    let again = Runner::new(engine.clone(), host.clone(), RunnerConfig { lease_ms: 60_000, ..Default::default() });
    again.start().await;
    let done = tokio::time::timeout(Duration::from_secs(120), async {
        loop {
            let rec = engine.record(&run).await.unwrap();
            if rec.stations["a"].status == StationStatus::Passed {
                return rec;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("the task passed after the restart");
    let sent = host.sent_to(&id);
    assert_eq!(sent.len(), 2, "{sent:#?}");
    assert!(sent[1].starts_with("Your flow task is not handed in yet"), "nudged, not told again: {}", sent[1]);
    assert_eq!(host.opened().iter().filter(|s| !s.main).count(), 1, "no second session was opened");
    assert_eq!(done.stations["a"].task("a").unwrap().output, Some(json!({"n": 1})));
    assert!(again.stop(Duration::from_secs(5)).await);
}

/// A host that cannot open a session fails the attempt with why, and the run says so.
#[tokio::test(start_paused = true)]
async fn a_session_the_host_refuses_fails_the_attempt_with_why() {
    let data = tempfile::tempdir().unwrap();
    let host = ScriptedHost::new();
    host.refuse(Some("no such profile here"));
    let extra = MemPacks::new().skill("do", "Do it.").flow(
        "{name: f, stations: [{id: a, executor: {kind: session, scene: flow-harnix, session: new}, skill: do, \
         output: {type: object}, exit: [{kind: output}]}]}",
    );
    let (rn, engine) = runner(host.clone(), Some(extra), data.path());
    rn.start().await;
    let run = start(&rn, "f", None).await;
    let rec = tokio::time::timeout(Duration::from_secs(60), async {
        loop {
            let rec = engine.record(&run).await.unwrap();
            if rec.stations["a"].task("a").unwrap().status == TaskStatus::Failed {
                return rec;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("the attempt failed");
    let error = rec.stations["a"].task("a").unwrap().error.clone().unwrap();
    assert!(error.contains("no session: refused: no such profile here"), "{error}");
    assert!(rn.stop(Duration::from_secs(5)).await);
}

/// A run started on a plugin's flow runs on once the plugin is gone (uninstalled: its directory
/// taken away): it pinned the definition and the plugin's policy when it started. The flow is no
/// longer offered.
#[tokio::test(start_paused = true)]
async fn a_run_of_a_plugins_flow_outlives_the_plugin() {
    let data = tempfile::tempdir().unwrap();
    let plugin = data.path().join("plugins/acme");
    std::fs::create_dir_all(plugin.join("flows")).unwrap();
    std::fs::create_dir_all(plugin.join("policies")).unwrap();
    std::fs::write(
        plugin.join("flows/plugged.yaml"),
        "name: plugged\nstations:\n  - {id: review, executor: {kind: session}, skill: review, policies: [acme-only], output: {type: object}, exit: [{kind: output}]}\n  - {id: approve, exit: [{kind: confirm}], rework_to: [review]}\n",
    )
    .unwrap();
    std::fs::write(
        plugin.join("policies/acme-only.yaml"),
        "name: acme-only\npermissions: [{effect: deny, tools: [WebFetch], message: not in acme's runs}]\n",
    )
    .unwrap();
    let host = ScriptedHost::new();
    host.main(vec![Turn::Ends { output: Some(json!({"first": true})) }, Turn::Ends { output: Some(json!({"again": true})) }]);
    let cfg = RunnerConfig {
        profiles: vec!["flow-harnix".into()],
        submit_profiles: ["flow-harnix".to_string()].into(),
        main_profile: Some("flow-harnix".into()),
        lease_ms: 60_000,
        tick_ms: 1000,
        ..Default::default()
    };
    let examples = Path::new(env!("CARGO_MANIFEST_DIR")).join("packs/examples");
    let with_plugin = DirPacks::new([examples]).with_plugin("acme", &plugin);
    assert_eq!((with_plugin.origin("plugged"), with_plugin.source("plugged").as_deref()), (Origin::Plugin, Some("acme")));
    let packs = Arc::new(CatalogPacks::over(&cfg, vec![], Arc::new(with_plugin)));
    let engine = Arc::new(
        Engine::new(Arc::new(MemStore::new()), packs.clone())
            .with_clock(Arc::new(ManualClock::new(T0)))
            .with_lease_ms(60_000)
            .with_submit_scenes(cfg.profiles.clone())
            .with_main_scene("flow-harnix")
            .with_workspaces(data.path().join("workspaces")),
    );
    let rn = Runner::new(engine.clone(), host.clone(), cfg);
    rn.start().await;
    let run = start(&rn, "plugged", None).await;
    let reviewed = |n: usize| {
        let engine = engine.clone();
        let run = run.clone();
        async move {
            tokio::time::timeout(Duration::from_secs(60), async {
                loop {
                    let rec = engine.record(&run).await.unwrap();
                    let review = &rec.stations["review"];
                    if review.round as usize >= n && review.status == StationStatus::Passed
                        && rec.stations.get("approve").is_some_and(|s| s.status == StationStatus::Working)
                    {
                        return;
                    }
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
            })
            .await
            .expect("the review handed in and approve waits")
        }
    };
    reviewed(1).await;

    // The plugin goes.
    std::fs::remove_dir_all(&plugin).unwrap();
    assert!(!engine.packs().flows().contains(&"plugged".to_string()));
    assert!(wire::defs_list(engine.packs().as_ref()).iter().all(|d| d.name != "plugged"));
    let rec = engine.record(&run).await.unwrap();
    assert_eq!(rec.def.name, "plugged");
    assert!(rec.parts.policies.contains_key("acme-only"), "pinned when it started");

    // It runs on: sent back, handed in again under the pinned policy, approved.
    let by = Actor::device("d");
    rn.settle(engine.confirm(&run, "approve", false, Some("again"), by.clone()).await).await.unwrap();
    rn.settle(engine.rework(&run, "review", None, "again", by.clone()).await).await.unwrap();
    reviewed(2).await;
    let main = host.opened().into_iter().find(|s| s.main).unwrap();
    assert!(main.asks.iter().any(|a| a == "WebFetch"), "{:?}", main.asks);
    rn.settle(engine.confirm(&run, "approve", true, None, by).await).await.unwrap();
    let done = settled(&engine, &run, RunStatus::Finished).await;
    assert_eq!(done.stations["review"].task("review").unwrap().output.as_ref().unwrap()["again"], true);
}
