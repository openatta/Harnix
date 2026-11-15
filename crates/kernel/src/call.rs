//! One call: who makes it and on what, typed, beside its params.
//!
//! What a handler used to learn from reserved keys in its params — which connection a watching
//! method is for, which extensions were waiting on the call — it reads here. Nothing on the wire
//! can set any of it: the transport makes a device's context, and everything else makes its own.

use std::sync::atomic::{AtomicU64, Ordering};

use harnix_contract::s1::CallMeta;
pub use harnix_contract::ChainLink;
use harnix_contract::{ExtId, RpcError, Subject};
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

/// The extensions waiting on a call, outermost first, each with the call it is held on: what a
/// host that runs one call per extension at a time refuses re-entry by — only while that call is
/// still running in it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Chain(Vec<ChainLink>);

impl Chain {
    pub fn links(&self) -> &[ChainLink] {
        &self.0
    }

    /// The extensions, outermost first.
    pub fn extensions(&self) -> impl Iterator<Item = &str> + '_ {
        self.0.iter().map(|link| link.ext())
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Both chains' links, each once: a turn that more than one call fed waits on all of them.
    pub fn union(&self, other: &Chain) -> Self {
        let mut links = self.0.clone();
        for link in &other.0 {
            if !links.contains(link) {
                links.push(link.clone());
            }
        }
        Self(links)
    }

    fn with(&self, link: ChainLink) -> Self {
        let mut links = self.0.clone();
        links.push(link);
        Self(links)
    }
}

impl From<Vec<ChainLink>> for Chain {
    fn from(links: Vec<ChainLink>) -> Self {
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
    pub on_behalf_of: Option<ExtId>,
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
    pub fn child(&self, by: &ExtId, serialized: bool) -> Self {
        let chain = if serialized {
            self.chain.with(ChainLink::new(by.as_str(), self.call_id.to_string()))
        } else {
            self.chain.clone()
        };
        Self {
            subject: Subject::extension(by.clone()),
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

    /// The same call, by `deadline` at the latest.
    pub fn by(mut self, deadline: Option<Deadline>) -> Self {
        self.deadline = match (self.deadline, deadline) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        self
    }

    /// What the call says about itself to another process, or to a host component: its id, its
    /// parent, the extensions waiting on it and its deadline.
    pub fn meta(&self) -> CallMeta {
        let meta = CallMeta::new(self.call_id.to_string()).chain(self.chain.links().iter().cloned());
        let meta = meta.with_parent(self.parent.map(|p| p.to_string()));
        meta.with_deadline(self.deadline.map(|d| d.0))
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
        let id = |s: &str| ExtId::native(s).or_else(|_| ExtId::package(s)).unwrap();
        let child = root.child(&id("system"), false);
        assert_eq!(child.parent, Some(root.call_id));
        assert!(child.call_id > root.call_id);
        assert_eq!(child.subject, Subject::extension(id("system")));
        assert_eq!(child.client, None, "an extension does not stand in for a connection");
        assert!(child.chain.is_empty());

        let held = root.child(&id("acme.notes"), true).child(&id("acme.todo"), true);
        assert_eq!(held.chain.extensions().collect::<Vec<_>>(), ["acme.notes", "acme.todo"]);
        assert_eq!(held.chain.links()[0].call(), root.call_id.to_string());
        let meta = held.meta();
        assert!(meta.holds("acme.todo", &held.parent.unwrap().to_string()));
        assert_eq!(meta.call_id(), held.call_id.to_string());

        root.cancel.cancel();
        assert!(child.cancel.is_cancelled(), "cancelled with its parent");
    }
}
