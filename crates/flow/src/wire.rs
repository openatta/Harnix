//! The library's own rows on the wire, beside the kernel's types a host answers with
//! (`RunRecord`, `RunSummary`, `RunView`, `FlowDef`, …). In Harnix, the interface's declarations of
//! all of them are generated (`crates/harnix/tests/ts_export.rs`, `docs/testing.md` §10).

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::kernel::host::{Origin, Withheld};

/// One flow on offer: a row of `flow.defs` and of the `defs` topic.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct DefRow {
    pub name: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    pub digest: String,
    /// Where it comes from: built in, saved over a built-in one, saved here, or the project's own
    /// (listed for a project: `flow.defs {project}`).
    pub origin: Origin,
    /// Whether it validates against this application's catalog; one that does not cannot be
    /// started.
    pub valid: bool,
    /// Why it is not valid without being read, when that is so: `untrusted` — the project's
    /// own, waiting on the project being trusted (title, digest and inputs are not read until
    /// then); `unreadable` — its file is not a definition of this name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub reason: Option<Withheld>,
    /// What a run of it is started with, as one JSON Schema of the `inputs` object
    /// (`FlowDef::inputs_schema`): a form can be drawn from it.
    #[cfg_attr(feature = "ts", ts(type = "Record<string, unknown>"))]
    pub inputs: Value,
}
