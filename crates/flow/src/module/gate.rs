//! Running a gate the kernel handed out: a `command` gate here, a `host` one by the application.

use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;

use super::dispatch::settle;
use super::shared::Shared;
use crate::kernel::host::GateResult;
use crate::kernel::parts::GateKind;
use crate::kernel::run::RunRecord;

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
    let r = engine
        .gate_result(&run, &station, round, &gate, result)
        .await;
    settle(sh, &run, "gate_result", r).await;
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
pub(crate) async fn command_gate(sh: &Shared, engine: &crate::Engine, rec: &RunRecord, station: &str, g: &crate::GateDef) -> GateResult {
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
