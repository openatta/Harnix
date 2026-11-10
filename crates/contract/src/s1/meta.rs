//! A call's context, as it crosses a process: what a call that arrives on the extension wire, or
//! that a host component makes as an extension it registered, says about itself.
//!
//! Every field is a hint. The receiving process issues its own call id, never takes a parent from
//! the wire, and tightens a deadline it is given; what it keeps is the chain of extensions waiting,
//! which it checks against the calls it knows are live.

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

    pub fn call_id(&self) -> &str {
        &self.call_id
    }

    pub fn parent_id(&self) -> Option<&str> {
        self.parent.as_deref()
    }

    pub fn links(&self) -> &[ChainLink] {
        &self.chain
    }

    pub fn deadline(&self) -> Option<u64> {
        self.deadline_ms
    }
}
