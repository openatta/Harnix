//! The module's own params on the wire, beside the library's rows (`DefRow`) and the kernel's
//! types it answers with (`RunRecord`, `RunSummary`, `RunView`, `FlowDef`, …). The interface's
//! declarations of all of them are generated (`crates/harnix/tests/ts_export.rs`, `docs/testing.md` §10).

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::kernel::run::Workspace;

pub use harnix_flow::wire::DefRow;

/// `flow.start`'s params.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(optional_fields))]
pub struct StartParams {
    pub flow: String,
    pub subject: String,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    #[cfg_attr(feature = "ts", ts(type = "Record<string, unknown>"))]
    pub inputs: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// The project the run works in — a task: an existing absolute directory.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_root: Option<String>,
    /// Where it works; absent: the project when there is one, else a scratch directory.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<Workspace>,
    /// Where its deliverables go: a template relative to the project (`${flow}`, `${date}`,
    /// `${seq}`, `${title}`, `${inputs.<k>}`), or an absolute path inside it; absent: the flow's,
    /// else `flows/${flow}-${date}${seq}`. Made at start; the run holds it resolved.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_dir: Option<String>,
    /// How it came to be started; absent: by the caller's hand. Only the kernel and extensions
    /// may say a schedule did.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trigger: Option<StartTrigger>,
    /// A space the run's main session is shown in besides the flow's own — the one the
    /// interface that starts a task lists the project's sessions in — to be looked at and talked
    /// to there (`harnix.session.expose`). Absent: shown nowhere else. `show_in_space`, its
    /// earlier name, is read too.
    #[serde(default, alias = "show_in_space", skip_serializing_if = "Option::is_none")]
    pub shown_in: Option<String>,
    /// The conversation it was started from (`/flow` in a conversation): a card linking to the
    /// run is written there (an entry `task_started`), and the conversation stays what it was.
    /// Absent: written nowhere.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_session: Option<StartedFrom>,
}

/// A conversation a start came from (`StartParams::from_session`), by its full name and the
/// space it belongs to: the card is written claiming that space, so it goes only into a
/// conversation of it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(optional_fields))]
pub struct StartedFrom {
    /// Absent: the main instance.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instance: Option<String>,
    pub session_id: String,
    pub space: String,
}

/// How a start says it came about. Who started it by hand is the caller, never what it claims.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StartTrigger {
    Manual {},
    /// A schedule fired: which, and the time it was due (epoch ms).
    Schedule { schedule: String, fire_at: u64 },
}
