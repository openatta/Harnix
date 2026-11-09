//! Taking a lock, the one way the kernel does it.
//!
//! A `std` lock is poisoned when a thread panics while holding it, and every
//! later `lock()` then returns an error. The kernel's locks guard maps and
//! counters whose every update is a single assignment or insert: a panic
//! halfway through leaves nothing half-written that a later reader could
//! trip on. So a poisoned lock is taken anyway, here, rather than turned into
//! a second panic at every caller — which is what `.lock().unwrap()` did: one
//! panic in a task holding the audit ring and every call after it panicked
//! too, inside its own spawned task, so the client was left with a request
//! that never answered instead of an error.
//!
//! A lock whose data *can* be left inconsistent by a panic should not use
//! these; none of the kernel's do.

use std::sync::{Mutex, MutexGuard, RwLock, RwLockReadGuard, RwLockWriteGuard};

/// `m.lock()`, taken even if a panic poisoned it.
pub fn lock<T: ?Sized>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// `l.read()`, taken even if a panic poisoned it.
pub fn read<T: ?Sized>(l: &RwLock<T>) -> RwLockReadGuard<'_, T> {
    l.read().unwrap_or_else(|e| e.into_inner())
}

/// `l.write()`, taken even if a panic poisoned it.
pub fn write<T: ?Sized>(l: &RwLock<T>) -> RwLockWriteGuard<'_, T> {
    l.write().unwrap_or_else(|e| e.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_poisoned_lock_is_still_taken() {
        let m = std::sync::Arc::new(Mutex::new(1));
        let l = std::sync::Arc::new(RwLock::new(1));
        let (m2, l2) = (m.clone(), l.clone());
        let _ = std::thread::spawn(move || {
            let _m = m2.lock().unwrap();
            let _l = l2.write().unwrap();
            panic!("poison both");
        })
        .join();
        assert!(m.is_poisoned() && l.is_poisoned());
        *lock(&m) += 1;
        *write(&l) += 1;
        assert_eq!(*lock(&m), 2);
        assert_eq!(*read(&l), 2);
    }
}
