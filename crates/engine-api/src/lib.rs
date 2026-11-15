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
//!
//! # The kernel's part
//!
//! [`link`], [`ops`] and [`events`] are not an extension's: they are how the hub reaches an
//! instance's engine — a link it is handed once, what it asks of the engine, and what the engine
//! says unasked — in Harnix's words, so the hub depends on this crate and not on the engine.

pub mod chain;
pub mod contribution;
pub mod credentials;
pub mod events;
pub mod gate;
pub mod link;
pub mod ops;
pub mod provider;
pub mod readers;
pub mod scene;
pub mod template;
pub mod tool;

pub use chain::{ChainSource, NoChains};
pub use contribution::EngineContribution;
pub use credentials::Secret;
pub use events::{Completion, DaemonEventKind, EngineEvent, Fragment, Joins, Prompt, SessionEventKind, Waiting};
pub use link::{EngineControl, EngineIdentity, EngineLink, EventReceiver, InstanceCarrier, LinkStatus, EVENT_CAPACITY};
pub use ops::{Answer, EngineOps, Entry, Said, SessionCreated, SessionInfo, Subscribed, Tasks, TurnInput, HUB_OWNED_OPTIONS};
pub use gate::{Delivery, PromptDecision, PromptFacts, PromptGate};
pub use provider::{HookDecision, ToolCallFacts, ToolHook, ToolProvider, ToolsChanged, ToolsWatch};
pub use scene::{BaseKind, BuiltinArgs, Interaction, LayerParts, PromptText, SceneBase, SceneFacts, SceneLayer};
pub use template::{AttachTo, EngineProfile, InstanceTemplate, TemplateCarrier};
pub use tool::{Cancel, CancelGuard, Permission, Progress, Tool, ToolCx, ToolError, ToolOutput, ToolSpec};
pub use harnix_contract::{ChainLink, SessionRef};
