//! One process per state directory (ipdrun-09).
//!
//! The state directory holds the token, the devices, the instance ledger and every module's data,
//! and nothing in it is written with a second process in mind: a second one on another port used to
//! start, overwrite the token the first one's clients read, and build the ledger's instances again
//! on the same data roots. So the process holds an exclusive lock on `<state>/harnix.lock` for as
//! long as it lives, taken before anything in the directory is read, and a second process is refused
//! with who holds it.
//!
//! The file says who: `{"pid": …, "url": …}`, the URL written once the listener is bound — the
//! reliable answer to "is one running already, and where", for a desktop shell or a command-line
//! tool. The lock is advisory (`flock`) and goes with the process, however it ends.

use std::fs::{File, OpenOptions};
use std::io::{Read as _, Seek as _, Write as _};
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

/// The lock file's name in the state directory.
pub const LOCK_FILE: &str = "harnix.lock";

pub(crate) struct StateLock {
    file: File,
    path: PathBuf,
}

impl StateLock {
    /// Take the lock, or say who has it.
    pub(crate) fn acquire(state: &Path) -> anyhow::Result<Self> {
        std::fs::create_dir_all(state)
            .map_err(|e| anyhow::anyhow!("the state directory {}: {e}", state.display()))?;
        let path = state.join(LOCK_FILE);
        // Not truncated on open: until the lock is ours, what is in it is the holder's.
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))?;
        if !try_lock(&file) {
            let holder = read_holder(&path);
            let pid = holder
                .get("pid")
                .and_then(Value::as_u64)
                .map(|p| p.to_string())
                .unwrap_or_else(|| "?".into());
            let url = holder
                .get("url")
                .and_then(Value::as_str)
                .map(|u| format!(" ({u})"))
                .unwrap_or_default();
            anyhow::bail!(
                "{} is in use by pid {pid}{url}; one process per state directory \
                 (HARNIX_STATE_DIR or another [brand] product_id gives a second one its own)",
                state.display()
            );
        }
        let lock = Self { file, path };
        lock.write(None);
        Ok(lock)
    }

    /// Say where this process listens.
    pub(crate) fn listening(&self, url: &str) {
        self.write(Some(url));
    }

    fn write(&self, url: Option<&str>) {
        let body = json!({"pid": std::process::id(), "url": url}).to_string();
        let mut file = &self.file;
        let written = file
            .set_len(0)
            .and_then(|()| file.rewind())
            .and_then(|()| file.write_all(body.as_bytes()))
            .and_then(|()| file.flush());
        if let Err(e) = written {
            tracing::warn!(path = %self.path.display(), error = %e, "could not write the lock file");
        }
    }
}

/// What a lock file says, or nothing when it says nothing readable.
fn read_holder(path: &Path) -> Value {
    let mut text = String::new();
    let _ = File::open(path).and_then(|mut f| f.read_to_string(&mut text));
    serde_json::from_str(&text).unwrap_or(Value::Null)
}

#[cfg(unix)]
fn try_lock(file: &File) -> bool {
    use std::os::unix::io::AsRawFd as _;
    // SAFETY: a valid descriptor for as long as `file` lives; `flock` touches nothing else.
    unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) == 0 }
}

/// No `flock` here: the lock is not taken, as before.
#[cfg(not(unix))]
fn try_lock(_file: &File) -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_second_holder_is_refused_with_the_first_ones_pid_and_url() {
        let dir = tempfile::tempdir().unwrap();
        let first = StateLock::acquire(dir.path()).unwrap();
        first.listening("http://127.0.0.1:4080/");
        let refused = StateLock::acquire(dir.path()).err().expect("a second lock was granted");
        let said = refused.to_string();
        assert!(said.contains(&format!("pid {}", std::process::id())), "{said}");
        assert!(said.contains("http://127.0.0.1:4080/"), "{said}");
        drop(first);
        // Gone with its holder.
        assert!(StateLock::acquire(dir.path()).is_ok());
    }
}
