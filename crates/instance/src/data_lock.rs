//! One daemon per instance on its data root (ipdrun-08).
//!
//! A spawned daemon learns its parent is gone when stdin closes, and then finishes gracefully —
//! the turns in flight, the transcripts written out — which takes a second or two. A Harnix killed
//! outright and started again at once spawns the same instance's new daemon on the same data root
//! well within that, and the two engines wrote the same session's transcript, name and settings at
//! the same time.
//!
//! So this binary's `daemon` subcommand, given `--instance`, holds an exclusive lock on
//! `<data_root>/.daemon-<instance>.lock` for its whole life and waits for it before building its
//! engine. Per instance rather than per data root: what has to be kept apart is the old and the new
//! daemon of one instance; instances an application has made share a data root keep doing so. The
//! lock is advisory (`flock`), says the holder's pid, and goes with its process however it ends.
//! A daemon from some other program is not given `--instance` and takes no lock.

use std::fs::{File, OpenOptions};
use std::io::{Read as _, Seek as _, Write as _};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// How long a daemon waits for the previous one to let go when nothing says otherwise.
pub const DEFAULT_LOCK_WAIT: Duration = Duration::from_secs(10);

/// Where one instance's daemon lock lives.
pub fn lock_path(data_root: &Path, instance: &str) -> PathBuf {
    data_root.join(format!(".daemon-{instance}.lock"))
}

/// The lock, held for as long as this lives.
pub struct DaemonLock {
    _file: File,
}

/// Somebody else has it: the pid they wrote, when they wrote one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Held {
    pub pid: Option<u32>,
}

impl std::fmt::Display for Held {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.pid {
            Some(pid) => write!(f, "the data root is held by pid {pid}"),
            None => write!(f, "the data root is held by another daemon"),
        }
    }
}

/// Take the lock, trying again every 100 ms for up to `wait`.
pub async fn acquire(path: &Path, wait: Duration) -> std::io::Result<Result<DaemonLock, Held>> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let started = Instant::now();
    loop {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)?;
        if try_lock(&file) {
            let mut handle = &file;
            handle.set_len(0)?;
            handle.rewind()?;
            handle.write_all(std::process::id().to_string().as_bytes())?;
            handle.flush()?;
            return Ok(Ok(DaemonLock { _file: file }));
        }
        drop(file);
        if started.elapsed() >= wait {
            return Ok(Err(Held { pid: read_pid(path) }));
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// Whether somebody holds the lock now, and who. Takes it for a moment to find out, and lets go.
pub fn holder(path: &Path) -> Option<Held> {
    let file = OpenOptions::new().read(true).write(true).open(path).ok()?;
    if try_lock(&file) {
        return None;
    }
    Some(Held { pid: read_pid(path) })
}

fn read_pid(path: &Path) -> Option<u32> {
    let mut text = String::new();
    File::open(path).ok()?.read_to_string(&mut text).ok()?;
    text.trim().parse().ok()
}

#[cfg(unix)]
fn try_lock(file: &File) -> bool {
    use std::os::unix::io::AsRawFd as _;
    // SAFETY: a valid descriptor for as long as `file` lives; `flock` touches nothing else.
    unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) == 0 }
}

/// No `flock` here: no lock, as before.
#[cfg(not(unix))]
fn try_lock(_file: &File) -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn the_second_daemon_of_an_instance_waits_for_the_first() {
        let root = tempfile::tempdir().unwrap();
        let path = lock_path(root.path(), "ci");
        let first = acquire(&path, Duration::ZERO).await.unwrap().unwrap();
        assert_eq!(holder(&path), Some(Held { pid: Some(std::process::id()) }));

        let waiting = tokio::spawn({
            let path = path.clone();
            async move {
                let lock = acquire(&path, Duration::from_secs(5)).await.unwrap();
                (lock.is_ok(), Instant::now())
            }
        });
        tokio::time::sleep(Duration::from_millis(400)).await;
        assert!(!waiting.is_finished(), "the second took the lock while the first held it");
        let released = Instant::now();
        drop(first);
        let (got, at) = waiting.await.unwrap();
        assert!(got);
        assert!(at.duration_since(released) < Duration::from_secs(1));
    }

    #[tokio::test]
    async fn other_instances_on_one_data_root_do_not_wait() {
        let root = tempfile::tempdir().unwrap();
        let _a = acquire(&lock_path(root.path(), "a"), Duration::ZERO).await.unwrap().unwrap();
        let b = acquire(&lock_path(root.path(), "b"), Duration::ZERO).await.unwrap();
        assert!(b.is_ok());
    }

    /// The link side: a daemon that gave up on the lock is not started again and again; the
    /// instance is down, with who holds its data root.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_daemon_that_cannot_have_the_lock_leaves_its_instance_down_saying_who() {
        use crate::link::{EngineLink as _, LinkStatus};
        use crate::{DaemonLink, DaemonSpec, Endpoint, SpawnSpec};

        let root = tempfile::tempdir().unwrap();
        let path = lock_path(root.path(), "ci");
        let _held = acquire(&path, Duration::ZERO).await.unwrap().unwrap();
        let link = DaemonLink::start(DaemonSpec {
            endpoint: Endpoint::Unix(root.path().join("d.sock")),
            spawn: Some(SpawnSpec {
                // What a daemon that gave up does: exit before listening.
                program: "/bin/sh".into(),
                args: vec!["-c".into(), "exit 1".into()],
                env: Vec::new(),
                lock: Some(path),
            }),
        });
        let mut status = link.status();
        let down = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if let LinkStatus::Down { reason } = status.borrow_and_update().clone() {
                    return reason;
                }
                status.changed().await.unwrap();
            }
        })
        .await
        .expect("the instance did not go down");
        assert!(down.contains(&format!("held by pid {}", std::process::id())), "{down}");
        link.shutdown().await;
    }

    #[tokio::test]
    async fn waiting_past_the_limit_says_who_holds_it() {
        let root = tempfile::tempdir().unwrap();
        let path = lock_path(root.path(), "ci");
        let _first = acquire(&path, Duration::ZERO).await.unwrap().unwrap();
        let asked = Instant::now();
        let held = acquire(&path, Duration::from_millis(300)).await.unwrap().err().unwrap();
        assert!(asked.elapsed() >= Duration::from_millis(300));
        assert_eq!(held.pid, Some(std::process::id()));
        assert!(held.to_string().contains(&format!("pid {}", std::process::id())));
    }
}
