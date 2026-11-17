//! The extension's rows and params on the wire: what `schedule.*` take and answer, and what the
//! topic `schedule.list` carries. The interface's declarations of them are generated
//! (`crates/harnix/tests/ts_export.rs`, `docs/testing.md` §10). Times are epoch milliseconds.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// One schedule a person or an extension made.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Schedule {
    pub id: String,
    pub title: String,
    /// Off: nothing is due until it is switched on again; what fell due meanwhile is not made up.
    pub enabled: bool,
    pub timing: Timing,
    /// The IANA zone every wall-clock time of `timing` is in.
    pub time_zone: String,
    pub target: Target,
    pub missed: Missed,
    /// What a task target does when its previous run is still going.
    pub concurrency: Concurrency,
    /// The space it belongs to: where its sessions are, and whose devices manage it.
    pub space: String,
    /// The project its target works in, when it has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub project: Option<String>,
    /// Who made it: `device:<id>`, `ext:<id>`.
    pub created_by: String,
    pub created_at: u64,
    /// Bumped by every change made to it; an update names the revision it read.
    pub revision: u64,
    /// When it is next due; absent while it is off or once nothing more is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub next_fire: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub last_delivery: Option<Receipt>,
}

/// When a schedule is due. Every wall-clock time is in the schedule's zone: a time a
/// daylight-saving change skips is not due that day, and one it repeats is due the first time.
/// `ScheduleTiming` in TypeScript, beside the readers' `Timing`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(rename = "ScheduleTiming"))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Timing {
    /// Once: RFC 3339 with an offset, or a wall-clock `YYYY-MM-DDTHH:MM[:SS]` in the zone.
    Once { at: String },
    /// Once, `seconds` after `from`.
    After {
        seconds: u64,
        /// When it was made or its timing last changed: set by the scheduler, never read from a
        /// call.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        from: Option<u64>,
    },
    /// Every `minutes` (at least 1) of elapsed time, counted from `from`.
    Every {
        minutes: u32,
        /// As `after`'s.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        from: Option<u64>,
    },
    /// Every day at `time` (`HH:MM`).
    Daily { time: String },
    /// On these ISO weekdays (Monday 1 … Sunday 7) at `time` (`HH:MM`).
    Weekly { weekdays: Vec<u8>, time: String },
    /// Five fields — minute, hour, day of month, month, day of week (0 or 7 is Sunday) — as Vixie
    /// cron reads them: `*`, numbers, ranges, lists, steps; no names, no macros.
    Cron { expr: String },
}

/// Where a delivery goes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Target {
    /// A turn in a session that exists: it waits behind a running turn, and a fire that comes
    /// while the last one still waits is merged into it.
    Session(SessionTarget),
    /// A turn in a new session.
    NewSession(NewSessionTarget),
    /// A task: a run of a flow in a project (`flow.start`), one per delivery.
    Task(TaskTarget),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct SessionTarget {
    pub instance: String,
    pub session_id: String,
    pub prompt: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct NewSessionTarget {
    #[serde(default)]
    pub mode: NewMode,
    /// The project it runs in: an existing absolute directory.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub project_root: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub scene: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub instance: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub model: Option<String>,
    pub prompt: String,
    /// The session `new_once_then_reuse` opened and turns to since: set by the scheduler as soon
    /// as it is opened, before the prompt is sent, and kept across restarts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub session: Option<SessionKey>,
}

/// Whether each delivery opens a session, or the first one does and the rest turn to it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum NewMode {
    NewEach,
    #[default]
    NewOnceThenReuse,
}

/// A session by its full name. `ScheduledSession` in TypeScript, beside the interface's own
/// `SessionKey`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(rename = "ScheduledSession"))]
pub struct SessionKey {
    pub instance: String,
    pub session_id: String,
}

/// `flow.start`'s params, but for the trigger, which is the schedule's.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct TaskTarget {
    pub flow: String,
    pub subject: String,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    #[cfg_attr(feature = "ts", ts(optional, type = "Record<string, unknown>"))]
    pub inputs: Value,
    /// An existing absolute directory.
    pub project_root: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub title: Option<String>,
    /// A path or a template, handed to the run as given.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub output_dir: Option<String>,
    /// `project` or `worktree`; absent: as the flow says.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub workspace: Option<String>,
    /// A space the run's main session is also shown in (`flow.start`'s `shown_in`): the
    /// one the interface that made the schedule lists the project's sessions in, so the task
    /// opens there. Absent: shown nowhere else.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub shown_in: Option<String>,
}

/// What happens to times that fell due while Harnix was not running (or were otherwise more
/// than a minute late).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum Missed {
    /// The latest of them is delivered, once.
    #[default]
    CatchUpLatest,
    /// None is: a receipt says it was skipped.
    Skip,
}

/// A task target whose previous run is still going.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum Concurrency {
    /// This time is skipped, and a receipt says so.
    #[default]
    Skip,
    /// Another run starts beside it (a worktree keeps them apart).
    Parallel,
}

/// One delivery, or one time that was not delivered. It says the target was handed the work,
/// not how the work went.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Receipt {
    pub id: String,
    /// When it was due; for «run now», when it was asked for.
    pub scheduled_at: u64,
    /// When it was handed over, or found it could not be.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub delivered_at: Option<u64>,
    /// Asked for by hand (`schedule.runNow`).
    #[serde(default)]
    pub manual: bool,
    pub outcome: Outcome,
    /// Skipped: `missed`, `min_interval`, `previous_run` or `merged` ([`SKIP_REASONS`]). Failed:
    /// what went wrong.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub target: Option<DeliveredTo>,
}

/// Why a time was skipped: it fell due while Harnix was not running and the schedule skips
/// those; it came within a minute of the last delivery; the task's previous run was still going;
/// the session still had the last delivery waiting in its queue.
pub const SKIP_REASONS: [&str; 4] = ["missed", "min_interval", "previous_run", "merged"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    /// Being delivered. One that Harnix stopped in the middle of is failed when it starts again,
    /// and not tried again.
    Pending,
    Delivered,
    Skipped,
    Failed,
}

/// Where a delivery went: a session (and the item it waits as, when queued), or a run.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct DeliveredTo {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub instance: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub session_id: Option<String>,
    /// The session was opened for this delivery.
    #[serde(default)]
    pub created: bool,
    /// The queued item it waits as, while the session is busy.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub item: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub run: Option<String>,
}

/// A job the engine keeps for one of its sessions — one a model made with `CronCreate`: five
/// fields in UTC, a turn in that session. Listed beside the schedules as made by the model; it can
/// be deleted here, not changed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ModelJob {
    pub instance: String,
    pub id: String,
    pub session_id: String,
    pub cron: String,
    pub prompt: String,
    /// Off: due once, then gone.
    pub recurring: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub created_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub next_fire: Option<u64>,
}

/// `schedule.list`'s answer and the topic `schedule.list`'s value.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ScheduleList {
    /// Soonest due first; those due no more after them.
    pub schedules: Vec<Schedule>,
    /// What models made (source «model»). The topic carries them as last read.
    pub model: Vec<ModelJob>,
    /// The instances whose jobs could not be read.
    #[serde(default)]
    pub unreachable: Vec<String>,
}

/// `schedule.list`'s params. `space` filters, as every `schedule.*` call's `space` is checked:
/// a call that claims a space reaches that space's schedules only.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(optional_fields))]
pub struct ListParams {
    #[serde(default)]
    pub space: Option<String>,
    /// The schedules whose target works in this project.
    #[serde(default)]
    pub project: Option<String>,
}

/// `schedule.get`, `delete`, `enable`, `disable`, `runNow`'s params.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct IdParams {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub space: Option<String>,
}

/// `schedule.create`'s params.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct CreateParams {
    /// Where it belongs; absent: the default space.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub space: Option<String>,
    pub title: String,
    pub timing: Timing,
    /// Absent: this machine's zone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub time_zone: Option<String>,
    pub target: Target,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub missed: Option<Missed>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub concurrency: Option<Concurrency>,
    /// Absent: on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub enabled: Option<bool>,
}

/// `schedule.update`'s params: what is given changes, the rest stays. `revision` is the one the
/// caller read; another means someone changed it since (`CONFLICT`, `data.revision` the current).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct UpdateParams {
    pub id: String,
    pub revision: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub space: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub timing: Option<Timing>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub time_zone: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub target: Option<Target>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub missed: Option<Missed>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub concurrency: Option<Concurrency>,
}

/// `schedule.history`'s params.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct HistoryParams {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub space: Option<String>,
    /// At most this many, newest first (default 50).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub limit: Option<usize>,
}

/// `schedule.history`'s answer: the latest 200 receipts of the last 30 days at most, newest first.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct History {
    pub receipts: Vec<Receipt>,
    /// How many are kept.
    pub total: usize,
}

/// `schedule.model.delete`'s params: a model's job, by its instance and id.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ModelDeleteParams {
    pub instance: String,
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub space: Option<String>,
}

/// `schedule.delete`'s and `schedule.model.delete`'s answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Deleted {
    pub deleted: bool,
}
