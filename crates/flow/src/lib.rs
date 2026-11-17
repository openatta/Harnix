//! Flows: a run moves one subject through a graph of stations. Each station's
//! tasks are done by a session working from a skill, by the application's own
//! code, by someone outside, or by a child run; whether a station passes is
//! judged on structured signals only — outputs checked against a contract, gates,
//! confirmations. While a task works, each tool call it makes is judged against its
//! permissions; rules are read by the sessions and never enforced. A station that does not
//! pass can be reworked: it and everything downstream of it start again.
//!
//! The library links nothing of Harnix or Agix (`tests/deps.rs`): it does not know where runs
//! are stored, who the users are, who may do what, or how a gate runs — the host hands those in
//! as the traits in [`kernel::host`]. Harnix's host is the extension `harnix-ext-flow`; how
//! another one takes the library in is `docs/embed.en.md`.
//!
//! ```text
//! kernel/def.rs       the definition (YAML) and its JSON Schema
//! kernel/validate.rs  structural validation: every problem at once
//! kernel/refs.rs      ${...} references and assertions
//! kernel/output.rs    the output directory's template
//! kernel/run.rs       the run record (snapshot + history) and its JSON Schema
//! kernel/host.rs      what the host implements; events, work items, outcomes
//! kernel/layout.rs    where the host keeps what the library writes for a project
//! kernel/engine.rs    every operation: load → compute → commit against revs
//! kernel/parts.rs     rules, policies and permissions, gates, hooks; judging a call
//! kernel/protocol.rs  the protocol layer put before a session task's skill
//! kernel/schema.rs    a minimal value for a JSON Schema (dry runs)
//! store.rs            MemStore, FileStore
//! packs.rs            MemPacks, DirPacks (both can be writable)
//! dryrun.rs           dry runs: the kernel in a sandbox, everything outside it mocked
//! wire.rs             the library's own rows (`DefRow`)
//! docs/guide.*.md     the flow guide: what a flow is and how one is written
//! ```

/// The library's Rust interface — the traits a host implements, what they hand it, the names
/// offered here, how the stores and packs are built (`tests/surface.rs`). It goes up by one with
/// a change that breaks a host; additions leave it.
pub const FLOW_API_VERSION: u32 = 1;

/// The format a flow definition and its parts are read in. Not yet written into a definition: a
/// file without one is this format.
pub const DEFINITION_FORMAT: u32 = 1;

/// The format of the run records, their histories and the run ids a store keeps. Not yet written
/// into a record or a store: one without it is this format.
pub const RUN_FORMAT: u32 = 1;

pub mod dryrun;
pub mod kernel;
pub mod packs;
pub mod store;
pub mod wire;

pub use dryrun::{Mocks, Sandbox};
pub use kernel::def::{
    definition_schema, Assert, Executor, ExitCond, FlowDef, Guard, InputDef, MainSession,
    OnUnmet, Retry, SessionBinding, Station, TaskSpec, Work, DEFAULT_OUTPUT_DIR, DEFAULT_REWORK_LIMIT,
    MAIN_SESSION_KEY, MAX_REWORK_LIMIT,
};
pub use kernel::engine::{
    view, worktree_may_end, CreateIn, Engine, Prepared, DEFAULT_LEASE_MS, DEFAULT_RETRIES, MAX_LEASE_MS, MAX_SUBFLOW_DEPTH,
};
pub use kernel::host::{
    digest, AllowAll, Authorizer, Catalog, ChangeSet, Clock, Delivery, Event, FixedClock,
    GateResult, Gates, ManualClock, NoTools, Op, Origin, Outcome, Packs, Placement,
    Programs, RunStore, StoreError, SystemClock, Tools, Withheld, WorkItem, EVENT_KINDS,
};
pub use kernel::layout::ProjectLayout;
pub use kernel::parts::{
    glob, judge, judge_under, part_schema, permissions_of, rules_of, Call, Effect, GateDef, GateKind, Granted,
    HookDef, HookKind, PartKind, Permission, Pinned, PolicyDef, Verdict,
};
pub use kernel::protocol::{agent_tag, ask_rules, skill_body, STRUCTURED_OUTPUT_TOOL};
pub use kernel::run::{
    run_schema, Actor, CheckRec, CheckStatus, ConfirmRec, ForcedRec, History, Lease, ParentRef,
    AskRec, PauseRec, Phase, RunQuery, RunRecord, RunStatus, RunSummary, RunView, StationRec,
    StationStatus, TaskRec, TaskStatus, Trigger, Usage, Workspace, WorktreeEnd, WorktreeOutcome,
    WorktreeRec, AGENT_KIND, DEVICE_KIND, FLOW_KIND,
};
pub use kernel::schema::minimal;
pub use kernel::validate::{part_problems, validate, validate_yaml, Problem};
pub use kernel::{FlowError, Result};
pub use packs::{project_pack_dir, DirPacks, MemPacks, ProjectPacks, ProjectTrust};
pub use store::{FileStore, Hooked, MemStore};

/// The flow guide (`docs/guide.*.md`): what a flow is and how one is written, for the people who
/// write flows and for the session that helps them.
pub const GUIDE_EN: &str = include_str!("../docs/guide.en.md");
pub const GUIDE_ZH: &str = include_str!("../docs/guide.zh-CN.md");
