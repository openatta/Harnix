//! What the host provides, and what the kernel hands back.
//!
//! The kernel neither stores nor runs anything. It reads and commits through
//! [`RunStore`], reads definitions and skills through [`Packs`], asks
//! [`Authorizer`] before every change made on someone's behalf, and calls
//! [`Tools`] to judge `tool` exits. [`Gates`], [`Programs`] and [`Hooks`] are
//! not called by the kernel: the module calls them when it carries out the
//! [`WorkItem`]s an operation returns.

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicU64, Ordering};

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::def::{Executor, FlowDef};
use super::run::{Actor, History, RunQuery, RunRecord, RunSummary};

// ── storage ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoreError {
    /// A record's stored rev is not the expected one (or a new record already exists).
    Conflict(String),
    Other(String),
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Conflict(m) => write!(f, "conflict: {m}"),
            Self::Other(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for StoreError {}

/// What one operation changes, committed atomically.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ChangeSet {
    /// Each record with the rev the store must hold now; `None` = must not exist yet.
    /// The record's own `rev` is the new one.
    pub records: Vec<(RunRecord, Option<u64>)>,
    /// Appended to the runs' histories in the same commit.
    pub history: Vec<History>,
}

/// Where runs live. `commit` must be atomic and compare revs; everything else is reads.
#[async_trait]
pub trait RunStore: Send + Sync {
    /// A fresh run id for `flow`, conventionally `<flow>-<n>`.
    async fn alloc_id(&self, flow: &str) -> Result<String, StoreError>;
    async fn load(&self, run: &str) -> Result<Option<RunRecord>, StoreError>;
    /// Matching runs, newest first ([`RunQuery::apply`] does the filtering for a store
    /// that has the summaries at hand).
    async fn list(&self, q: &RunQuery) -> Result<Vec<RunSummary>, StoreError>;
    /// All or nothing; any rev mismatch is [`StoreError::Conflict`] and writes nothing.
    async fn commit(&self, cs: &ChangeSet) -> Result<(), StoreError>;
    /// A run's history in `seq` order.
    async fn history(&self, run: &str) -> Result<Vec<History>, StoreError>;
}

// ── packs ────────────────────────────────────────────────────────────────────

/// Names the application registered, for validation and for authors.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Catalog {
    pub scenes: BTreeSet<String>,
    /// Program steps (`Programs`).
    pub steps: BTreeSet<String>,
    pub gates: BTreeSet<String>,
    /// Tools a `tool` exit may call (`Tools`).
    pub tools: BTreeSet<String>,
    /// Hook actions (`Hooks`).
    pub actions: BTreeSet<String>,
}

/// Where definitions and skills come from.
pub trait Packs: Send + Sync {
    /// Names of the flows on offer.
    fn flows(&self) -> Vec<String>;
    /// A definition and the digest of its text.
    fn flow(&self, name: &str) -> Option<(FlowDef, String)>;
    /// A skill's body and its digest.
    fn skill(&self, name: &str) -> Option<(String, String)>;
    fn catalog(&self) -> Catalog;
}

/// Hex SHA-256 of a text: the digest pinned for definitions and skills.
pub fn digest(text: &str) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(text.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

// ── authorization ────────────────────────────────────────────────────────────

/// A change about to be made on someone's behalf. `who` carries the definition's opaque
/// label (an external executor's or a confirm exit's) for the authorizer to interpret.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Op {
    Create {
        flow: String,
        subject: String,
    },
    Claim {
        station: String,
        task: String,
        who: Option<String>,
    },
    Submit {
        station: String,
        task: String,
        who: Option<String>,
    },
    Fail {
        station: String,
        task: String,
    },
    Confirm {
        station: String,
        pass: bool,
        who: Option<String>,
    },
    Force {
        station: String,
    },
    Rework {
        to: String,
        tasks: Option<Vec<String>>,
    },
    Release {
        station: String,
    },
    Retry {
        station: String,
        task: String,
    },
    Cancel,
    AttachSession {
        station: String,
        task: String,
        session: String,
    },
}

impl Op {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Create { .. } => "create",
            Self::Claim { .. } => "claim",
            Self::Submit { .. } => "submit",
            Self::Fail { .. } => "fail",
            Self::Confirm { .. } => "confirm",
            Self::Force { .. } => "force",
            Self::Rework { .. } => "rework",
            Self::Release { .. } => "release",
            Self::Retry { .. } => "retry",
            Self::Cancel => "cancel",
            Self::AttachSession { .. } => "attach_session",
        }
    }
}

/// Decides who may do what. The one place authorization happens, whatever the entry point.
pub trait Authorizer: Send + Sync {
    /// `run` is `None` only for `create`. `Err` carries the reason shown to the caller.
    fn check(&self, op: &Op, by: &Actor, run: Option<&RunRecord>) -> Result<(), String>;
}

/// Everyone may do everything.
pub struct AllowAll;

impl Authorizer for AllowAll {
    fn check(&self, _: &Op, _: &Actor, _: Option<&RunRecord>) -> Result<(), String> {
        Ok(())
    }
}

// ── judgement and execution ─────────────────────────────────────────────────

/// Read-only application tools a `tool` exit calls.
#[async_trait]
pub trait Tools: Send + Sync {
    async fn call(&self, tool: &str, args: &Value) -> Result<Value, String>;
}

/// No tools: every `tool` exit fails.
pub struct NoTools;

#[async_trait]
impl Tools for NoTools {
    async fn call(&self, tool: &str, _: &Value) -> Result<Value, String> {
        Err(format!("no tool {tool}"))
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct GateResult {
    pub pass: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub detail: String,
}

/// Runs a gate (called by the module for [`WorkItem::Gate`]; the result goes to
/// `Engine::gate_result`).
#[async_trait]
pub trait Gates: Send + Sync {
    async fn run(&self, run: &RunRecord, station: &str, gate: &str) -> GateResult;
}

/// Runs a program step (called by the module for a program [`WorkItem::Task`]; the
/// result goes to `Engine::submit` or `Engine::fail`).
#[async_trait]
pub trait Programs: Send + Sync {
    async fn run(&self, step: &str, run: &RunRecord, item: Option<&Value>)
        -> Result<Value, String>;
}

/// Carries out a hook action (called by the module for [`WorkItem::Hook`]). A failure
/// is logged and blocks nothing.
#[async_trait]
pub trait Hooks: Send + Sync {
    async fn fire(&self, action: &str, event: &Event);
}

// ── time ─────────────────────────────────────────────────────────────────────

pub trait Clock: Send + Sync {
    /// Epoch milliseconds.
    fn now_ms(&self) -> u64;
}

pub struct SystemClock;

impl Clock for SystemClock {
    fn now_ms(&self) -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as u64)
    }
}

/// Always the same instant.
pub struct FixedClock(pub u64);

impl Clock for FixedClock {
    fn now_ms(&self) -> u64 {
        self.0
    }
}

/// A clock a test moves by hand.
#[derive(Default)]
pub struct ManualClock(AtomicU64);

impl ManualClock {
    pub fn new(at: u64) -> Self {
        Self(AtomicU64::new(at))
    }

    pub fn set(&self, at: u64) {
        self.0.store(at, Ordering::SeqCst);
    }

    pub fn advance(&self, ms: u64) {
        self.0.fetch_add(ms, Ordering::SeqCst);
    }
}

impl Clock for ManualClock {
    fn now_ms(&self) -> u64 {
        self.0.load(Ordering::SeqCst)
    }
}

// ── what an operation returns ───────────────────────────────────────────────

/// Something that happened, after it was committed.
///
/// Kinds: `run.{created, finished, cancelled}`,
/// `station.{entered, held, released, passed, unmet, forced, skipped, stale, reworked, confirmed}`,
/// `task.{ready, claimed, submitted, failed, cancelled, expired}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Event {
    pub kind: String,
    pub run: String,
    pub at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub station: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task: Option<String>,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub data: Value,
}

/// Every event kind, for validating `hooks[].on`.
pub const EVENT_KINDS: &[&str] = &[
    "run.created",
    "run.finished",
    "run.cancelled",
    "station.entered",
    "station.held",
    "station.released",
    "station.passed",
    "station.unmet",
    "station.forced",
    "station.skipped",
    "station.stale",
    "station.reworked",
    "station.confirmed",
    "task.ready",
    "task.claimed",
    "task.submitted",
    "task.failed",
    "task.cancelled",
    "task.expired",
];

/// Work for the module to carry out after the commit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WorkItem {
    /// A session or program task, already claimed by `flow:dispatch` under `token`.
    Task {
        run: String,
        station: String,
        round: u32,
        task: String,
        token: String,
        attempt: u32,
        executor: Executor,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        skill: Option<String>,
        /// The skill digest pinned on station entry.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        skill_digest: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        item: Option<Value>,
        /// The station's output contract (for the session's `output_schema`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        output_schema: Option<Value>,
        /// The protocol layer to put before the skill body (session tasks).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        protocol: Option<String>,
        /// Tasks with the same key share a session (`per_run` → `run`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session_key: Option<String>,
        /// The session already bound to `session_key`, if any.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session: Option<String>,
    },
    /// Run a gate; report through `gate_result(run, station, round, gate, …)`.
    Gate {
        run: String,
        station: String,
        round: u32,
        gate: String,
    },
    /// A definition hook matched an event.
    Hook { action: String, event: Event },
    /// A session served a task that was cancelled.
    CloseSession { session: String },
}

/// The result of one operation: produced only after its commit succeeded.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Outcome {
    pub value: Value,
    pub events: Vec<Event>,
    pub work: Vec<WorkItem>,
}
