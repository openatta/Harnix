//! The hub's side of the instances: the links it was handed, and what it does as one comes, goes
//! or is replaced (concept §3.1.1).
//!
//! The instance registry builds and lists instances; the hub is the only holder of their links.
//! Each link is read by one router task from the moment it is handed over — before the registry
//! lists the instance — so no caller can reach an instance whose events nobody reads.

use std::sync::{Arc, Mutex as StdMutex};

use harnix_contract::sync::{lock, write};
use harnix_instance::link::EngineLink;
use harnix_kernel::instances::{holds, Side};
use harnix_kernel::{InstanceObserver, InstanceTrust, TopicChanged};
use serde_json::{json, Value};

use crate::Hub;

/// One instance's link, and the task reading its events.
pub(crate) struct Linked {
    pub link: Arc<dyn EngineLink>,
    pub trust: InstanceTrust,
    router: StdMutex<Option<tokio::task::JoinHandle<()>>>,
}

impl Linked {
    /// Stop reading the link. Shutting it down is the registry's.
    pub(crate) fn stop(&self) {
        if let Some(task) = lock(&self.router).take() {
            task.abort();
        }
    }
}

impl Drop for Linked {
    fn drop(&mut self) {
        self.stop();
    }
}

impl Hub {
    /// Start reading `link`'s events and route `id` to it, in place of whatever link it had.
    fn take(&self, id: &str, link: Arc<dyn EngineLink>, trust: InstanceTrust) {
        let hub = self.arc();
        let router = link.take_events().map(|events| tokio::spawn(hub.route(id.to_string(), events)));
        let linked = Arc::new(Linked {
            link,
            trust,
            router: StdMutex::new(router),
        });
        if let Some(previous) = write(&self.links).insert(id.to_string(), linked) {
            previous.stop();
        }
    }

    /// How far the hub trusts an instance's engine.
    pub fn trust(&self, id: &str) -> Option<InstanceTrust> {
        self.linked(id).map(|l| l.trust.clone())
    }
}

#[async_trait::async_trait]
impl InstanceObserver<dyn EngineLink> for Hub {
    async fn added(&self, id: &str, link: Arc<dyn EngineLink>, trust: InstanceTrust) {
        debug_assert!(!holds(Side::Registry), "the registry calls the hub holding its own lock");
        self.take(id, link, trust);
    }

    async fn removed(&self, id: &str) {
        debug_assert!(!holds(Side::Registry), "the registry calls the hub holding its own lock");
        if let Some(linked) = write(&self.links).remove(id) {
            linked.stop();
        }
        self.inner.lock().await.sessions.retain(|key, _| key.0 != id);
        let hub = self.arc();
        hub.refresh("harnix.sessions.status", json!({}));
    }

    async fn restarted(&self, id: &str, link: Arc<dyn EngineLink>) {
        debug_assert!(!holds(Side::Registry), "the registry calls the hub holding its own lock");
        let trust = self.trust(id).unwrap_or(InstanceTrust::Product);
        self.take(id, link, trust);
        // Every subscription belonged to the old link.
        tokio::spawn(self.arc().resync_instance(id.to_string()));
    }

    fn has_space(&self, space: &str) -> bool {
        lock(&self.spaces).exists(space)
    }
}

/// A computed topic's owner says it changed: computed again soon, if anyone watches it.
impl TopicChanged for Hub {
    fn changed(&self, topic: &str, params: Value) {
        self.arc().refresh(topic, params);
    }
}
