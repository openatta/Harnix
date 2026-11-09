//! What one store carries, and how its memory is counted.
//!
//! Every instance — the resident backend, each tool call — gets its own
//! `Store`, and the store's data is an [`AppState`]: the WASI context built
//! from the application's capabilities and nothing else, the memory limiter,
//! and a handle on the VM's shared parts so the host functions can reach the
//! application's services.

use crate::spec::Resolved;
use crate::vm::Shared;
use crate::ProgressSink;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use wasmtime::ResourceLimiter;
use wasmtime_wasi::{
    DirPerms, FilePerms, ResourceTable, WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView,
};

/// Which kind of instance a store is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Role {
    /// The resident backend: may subscribe and arm timers.
    Backend,
    /// A tool call or an event: gone when it returns, so anything it
    /// subscribed to or armed would outlive it with nowhere to go.
    Transient,
}

pub(crate) struct AppState {
    pub shared: Arc<Shared>,
    pub role: Role,
    pub progress: Option<Arc<dyn ProgressSink>>,
    pub limiter: StoreLimiter,
    /// Whether this store has a writable `/data`, and if so whether the application was over its
    /// storage quota when the store was made (then its files are read-only here). `None`: nothing
    /// it can write, so nothing to measure after its calls.
    pub over_quota: Option<bool>,
    wasi: WasiCtx,
    table: ResourceTable,
}

impl AppState {
    /// `project` is the calling session's project, for a tool call; the
    /// `${project}` grants are placed under it. Nothing else has one.
    pub fn new(
        shared: Arc<Shared>,
        role: Role,
        progress: Option<Arc<dyn ProgressSink>>,
        project: Option<&Path>,
    ) -> anyhow::Result<Self> {
        let full = shared.storage_full();
        let wasi = wasi_for(&shared.caps, project, full)?;
        let limiter = StoreLimiter::new(shared.memory.clone());
        let over_quota = shared.caps.preopens.iter().any(|p| p.writable).then_some(full);
        Ok(Self {
            shared,
            role,
            progress,
            limiter,
            over_quota,
            wasi,
            table: ResourceTable::new(),
        })
    }
}

/// The WASI context: preopens for the declared directories, and nothing else.
/// No inherited environment, arguments or stdio, and no sockets — the
/// network is `host.http-request`, where `net` is checked. `full`: the
/// application is over its storage quota, so `/data`'s files are offered
/// read-only (and may still be removed, which is how it gets under again).
fn wasi_for(caps: &Resolved, project: Option<&Path>, full: bool) -> anyhow::Result<WasiCtx> {
    let mut builder = WasiCtxBuilder::new();
    builder
        .allow_tcp(false)
        .allow_udp(false)
        .allow_ip_name_lookup(false);
    let placed = project
        .map(|p| caps.project_preopens(p))
        .unwrap_or_default();
    for p in caps.preopens.iter().chain(&placed) {
        let (dirs, files) = if p.writable && !full {
            (DirPerms::all(), FilePerms::all())
        } else if p.writable {
            // Over the quota: nothing written, but it may still delete to make room.
            (DirPerms::all(), FilePerms::READ)
        } else {
            (DirPerms::READ, FilePerms::READ)
        };
        builder
            .preopened_dir(&p.host, &p.guest, dirs, files)
            .map_err(|e| anyhow::anyhow!("preopening {} as {}: {e}", p.host.display(), p.guest))?;
    }
    Ok(builder.build())
}

impl WasiView for AppState {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi,
            table: &mut self.table,
        }
    }
}

/// Linear memory held by one application, across all of its stores.
#[derive(Debug)]
pub(crate) struct MemoryAccount {
    pub limit: u64,
    used: AtomicU64,
    peak: AtomicU64,
}

impl MemoryAccount {
    pub fn new(limit: u64) -> Self {
        Self {
            limit,
            used: AtomicU64::new(0),
            peak: AtomicU64::new(0),
        }
    }

    fn try_reserve(&self, bytes: u64) -> bool {
        let reserved = self
            .used
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |used| {
                used.checked_add(bytes).filter(|&after| after <= self.limit)
            });
        match reserved {
            Ok(before) => {
                self.peak.fetch_max(before + bytes, Ordering::Relaxed);
                true
            }
            Err(_) => false,
        }
    }

    fn release(&self, bytes: u64) {
        self.used.fetch_sub(bytes, Ordering::AcqRel);
    }

    pub fn used(&self) -> u64 {
        self.used.load(Ordering::Acquire)
    }

    pub fn peak(&self) -> u64 {
        self.peak.load(Ordering::Relaxed)
    }
}

/// Upper bounds on a store's structure — not its size, which is the memory
/// account's job. Not 1: instantiating a component produces several core
/// instances and memories (the guest's modules plus the WASI adapter). These
/// exist to stop unbounded growth.
const MAX_INSTANCES: usize = 64;
const MAX_MEMORIES: usize = 16;
const MAX_TABLES: usize = 64;
const MAX_TABLE_ELEMENTS: usize = 1 << 20;

/// A store's view of the application's memory account.
///
/// Growth is refused (the guest's `memory.grow` returns -1) rather than
/// trapped on, so a guest that can cope with an allocation failure may; one
/// that cannot will trap, and the VM then reports the trap as the memory
/// limit because [`StoreLimiter::refused`] says so.
pub(crate) struct StoreLimiter {
    account: Arc<MemoryAccount>,
    held: u64,
    last_grant: u64,
    refused: bool,
}

impl StoreLimiter {
    fn new(account: Arc<MemoryAccount>) -> Self {
        Self {
            account,
            held: 0,
            last_grant: 0,
            refused: false,
        }
    }

    /// Whether this store was ever refused memory.
    pub fn refused(&self) -> bool {
        self.refused
    }
}

impl ResourceLimiter for StoreLimiter {
    fn memory_growing(
        &mut self,
        current: usize,
        desired: usize,
        _maximum: Option<usize>,
    ) -> wasmtime::Result<bool> {
        let grow = desired.saturating_sub(current) as u64;
        if self.account.try_reserve(grow) {
            self.held += grow;
            self.last_grant = grow;
            Ok(true)
        } else {
            self.refused = true;
            Ok(false)
        }
    }

    /// A growth that was permitted and then failed anyway (the memory's own
    /// maximum, the OS): give back what was reserved for it.
    fn memory_grow_failed(&mut self, _error: wasmtime::Error) -> wasmtime::Result<()> {
        self.account.release(self.last_grant);
        self.held -= self.last_grant;
        self.last_grant = 0;
        Ok(())
    }

    fn table_growing(
        &mut self,
        _current: usize,
        desired: usize,
        _maximum: Option<usize>,
    ) -> wasmtime::Result<bool> {
        Ok(desired <= MAX_TABLE_ELEMENTS)
    }

    fn instances(&self) -> usize {
        MAX_INSTANCES
    }

    fn tables(&self) -> usize {
        MAX_TABLES
    }

    fn memories(&self) -> usize {
        MAX_MEMORIES
    }
}

impl Drop for StoreLimiter {
    /// The store is gone, and its memory with it.
    fn drop(&mut self) {
        self.account.release(self.held);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stores_share_one_account_and_give_it_back_when_dropped() {
        let account = Arc::new(MemoryAccount::new(100));
        let mut a = StoreLimiter::new(account.clone());
        let mut b = StoreLimiter::new(account.clone());
        assert!(a.memory_growing(0, 60, None).unwrap());
        assert!(
            !b.memory_growing(0, 50, None).unwrap(),
            "60 + 50 is over 100"
        );
        assert!(b.refused());
        assert!(b.memory_growing(0, 40, None).unwrap());
        assert_eq!(account.used(), 100);
        drop(a);
        assert_eq!(account.used(), 40);
        assert_eq!(account.peak(), 100);
        drop(b);
        assert_eq!(account.used(), 0);
    }

    #[test]
    fn a_permitted_growth_that_fails_is_given_back() {
        let account = Arc::new(MemoryAccount::new(100));
        let mut a = StoreLimiter::new(account.clone());
        assert!(a.memory_growing(10, 30, None).unwrap());
        a.memory_grow_failed(wasmtime::Error::msg("os said no"))
            .unwrap();
        assert_eq!(account.used(), 0);
    }
}
