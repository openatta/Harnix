//! The skeleton's own interfaces: what the transport, the router, the authorizer and the hub say
//! to each other, and nothing they say to anyone else.
//!
//! - **A call** ([`CallContext`]): who makes it, on which connection, its id and the call it was
//!   made while handling, the extensions waiting on it, how it is cancelled. Typed, and carried
//!   beside the params rather than inside them.
//! - **What answers it** ([`Handler`]: one row's; [`Dispatch`]: the whole table's).
//! - **Who owns a name** ([`Owner`]), **who may reach it** ([`Access`]), and **what each
//!   extension holds** ([`GrantTable`], read as a [`GrantBook`]).
//! - **The transport's two questions**: where a client is registered ([`ClientRegistry`]), and
//!   whether a subject may reach an extension's routes ([`RouteDecider`]).
//! - **A computed topic** ([`TopicSource`]), and where its owner says it changed ([`TopicChanged`]).
//! - **The instance seam** ([`InstanceObserver`], [`InstanceBook`]): what the instance registry
//!   tells the hub and what the hub asks it, and the lock order between them
//!   ([`instances::holding`]).
//! - **A turn's chain** ([`TurnChains`]): which calls a running turn is on behalf of, written by
//!   the hub and read by the tools its session calls.
//! - **An extension's seat** ([`seat::Seat`]): the kernel's services an extension's context
//!   reaches the process through, handed to its declarations by the extension host
//!   ([`seat::Seated`]).
//!
//! It depends on the wire vocabulary and nothing else of Harnix: every crate of the skeleton may
//! name it, so none of them has to name another to be called.

mod call;
mod clients;
mod grants;
pub mod instances;
mod owner;
pub mod seat;
mod topics;
mod turns;

pub use grants::{holds, GrantBook, GrantTable};
/// How far a row is open, in the extension interface's own words.
pub use harnix_contract::s1::{Access, RowScope};
pub use call::{CallContext, CallId, Chain, ChainLink, Deadline, Dispatch, Handler, Rpc};
pub use clients::{ClientId, ClientRegistry};
pub use instances::{InstanceBook, InstanceObserver, InstanceTrust, Listed};
pub use owner::{KernelPart, Owner};
pub use topics::{TopicChanged, TopicSource};
pub use turns::{TurnChains, TurnWatch};

use harnix_contract::{RpcError, Subject};

/// The span the host opens around everything one extension is asked to do — a call, a topic, its
/// start and stop — with the extension's id in its `extension` field: what is logged inside it is
/// that extension's log (`harnix-ext.md` §17.9).
pub const EXTENSION_SPAN: &str = "harnix.extension";

/// Whether `subject` may reach the routes an extension serves under its id. The transport asks
/// this and nothing else: what a route row is, and who decides it, are not its business.
pub trait RouteDecider: Send + Sync {
    fn decide_route(&self, subject: &Subject, ext: &str, path: &str) -> Result<(), RpcError>;
}
