//! Flows: a run moves one subject through a graph of stations. Each station's
//! tasks are done by a session working from a skill, by the application's own
//! code, by someone outside, or by a child run; whether a station passes is
//! judged on structured signals only — outputs checked against a contract, gate
//! results, application tool results, confirmations. A station that does not
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
//! kernel/protocol.rs  the protocol layer put before a session task's skill
//! store.rs            MemStore, FileStore
//! packs.rs            MemPacks, DirPacks
//! ```

pub mod kernel;
pub mod packs;
pub mod store;

// module: the Harnix module (sessions, programs, native tools, methods, topics) goes here.

pub use kernel::def::{
    definition_schema, Assert, Executor, ExitCond, FlowDef, HookRef, InputDef, OnUnmet, Retry,
    SessionBinding, Station, TaskSpec, Work, DEFAULT_REWORK_LIMIT, MAX_REWORK_LIMIT,
};
pub use kernel::engine::{view, CreateIn, Engine, DEFAULT_LEASE_MS, DEFAULT_RETRIES};
pub use kernel::host::{
    digest, AllowAll, Authorizer, Catalog, ChangeSet, Clock, Event, FixedClock, GateResult, Gates,
    Hooks, ManualClock, NoTools, Op, Outcome, Packs, Programs, RunStore, StoreError, SystemClock,
    Tools, WorkItem, EVENT_KINDS,
};
pub use kernel::protocol::skill_body;
pub use kernel::run::{
    run_schema, Actor, CheckRec, CheckStatus, ConfirmRec, ForcedRec, History, Lease, ParentRef,
    RunQuery, RunRecord, RunStatus, RunSummary, RunView, StationRec, StationStatus, TaskRec,
    TaskStatus,
};
pub use kernel::validate::{validate, validate_yaml, Problem};
pub use kernel::{FlowError, Result};
pub use packs::{DirPacks, MemPacks};
pub use store::{FileStore, MemStore};
