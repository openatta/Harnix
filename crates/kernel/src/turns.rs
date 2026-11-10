//! Which calls a running turn is on behalf of: what a tool its session calls carries as its chain.
//!
//! The hub writes it — a turn's chain is the chain of the queue item that started it, steered
//! messages' joined in — and clears it when the turn settles. What reads it is the engine's tool
//! adapter, through `harnix_engine_api::ChainSource` (the instance layer hands one over as it builds
//! each engine in this process), and a carrier that tells an extension process of the turns on its
//! own engine.

use std::collections::HashMap;
use std::sync::Mutex;

use harnix_contract::sync::lock;

use crate::call::Chain;

/// The chain of every turn running, by its instance and session.
#[derive(Default)]
pub struct TurnChains {
    turns: Mutex<HashMap<(String, String), Chain>>,
}

impl TurnChains {
    pub fn new() -> Self {
        Self::default()
    }

    /// A turn starts on behalf of `chain`, in place of whatever the session's last turn left.
    pub fn start(&self, instance: &str, session: &str, chain: Chain) {
        let key = (instance.to_string(), session.to_string());
        let mut turns = lock(&self.turns);
        if chain.is_empty() {
            turns.remove(&key);
        } else {
            turns.insert(key, chain);
        }
    }

    /// A message steered into the running turn: the turn is on behalf of its calls too. Taking the
    /// union refuses nothing wrongly — re-entry is decided by the calls still running.
    pub fn join(&self, instance: &str, session: &str, chain: &Chain) {
        if chain.is_empty() {
            return;
        }
        let mut turns = lock(&self.turns);
        let entry = turns.entry((instance.to_string(), session.to_string())).or_default();
        *entry = entry.union(chain);
    }

    /// The turn settled.
    pub fn settle(&self, instance: &str, session: &str) {
        lock(&self.turns).remove(&(instance.to_string(), session.to_string()));
    }

    /// The running turn's chain; none when no turn runs, or it runs on behalf of no call.
    pub fn chain(&self, instance: &str, session: &str) -> Chain {
        lock(&self.turns)
            .get(&(instance.to_string(), session.to_string()))
            .cloned()
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ChainLink;

    fn chain(links: &[(&str, &str)]) -> Chain {
        links.iter().map(|(e, c)| ChainLink::new(*e, *c)).collect::<Vec<_>>().into()
    }

    #[test]
    fn a_turns_chain_is_its_items_joined_by_what_steered_it_until_it_settles() {
        let turns = TurnChains::new();
        assert!(turns.chain("main", "s1").is_empty());
        turns.start("main", "s1", chain(&[("acme.a", "1")]));
        turns.join("main", "s1", &chain(&[("acme.b", "2"), ("acme.a", "1")]));
        assert_eq!(turns.chain("main", "s1"), chain(&[("acme.a", "1"), ("acme.b", "2")]));
        assert!(turns.chain("main", "s2").is_empty(), "per session");
        // The next turn is its own item's, not the last one's.
        turns.start("main", "s1", chain(&[("acme.c", "3")]));
        assert_eq!(turns.chain("main", "s1"), chain(&[("acme.c", "3")]));
        turns.settle("main", "s1");
        assert!(turns.chain("main", "s1").is_empty());
        turns.start("main", "s1", Chain::default());
        assert!(turns.chain("main", "s1").is_empty());
    }
}
