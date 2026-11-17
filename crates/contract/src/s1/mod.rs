//! The vocabulary an extension is written against, and the skeleton decides by: who calls
//! ([`Subject`]), under which id ([`ExtId`]), with which privileges ([`Capability`], and the
//! kernel's catalogue in [`caps`]), trusted how far ([`Trust`]), and how far a row is open to it
//! ([`Access`], [`RowScope`]); whose engine an instance runs ([`InstanceTrust`]); a call's context
//! as it crosses a process ([`CallMeta`]); a session's name ([`SessionRef`]) and the chain a turn
//! runs on behalf of ([`ChainLink`]); a space as an extension declares it ([`SpaceDecl`]); what a
//! failed call answers ([`RpcError`], [`codes`]). The extension interface names the contract's
//! types through this module alone (`crates/harnix/tests/layering.rs`).
//!
//! Every public type here is `#[non_exhaustive]` or has only private fields and is built through
//! its constructors, so that adding to it is never a breaking change for an extension.

mod access;
mod capability;
mod error;
mod id;
mod instance;
mod meta;
pub mod product;
mod session;
mod space;
mod subject;
mod trust;

pub use access::{Access, RowScope};
pub use capability::{caps, Capability, CapabilitySpec};
pub use error::{codes, RpcError};
pub use id::{ExtId, IdError, RESERVED};
pub use instance::InstanceTrust;
pub use meta::CallMeta;
pub use session::{ChainLink, SessionRef};
pub use space::{DefaultInstance, SpaceDecl};
pub use subject::Subject;
pub use trust::{PackageCarrier, Trust};

/// The constants an extension is written against: the default instance's name, the largest frame
/// a connection carries, the contribution point API's number.
pub use crate::{CONTRIB_API_VERSION, MAIN_INSTANCE, MAX_FRAME_BYTES};
