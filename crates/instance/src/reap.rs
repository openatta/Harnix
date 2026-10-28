//! Ending every process left in a spawned daemon's session (ipdrun-07).
//!
//! A template that asks for it (`InstanceTemplate::reap_on_stop`) has its daemon start a session
//! of its own (`setsid`), so its id is the daemon's pid. Every command the engine runs stays in
//! that session even after it makes a process group of its own, and so does whatever such a
//! command leaves in the background. Once the daemon is gone — stopped, killed, crashed, about to
//! be started again — the link ends what is still in it: TERM, a grace, KILL.
//!
//! Listing processes is the platform part, and it is all here: Linux reads the session field of
//! `/proc/<pid>/stat`, macOS lists every pid (`proc_listallpids`) and asks each `getsid`. A
//! process that left the session itself (its own `setsid`) is not found; nothing short of a
//! container finds those.

use std::time::{Duration, Instant};

/// How long processes in the session get between TERM and KILL.
pub const REAP_GRACE: Duration = Duration::from_millis(1500);

/// Every live process whose session id is `sid`, apart from this one.
pub fn session_members(sid: i32) -> Vec<i32> {
    if sid <= 1 {
        return Vec::new();
    }
    let me = std::process::id() as i32;
    all_pids()
        .into_iter()
        .filter(|&pid| pid > 1 && pid != me && session_of(pid) == Some(sid))
        .collect()
}

/// End every process in session `sid`: TERM, wait up to `grace` for them to go, KILL the rest.
/// Refuses this process's own session — that would be Harnix ending itself. Answers how many
/// processes it found.
pub async fn reap_session(sid: i32, grace: Duration) -> usize {
    if sid <= 1 || own_session() == Some(sid) {
        return 0;
    }
    let found = session_members(sid);
    if found.is_empty() {
        return 0;
    }
    tracing::info!(session = sid, processes = found.len(), "ending what the daemon left running");
    for &pid in &found {
        signal(pid, libc::SIGTERM);
    }
    let deadline = Instant::now() + grace;
    loop {
        let left = session_members(sid);
        if left.is_empty() {
            break;
        }
        if Instant::now() >= deadline {
            for pid in left {
                signal(pid, libc::SIGKILL);
            }
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    found.len()
}

fn signal(pid: i32, sig: i32) {
    // SAFETY: kill(2) with a pid we just listed; the worst case is ESRCH.
    unsafe {
        libc::kill(pid, sig);
    }
}

fn own_session() -> Option<i32> {
    // SAFETY: getsid(0) reads this process's own session id.
    let sid = unsafe { libc::getsid(0) };
    (sid > 0).then_some(sid)
}

#[cfg(target_os = "linux")]
fn all_pids() -> Vec<i32> {
    let Ok(dir) = std::fs::read_dir("/proc") else {
        return Vec::new();
    };
    dir.filter_map(|e| e.ok()?.file_name().to_str()?.parse().ok())
        .collect()
}

/// The session field of `/proc/<pid>/stat`: the fourth after the command, which is in parentheses
/// and may itself contain spaces or parentheses — so counting starts after the last `)`. A zombie
/// is left out: it is already dead and only waits for its parent.
#[cfg(target_os = "linux")]
fn session_of(pid: i32) -> Option<i32> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let rest = &stat[stat.rfind(')')? + 1..];
    let mut fields = rest.split_whitespace();
    let state = fields.next()?;
    if state == "Z" {
        return None;
    }
    let _ppid = fields.next()?;
    let _pgrp = fields.next()?;
    fields.next()?.parse().ok()
}

#[cfg(target_os = "macos")]
fn all_pids() -> Vec<i32> {
    // SAFETY: a null buffer asks for the count; the second call fills a buffer of that size (with
    // room for processes started in between), and the answer says how many it wrote.
    unsafe {
        let n = libc::proc_listallpids(std::ptr::null_mut(), 0);
        if n <= 0 {
            return Vec::new();
        }
        let mut pids = vec![0 as libc::pid_t; n as usize + 64];
        let bytes = (pids.len() * std::mem::size_of::<libc::pid_t>()) as libc::c_int;
        let got = libc::proc_listallpids(pids.as_mut_ptr().cast(), bytes);
        if got <= 0 {
            return Vec::new();
        }
        pids.truncate((got as usize).min(pids.len()));
        pids
    }
}

#[cfg(target_os = "macos")]
fn session_of(pid: i32) -> Option<i32> {
    // SAFETY: getsid(2) on any pid; -1 when it is gone.
    let sid = unsafe { libc::getsid(pid) };
    (sid > 0).then_some(sid)
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn all_pids() -> Vec<i32> {
    Vec::new()
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn session_of(_pid: i32) -> Option<i32> {
    None
}

/// Turn on the engine's own reaping for the instance at `data_root`: `sandbox.reap_process_groups`
/// in its global settings tier, which every project's settings are layered over. Written to the
/// file rather than set in memory because the engine reads each project's settings from disk, and
/// a `config.reload` reads them all again. Everything else in the file is left as it was.
pub(crate) fn engine_reaps(data_root: &std::path::Path) -> std::io::Result<()> {
    use serde_json::{json, Value};
    let path = data_root.join("settings.json");
    let mut settings: Value = match std::fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str(&text)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, format!("{}: {e}", path.display())))?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => json!({}),
        Err(e) => return Err(e),
    };
    if !settings.is_object() {
        settings = json!({});
    }
    if settings.pointer("/sandbox/reap_process_groups") == Some(&Value::Bool(true)) {
        return Ok(());
    }
    if !settings.get("sandbox").is_some_and(Value::is_object) {
        settings["sandbox"] = json!({});
    }
    settings["sandbox"]["reap_process_groups"] = json!(true);
    std::fs::create_dir_all(data_root)?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(&settings).unwrap_or_default())?;
    std::fs::rename(&tmp, &path)
}

/// Make the child about to be spawned the leader of a session of its own.
pub(crate) fn own_session_for(command: &mut tokio::process::Command) {
    // SAFETY: setsid(2) is async-signal-safe, which is all pre_exec asks of the closure.
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncBufReadExt;

    fn alive(pid: i32) -> bool {
        // SAFETY: signal 0 only asks whether the pid exists.
        unsafe { libc::kill(pid, 0) == 0 }
    }

    /// A shell in a session of its own leaves a `sleep` in the background and is then killed
    /// outright, as a crashed daemon would be. The sleep is still there, re-parented; reaping the
    /// session ends it.
    #[tokio::test]
    async fn a_session_whose_leader_was_killed_is_reaped() {
        let mut command = tokio::process::Command::new("/bin/sh");
        command
            .args(["-c", "sleep 300 >/dev/null 2>&1 & echo $!; exec cat"])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .kill_on_drop(true);
        own_session_for(&mut command);
        let mut child = command.spawn().unwrap();
        let leader = child.id().unwrap() as i32;
        let mut lines = tokio::io::BufReader::new(child.stdout.take().unwrap()).lines();
        let sleeper: i32 = lines.next_line().await.unwrap().unwrap().trim().parse().unwrap();

        assert!(session_members(leader).contains(&sleeper));
        child.kill().await.unwrap();
        let _ = child.wait().await;
        assert!(alive(sleeper), "the background process went with its parent");

        assert!(reap_session(leader, REAP_GRACE).await >= 1);
        let deadline = Instant::now() + Duration::from_secs(3);
        while alive(sleeper) && Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert!(!alive(sleeper), "{sleeper} outlived the reap");
    }

    #[test]
    fn the_engine_s_reaping_is_turned_on_in_the_global_tier_and_nothing_else_moves() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, r#"{"model":"m","sandbox":{"subprocess_env":{"mode":"allowlist"}}}"#).unwrap();
        engine_reaps(dir.path()).unwrap();
        let written: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(written["sandbox"]["reap_process_groups"], true);
        assert_eq!(written["sandbox"]["subprocess_env"]["mode"], "allowlist");
        assert_eq!(written["model"], "m");
        let fresh = tempfile::tempdir().unwrap();
        engine_reaps(fresh.path()).unwrap();
        let written: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(fresh.path().join("settings.json")).unwrap()).unwrap();
        assert_eq!(written, serde_json::json!({"sandbox": {"reap_process_groups": true}}));
    }

    #[tokio::test]
    async fn this_process_never_reaps_its_own_session() {
        let own = own_session().unwrap();
        assert_eq!(reap_session(own, Duration::from_millis(10)).await, 0);
    }
}
