//! Whether an application is still worth calling, and what to report about it.
//!
//! Isolation makes a fault cost one call rather than the process, which is
//! what makes it safe to keep calling an application that just failed. It
//! does not make it useful: a tool that traps on every call turns each of the
//! model's attempts into an error, and the model will keep trying. So
//! something has to decide the application is broken and stop asking — and
//! say so, in the system space, where whoever installed it can see why.
//!
//! Two rules, because the two kinds of instance fail differently:
//!
//! - **Tool calls and events**: [`FAULT_LIMIT`] consecutive faults disable
//!   the application. Only faults count — a timeout or a cancellation says
//!   something about the work or the user, not about the component — and an
//!   error *result* is the component answering, so it resets the streak.
//! - **The backend**: a trap loses the instance and the state in it. It is
//!   rebuilt once; a second crash disables the application, because a backend
//!   that crashes again after a clean start is not going to be saved by a
//!   third. "Again" means within [`crate::HEALTHY_PERIOD`]: a backend that
//!   has run that long since its last crash has its count cleared, and its
//!   next crash is rebuilt like a first one.

use crate::World;
use serde::Serialize;

/// Consecutive tool or event faults before the application is set aside.
/// Three rather than one: a fault can come from an input the component
/// mishandles, and one bad argument should not cost the user the tool.
pub const FAULT_LIMIT: u32 = 3;

/// Where an application's VM is in its life.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum VmState {
    Running,
    /// [`crate::AppVm::stop`] was called.
    Stopped,
    /// Set aside after faulting; only starting a new VM brings it back.
    Disabled {
        reason: String,
    },
}

/// Counters, kept under one lock: they are updated together and read
/// together, and none of them is hot enough for that to matter.
#[derive(Debug, Default)]
pub(crate) struct Counters {
    pub calls: u64,
    pub faults: u64,
    pub timeouts: u64,
    pub consecutive_faults: u32,
    pub crashes: u32,
    pub rebuilds: u32,
    pub recycles: u32,
    pub dropped_deliveries: u64,
    pub last_error: Option<String>,
    /// When the backend last crashed: a crash older than the healthy period is forgiven.
    pub last_crash: Option<std::time::Instant>,
}

/// One application's VM, as the system space shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HealthSnapshot {
    pub app: String,
    pub world: World,
    #[serde(flatten)]
    pub state: VmState,
    /// Calls into the component: methods, tools, events, deliveries.
    pub calls: u64,
    /// Calls that trapped or could not be run.
    pub faults: u64,
    /// Calls abandoned at their deadline.
    pub timeouts: u64,
    pub consecutive_faults: u32,
    /// Times the backend was rebuilt after a crash (at most one).
    pub rebuilds: u32,
    /// Times the backend was replaced after a call was abandoned at its
    /// deadline. Its in-memory state went with it, and `init` ran again.
    pub recycles: u32,
    /// Topic values and timer firings dropped because the backend's queue
    /// was full.
    pub dropped_deliveries: u64,
    pub last_error: Option<String>,
    /// Linear memory held right now, across every instance.
    pub memory_bytes: u64,
    pub memory_peak_bytes: u64,
    pub memory_limit_bytes: u64,
    /// The key-value store and the files under `${data}`, together: what `storage_mb` bounds.
    pub storage_bytes: u64,
    /// The files' part of it, as last measured.
    pub storage_files_bytes: u64,
    pub storage_limit_bytes: u64,
    pub subscriptions: usize,
    pub timers: usize,
}
