//! The module's own rows and params on the wire, beside the kernel's types it answers with
//! (`RunRecord`, `RunSummary`, `RunView`, `FlowDef`, …). The interface's declarations of all of
//! them are generated (`crates/harnix/tests/ts_export.rs`, `docs/testing.md` §10).

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::kernel::host::Origin;
use crate::kernel::run::Workspace;

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

/// One flow on offer: a row of `flow.defs` and of the `defs` topic.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct DefRow {
    pub name: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    pub digest: String,
    /// Where it comes from: built in, saved over a built-in one, or saved here.
    pub origin: Origin,
    /// Whether it validates against this application's catalog; one that does not cannot be
    /// started.
    pub valid: bool,
    /// What a run of it is started with, as one JSON Schema of the `inputs` object
    /// (`FlowDef::inputs_schema`): a form can be drawn from it.
    #[cfg_attr(feature = "ts", ts(type = "Record<string, unknown>"))]
    pub inputs: Value,
}
