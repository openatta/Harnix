//! One call: who makes it and on what, typed, beside its params.
//!
//! What a handler used to learn from reserved keys in its params — which connection a watching
//! method is for, which extensions were waiting on the call — it reads here. Nothing on the wire
//! can set any of it: the transport makes a device's context, and everything else makes its own.

use std::sync::atomic::{AtomicU64, Ordering};

use harnix_contract::{RpcError, Subject};
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use crate::ClientId;

/// What every handler answers.
pub type Rpc<T> = Result<T, RpcError>;

/// A call's id, issued by this process: unique within it, never taken from a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CallId(u64);

impl CallId {
    /// The next id. Ids only grow, so a later call's is the larger.
    pub fn next() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Self(NEXT.fetch_add(1, Ordering::Relaxed))
    }

    pub fn get(self) -> u64 {
        self.0
    }
}

impl std::fmt::Display for CallId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// When a call stops being worth answering: milliseconds since the epoch, by the wall clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Deadline(pub u64);

/// One extension waiting on a call, and the call it is held in, when that is known.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChainLink {
    pub ext: String,
    pub call: Option<CallId>,
}

/// The extensions waiting on a call, outermost first: what a host that runs one call per
/// extension at a time refuses re-entry by.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Chain(Vec<ChainLink>);

impl Chain {
    /// A chain of extensions alone, with no calls known: what arrives from a host that keeps
    /// only who is waiting.
    pub fn of_extensions<S: Into<String>>(exts: impl IntoIterator<Item = S>) -> Self {
        Self(exts.into_iter().map(|ext| ChainLink { ext: ext.into(), call: None }).collect())
    }

    pub fn links(&self) -> &[ChainLink] {
        &self.0
    }

    /// The extensions, outermost first.
    pub fn extensions(&self) -> impl Iterator<Item = &str> + '_ {
        self.0.iter().map(|link| link.ext.as_str())
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    fn with(&self, link: ChainLink) -> Self {
        let mut links = self.0.clone();
        links.push(link);
        Self(links)
    }
}

/// One call, as every layer it passes through sees it.
#[derive(Debug, Clone)]
pub struct CallContext {
    pub subject: Subject,
    /// The connection a device's call came on: what a watching method registers. `None` for a
    /// call that came on none.
    pub client: Option<ClientId>,
    /// The extension whose page made this device's call, through the shell's bridge.
    pub on_behalf_of: Option<String>,
    pub call_id: CallId,
    /// The call this one was made while handling.
    pub parent: Option<CallId>,
    pub chain: Chain,
    pub cancel: CancellationToken,
    pub deadline: Option<Deadline>,
}

impl CallContext {
    /// A call nothing else was handling when it was made.
    pub fn root(subject: Subject, client: Option<ClientId>) -> Self {
        Self {
            subject,
            client,
            on_behalf_of: None,
            call_id: CallId::next(),
            parent: None,
            chain: Chain::default(),
            cancel: CancellationToken::new(),
            deadline: None,
        }
    }

    /// A call the extension `by` makes, as itself, while handling this one: a new id with this
    /// one as its parent, cancelled with it, by the same deadline. When `by` answers one call at
    /// a time (`serialized`), it is waiting on the new call, held in this one, and the chain says
    /// so. No connection: an extension cannot stand in for one.
    pub fn child(&self, by: &str, serialized: bool) -> Self {
        let chain = if serialized {
            self.chain.with(ChainLink { ext: by.to_string(), call: Some(self.call_id) })
        } else {
            self.chain.clone()
        };
        Self {
            subject: Subject::Module { id: by.to_string() },
            client: None,
            on_behalf_of: None,
            call_id: CallId::next(),
            parent: Some(self.call_id),
            chain,
            cancel: self.cancel.child_token(),
            deadline: self.deadline,
        }
    }

    /// The same call, with the extensions already waiting on it.
    pub fn chained(mut self, chain: Chain) -> Self {
        self.chain = chain;
        self
    }
}

/// What answers one row.
#[async_trait::async_trait]
pub trait Handler: Send + Sync {
    async fn call(&self, cx: CallContext, params: Value) -> Rpc<Value>;
}

/// What answers any name: the route table, deciding first.
#[async_trait::async_trait]
pub trait Dispatch: Send + Sync {
    async fn call(&self, cx: CallContext, method: &str, params: Value) -> Rpc<Value>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_child_is_its_parents_with_a_new_id() {
        let root = CallContext::root(Subject::Device { id: "d".into() }, Some(ClientId(3)));
        let child = root.child("system", false);
        assert_eq!(child.parent, Some(root.call_id));
        assert!(child.call_id > root.call_id);
        assert_eq!(child.subject, Subject::Module { id: "system".into() });
        assert_eq!(child.client, None, "an extension does not stand in for a connection");
        assert!(child.chain.is_empty());

        let held = root.child("acme.notes", true).child("acme.todo", true);
        assert_eq!(held.chain.extensions().collect::<Vec<_>>(), ["acme.notes", "acme.todo"]);
        assert_eq!(held.chain.links()[0].call, Some(root.call_id));

        root.cancel.cancel();
        assert!(child.cancel.is_cancelled(), "cancelled with its parent");
    }
}
