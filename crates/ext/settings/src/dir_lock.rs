//! An exclusive hold on a settings tier's directory, taken the way Agix takes it: other threads of
//! this process queue on a mutex, and on unix other processes (the engine writing a "permanent
//! allow" or `project.trust` into the same tier) through `flock` on the directory itself.
//!
//! The same lock as `harnix_contract::dir_lock`, kept here because an extension names no Harnix
//! crate but `harnix-ext`; it goes when providers are written through the engine.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, OnceLock, PoisonError};

pub(crate) struct DirLock {
    _process: MutexGuard<'static, ()>,
    #[cfg(unix)]
    _handle: std::fs::File,
}

impl DirLock {
    /// Create `dir` if it is missing and wait until no one else holds it.
    pub(crate) fn acquire(dir: &Path) -> std::io::Result<Self> {
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
            _process: process,
            #[cfg(unix)]
            _handle: handle,
        })
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
