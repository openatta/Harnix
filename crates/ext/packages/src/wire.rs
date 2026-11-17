//! `packages.list`'s value, as structures: the one place its shape is stated. The interface's
//! declaration is generated from these (`@harnix/core/protocol`, by
//! `crates/harnix/tests/ts_export.rs`), so a field added here is a field every reader of the list
//! knows — the shell's inline points and frames, the packages pages — rather than one more copy.
//!
//! A package's status is what its carrier says of it, built up by the host, the carrier and this
//! component in turn. It is read into [`PackageRuntime`] here, at the edge: the fields the list
//! promises are named, and whatever else the carrier said rides along in `rest`, so nothing a
//! newer carrier adds is lost before the field has a name.

use std::collections::BTreeMap;

use harnix_package::inline::Inline;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// `packages.list`'s answer, and the topic of the same name's value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct PackagesList {
    /// Where the packages are installed.
    pub dir: String,
    pub packages: Vec<PackageRow>,
    /// Install records that did not read, with why.
    pub unreadable: Vec<Unreadable>,
    /// Every extension registered while the process runs, as the host lists it.
    #[cfg_attr(feature = "ts", ts(type = "unknown[]"))]
    pub registered: Vec<Value>,
    /// The carrier every package runs on.
    pub runtime: PackagesRuntime,
}

/// An install record that could not be read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Unreadable {
    pub dir: String,
    pub reason: String,
}

/// The carrier packages run on in this process.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct PackagesRuntime {
    /// `wasm` or `process`, as the profile names it.
    pub carrier: String,
    /// What a package's precompiled code must have been built for.
    pub fingerprint: String,
    /// Developer mode: unsigned packages install, and their interface is watched for changes.
    pub dev: bool,
    /// The sandbox each process starts in; null on a carrier that starts none.
    pub sandbox: Option<Sandbox>,
}

/// The sandbox the host applies to each extension process it starts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Sandbox {
    /// `on`; `off` (the profile's `[ext.packages] sandbox = "off"`, for development);
    /// `unavailable` (this machine has no mechanism, and `why` says).
    pub mode: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub mechanism: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub why: Option<String>,
}

/// One installed package.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct PackageRow {
    pub id: String,
    pub version: String,
    pub title: Option<String>,
    pub enabled: bool,
    /// Registered as an extension in this process now.
    pub registered: bool,
    /// `store`, `trusted` or `unsigned`.
    pub trust: String,
    /// Where it came from: `file`, or the store's name.
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub source: Value,
    /// What it keeps, in bytes: as its VM counts it while it runs, measured on disk when it does
    /// not; null when neither could say.
    pub data_bytes: Option<u64>,
    /// What its carrier says of it now.
    pub runtime: PackageRuntime,
    /// What its manifest describes; null while it is being unpacked or when it is broken.
    pub ext: Option<PackageDescribed>,
    /// On the process carrier: the mode of the sandbox its process starts in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub sandbox: Option<String>,
    /// Why the store withdrew this version, when it did.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub revoked: Option<String>,
    /// Why it is switched off when nobody switched it off.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub reason: Option<String>,
}

/// A package's status, as its carrier says it. The named fields are what the list promises;
/// `rest` is whatever else the carrier said.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct PackageRuntime {
    /// `running`, `hibernating` (its backend let go while idle), `faulted` (set aside after
    /// crashing), `stopped` or `failed` (did not start; `reason` says).
    pub state: String,
    #[serde(default)]
    pub reason: Option<String>,
    /// `wasm` or `process`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub carrier: Option<String>,
    /// `tools` or `backend`: which world its component implements.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub world: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub methods: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub topics: Option<Vec<String>>,
    /// The calls into it and the calls it made, counted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub calls: Option<CallStats>,
    /// Its VM, while it runs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub health: Option<PackageHealth>,
    /// On the process carrier: its process.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub process: Option<ProcessFacts>,
    /// The interface modules this process serves for it, by path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub ui: Option<Vec<String>>,
    /// Its surfaces, `<point>:<id>` each.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub surfaces: Option<Vec<String>>,
    /// Changes when an unsigned package's interface changes under it, in developer mode: the page
    /// reloads its frames. Null when its interface is not watched.
    #[serde(default)]
    pub ui_rev: Option<String>,
    /// What the shell draws for it inline, as its manifest declares.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub inline: Option<Inline>,
    /// Whatever else the carrier said of it.
    #[serde(flatten)]
    #[cfg_attr(feature = "ts", ts(skip))]
    pub rest: BTreeMap<String, Value>,
}

impl PackageRuntime {
    /// A carrier's status read into the list's shape. One that does not read is a status in
    /// itself — `failed`, with what did not read as the reason — never a row that is missing.
    pub fn read(status: Value) -> Self {
        match serde_json::from_value::<PackageRuntime>(status.clone()) {
            Ok(runtime) => runtime,
            Err(e) => {
                tracing::warn!(error = %e, status = %status, "a package's status did not read");
                PackageRuntime { state: "failed".into(), reason: Some(format!("the status did not read: {e}")), ..Default::default() }
            }
        }
    }
}

/// The calls into a package and the calls it made as itself, counted by the host.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct CallStats {
    #[serde(default)]
    pub calls: u64,
    #[serde(default)]
    pub errors: u64,
    /// Calls it made as itself.
    #[serde(default)]
    pub hub_calls: u64,
    /// The calls it made that the host refused, newest first.
    #[serde(default)]
    pub refused: Vec<Refusal>,
    /// Its methods' recent failures, newest first.
    #[serde(default)]
    pub recent_errors: Vec<RecentError>,
    #[serde(flatten)]
    #[cfg_attr(feature = "ts", ts(skip))]
    pub rest: BTreeMap<String, Value>,
}

/// A call a package made as itself that was refused.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Refusal {
    pub method: String,
    pub reason: String,
    pub at_ms: u64,
    #[serde(flatten)]
    #[cfg_attr(feature = "ts", ts(skip))]
    pub rest: BTreeMap<String, Value>,
}

/// One of a package's methods failing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct RecentError {
    pub method: String,
    pub message: String,
    #[serde(flatten)]
    #[cfg_attr(feature = "ts", ts(skip))]
    pub rest: BTreeMap<String, Value>,
}

/// A package's VM, as the host reports it while the package runs: what the list promises of it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct PackageHealth {
    /// `tools` or `backend`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub world: Option<String>,
    /// Its backend: `running`, `stopped` or `disabled`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub state: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub calls: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub faults: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub timeouts: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub rebuilds: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub recycles: Option<u64>,
    #[serde(default)]
    pub last_error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub memory_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub memory_limit_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub storage_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub storage_limit_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub subscriptions: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub timers: Option<u64>,
    /// An on-demand backend stopped while idle; the next call starts it again.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub hibernating: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub wakes: Option<u64>,
    /// The rest of what the host counts.
    #[serde(flatten)]
    #[cfg_attr(feature = "ts", ts(skip))]
    pub rest: BTreeMap<String, Value>,
}

/// A package's process, on the process carrier.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ProcessFacts {
    #[serde(default)]
    pub pid: Option<u32>,
    #[serde(default)]
    pub starts: u32,
    /// The last lines it wrote to its stderr.
    #[serde(default)]
    pub stderr: Vec<String>,
    #[serde(flatten)]
    #[cfg_attr(feature = "ts", ts(skip))]
    pub rest: BTreeMap<String, Value>,
}

/// What a package's manifest describes, as it is disclosed before installing.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct PackageDescribed {
    pub publisher: String,
    #[serde(default)]
    pub description: Option<String>,
    /// Its disclosure, one line per capability it asks for.
    #[serde(default)]
    pub lines: Vec<CapabilityLine>,
    /// Its interface modules: which point each is for, and the module.
    #[serde(default)]
    pub ui: Vec<UiModuleDecl>,
    /// What the shell draws for it inline.
    #[serde(default)]
    pub inline: Option<Inline>,
    /// The rest of the manifest's account of itself: its capabilities, the events it hears, its
    /// component.
    #[serde(flatten)]
    #[cfg_attr(feature = "ts", ts(skip))]
    pub rest: BTreeMap<String, Value>,
}

/// One line of a package's disclosure: a capability, and what it asks of it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct CapabilityLine {
    pub capability: String,
    #[cfg_attr(feature = "ts", ts(type = "string | number | boolean"))]
    pub value: Value,
    #[serde(flatten)]
    #[cfg_attr(feature = "ts", ts(skip))]
    pub rest: BTreeMap<String, Value>,
}

/// An interface module a package's manifest declares.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct UiModuleDecl {
    pub point: String,
    pub module: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// What the carrier said comes through whole: the named fields typed, the rest kept.
    #[test]
    fn a_status_reads_and_writes_back_as_it_was() {
        let said = json!({
            "state": "running", "reason": null, "resident": true, "world": "backend",
            "health": {"world": "backend", "state": "running", "calls": 3, "memory_peak_bytes": 10},
            "calls": {"calls": 2, "errors": 0, "methods": [{"name": "m", "calls": 2, "errors": 0}], "hub_calls": 1, "refused": [], "recent_errors": []},
            "ui_rev": null, "inline": {},
        });
        // Every field the list promises is written, so `last_error` appears where the carrier said nothing.
        let mut expected = said.clone();
        expected["health"]["last_error"] = Value::Null;
        let runtime = PackageRuntime::read(said.clone());
        assert_eq!(runtime.state, "running");
        assert_eq!(runtime.health.as_ref().and_then(|h| h.calls), Some(3));
        assert_eq!(runtime.ui_rev, None);
        assert_eq!(serde_json::to_value(&runtime).unwrap(), expected);
    }

    #[test]
    fn a_status_that_does_not_read_is_a_failed_one() {
        let runtime = PackageRuntime::read(json!({"state": 7}));
        assert_eq!(runtime.state, "failed");
        assert!(runtime.reason.unwrap().contains("did not read"));
    }
}
