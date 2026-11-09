//! Whether an application is still worth calling, and what to report about it.
//!
//! Isolation makes a fault cost one call rather than the process, which is
//! what makes it safe to keep calling an application that just failed. It
//! does not make it useful: a tool that traps on every call turns each of the
//! model's attempts into an error, and the model will keep trying. So
//! something has to decide the application is broken and stop asking — and
//! say so, in the system space, where whoever installed it can see why.
//!
//! Three rules, because the three kinds of call fail differently and cost
//! different things:
//!
//! - **A tool**: [`FAULT_LIMIT`] consecutive faults of one tool set *that tool*
//!   aside; the application's other tools, its backend and its interface go on.
//!   A tool's input comes from the model, which retries — and from whatever was
//!   injected into the conversation — so a fault there must not be able to take
//!   down more than the tool it happened in. Only faults count: a timeout or a
//!   cancellation says something about the work or the user, not about the
//!   component, and an error *result* is the component answering, so it resets
//!   the tool's streak.
//! - **An event**: not counted towards anything. A hook that fails lets the
//!   call through already, so it has nothing to stop retrying.
//! - **The backend**: a trap loses the instance and the state in it. It is
//!   rebuilt once; a second crash disables the application, because a backend
//!   that crashes again after a clean start is not going to be saved by a
//!   third. "Again" means within [`crate::HEALTHY_PERIOD`]: a backend that
//!   has run that long since its last crash has its count cleared, and its
//!   next crash is rebuilt like a first one.

use crate::World;
use serde::Serialize;

/// Consecutive faults of one tool before that tool is set aside.
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
    /// Each tool's current run of faults.
    pub streaks: std::collections::HashMap<String, u32>,
    /// The tools set aside, and why.
    pub aside: std::collections::BTreeMap<String, String>,
    pub crashes: u32,
    pub rebuilds: u32,
    pub recycles: u32,
    pub storage_recycles: u32,
    pub queue_timeouts: u64,
    pub wakes: u32,
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
    /// The longest run of faults any one tool is on now.
    pub consecutive_faults: u32,
    /// Tools set aside after [`FAULT_LIMIT`] consecutive faults, and why. The rest of the
    /// application runs on; only starting a new VM brings them back.
    pub tools_aside: std::collections::BTreeMap<String, String>,
    /// Times the backend was rebuilt after a crash (at most one).
    pub rebuilds: u32,
    /// Times the backend was replaced after a call was abandoned at its
    /// deadline. Its in-memory state went with it, and `init` ran again.
    pub recycles: u32,
    /// Times the backend was replaced because its files under `/data` crossed its
    /// `storage_mb`: over it, so that what it writes from then on is refused; or back under it,
    /// so that it may write again. Its in-memory state went with it, as in a recycle.
    pub storage_recycles: u32,
    /// Method calls answered `busy` because the backend did not come free within the timeout.
    pub queue_timeouts: u64,
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
    /// An on-demand backend let go while idle (`AppVm::hibernate`); its tools still run.
    pub hibernating: bool,
    /// Times a hibernating backend was started again by a call into it. Not recycles: nothing
    /// was abandoned, and nothing it kept in its storage was lost.
    pub wakes: u32,
}
