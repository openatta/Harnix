//! Entries of Harnix's own in a session's timeline.
//!
//! The engine keeps, beside a session's messages, entries filed under a namespace it does not
//! interpret: an unknown namespace is inert, never an error, so a session whose log holds entries
//! from something no longer there still loads, forks and resumes unchanged. Harnix files its
//! audit there.

use std::sync::Arc;

use history::entry::LogEntry;
use history::store::HistoryStore;
use serde_json::Value;

/// Where an engine's sessions keep their timelines.
pub struct Timeline {
    store: Arc<dyn HistoryStore>,
    handle: tokio::runtime::Handle,
}

impl Timeline {
    /// On the runtime this is called on: the writes are spawned there.
    pub fn new(store: Arc<dyn HistoryStore>) -> Self {
        Self { store, handle: tokio::runtime::Handle::current() }
    }

    /// File `payload` under `ns` and `event` in the timeline of `session`, without waiting for
    /// it: a write must never be able to fail or slow its caller. False, and nothing filed, when
    /// `session` is not a session id at all.
    pub fn append(&self, session: &str, ns: &str, event: &str, payload: Value) -> bool {
        let Ok(session) = base::session::SessionId::parse(session) else {
            return false;
        };
        let record = LogEntry::Extension { ns: ns.to_string(), event: event.to_string(), payload };
        let store = self.store.clone();
        self.handle.spawn(async move {
            if let Err(e) = store.append(session, record).await {
                tracing::warn!(error = %e, "entry not written to the timeline");
            }
        });
        true
    }
}
