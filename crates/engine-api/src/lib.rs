//! The engine, in Harnix's words.
//!
//! What an extension gives the engine — tools, the providers that hand a session its tools, hooks
//! around a tool call, scenes as a layer over another scene, gates on a message, a source of
//! provider keys — and the readers it reads the engine's records through. Nothing here names a
//! type of Agix's: `harnix-engine` is the one crate that does, and it turns these into Agix's
//! own (`crates/harnix/tests/layering.rs` says so of the dependency graph).
//!
//! Every public structure and enum is `#[non_exhaustive]` and built by constructors and builders,
//! so a field or variant added later is an addition.

pub mod contribution;
pub mod credentials;
pub mod gate;
pub mod provider;
pub mod readers;
pub mod scene;
pub mod tool;

pub use contribution::{EngineContribution, ForeignHook};
pub use credentials::{CredentialSource, Secret};
pub use gate::{Delivery, PromptDecision, PromptFacts, PromptGate};
pub use provider::{HookDecision, SessionFacts, ToolCallFacts, ToolHook, ToolProvider};
pub use scene::{BaseKind, BuiltinArgs, Interaction, LayerParts, PromptText, SceneBase, SceneFacts, SceneLayer};
pub use tool::{Cancel, CancelGuard, Permission, Progress, Tool, ToolCx, ToolError, ToolOutput, ToolSpec};
pub use harnix_contract::{ChainLink, SessionRef};
