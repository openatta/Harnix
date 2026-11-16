//! Which calls a running turn is on behalf of: what a tool its session calls carries as its chain.
//!
//! The hub writes it — a turn's chain is the chain of the queue item that started it, steered
//! messages' joined in — and clears it when the turn settles. What reads it is the engine's tool
//! adapter, through `harnix_engine_link::ChainSource` (the instance layer hands one over as it builds
//! each engine in this process), and a carrier that tells an extension process of the turns on its
//! own engine: that one watches its instance's turns ([`TurnChains::watch`]) and is told each as it
//! starts, grows and settles.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use harnix_contract::sync::lock;

use crate::call::Chain;

/// Told of the turns of one instance, as they go. Called holding no lock of the hub's or of
/// [`TurnChains`]'s.
pub trait TurnWatch: Send + Sync {
    /// A turn started in `session` on behalf of `chain`, or the running one now is on behalf of
    /// more (a message steered into it): `chain` is the whole of it.
    fn started(&self, session: &str, chain: &Chain);

    fn settled(&self, session: &str);
}

/// The chain of every turn running, by its instance and session.
#[derive(Default)]
pub struct TurnChains {
    turns: Mutex<HashMap<(String, String), Chain>>,
    watches: Mutex<HashMap<String, Arc<dyn TurnWatch>>>,
}

impl TurnChains {
    pub fn new() -> Self {
        Self::default()
    }

    /// Tell `watch` of `instance`'s turns from now on, in place of whoever was told before.
    pub fn watch(&self, instance: &str, watch: Arc<dyn TurnWatch>) {
        lock(&self.watches).insert(instance.to_string(), watch);
    }

    pub fn unwatch(&self, instance: &str) {
        lock(&self.watches).remove(instance);
    }

    fn watcher(&self, instance: &str) -> Option<Arc<dyn TurnWatch>> {
        lock(&self.watches).get(instance).cloned()
    }

    /// A turn starts on behalf of `chain`, in place of whatever the session's last turn left.
    pub fn start(&self, instance: &str, session: &str, chain: Chain) {
        let key = (instance.to_string(), session.to_string());
        {
            let mut turns = lock(&self.turns);
            if chain.is_empty() {
                turns.remove(&key);
            } else {
                turns.insert(key, chain.clone());
            }
        }
        if let Some(watch) = self.watcher(instance) {
            watch.started(session, &chain);
        }
    }

    /// A message steered into the running turn: the turn is on behalf of its calls too. Taking the
    /// union refuses nothing wrongly — re-entry is decided by the calls still running.
    pub fn join(&self, instance: &str, session: &str, chain: &Chain) {
        if chain.is_empty() {
            return;
        }
        let joined = {
            let mut turns = lock(&self.turns);
            let entry = turns.entry((instance.to_string(), session.to_string())).or_default();
            *entry = entry.union(chain);
            entry.clone()
        };
        if let Some(watch) = self.watcher(instance) {
            watch.started(session, &joined);
        }
    }

    /// The turn settled.
    pub fn settle(&self, instance: &str, session: &str) {
        lock(&self.turns).remove(&(instance.to_string(), session.to_string()));
        if let Some(watch) = self.watcher(instance) {
            watch.settled(session);
        }
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

    #[derive(Default)]
    struct Told(Mutex<Vec<String>>);

    impl TurnWatch for Told {
        fn started(&self, session: &str, chain: &Chain) {
            let links: Vec<String> = chain.links().iter().map(|l| format!("{}:{}", l.ext(), l.call())).collect();
            lock(&self.0).push(format!("started {session} [{}]", links.join(",")));
        }
        fn settled(&self, session: &str) {
            lock(&self.0).push(format!("settled {session}"));
        }
    }

    #[test]
    fn a_watcher_is_told_its_own_instances_turns_whole() {
        let turns = TurnChains::new();
        let told = Arc::new(Told::default());
        turns.watch("proc.acme.a", told.clone());
        turns.start("proc.acme.a", "s1", chain(&[("acme.a", "1")]));
        turns.join("proc.acme.a", "s1", &chain(&[("acme.b", "2")]));
        turns.start("main", "s9", chain(&[("acme.c", "3")]));
        turns.settle("proc.acme.a", "s1");
        turns.unwatch("proc.acme.a");
        turns.start("proc.acme.a", "s2", Chain::default());
        assert_eq!(*lock(&told.0), ["started s1 [acme.a:1]", "started s1 [acme.a:1,acme.b:2]", "settled s1"]);
    }
}
