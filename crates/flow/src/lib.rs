//! Flows: a run moves one subject through a graph of stations. Each station's
//! tasks are done by a session working from a skill, by the application's own
//! code, by someone outside, or by a child run; whether a station passes is
//! judged on structured signals only — outputs checked against a contract, gates,
//! confirmations. While a task works, each tool call it makes is judged against its
//! permissions; rules are read by the sessions and never enforced. A station that does not
//! pass can be reworked: it and everything downstream of it start again.
//!
//! An optional crate: an application that does not register it gets nothing
//! different. The flow does not know where runs are stored, who the users
//! are, who may do what, or how a gate runs — the application hands those in
//! as the traits in [`kernel::host`].
//!
//! ```text
//! kernel/def.rs       the definition (YAML) and its JSON Schema
//! kernel/validate.rs  structural validation: every problem at once
//! kernel/refs.rs      ${...} references and assertions
//! kernel/run.rs       the run record (snapshot + history) and its JSON Schema
//! kernel/host.rs      what the host implements; events, work items, outcomes
//! kernel/engine.rs    every operation: load → compute → commit against revs
//! kernel/parts.rs     rules, policies and permissions, gates, hooks; judging a call
//! kernel/protocol.rs  the protocol layer put before a session task's skill
//! kernel/schema.rs    a minimal value for a JSON Schema (dry runs)
//! store.rs            MemStore, FileStore
//! packs.rs            MemPacks, DirPacks (both can be writable)
//! dryrun.rs           dry runs: the kernel in a sandbox, everything outside it mocked
//! module/             the Harnix module (feature `module`, on by default): Flow, FlowHandle
//! ```

pub mod dryrun;
pub mod kernel;
#[cfg(feature = "module")]
pub mod module;
pub mod packs;
pub mod store;

pub use dryrun::{Mocks, Sandbox};
pub use kernel::def::{
    definition_schema, Assert, Executor, ExitCond, FlowDef, InputDef, MainSession,
    OnUnmet, Retry, SessionBinding, Station, TaskSpec, Work, DEFAULT_REWORK_LIMIT,
    MAIN_SESSION_KEY, MAX_REWORK_LIMIT,
};
pub use kernel::engine::{
    view, CreateIn, Engine, DEFAULT_LEASE_MS, DEFAULT_RETRIES, MAX_LEASE_MS, MAX_SUBFLOW_DEPTH,
};
pub use kernel::host::{
    digest, AllowAll, Authorizer, Catalog, ChangeSet, Clock, Delivery, Event, FixedClock,
    GateResult, Gates, ManualClock, NoTools, Op, Origin, Outcome, Packs, Placement,
    Programs, RunStore, StoreError, SystemClock, Tools, WorkItem, EVENT_KINDS,
};
pub use kernel::parts::{
    glob, judge, part_schema, permissions_of, rules_of, Call, Effect, GateDef, GateKind, Granted,
    HookDef, HookKind, PartKind, Permission, Pinned, PolicyDef, Verdict,
};
pub use kernel::protocol::{agent_tag, ask_rules, skill_body, STRUCTURED_OUTPUT_TOOL};
pub use kernel::run::{
    run_schema, Actor, CheckRec, CheckStatus, ConfirmRec, ForcedRec, History, Lease, ParentRef,
    AskRec, PauseRec, Phase, RunQuery, RunRecord, RunStatus, RunSummary, RunView, StationRec,
    StationStatus, TaskRec, TaskStatus, Usage, AGENT_KIND, FLOW_KIND,
};
pub use kernel::schema::minimal;
pub use kernel::validate::{part_problems, validate, validate_yaml, Problem};
pub use kernel::{FlowError, Result};
#[cfg(feature = "module")]
pub use module::{builtin_scenes, Flow, FlowBuilder, FlowHandle};
pub use packs::{DirPacks, MemPacks};
pub use store::{FileStore, MemStore};
