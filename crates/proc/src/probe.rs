//! `ext-proc --probe <targets>` (feature `probe`, for tests): what the sandbox lets this process,
//! and the processes it starts, reach — tried, not assumed. It runs where the process would start
//! its package: on its wire, inside its sandbox. It answers one line on the wire and exits.
//!
//! `<targets>` names what the product keeps from it, by what each is:
//!
//! ```json
//! {"unreadable": {"token": "/state/token", "devices.json": "…", …},
//!  "unwritable": {"its package": "/state/ext/<id>/<version>", "install.json": "…"},
//!  "tcp": "127.0.0.1:<port>"}
//! ```
//!
//! What it must reach it takes from its own command line: it reads `--pkg`, and reads and writes
//! `--run` and every directory in it. The answer is `{"checks": [{"check", "ok", "saw"}]}`. A
//! check that should be refused passes only on a refusal (a target that is not there fails it),
//! and one about a child passes only if the child ran at all.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Context};
use harnix_wasm_host::{Host, HostOptions};
use harnix_wasm_port::rpc::{KernelClient, Wire};
use harnix_wasm_port::socket::{Options as WireOptions, SocketWire};
use serde_json::{json, Value};

use crate::agent::NoEngine;
use crate::answer::Answering;
use crate::Args;

struct Check {
    check: String,
    ok: bool,
    saw: String,
}

/// Run every check and answer them on `wire`.
pub(crate) fn run(targets: &str, args: &Args, mut wire: UnixStream) -> anyhow::Result<()> {
    let targets: Value = serde_json::from_str(targets).context("`--probe` takes the targets as JSON")?;
    let named = |key: &str| -> BTreeMap<String, PathBuf> {
        targets[key]
            .as_object()
            .map(|o| o.iter().filter_map(|(k, v)| Some((k.clone(), PathBuf::from(v.as_str()?)))).collect())
            .unwrap_or_default()
    };
    let (unreadable, unwritable) = (named("unreadable"), named("unwritable"));
    let mut checks = Vec::new();

    for (what, path) in &unreadable {
        checks.push(refused(format!("cannot read {what}"), read(path)));
    }
    for (what, path) in &unwritable {
        checks.push(refused(format!("cannot write {what}"), write(path)));
    }
    if let Some(pkg) = &args.pkg {
        checks.push(allowed("can read its package".into(), read(pkg)));
    }
    if let Some(run) = &args.run {
        let mut dirs = vec![run.clone()];
        if let Ok(entries) = std::fs::read_dir(run) {
            let mut inner: Vec<PathBuf> = entries.filter_map(|e| Some(e.ok()?.path())).filter(|p| p.is_dir()).collect();
            inner.sort();
            dirs.extend(inner);
        }
        for dir in dirs {
            let name = dir.strip_prefix(run).ok().filter(|p| !p.as_os_str().is_empty()).map(|p| format!("run/{}", p.display()));
            checks.push(allowed(format!("can read and write {}", name.as_deref().unwrap_or("run")), read_write(&dir)));
        }
    }
    match targets["tcp"].as_str() {
        Some(addr) => checks.push(allowed(format!("can reach {addr}"), connect(addr))),
        None => checks.push(Check { check: "can reach loopback TCP".into(), ok: false, saw: "no `tcp` target".into() }),
    }

    // What it starts inherits the sandbox: a shell that runs at all, and still cannot reach what
    // this process cannot.
    if let Some(run) = &args.run {
        let script = r#"echo probe > "$1/.probe-child" && cat -- "$1/.probe-child" && rm -- "$1/.probe-child""#;
        let out = sh(script, run);
        let ok = matches!(&out, Ok((true, stdout, _)) if stdout.trim() == "probe");
        checks.push(Check { check: "a child can write run".into(), ok, saw: shown(&out) });
    }
    for (what, path) in &unreadable {
        let out = sh(r#"if [ -d "$1" ]; then ls -- "$1"; else cat -- "$1"; fi"#, path);
        checks.push(Check { check: format!("a child cannot read {what}"), ok: child_refused(&out, path), saw: shown(&out) });
    }
    for (what, path) in &unwritable {
        let out = sh(r#"if [ -d "$1" ]; then : > "$1/.probe-child"; else : >> "$1"; fi"#, path);
        checks.push(Check { check: format!("a child cannot write {what}"), ok: child_refused(&out, path), saw: shown(&out) });
    }

    checks.push(match vm_engine() {
        Ok(fingerprint) => Check { check: "its VM engine builds".into(), ok: true, saw: fingerprint },
        Err(e) => Check { check: "its VM engine builds".into(), ok: false, saw: format!("{e:#}") },
    });

    let checks: Vec<Value> = checks.into_iter().map(|c| json!({"check": c.check, "ok": c.ok, "saw": c.saw})).collect();
    wire.set_nonblocking(false)?;
    let mut line = serde_json::to_vec(&json!({"checks": checks}))?;
    line.push(b'\n');
    wire.write_all(&line)?;
    Ok(())
}

fn refused(check: String, tried: std::io::Result<()>) -> Check {
    match tried {
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => Check { check, ok: true, saw: e.to_string() },
        Err(e) => Check { check, ok: false, saw: format!("not a refusal: {e}") },
        Ok(()) => Check { check, ok: false, saw: "it could".into() },
    }
}

fn allowed(check: String, tried: std::io::Result<()>) -> Check {
    match tried {
        Ok(()) => Check { check, ok: true, saw: "it could".into() },
        Err(e) => Check { check, ok: false, saw: e.to_string() },
    }
}

/// A directory listed and its first file read, or a file read.
fn read(path: &Path) -> std::io::Result<()> {
    if !path.is_dir() {
        return std::fs::File::open(path)?.read_to_end(&mut Vec::new()).map(drop);
    }
    let mut stack = vec![path.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir)? {
            let path = entry?.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                return std::fs::File::open(&path)?.read_to_end(&mut Vec::new()).map(drop);
            }
        }
    }
    Ok(())
}

/// A file made in a directory, or a file opened to append to.
fn write(path: &Path) -> std::io::Result<()> {
    if path.is_dir() {
        let made = path.join(".probe");
        std::fs::OpenOptions::new().write(true).create_new(true).open(&made)?;
        let _ = std::fs::remove_file(made);
        return Ok(());
    }
    std::fs::OpenOptions::new().append(true).open(path).map(drop)
}

/// A file made in `dir`, read back and removed.
fn read_write(dir: &Path) -> std::io::Result<()> {
    let file = dir.join(".probe");
    std::fs::write(&file, b"probe")?;
    let back = std::fs::read(&file)?;
    std::fs::remove_file(&file)?;
    if back != b"probe" {
        return Err(std::io::Error::other("read back something else"));
    }
    Ok(())
}

fn connect(addr: &str) -> std::io::Result<()> {
    let addr = addr.parse().map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, format!("{addr}: {e}")))?;
    std::net::TcpStream::connect_timeout(&addr, Duration::from_secs(5)).map(drop)
}

/// `/bin/sh -c <script> probe <arg>`: whether it succeeded, its stdout, its stderr.
fn sh(script: &str, arg: &Path) -> std::io::Result<(bool, String, String)> {
    let out = Command::new("/bin/sh").args(["-c", script, "probe"]).arg(arg).output()?;
    Ok((
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    ))
}

/// The shell ran, failed, and said it was not allowed to reach `path`: not that it is missing.
fn child_refused(out: &std::io::Result<(bool, String, String)>, path: &Path) -> bool {
    let Ok((false, _, stderr)) = out else {
        return false;
    };
    let path = path.to_string_lossy();
    stderr.lines().any(|l| l.contains(path.as_ref()) && (l.contains("not permitted") || l.contains("denied")))
}

fn shown(out: &std::io::Result<(bool, String, String)>) -> String {
    match out {
        Ok((ok, stdout, stderr)) => format!("{}: {}{}", if *ok { "succeeded" } else { "failed" }, stdout.trim(), stderr.trim()),
        Err(e) => format!("did not start: {e}"),
    }
}

/// The host this process would run its package on, made as it would be: its fingerprint.
fn vm_engine() -> anyhow::Result<String> {
    let runtime = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
    runtime.block_on(async {
        // A kernel at the end of a pair nothing answers: the host is made, not used.
        let (stream, _other) = tokio::net::UnixStream::pair()?;
        let wire = SocketWire::client(stream, Arc::new(Answering::default()), WireOptions::default());
        let host = Host::new(HostOptions::default(), Arc::new(KernelClient(wire.clone() as Arc<dyn Wire>)), Arc::new(NoEngine))
            .map_err(|e| anyhow!("{e:#}"))?;
        let fingerprint = host.engine().fingerprint().to_string();
        wire.close("the probe is done").await;
        Ok(fingerprint)
    })
}
