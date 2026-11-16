//! The engine as the skeleton reaches it: a link, what it is asked, what it says unasked.
//!
//! The hub is handed an instance's engine once, as an [`EngineLink`]: what it asks of the engine
//! ([`EngineOps`], one method each, so the hub spells none of the engine's names), the one stream
//! of what the engine says unasked ([`EngineEvent`]), and the link's health ([`EngineControl`]).
//! Beside the link: the scene layers of an instance, expanded for a session to be set up from
//! ([`SceneLayers`]); where a tool call learns the chain its turn runs on behalf of
//! ([`ChainSource`]); and both readers of one instance ([`Readers`]). Nothing here names a type
//! of Agix's: `harnix-engine` is the one crate that does, and implements these over its protocol.
//!
//! **Not an extension's.** This is the skeleton's half of the engine vocabulary (S3 in
//! `docs/harnix-ext.md` §16): the hub, the instance layer, the engine crate, the assembly and the
//! extension process depend on it, and it changes when the skeleton does. The extension's half
//! is `harnix-engine-api` (S1), which this crate builds on and which never names this one
//! (`crates/harnix/tests/layering.rs`).

pub mod chain;
pub mod events;
pub mod link;
pub mod ops;
pub mod readers;
pub mod scene;

pub use chain::{ChainSource, NoChains};
pub use events::{Completion, DaemonEventKind, EngineEvent, FileWrite, Fragment, Joins, Prompt, SessionEventKind, Waiting};
pub use link::{EngineControl, EngineIdentity, EngineLink, EventReceiver, InstanceCarrier, LinkStatus, EVENT_CAPACITY};
pub use ops::{Answer, EngineOps, Entry, Said, SessionCreated, SessionInfo, SessionSetup, Subscribed, Tasks, TurnInput, HUB_OWNED_OPTIONS};
pub use readers::Readers;
pub use scene::{ExpandedScene, SceneLayers};
