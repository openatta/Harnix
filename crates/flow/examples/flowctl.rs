//! `flowctl`: the flow library from the command line, with nothing of any host.
//!
//! ```text
//! cargo run -p harnix-flow --example flowctl -- validate <pack-dir> [--catalog catalog.json]
//! cargo run -p harnix-flow --example flowctl -- schema definition|run|tools|<part kind>
//! cargo run -p harnix-flow --example flowctl -- dry-run <pack-dir> <flow> [--inputs '{}'] [--mocks '{}']
//! cargo run -p harnix-flow --example flowctl -- run <pack-dir> <flow> --script script.yaml [--state <dir>] [--project <dir>] [--subject <text>]
//! cargo run -p harnix-flow --example flowctl -- show <state-dir> <run>
//! ```
//!
//! `run` drives the flow with the runner over a `FileStore` (`--state`, a temporary directory
//! when absent) and sessions played from `script.yaml`:
//!
//! ```yaml
//! main:                       # the run's main session, one turn per message it is sent
//!   - ends: {output: {summary: done, findings: []}}
//! stations:
//!   review:                   # a station's own sessions
//!     - asks: {tool: Write, paths: [/p/a.md], then: {ends: {output: {ok: true}}}}
//!     - runs: {}
//! shared:
//!   run: [{ends: {}}]         # sessions shared under a key
//! trust: [/path/to/project]   # projects the host says are trusted
//! ```
//!
//! Each event of the run goes to stdout as a line of JSON; `run` ends when the run is over or
//! waits for a person, and says which. A host that copies the library runs this unchanged, which
//! is the check that the copy loads, validates and runs a pack.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use harnix_flow::dryrun::{SandboxConfig, StartIn};
use harnix_flow::runner::{CatalogPacks, Runner, RunnerConfig, ScriptedHost, Turn};
use harnix_flow::*;
use serde::Deserialize;
use serde_json::{json, Value};

const PROFILE: &str = "flow-harnix";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = match args.first().map(String::as_str) {
        Some("validate") => validate_pack(&args[1..]),
        Some("schema") => schema(&args[1..]),
        Some("dry-run") => block_on(dry_run(&args[1..])),
        Some("run") => block_on(run(&args[1..])),
        Some("show") => block_on(show(&args[1..])),
        _ => {
            eprintln!("{}", USAGE);
            2
        }
    };
    std::process::exit(code);
}

const USAGE: &str = "flowctl validate <pack-dir> [--catalog catalog.json]
flowctl schema definition|run|tools|skill|rule|policy|gate|hook
flowctl dry-run <pack-dir> <flow> [--inputs '{}'] [--mocks '{}']
flowctl run <pack-dir> <flow> --script script.yaml [--state <dir>] [--project <dir>] [--subject <text>]
flowctl show <state-dir> <run>";

fn block_on<F: std::future::Future<Output = i32>>(f: F) -> i32 {
    tokio::runtime::Builder::new_current_thread().enable_all().build().expect("a runtime").block_on(f)
}

/// `--name value` pairs and the positional arguments.
fn parse(args: &[String]) -> (Vec<String>, BTreeMap<String, String>) {
    let (mut positional, mut named) = (vec![], BTreeMap::new());
    let mut i = 0;
    while i < args.len() {
        match args[i].strip_prefix("--") {
            Some(name) if i + 1 < args.len() => {
                named.insert(name.to_string(), args[i + 1].clone());
                i += 2;
            }
            Some(name) => {
                named.insert(name.to_string(), String::new());
                i += 1;
            }
            None => {
                positional.push(args[i].clone());
                i += 1;
            }
        }
    }
    (positional, named)
}

fn catalog_of(named: &BTreeMap<String, String>) -> Catalog {
    let mut c: Catalog = match named.get("catalog") {
        Some(path) => serde_json::from_str(&std::fs::read_to_string(path).expect("the catalog file")).expect("a catalog"),
        None => Catalog::default(),
    };
    c.scenes.insert(PROFILE.into());
    c.submit_scenes.insert(PROFILE.into());
    if c.main_scene.is_none() {
        c.main_scene = Some(PROFILE.into());
    }
    c
}

fn packs_at(dir: &str, catalog: Catalog) -> Arc<DirPacks> {
    Arc::new(DirPacks::new([PathBuf::from(dir)]).with_catalog(catalog))
}

fn validate_pack(args: &[String]) -> i32 {
    let (positional, named) = parse(args);
    let Some(dir) = positional.first() else {
        eprintln!("{USAGE}");
        return 2;
    };
    let packs = packs_at(dir, catalog_of(&named));
    let mut bad = 0;
    for name in packs.flows() {
        let Some((def, digest)) = packs.flow(&name) else { continue };
        let problems = validate(&def, packs.as_ref());
        if problems.is_empty() {
            println!("ok    {name}  {digest}");
        } else {
            bad += 1;
            println!("FAIL  {name}");
            for p in problems {
                println!("      {}{}", p.station.as_deref().map(|s| format!("{s}: ")).unwrap_or_default(), p.message);
            }
        }
    }
    for kind in PartKind::ALL {
        for name in packs.parts(kind) {
            let Some((text, _)) = packs.part(kind, &name) else { continue };
            let problems = part_problems(kind, &name, &text, packs.as_ref());
            if !problems.is_empty() {
                bad += 1;
                println!("FAIL  {kind} {name}");
                for p in problems {
                    println!("      {p}");
                }
            }
        }
    }
    for (name, why) in packs.withheld_flows() {
        println!("held  {name}  {why:?}");
    }
    i32::from(bad > 0)
}

fn schema(args: &[String]) -> i32 {
    let what = args.first().map(String::as_str).unwrap_or("definition");
    let v = match what {
        "definition" | "def" | "flow" => Some(definition_schema()),
        "run" => Some(run_schema()),
        "tools" => Some(json!(harnix_flow::runner::tool_specs())),
        other => match serde_json::from_value::<PartKind>(json!(other)) {
            Ok(kind) => part_schema(kind),
            Err(_) => {
                eprintln!("{USAGE}");
                return 2;
            }
        },
    };
    let Some(v) = v else {
        eprintln!("{what} has no schema: it is text");
        return 1;
    };
    println!("{}", serde_json::to_string_pretty(&v).unwrap());
    0
}

async fn dry_run(args: &[String]) -> i32 {
    let (positional, named) = parse(args);
    let [dir, flow] = positional.as_slice() else {
        eprintln!("{USAGE}");
        return 2;
    };
    let packs = packs_at(dir, catalog_of(&named));
    let config = SandboxConfig { guarded: vec![], submit_scenes: vec![PROFILE.into()], main_scene: Some(PROFILE.into()) };
    let sandbox = Sandbox::new(packs, config);
    let json_of = |k: &str| named.get(k).map(|s| serde_json::from_str::<Value>(s).expect("JSON")).unwrap_or(Value::Null);
    let input = StartIn {
        flow: Some(flow.clone()),
        inputs: json_of("inputs"),
        mocks: serde_json::from_value(json_of("mocks")).unwrap_or_default(),
        ..Default::default()
    };
    let run = match sandbox.start(input, Actor::device("flowctl")).await {
        Ok(r) => r,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };
    match sandbox.run(&run).await {
        Ok(progress) => {
            println!("{}", serde_json::to_string_pretty(&progress).unwrap());
            0
        }
        Err(e) => {
            eprintln!("{e}");
            1
        }
    }
}

/// What `--script` holds.
#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields, default)]
struct Script {
    #[serde(with = "serde_yaml::with::singleton_map_recursive")]
    main: Vec<Turn>,
    #[serde(with = "serde_yaml::with::singleton_map_recursive")]
    stations: BTreeMap<String, Vec<Turn>>,
    #[serde(with = "serde_yaml::with::singleton_map_recursive")]
    shared: BTreeMap<String, Vec<Turn>>,
    trust: Vec<PathBuf>,
}

async fn run(args: &[String]) -> i32 {
    let (positional, named) = parse(args);
    let ([dir, flow], Some(script)) = (positional.as_slice(), named.get("script")) else {
        eprintln!("{USAGE}");
        return 2;
    };
    let script: Script = serde_yaml::from_str(&std::fs::read_to_string(script).expect("the script file")).expect("a script");
    let host = ScriptedHost::new();
    host.main(script.main);
    for (station, turns) in script.stations {
        host.station(&station, turns);
    }
    for (key, turns) in script.shared {
        host.shared(&key, turns);
    }
    for root in &script.trust {
        host.trust(root, true);
    }
    let temp = tempfile_dir();
    let state = named.get("state").map(PathBuf::from).unwrap_or_else(|| temp.clone());
    std::fs::create_dir_all(&state).expect("the state directory");
    let cfg = RunnerConfig {
        profiles: vec![PROFILE.into()],
        submit_profiles: [PROFILE.to_string()].into(),
        main_profile: Some(PROFILE.into()),
        tick_ms: 200,
        ..Default::default()
    };
    let packs = CatalogPacks::over(&cfg, vec![], packs_at(dir, catalog_of(&named)));
    let layout = ProjectLayout::new(".flow", state.join("worktrees"), "flow");
    packs.set_layout(layout.clone());
    let engine = Arc::new(
        Engine::new(Arc::new(FileStore::new(state.join("runs"))), Arc::new(packs))
            .with_submit_scenes([PROFILE])
            .with_main_scene(PROFILE)
            .with_guarded(cfg.guarded.clone())
            .with_workspaces(state.join("workspaces"))
            .with_layout(layout),
    );
    let rn = Runner::new(engine.clone(), host, cfg);
    let mut events = rn.events();
    rn.start().await;
    let input = CreateIn {
        flow: flow.clone(),
        subject: named.get("subject").cloned().unwrap_or_else(|| "the subject".into()),
        project_root: named.get("project").cloned(),
        ..Default::default()
    };
    let run = match rn.start_run(input, Actor::device("flowctl")).await {
        Ok(o) => {
            let run = o.value["run"].as_str().unwrap().to_string();
            rn.apply(o).await;
            run
        }
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };
    eprintln!("run {run} in {}", state.display());
    // Over, or waiting for a person: quiet for a while with nothing in hand.
    let mut quiet = 0;
    loop {
        match tokio::time::timeout(Duration::from_millis(500), events.recv()).await {
            Ok(Ok(e)) => {
                quiet = 0;
                println!("{}", serde_json::to_string(&e).unwrap());
                if matches!(e.kind.as_str(), "run.finished" | "run.cancelled") {
                    break;
                }
            }
            Ok(Err(_)) => break,
            Err(_) => {
                quiet += 1;
                let rec = engine.record(&run).await.expect("the run");
                let in_hand = rec.stations.values().flat_map(|s| &s.tasks).any(|t| t.status == TaskStatus::Claimed);
                if !rec.is_active() || (quiet >= 4 && !in_hand) {
                    break;
                }
            }
        }
    }
    rn.stop(Duration::from_secs(5)).await;
    let v = engine.get(&run).await.expect("the run");
    eprintln!("{}: {:?} ({})", run, v.record.status, v.record.phase().state);
    0
}

async fn show(args: &[String]) -> i32 {
    let [state, run] = args else {
        eprintln!("{USAGE}");
        return 2;
    };
    let engine = Engine::new(Arc::new(FileStore::new(Path::new(state).join("runs"))), Arc::new(MemPacks::new()));
    match engine.get(run).await {
        Ok(v) => {
            println!("{}", serde_json::to_string_pretty(&v).unwrap());
            0
        }
        Err(e) => {
            eprintln!("{e}");
            1
        }
    }
}

/// A temporary directory of this process's own, left behind (the state is what was asked for).
fn tempfile_dir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("flowctl-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("a temporary directory");
    dir
}
