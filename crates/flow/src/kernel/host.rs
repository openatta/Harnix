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
///
/// `scenes`, `steps`, `gates`, `tools` and `actions` are always checked: a name a definition uses
/// must be here. `templates`, `instances`, `spaces` and `models` are checked only when the
/// application filled them in — an empty set says "not known here", not "none".
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Catalog {
    #[serde(default)]
    pub scenes: BTreeSet<String>,
    /// Program steps (`Programs`).
    #[serde(default)]
    pub steps: BTreeSet<String>,
    #[serde(default)]
    pub gates: BTreeSet<String>,
    /// Tools a `tool` exit may call (`Tools`).
    #[serde(default)]
    pub tools: BTreeSet<String>,
    /// Hook actions (`Hooks`).
    #[serde(default)]
    pub actions: BTreeSet<String>,
    /// Templates a session station may name (full names, `<module>.<template>`).
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub templates: BTreeSet<String>,
    /// Instances a session station may name.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub instances: BTreeSet<String>,
    /// Spaces a session station may name.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub spaces: BTreeSet<String>,
    /// Models a session station may name: `model`, or `provider/model` to tie it to a provider.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub models: BTreeSet<String>,
    /// The tools the host refuses as writes unless a station's `writes` open them (names, or
    /// `ns__*` groups). Filled by the host that enforces them; validation reports a write that
    /// opens part of a group, and stations sharing a session that would be refused different
    /// tools.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub guarded: Vec<String>,
    /// Scenes whose sessions hand in with `flow_submit` (the module's `flow-*`): their output
    /// contract is checked by the flow, not kept by the session, so tasks sharing such a
    /// session may have different contracts.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub submit_scenes: BTreeSet<String>,
    /// The scene a run's main session is opened on when the flow's `main` names none
    /// (xops-03 §1). Absent: a flow with tasks in the main session must name one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub main_scene: Option<String>,
}

/// Where the flow a pack offers under a name comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Origin {
    /// Shipped with the application; read-only.
    Builtin,
    /// A built-in flow saved again in the writable layer: deleting it brings the built-in back.
    Override,
    /// Only in the writable layer.
    User,
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
    /// The text a flow was given as, for people reading it. `None` when the packs cannot say.
    fn flow_text(&self, _name: &str) -> Option<String> {
        None
    }
    /// Names of the skills on offer, for authors. Empty when the packs cannot list them.
    fn skills(&self) -> Vec<String> {
        vec![]
    }

    // ── writing (xops-02) ────────────────────────────────────────────────────
    //
    // The packs store what they are given once it parses; whether it is a *valid* flow is
    // judged against the whole catalog by whoever saves — `Engine::save_flow` validates first
    // and saves nothing that has problems. Runs already started keep the definition they pinned.

    /// Whether flows and skills can be saved here.
    fn writable(&self) -> bool {
        false
    }
    /// Where the flow offered as `flow` comes from.
    fn origin(&self, _flow: &str) -> Origin {
        Origin::Builtin
    }
    /// Save a definition under the name it declares; the digest of the saved text.
    fn save_flow(&self, _yaml: &str) -> Result<String, String> {
        Err("read-only".into())
    }
    /// Delete a saved definition: a user's flow goes, an override gives way to the built-in.
    fn delete_flow(&self, _name: &str) -> Result<(), String> {
        Err("read-only".into())
    }
    /// Save a skill's body; its digest.
    fn save_skill(&self, _name: &str, _body: &str) -> Result<String, String> {
        Err("read-only".into())
    }
    fn delete_skill(&self, _name: &str) -> Result<(), String> {
        Err("read-only".into())
    }
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
    /// `executor`: the task's executor kind (`session`, `program`, `external`, `subflow`).
    Submit {
        station: String,
        task: String,
        executor: String,
        who: Option<String>,
    },
    Fail {
        station: String,
        task: String,
        executor: String,
    },
    Heartbeat {
        station: String,
        task: String,
        executor: String,
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
    /// Pausing a run, or resuming it (xops-03 §3).
    Pause,
    Resume,
    AttachSession {
        station: String,
        task: String,
        session: String,
    },
    /// Saving or deleting a flow definition (no run: `run` is `None`).
    EditFlow {
        flow: String,
    },
    /// Saving or deleting a skill (no run: `run` is `None`).
    EditSkill {
        skill: String,
    },
}

impl Op {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Create { .. } => "create",
            Self::Pause => "pause",
            Self::Resume => "resume",
            Self::Claim { .. } => "claim",
            Self::Submit { .. } => "submit",
            Self::Fail { .. } => "fail",
            Self::Heartbeat { .. } => "heartbeat",
            Self::Confirm { .. } => "confirm",
            Self::Force { .. } => "force",
            Self::Rework { .. } => "rework",
            Self::Release { .. } => "release",
            Self::Retry { .. } => "retry",
            Self::Cancel => "cancel",
            Self::AttachSession { .. } => "attach_session",
            Self::EditFlow { .. } => "edit_flow",
            Self::EditSkill { .. } => "edit_skill",
        }
    }
}

/// Decides who may do what. The one place authorization happens, whatever the entry point.
pub trait Authorizer: Send + Sync {
    /// `run` is `None` only for `create`, `edit_flow` and `edit_skill`. `Err` carries the reason
    /// shown to the caller.
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
///
/// **They must be read-only and idempotent.** A `tool` exit is called again whenever its
/// station is judged — every `evaluate`, every turn a session task ends, every resubmission —
/// and again when an operation is recomputed after a store conflict, whose first result is
/// thrown away. A tool that changes something would change it any number of times.
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
// A task is the common item and the big one; boxing it would only move the allocation.
#[allow(clippy::large_enum_variant)]
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
        /// Where a session task's session goes and which model it talks to, references
        /// resolved (`None` for program tasks).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        placement: Option<Placement>,
        /// How a session task hands its output in.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        delivery: Option<Delivery>,
        /// The scene a session task's session is opened on: the station's, or the main
        /// session's for a task in the main session (`None` for program tasks, and when the
        /// module has no main scene).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        scene: Option<String>,
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

/// A session task's placement, as the station gave it with `params` resolved for the run.
/// Everything absent falls back to the module's space and that space's instance and model.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Placement {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instance: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template: Option<String>,
    /// The template's parameters, resolved.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub params: std::collections::BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub space: Option<String>,
}

impl Placement {
    /// The instance a template's session goes to: the `name` parameter, else
    /// [`default_instance`]`(run, template)`.
    pub fn template_instance(&self, run: &str) -> Option<String> {
        let template = self.template.as_ref()?;
        Some(
            self.params
                .get("name")
                .cloned()
                .filter(|n| !n.is_empty())
                .unwrap_or_else(|| default_instance(run, template)),
        )
    }

    /// Whether the instance is named by the module ([`default_instance`]), not by the station:
    /// such an instance is the run's, and stopped when the run is over.
    pub fn names_its_instance(&self) -> bool {
        self.template.is_some() && self.params.get("name").is_none_or(|n| n.is_empty())
    }
}

/// The name of the instance the module makes from `template` for `run` when the station names
/// none: `flow-<run, at most 40 characters>-<8 hex of the run and template>` — at most 54
/// characters (instance names are capped at 64), one per run and template.
pub fn default_instance(run: &str, template: &str) -> String {
    let safe: String = run
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '-'
            }
        })
        .take(40)
        .collect();
    let hash = digest(&format!("{run}\u{0}{template}"));
    format!("flow-{}-{}", safe.trim_end_matches('-'), &hash[..8])
}

/// How a session task hands in its output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Delivery {
    /// The session's `output_schema` is the station's contract; the model calls
    /// `StructuredOutput`, and each turn's structured output is submitted. The default.
    StructuredOutput,
    /// A wrapped `flow-*` scene: the model calls the native tool `flow_submit`.
    FlowSubmit,
}

/// The result of one operation: produced only after its commit succeeded.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Outcome {
    pub value: Value,
    pub events: Vec<Event>,
    pub work: Vec<WorkItem>,
}
