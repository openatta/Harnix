//! The vocabulary an extension is written against, and the skeleton decides by: who calls
//! ([`Subject`]), under which id ([`ExtId`]), with which privileges ([`Capability`], and the
//! kernel's catalogue in [`caps`]), trusted how far ([`Trust`]), and how far a row is open to it
//! ([`Access`], [`RowScope`]); whose engine an instance runs ([`InstanceTrust`]); a call's context
//! as it crosses a process ([`CallMeta`]).
//!
//! Every public type here is `#[non_exhaustive]` or has only private fields and is built through
//! its constructors, so that adding to it is never a breaking change for an extension.

mod access;
mod capability;
mod id;
mod instance;
mod meta;
mod subject;
mod trust;

pub use access::{Access, RowScope};
pub use capability::{caps, Capability, CapabilitySpec};
pub use id::{ExtId, IdError, RESERVED};
pub use instance::InstanceTrust;
pub use meta::CallMeta;
pub use subject::Subject;
pub use trust::{PackageCarrier, Trust};
