//! The skeleton's own interfaces: what the transport, the router, the authorizer and the hub say
//! to each other, and nothing they say to anyone else.
//!
//! - **A call** ([`CallContext`]): who makes it, on which connection, its id and the call it was
//!   made while handling, the extensions waiting on it, how it is cancelled. Typed, and carried
//!   beside the params rather than inside them.
//! - **What answers it** ([`Handler`]: one row's; [`Dispatch`]: the whole table's).
//! - **Who owns a name** ([`Owner`]) and **who may reach it** ([`Access`]).
//! - **The transport's two questions**: where a client is registered ([`ClientRegistry`]), and
//!   whether a subject may reach an extension's routes ([`RouteDecider`]).
//!
//! It depends on the wire vocabulary and nothing else of Harnix: every crate of the skeleton may
//! name it, so none of them has to name another to be called.

mod access;
mod call;
mod clients;
mod owner;

pub use access::Access;
pub use call::{CallContext, CallId, Chain, ChainLink, Deadline, Dispatch, Handler, Rpc};
pub use clients::{ClientId, ClientRegistry};
pub use owner::{KernelPart, Owner};

use harnix_contract::{RpcError, Subject};

/// Whether `subject` may reach the routes an extension serves under its id. The transport asks
/// this and nothing else: what a route row is, and who decides it, are not its business.
pub trait RouteDecider: Send + Sync {
    fn decide_route(&self, subject: &Subject, ext: &str, path: &str) -> Result<(), RpcError>;
}
