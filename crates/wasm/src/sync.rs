//! Taking a lock, the one way this crate does it.
//!
//! A `std` lock is poisoned when a thread panics while holding it, and every later `lock()` then
//! returns an error. The VM's locks guard counters, its state, and the maps of what a backend
//! holds on the host, whose every update is a single assignment, insert or removal: a panic
//! halfway through leaves nothing half-written that a later reader could trip on. So a poisoned
//! lock is taken anyway, here, rather than turned into a second panic at every caller.
//!
//! The same rule as the kernel's (`harnix_contract::sync`), written out again because this crate
//! depends on no other Harnix crate (`crates/harnix/tests/layering.rs`).

use std::sync::{Mutex, MutexGuard};

/// `m.lock()`, taken even if a panic poisoned it.
pub(crate) fn lock<T: ?Sized>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_poisoned_lock_is_still_taken() {
        let m = std::sync::Arc::new(Mutex::new(1));
        let m2 = m.clone();
        let _ = std::thread::spawn(move || {
            let _m = m2.lock().unwrap();
            panic!("poison it");
        })
        .join();
        assert!(m.is_poisoned());
        *lock(&m) += 1;
        assert_eq!(*lock(&m), 2);
    }
}
