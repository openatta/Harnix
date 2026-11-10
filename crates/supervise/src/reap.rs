//! Ending every process left in a spawned child's session (ipdrun-07).
//!
//! A child that asks for it (`ChildSpec::own_session`) starts a session of its own
//! (`setsid`), so its id is its pid. Every command the engine runs stays in
//! that session even after it makes a process group of its own, and so does whatever such a
//! command leaves in the background. Once the child is gone — stopped, killed, crashed, about to
//! be started again — the supervisor ends what is still in it: TERM, a grace, KILL.
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
    tracing::info!(session = sid, processes = found.len(), "ending what the child left running");
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
    // room for processes started in between), and the answer says how many pids it wrote. A count,
    // not bytes: libproc's `proc_listallpids` divides `proc_listpids`'s byte count by the size of
    // a pid (`the_listing_counts_pids_not_bytes` pins it).
    unsafe {
        let n = libc::proc_listallpids(std::ptr::null_mut(), 0);
        if n <= 0 {
            return Vec::new();
        }
        let mut room = n as usize + 64;
        // A buffer it filled to the last slot may have been too small — more processes started
        // between the two calls than the room left for them, which a busy machine does — and
        // the pids that did not fit are missing, perhaps the very one being looked for. So a
        // full answer is asked for again with twice the room, a few times.
        let mut pids = Vec::new();
        for _ in 0..4 {
            pids = vec![0 as libc::pid_t; room];
            let bytes = (pids.len() * std::mem::size_of::<libc::pid_t>()) as libc::c_int;
            let got = libc::proc_listallpids(pids.as_mut_ptr().cast(), bytes);
            if got <= 0 {
                return Vec::new();
            }
            pids.truncate((got as usize).min(pids.len()));
            if pids.len() < room {
                break;
            }
            room *= 2;
        }
        pids
    }
}

#[cfg(target_os = "macos")]
fn session_of(pid: i32) -> Option<i32> {
    // SAFETY: getsid(2) on any pid; -1 when it is gone.
    let sid = unsafe { libc::getsid(pid) };
    (sid > 0).then_some(sid)
}

/// Another unix: no way to list processes here, so nothing a child left is found. Said once,
/// rather than reaping being a silent no-op.
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn all_pids() -> Vec<i32> {
    static SAID: std::sync::Once = std::sync::Once::new();
    SAID.call_once(|| {
        tracing::warn!(
            "this platform cannot list processes: what a stopped child leaves running is not ended"
        )
    });
    Vec::new()
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn session_of(_pid: i32) -> Option<i32> {
    None
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
    /// outright, as a crashed child would be. The sleep is still there, re-parented; reaping the
    /// session ends it.
    #[tokio::test]
    async fn a_session_whose_leader_was_killed_is_reaped() {
        let mut command = tokio::process::Command::new("/bin/sh");
        command
            .args(["-c", "sleep 300 >/dev/null 2>&1 & echo $!; exec cat"])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .kill_on_drop(true);
        crate::child::own_session_for(&mut command);
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

    #[tokio::test]
    async fn this_process_never_reaps_its_own_session() {
        let own = own_session().unwrap();
        assert_eq!(reap_session(own, Duration::from_millis(10)).await, 0);
    }

    /// Every live process is listed once, this one and a child of it among them, and nothing
    /// that is not a pid is: a listing read as bytes rather than pids would be cut to a quarter,
    /// or padded with zeros.
    #[test]
    fn the_listing_counts_pids_not_bytes() {
        if !cfg!(any(target_os = "linux", target_os = "macos")) {
            return;
        }
        let mut child = std::process::Command::new("/bin/sleep").arg("30").spawn().unwrap();
        let pids = all_pids();
        let _ = child.kill();
        let _ = child.wait();
        let me = std::process::id() as i32;
        assert!(pids.contains(&me), "this process is not listed");
        assert!(pids.contains(&(child.id() as i32)), "its child is not listed");
        assert!(pids.iter().filter(|&&p| p == 0).count() <= 1, "zeros past the end: {} pids", pids.len());
        let unique: std::collections::BTreeSet<_> = pids.iter().collect();
        assert_eq!(unique.len(), pids.len(), "a pid listed twice");
    }
}
