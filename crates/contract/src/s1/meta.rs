//! A call's context, as it crosses a process: what a call that arrives on the extension wire, or
//! that a host component makes as an extension it registered, says about itself.
//!
//! Every field is a hint to whoever receives it from another process. The product issues its own
//! call id, keeps a parent only when it names a call it sent and still waits on, computes the chain
//! from that parent, and tightens a deadline it is given. An extension's host takes the product's
//! word, and checks the chain against the calls it knows are live: a link names an extension and
//! the call it is held on, by the id that extension's host was given for it.

use serde::{Deserialize, Serialize};

use crate::ChainLink;

/// A call's id, the call it was made while handling, the extensions waiting on it, and when it
/// stops being worth answering (milliseconds since the epoch, by the wall clock).
#[non_exhaustive]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallMeta {
    call_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    parent: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    chain: Vec<ChainLink>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    deadline_ms: Option<u64>,
}

impl CallMeta {
    pub fn new(call_id: impl Into<String>) -> Self {
        Self { call_id: call_id.into(), ..Self::default() }
    }

    pub fn parent(mut self, parent: impl Into<String>) -> Self {
        self.parent = Some(parent.into());
        self
    }

    pub fn chain(mut self, chain: impl IntoIterator<Item = ChainLink>) -> Self {
        self.chain = chain.into_iter().collect();
        self
    }

    pub fn deadline_ms(mut self, deadline_ms: u64) -> Self {
        self.deadline_ms = Some(deadline_ms);
        self
    }

    /// The same metadata under another id.
    pub fn with_call_id(mut self, call_id: impl Into<String>) -> Self {
        self.call_id = call_id.into();
        self
    }

    /// The same metadata with this parent, or none.
    pub fn with_parent(mut self, parent: Option<String>) -> Self {
        self.parent = parent;
        self
    }

    /// The same metadata with this deadline, or none.
    pub fn with_deadline(mut self, deadline_ms: Option<u64>) -> Self {
        self.deadline_ms = deadline_ms;
        self
    }

    /// The metadata of a call made while handling this one, by `ext`: a new id (empty, for
    /// whoever sends it to issue), this one as its parent, the same deadline, and the chain with
    /// `ext` held on this call when `ext` answers one call at a time (`serialized`).
    pub fn child(&self, ext: &str, serialized: bool) -> Self {
        let mut chain = self.chain.clone();
        if serialized {
            chain.push(ChainLink::new(ext, self.call_id.as_str()));
        }
        Self {
            call_id: String::new(),
            parent: Some(self.call_id.clone()),
            chain,
            deadline_ms: self.deadline_ms,
        }
    }

    /// Whether `ext` is held on `call` somewhere in the chain.
    pub fn holds(&self, ext: &str, call: &str) -> bool {
        self.chain.iter().any(|link| link.ext() == ext && link.call() == call)
    }

    /// Whether the deadline has passed at `now_ms`.
    pub fn expired(&self, now_ms: u64) -> bool {
        self.deadline_ms.is_some_and(|d| d <= now_ms)
    }

    pub fn call_id(&self) -> &str {
        &self.call_id
    }

    pub fn parent_id(&self) -> Option<&str> {
        self.parent.as_deref()
    }

    pub fn links(&self) -> &[ChainLink] {
        &self.chain
    }

    pub fn into_links(self) -> Vec<ChainLink> {
        self.chain
    }

    pub fn deadline(&self) -> Option<u64> {
        self.deadline_ms
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_child_is_held_on_its_parent_only_when_serialized() {
        let parent = CallMeta::new("7").chain([ChainLink::new("acme.a", "3")]).deadline_ms(99);
        let child = parent.child("acme.b", true);
        assert_eq!((child.call_id(), child.parent_id(), child.deadline()), ("", Some("7"), Some(99)));
        assert!(child.holds("acme.b", "7") && child.holds("acme.a", "3"));
        assert!(!parent.child("acme.b", false).holds("acme.b", "7"));
        assert!(child.expired(99) && !child.expired(98));
        let wire = serde_json::to_value(CallMeta::new("1")).unwrap();
        assert_eq!(wire, serde_json::json!({"call_id": "1"}), "what is empty is left out");
    }
}
