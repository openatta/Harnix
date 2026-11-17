//! The engine, in Harnix's words: what an extension gives it, and reads of it.
//!
//! What an extension gives the engine — tools, the providers that hand a session its tools, hooks
//! around a tool call, scenes as a layer over another scene, gates on a message, instance
//! templates — and the readers it reads the engine's records through. Nothing here names a type
//! of Agix's: `harnix-engine` is the one crate that does, and it offers these to the engine over
//! its protocol (`crates/harnix/tests/layering.rs` says so of the dependency graph).
//!
//! **Every public item here is the extension interface** (S1 in `docs/harnix-ext.md` §16), and
//! `harnix::engine` is this crate whole: every public structure and enum is `#[non_exhaustive]`
//! and built by constructors and builders, so a field or variant added later is an addition. How
//! the skeleton reaches an engine — its link, what it is asked, what it says unasked — is not
//! here but in `harnix-engine-link`, which builds on this crate and is not an extension's.

pub mod contribution;
pub mod gate;
pub mod provider;
pub mod readers;
pub mod scene;
pub mod template;
pub mod tool;

pub use contribution::EngineContribution;
pub use gate::{Delivery, PromptDecision, PromptFacts, PromptGate};
pub use provider::{HookDecision, ToolCallFacts, ToolHook, ToolProvider, ToolsChanged, ToolsWatch};
pub use scene::{BaseKind, BuiltinArgs, Interaction, LayerParts, PromptText, SceneBase, SceneLayer};
pub use template::{check_value, AttachTo, Contribute, EngineProfile, InstanceTemplate, TemplateCarrier};
pub use tool::{Cancel, CancelGuard, Permission, Progress, Tool, ToolCx, ToolError, ToolOutput, ToolSpec};
/// The directory a project keeps its Harnix data in, the engine's and Harnix's own:
/// `<project>/.harnix/` holds the engine's project tier (settings, agents, hooks, rules, memory,
/// its worktrees) beside what Harnix writes there (a flow's `runs/`, a project's `flows/`).
pub const PROJECT_DIR: &str = ".harnix";

/// A session's name and a call chain's link: the contract's words, in a tool call's context.
pub use harnix_contract::s1::{ChainLink, SessionRef};
