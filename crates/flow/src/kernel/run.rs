//! The run record: a snapshot the kernel computes and the host stores, plus an
//! append-only history written in the same commit.
//!
//! The JSON shape is serde's output; [`run_schema`] is its JSON Schema.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::def::FlowDef;

/// Whoever did something. The kernel records it and never interprets it.
#[derive(
    Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
pub struct Actor {
    pub kind: String,
    pub id: String,
}

impl Actor {
    pub fn new(kind: impl Into<String>, id: impl Into<String>) -> Self {
        Self {
            kind: kind.into(),
            id: id.into(),
        }
    }

    /// The kernel itself, under a role name: `dispatch` (auto-claims), `auto` (on_unmet
    /// rework), `clock` (lease expiry), `gate`, `evaluate`.
    pub fn flow(role: &str) -> Self {
        Self::new("flow", role)
    }
}

impl std::fmt::Display for Actor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.kind, self.id)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Active,
    Finished,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum StationStatus {
    /// Not entered yet: some of its needs have not finished.
    Pending,
    /// Entered with `hold`; waits for a release.
    Held,
    /// Tasks running, or waiting on a gate, a confirmation or children.
    Working,
    /// Judged and some exit condition failed.
    Unmet,
    Passed,
    /// `when` did not hold. Counts as finished for whoever needs it.
    Skipped,
    /// Swept by a rework; re-entered once its needs have finished again.
    Stale,
}

impl StationStatus {
    /// Finished, as far as downstream `needs` are concerned.
    pub fn is_done(self) -> bool {
        matches!(self, Self::Passed | Self::Skipped)
    }

    /// Entered in the current round and not finished.
    pub fn is_open(self) -> bool {
        matches!(self, Self::Held | Self::Working | Self::Unmet)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Ready,
    Claimed,
    Submitted,
    Failed,
    Cancelled,
}

/// A claim on a task: who, the fencing token, and until when.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Lease {
    pub by: Actor,
    pub token: String,
    /// Epoch milliseconds. Subflow tasks hold a lease that never expires.
    pub until: u64,
}

/// One task of a station.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TaskRec {
    pub id: String,
    /// The `foreach` item this task works on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item: Option<Value>,
    pub status: TaskStatus,
    /// Claims made in the current series (a manual retry starts a new one).
    #[serde(default)]
    pub attempt: u32,
    /// The current claim. Kept after a submit: its token allows a resubmission.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lease: Option<Lease>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// The session working on it, as the module reported (`attach_session`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    /// The child run of a subflow task.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub child: Option<String>,
}

impl TaskRec {
    pub fn new(id: impl Into<String>, item: Option<Value>) -> Self {
        Self {
            id: id.into(),
            item,
            status: TaskStatus::Ready,
            attempt: 0,
            lease: None,
            output: None,
            error: None,
            session: None,
            child: None,
        }
    }

    /// Ready or claimed: still to be done.
    pub fn in_flight(&self) -> bool {
        matches!(self.status, TaskStatus::Ready | TaskStatus::Claimed)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CheckStatus {
    Pass,
    Fail,
    /// Not decided yet: a gate running, a confirmation or children outstanding.
    Wait,
}

/// The latest result of one exit condition (or of the tasks, `cond: None`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CheckRec {
    /// Index into the station's `exit`; `None` for the station's tasks themselves.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cond: Option<usize>,
    pub kind: String,
    pub label: String,
    pub round: u32,
    pub status: CheckStatus,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub detail: String,
    pub at: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ConfirmRec {
    pub by: Actor,
    pub at: u64,
    pub round: u32,
    pub pass: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// A station passed by force: who, why, and what was not met at the time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ForcedRec {
    pub by: Actor,
    pub at: u64,
    pub reason: String,
    pub unmet: Vec<CheckRec>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct StationRec {
    pub status: StationStatus,
    /// The current round: bumped on every entry (a skip included). 0 = never entered.
    #[serde(default)]
    pub round: u32,
    /// Rounds that ran (skips excluded); the rework limit is checked against it.
    #[serde(default)]
    pub rounds_total: u32,
    /// Automatic reworks `on_unmet` has done from this station, over all rounds.
    #[serde(default)]
    pub auto_reworks: u32,
    /// Skill name → content digest, pinned on entry.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub skill_digest: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tasks: Vec<TaskRec>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub checks: Vec<CheckRec>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub confirms: Vec<ConfirmRec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub forced: Option<ForcedRec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entered_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub left_at: Option<u64>,
}

impl StationRec {
    pub fn new() -> Self {
        Self {
            status: StationStatus::Pending,
            round: 0,
            rounds_total: 0,
            auto_reworks: 0,
            skill_digest: BTreeMap::new(),
            tasks: vec![],
            checks: vec![],
            confirms: vec![],
            forced: None,
            entered_at: None,
            left_at: None,
        }
    }

    pub fn task(&self, id: &str) -> Option<&TaskRec> {
        self.tasks.iter().find(|t| t.id == id)
    }

    pub fn task_mut(&mut self, id: &str) -> Option<&mut TaskRec> {
        self.tasks.iter_mut().find(|t| t.id == id)
    }

    /// No task is ready or claimed.
    pub fn tasks_settled(&self) -> bool {
        !self.tasks.iter().any(TaskRec::in_flight)
    }
}

impl Default for StationRec {
    fn default() -> Self {
        Self::new()
    }
}

/// The task of a parent run a child run serves.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ParentRef {
    pub run: String,
    pub station: String,
    pub task: String,
}

/// One execution of a flow on one subject.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct RunRecord {
    pub id: String,
    pub flow: String,
    /// Digest of the definition text the run was started from.
    pub def_digest: String,
    /// The definition, pinned at creation: a changed pack does not affect a running run.
    pub def: FlowDef,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub title: String,
    pub subject: String,
    #[serde(default)]
    pub inputs: Value,
    pub status: RunStatus,
    /// Bumped by every commit; the store compares it.
    pub rev: u64,
    /// The last history entry's `seq`.
    #[serde(default)]
    pub seq: u64,
    pub created_at: u64,
    pub created_by: Actor,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<ParentRef>,
    pub stations: BTreeMap<String, StationRec>,
    /// Session key (`run` for `per_run`, the resolved key otherwise) → session id.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub session_keys: BTreeMap<String, String>,
}

impl RunRecord {
    pub fn station(&self, id: &str) -> Option<&StationRec> {
        self.stations.get(id)
    }

    pub fn is_active(&self) -> bool {
        self.status == RunStatus::Active
    }

    /// Stations entered and not finished, in definition order.
    pub fn open_stations(&self) -> Vec<String> {
        self.def
            .stations
            .iter()
            .filter(|s| self.stations.get(&s.id).is_some_and(|r| r.status.is_open()))
            .map(|s| s.id.clone())
            .collect()
    }

    /// The earliest expiry among claimed tasks' leases.
    pub fn next_expiry(&self) -> Option<u64> {
        self.stations
            .values()
            .flat_map(|s| &s.tasks)
            .filter(|t| t.status == TaskStatus::Claimed)
            .filter_map(|t| t.lease.as_ref().map(|l| l.until))
            .min()
    }

    pub fn summary(&self) -> RunSummary {
        RunSummary {
            id: self.id.clone(),
            flow: self.flow.clone(),
            title: self.title.clone(),
            subject: self.subject.clone(),
            status: self.status,
            rev: self.rev,
            created_at: self.created_at,
            finished_at: self.finished_at,
            parent: self.parent.as_ref().map(|p| p.run.clone()),
            open: self.open_stations(),
            next_expiry: self.next_expiry(),
        }
    }
}

/// One line of a run's history. Appended, never changed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct History {
    pub run: String,
    /// 1, 2, … within the run.
    pub seq: u64,
    pub at: u64,
    pub by: Actor,
    pub op: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub station: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task: Option<String>,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub detail: Value,
}

/// A run for a list.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct RunSummary {
    pub id: String,
    pub flow: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub title: String,
    pub subject: String,
    pub status: RunStatus,
    pub rev: u64,
    pub created_at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    /// Stations entered and not finished.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub open: Vec<String>,
    /// The earliest lease expiry; `tick` only loads runs where it has passed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_expiry: Option<u64>,
}

/// Which runs to list. Every field set must match.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct RunQuery {
    pub flow: Option<String>,
    pub status: Option<RunStatus>,
    pub subject: Option<String>,
    pub parent: Option<String>,
    /// The newest `limit` runs.
    pub limit: Option<usize>,
}

impl RunQuery {
    pub fn matches(&self, s: &RunSummary) -> bool {
        self.flow.as_ref().is_none_or(|f| *f == s.flow)
            && self.status.is_none_or(|st| st == s.status)
            && self.subject.as_ref().is_none_or(|x| *x == s.subject)
            && self
                .parent
                .as_ref()
                .is_none_or(|p| s.parent.as_ref() == Some(p))
    }

    /// Filter, order newest first, and cut to `limit` — what a store returns.
    pub fn apply(&self, mut all: Vec<RunSummary>) -> Vec<RunSummary> {
        all.retain(|s| self.matches(s));
        all.sort_by(|a, b| b.created_at.cmp(&a.created_at).then(b.id.cmp(&a.id)));
        if let Some(n) = self.limit {
            all.truncate(n);
        }
        all
    }
}

/// A run as a reader sees it: the record (with its pinned definition) and, per open
/// station, what is not met yet.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct RunView {
    pub record: RunRecord,
    /// Station id → its exit conditions not passed (failed or waiting).
    pub unmet: BTreeMap<String, Vec<CheckRec>>,
}

impl RunView {
    pub fn def(&self) -> &FlowDef {
        &self.record.def
    }
}

/// The JSON Schema of a run record.
pub fn run_schema() -> Value {
    serde_json::to_value(schemars::schema_for!(RunRecord)).unwrap_or(Value::Null)
}
