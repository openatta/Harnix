//! The vocabulary an extension is written against, and the skeleton decides by: who calls
//! ([`Subject`]), under which id ([`ExtId`]).
//!
//! Every public type here is `#[non_exhaustive]` and built through its constructors, so that
//! adding to it is never a breaking change for an extension.

mod id;
mod subject;

pub use id::{ExtId, IdError, RESERVED};
pub use subject::Subject;
