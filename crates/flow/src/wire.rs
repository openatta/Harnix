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
    /// Where it comes from: built in, saved over a built-in or plugin's one, saved here, the
    /// project's own (listed for a project: `flow.defs {project}`), or a plugin's.
    pub origin: Origin,
    /// The plugin it is of (`Packs::source`): for a plugin's, and for an override of one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub source: Option<String>,
    /// Whether it validates against this application's catalog; one that does not cannot be
    /// started.
    pub valid: bool,
    /// Why it is not valid without being read, when that is so: `untrusted` — the project's
    /// own, waiting on the project being trusted (title, digest and inputs are not read until
    /// then); `unreadable` — its file is not a definition of this name. (`shadowed` is not on a
    /// row: the name is listed once, as the flow offered under it.)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub reason: Option<Withheld>,
    /// What a run of it is started with, as one JSON Schema of the `inputs` object
    /// (`FlowDef::inputs_schema`): a form can be drawn from it.
    #[cfg_attr(feature = "ts", ts(type = "Record<string, unknown>"))]
    pub inputs: Value,
}

/// The flows on offer, as a host's listing carries them; and, for a project, the project's own
/// that are withheld (not valid, with the reason), in name order with the rest. One row a name: a
/// withheld flow whose name is offered all the same (`Withheld::Shadowed`) has none of its own.
pub fn defs_list(packs: &dyn crate::kernel::host::Packs) -> Vec<DefRow> {
    let mut rows: Vec<DefRow> = packs
        .flows()
        .into_iter()
        .filter_map(|n| {
            let (def, digest) = packs.flow(&n)?;
            let valid = crate::kernel::validate::validate(&def, packs).is_empty();
            Some(DefRow {
                origin: packs.origin(&n),
                source: packs.source(&n),
                inputs: def.inputs_schema(),
                name: n,
                title: def.title,
                description: def.description,
                digest,
                valid,
                reason: None,
            })
        })
        .collect();
    let offered: std::collections::BTreeSet<String> = rows.iter().map(|r| r.name.clone()).collect();
    let withheld: Vec<(String, Withheld)> =
        packs.withheld_flows().into_iter().filter(|(n, _)| !offered.contains(n)).collect();
    if withheld.is_empty() {
        return rows;
    }
    rows.extend(withheld.into_iter().map(|(name, why)| DefRow {
        origin: packs.origin(&name),
        source: packs.source(&name),
        title: String::new(),
        description: String::new(),
        digest: String::new(),
        valid: false,
        reason: Some(why),
        inputs: serde_json::json!({"type": "object"}),
        name,
    }));
    rows.sort_by(|a, b| a.name.cmp(&b.name));
    rows
}
