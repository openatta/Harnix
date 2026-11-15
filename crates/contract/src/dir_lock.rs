//! An exclusive hold on a directory, taken the way Agix takes it on a settings tier: other
//! threads of this process queue on a mutex, and on unix other processes (the engine writing a
//! "permanent allow" or `project.trust` into the same tier) through `flock` on the directory
//! itself. A writer of an Agix settings file holds it over the whole read-modify-write.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, OnceLock, PoisonError};

pub struct DirLock {
    dir: PathBuf,
    _process: MutexGuard<'static, ()>,
    #[cfg(unix)]
    _handle: std::fs::File,
}

impl DirLock {
    /// Create `dir` if it is missing and wait until no one else holds it.
    pub fn acquire(dir: &Path) -> std::io::Result<Self> {
        std::fs::create_dir_all(dir)?;
        let dir = dir.canonicalize()?;
        let process = process_lock(&dir);
        #[cfg(unix)]
        let handle = {
            let handle = std::fs::File::open(&dir)?;
            flock_exclusive(&handle)?;
            handle
        };
        Ok(Self {
            dir,
            _process: process,
            #[cfg(unix)]
            _handle: handle,
        })
    }

    /// The directory, canonical.
    pub fn dir(&self) -> &Path {
        &self.dir
    }
}

fn process_lock(dir: &Path) -> MutexGuard<'static, ()> {
    static LOCKS: OnceLock<Mutex<HashMap<PathBuf, &'static Mutex<()>>>> = OnceLock::new();
    let mut locks = LOCKS.get_or_init(Default::default).lock().unwrap_or_else(PoisonError::into_inner);
    let lock: &'static Mutex<()> = match locks.get(dir) {
        Some(lock) => lock,
        None => {
            let lock = Box::leak(Box::default());
            locks.insert(dir.to_path_buf(), lock);
            lock
        }
    };
    drop(locks);
    lock.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(unix)]
fn flock_exclusive(file: &std::fs::File) -> std::io::Result<()> {
    use std::os::fd::AsRawFd;
    loop {
        // SAFETY: `flock` only reads the descriptor, which `file` keeps open.
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } == 0 {
            return Ok(());
        }
        let e = std::io::Error::last_os_error();
        if e.kind() != std::io::ErrorKind::Interrupted {
            return Err(e);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    /// With the lock held, a second taker waits until it is let go.
    #[test]
    fn a_second_taker_waits_for_the_first() {
        let dir = std::env::temp_dir().join(format!("harnix-dir-lock-{}", std::process::id()));
        let held = DirLock::acquire(&dir).unwrap();
        let got = Arc::new(AtomicBool::new(false));
        let waiter = {
            let (dir, got) = (dir.clone(), got.clone());
            std::thread::spawn(move || {
                let _second = DirLock::acquire(&dir).unwrap();
                got.store(true, Ordering::SeqCst);
            })
        };
        std::thread::sleep(Duration::from_millis(200));
        assert!(!got.load(Ordering::SeqCst), "taken while held");
        drop(held);
        waiter.join().unwrap();
        assert!(got.load(Ordering::SeqCst));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The `flock` itself, which another process meets: a second descriptor on the directory
    /// cannot take it while the first holds it.
    #[cfg(unix)]
    #[test]
    fn the_directory_is_flocked_for_other_descriptors() {
        use std::os::fd::AsRawFd;
        let dir = std::env::temp_dir().join(format!("harnix-dir-flock-{}", std::process::id()));
        let held = DirLock::acquire(&dir).unwrap();
        let other = std::fs::File::open(&dir).unwrap();
        // SAFETY: `other` stays open across the call.
        let busy = unsafe { libc::flock(other.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
        assert_eq!(busy, -1);
        drop(held);
        let free = unsafe { libc::flock(other.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
        assert_eq!(free, 0);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
