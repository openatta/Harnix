//! Running a gate the kernel handed out: a `command` gate here, a `host` one by the application.

use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;
use tokio_util::sync::CancellationToken;

use super::dispatch::settle;
use super::shared::Shared;
use crate::kernel::engine::Engine;
use crate::kernel::host::{GateResult, Outcome};
use crate::kernel::parts::GateKind;
use crate::kernel::run::RunRecord;
use crate::kernel::FlowError;

/// A gate's work item: run it, and hand its result to the kernel.
pub(crate) async fn run_gate(sh: &Arc<Shared>, run: String, station: String, round: u32, gate: String) {
    let Ok(engine) = sh.engine() else { return };
    let rec = match engine.record(&run).await {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(run, error = %e, "flow: gate not run");
            return;
        }
    };
    let result = match rec.parts.gates.get(&gate) {
        Some(g) if g.kind == GateKind::Command => command_gate(sh, &engine, &rec, &station, g).await,
        Some(_) => match &sh.cfg.gates {
            Some(gs) => gs.run(&rec, &station, &gate).await,
            None => GateResult {
                pass: false,
                detail: format!("the application runs no checks: {gate} cannot run"),
            },
        },
        None => GateResult {
            pass: false,
            detail: format!("gate {gate} was not in the packs when the run started"),
        },
    };
    let r = offer_result(&engine, &sh.cancel, &run, &station, round, &gate, result).await;
    settle(sh, &run, "gate_result", r).await;
}

/// The first wait before a gate's result is offered again after a conflict; it doubles each time.
const OFFER_AGAIN_AFTER: Duration = Duration::from_millis(50);
/// The longest wait between two offers.
const OFFER_AGAIN_AT_MOST: Duration = Duration::from_secs(5);

/// Hand a gate's result to the kernel until it is taken. Each offer is a whole operation — the
/// run loaded afresh, the result applied, the commit retried on a conflict as every operation
/// is — so a run busy enough to outlast those retries only delays the result: it is offered
/// again after a wait that grows, for as long as the module runs. A command may take minutes and
/// change things outside the run; running it again because one commit lost a race would be
/// wrong, and dropping its result would leave the station waiting for good.
///
/// Stopping is the one way out without an answer, and it loses nothing either: the gate is still
/// waiting in the run, and the next start runs it again (`recover`).
#[allow(clippy::too_many_arguments)]
pub(crate) async fn offer_result(
    engine: &Engine,
    stopping: &CancellationToken,
    run: &str,
    station: &str,
    round: u32,
    gate: &str,
    result: GateResult,
) -> crate::Result<Outcome> {
    let mut wait = OFFER_AGAIN_AFTER;
    loop {
        match engine.gate_result(run, station, round, gate, result.clone()).await {
            Err(FlowError::Conflict(why)) => {
                tracing::warn!(run, station, gate, why, "flow: a gate's result met conflicts; offered again");
                tokio::select! {
                    _ = tokio::time::sleep(wait) => {}
                    _ = stopping.cancelled() => {
                        tracing::warn!(run, station, gate, "flow: a gate's result not recorded before stopping; the gate runs again at the next start");
                        return Err(FlowError::Conflict(why));
                    }
                }
                wait = (wait * 2).min(OFFER_AGAIN_AT_MOST);
            }
            r => return r,
        }
    }
}

/// Run a `command` gate where the run works (xops-04 §4): `sh -c` in its `cwd` (references
/// resolved; a relative one under the run's workspace), else the run's workspace; passes on exit
/// status 0. Refused unless the application turned command gates on.
///
/// The command sees none of this process's environment but the basics (`PATH`, `HOME`, the
/// locale, `TMPDIR`…): no provider key, no hook secret. The run reaches it there, never in the
/// command line — `FLOW_RUN`, `FLOW_STATION`, `FLOW_WORKSPACE`, `FLOW_CONTEXT_FILE` (the reference
/// context as JSON, written under the workspace's `.flow/`) and, when it is small,
/// `FLOW_CONTEXT` itself. It runs in a process group of its own, killed whole when it runs out of
/// time; only the end of its output is kept.
pub(crate) async fn command_gate(sh: &Shared, engine: &Engine, rec: &RunRecord, station: &str, g: &crate::GateDef) -> GateResult {
    use tokio::io::AsyncReadExt;
    let fail = |detail: String| GateResult { pass: false, detail };
    if !sh.cfg.command_gates {
        return fail(format!("command gates are not turned on here: {} cannot run", g.name));
    }
    let Some(command) = g.command.clone().filter(|c| !c.trim().is_empty()) else {
        return fail(format!("gate {} has no command", g.name));
    };
    let Some(ws) = engine.workspace(&rec.id) else {
        return fail(format!("gate {}: the run has no workspace", g.name));
    };
    let ctx = crate::kernel::refs::context_in(rec, None, None, ws.to_str());
    let cwd = match &g.cwd {
        Some(c) => match crate::kernel::refs::resolve_str(c, &ctx) {
            Value::String(s) if !s.is_empty() => {
                let p = std::path::PathBuf::from(&s);
                if p.is_absolute() {
                    p
                } else {
                    ws.join(p)
                }
            }
            _ => return fail(format!("gate {}: cwd {c} resolves to nothing", g.name)),
        },
        None => ws.clone(),
    };
    let context = ctx.to_string();
    let context_file = ws.join(".flow").join(format!("context-{station}.json"));
    let written = std::fs::create_dir_all(&cwd)
        .and_then(|_| std::fs::create_dir_all(ws.join(".flow")))
        .and_then(|_| std::fs::write(&context_file, &context));
    if let Err(e) = written {
        return fail(format!("gate {}: {}: {e}", g.name, cwd.display()));
    }
    let mut cmd = tokio::process::Command::new("sh");
    cmd.arg("-c")
        .arg(&command)
        .current_dir(&cwd)
        .env_clear()
        .env("FLOW_RUN", &rec.id)
        .env("FLOW_STATION", station)
        .env("FLOW_WORKSPACE", &ws)
        .env("FLOW_CONTEXT_FILE", &context_file)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    for k in GATE_ENV {
        if let Some(v) = std::env::var_os(k) {
            cmd.env(k, v);
        }
    }
    if context.len() <= GATE_CONTEXT_INLINE {
        cmd.env("FLOW_CONTEXT", &context);
    }
    #[cfg(unix)]
    cmd.process_group(0);
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return fail(format!("`{command}` did not start: {e}")),
    };
    let pid = child.id();
    let tail = |r: Option<Box<dyn tokio::io::AsyncRead + Unpin + Send>>| async move {
        let mut kept: Vec<u8> = vec![];
        let Some(mut r) = r else { return kept };
        let mut buf = [0u8; 8192];
        while let Ok(n) = r.read(&mut buf).await {
            if n == 0 {
                break;
            }
            kept.extend_from_slice(&buf[..n]);
            if kept.len() > 2 * GATE_OUTPUT_READ {
                kept.drain(..kept.len() - GATE_OUTPUT_READ);
            }
        }
        kept
    };
    let out = tail(child.stdout.take().map(|o| Box::new(o) as _));
    let err = tail(child.stderr.take().map(|o| Box::new(o) as _));
    let waited = tokio::time::timeout(Duration::from_secs(g.timeout_seconds()), async {
        let (o, e, status) = tokio::join!(out, err, child.wait());
        (o, e, status)
    })
    .await;
    let (o, e, status) = match waited {
        Ok(v) => v,
        Err(_) => {
            // The whole group: a test runner's children too.
            #[cfg(unix)]
            if let Some(pid) = pid {
                let _ = std::process::Command::new("kill").arg("-KILL").arg(format!("-{pid}")).status();
            }
            let _ = child.kill().await;
            return fail(format!("`{command}` ran longer than {}s", g.timeout_seconds()));
        }
    };
    let status = match status {
        Ok(s) => s,
        Err(e) => return fail(format!("`{command}` was lost: {e}")),
    };
    let mut text = String::from_utf8_lossy(&o).into_owned();
    text.push_str(&String::from_utf8_lossy(&e));
    let tail: String = {
        let chars: Vec<char> = text.trim().chars().collect();
        chars[chars.len().saturating_sub(GATE_OUTPUT_KEPT)..].iter().collect()
    };
    match status.code() {
        Some(0) => GateResult { pass: true, detail: tail },
        code => fail(format!(
            "`{command}` exited with {}{}",
            code.map_or("a signal".to_string(), |c| c.to_string()),
            if tail.is_empty() { String::new() } else { format!(": {tail}") }
        )),
    }
}

/// What a command gate's process keeps of this one's environment.
const GATE_ENV: &[&str] = &["PATH", "HOME", "USER", "LOGNAME", "LANG", "LC_ALL", "LC_CTYPE", "TMPDIR", "TERM", "SHELL"];
/// The largest context also given inline (`FLOW_CONTEXT`); a larger one only by its file.
const GATE_CONTEXT_INLINE: usize = 32 * 1024;
/// How much of each output stream is read and held at most.
const GATE_OUTPUT_READ: usize = 64 * 1024;

/// How much of a command gate's output its result keeps (the end of it).
const GATE_OUTPUT_KEPT: usize = 2000;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::host::{Gates, RunStore, StoreError, WorkItem};
    use crate::kernel::run::{Actor, CheckStatus, StationStatus};
    use crate::{CreateIn, Hooked, ManualClock, MemPacks, MemStore};
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A store whose next `n` commits conflict, `n` being what the counter holds.
    fn flaky() -> (Arc<Hooked<MemStore>>, Arc<AtomicUsize>) {
        let conflicts = Arc::new(AtomicUsize::new(0));
        let left = conflicts.clone();
        let store = Hooked::new(MemStore::new()).check_commits(move |_| {
            match left.fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1)) {
                Ok(_) => Err(StoreError::Conflict("injected".into())),
                Err(_) => Ok(()),
            }
        });
        (Arc::new(store), conflicts)
    }

    /// The application's checks: every one passes.
    struct Passes;

    #[async_trait::async_trait]
    impl Gates for Passes {
        async fn run(&self, _: &RunRecord, _: &str, gate: &str) -> GateResult {
            GateResult {
                pass: true,
                detail: format!("{gate} passed"),
            }
        }
    }

    /// A run whose first station waits for gate `gate`, and the module it runs in.
    struct Gated {
        sh: Arc<Shared>,
        engine: Arc<Engine>,
        run: String,
        work: WorkItem,
        _ws: tempfile::TempDir,
    }

    impl Gated {
        async fn new(gate_def: &str, store: Arc<dyn RunStore>, configure: impl FnOnce(&mut Shared)) -> Self {
            let name = serde_yaml::from_str::<Value>(gate_def).unwrap()["name"].as_str().unwrap().to_string();
            let packs = MemPacks::new()
                .command_gates()
                .check("lint")
                .gate_def(gate_def)
                .flow(&format!(
                    "{{name: g, stations: [{{id: a, exit: [{{kind: gate, gate: {name}}}]}}, {{id: b, exit: [{{kind: confirm}}]}}]}}"
                ));
            let ws = tempfile::tempdir().unwrap();
            let engine = Arc::new(
                Engine::new(store, Arc::new(packs))
                    .with_clock(Arc::new(ManualClock::new(1_000_000)))
                    .with_workspaces(ws.path()),
            );
            let mut sh = Shared::for_tests();
            configure(&mut sh);
            let sh = Arc::new(sh);
            let _ = sh.engine.set(engine.clone());
            let o = engine
                .create(
                    CreateIn {
                        flow: "g".into(),
                        subject: "s".into(),
                        ..Default::default()
                    },
                    Actor::new("device", "d"),
                )
                .await
                .unwrap();
            let run = o.value["run"].as_str().unwrap().to_string();
            let work = o
                .work
                .into_iter()
                .find(|w| matches!(w, WorkItem::Gate { .. }))
                .expect("the gate is handed out");
            Gated { sh, engine, run, work, _ws: ws }
        }
        async fn run_it(&self) {
            let WorkItem::Gate { run, station, round, gate } = self.work.clone() else {
                unreachable!()
            };
            run_gate(&self.sh, run, station, round, gate).await;
        }
        /// The station's status and its gate check's.
        async fn judged(&self) -> (StationStatus, CheckStatus, String) {
            let rec = self.engine.record(&self.run).await.unwrap();
            let st = &rec.stations["a"];
            let c = st.checks.iter().find(|c| c.kind == "gate").unwrap();
            (st.status, c.status, c.detail.clone())
        }
    }

    fn command(sh: &mut Shared) {
        sh.cfg.command_gates = true;
    }

    /// Where the run works, with the run in its environment and nothing of this process's but
    /// the basics.
    #[tokio::test]
    async fn a_command_gate_passes_on_status_0_where_the_run_works() {
        std::env::set_var("HARNIX_FLOW_GATE_TEST_SECRET", "not for gates");
        let g = Gated::new(
            r#"{name: build, kind: command, command: 'test "$FLOW_STATION" = a && test -z "$HARNIX_FLOW_GATE_TEST_SECRET" && test -f "$FLOW_CONTEXT_FILE" && test "$(pwd -P)" = "$(cd "$FLOW_WORKSPACE" && pwd -P)" && echo built "$FLOW_RUN"'}"#,
            Arc::new(MemStore::new()),
            command,
        )
        .await;
        g.run_it().await;
        let (station, check, detail) = g.judged().await;
        assert_eq!((station, check), (StationStatus::Passed, CheckStatus::Pass), "{detail}");
        assert_eq!(detail, format!("built {}", g.run));
    }

    #[tokio::test]
    async fn a_command_gate_fails_on_any_other_status_with_the_end_of_its_output() {
        let g = Gated::new(
            "{name: build, kind: command, command: 'echo nope >&2; exit 3'}",
            Arc::new(MemStore::new()),
            command,
        )
        .await;
        g.run_it().await;
        let (station, check, detail) = g.judged().await;
        assert_eq!(check, CheckStatus::Fail);
        assert_ne!(station, StationStatus::Passed);
        assert!(detail.contains("exited with 3: nope"), "{detail}");
    }

    #[tokio::test]
    async fn a_command_gate_runs_only_where_command_gates_are_turned_on() {
        let g = Gated::new("{name: build, kind: command, command: 'true'}", Arc::new(MemStore::new()), |_| {}).await;
        g.run_it().await;
        let (_, check, detail) = g.judged().await;
        assert_eq!(check, CheckStatus::Fail);
        assert!(detail.contains("command gates are not turned on here"), "{detail}");
    }

    #[tokio::test]
    async fn a_command_gate_that_runs_too_long_is_killed_and_fails() {
        let g = Gated::new(
            "{name: build, kind: command, command: 'sleep 30', timeout_seconds: 1}",
            Arc::new(MemStore::new()),
            command,
        )
        .await;
        let started = std::time::Instant::now();
        g.run_it().await;
        assert!(started.elapsed() < Duration::from_secs(10));
        let (_, check, detail) = g.judged().await;
        assert_eq!(check, CheckStatus::Fail);
        assert!(detail.contains("ran longer than 1s"), "{detail}");
    }

    /// More conflicts than one operation retries: the result is offered again, not dropped.
    #[tokio::test(start_paused = true)]
    async fn a_gates_result_outlasts_more_conflicts_than_one_commit_retries() {
        let (store, conflicts) = flaky();
        let g = Gated::new("{name: lint, kind: host}", store.clone(), |sh| sh.cfg.gates = Some(Arc::new(Passes))).await;
        conflicts.store(3 * crate::DEFAULT_RETRIES - 1, Ordering::SeqCst);
        g.run_it().await;
        assert_eq!(conflicts.load(Ordering::SeqCst), 0);
        let (station, check, _) = g.judged().await;
        assert_eq!((station, check), (StationStatus::Passed, CheckStatus::Pass));
    }

    /// Stopping while the result still meets conflicts: the gate is still waiting, and the next
    /// start hands it out again.
    #[tokio::test(start_paused = true)]
    async fn a_gates_result_not_recorded_before_stopping_runs_again_at_the_next_start() {
        let (store, conflicts) = flaky();
        let g = Gated::new("{name: lint, kind: host}", store.clone(), |sh| sh.cfg.gates = Some(Arc::new(Passes))).await;
        conflicts.store(usize::MAX, Ordering::SeqCst);
        let stop = async {
            tokio::time::sleep(Duration::from_secs(60)).await;
            g.sh.cancel.cancel();
        };
        tokio::join!(g.run_it(), stop);
        conflicts.store(0, Ordering::SeqCst);
        let (_, check, _) = g.judged().await;
        assert_eq!(check, CheckStatus::Wait);
        let rec = g.engine.record(&g.run).await.unwrap();
        let again = g.engine.dispatched_work(&rec, g.engine.clock().now_ms());
        assert_eq!(again, vec![g.work.clone()]);
    }
}
