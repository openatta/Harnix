//! What this process admits besides a device: the engines it spawned, and the extension
//! processes it started.
//!
//! - **An engine** presents a credential when it calls back over HTTP (a package's tool
//!   endpoint), admitted as `Subject::Engine`: one per daemon process, issued before it is spawned
//!   and revoked when it goes, so a credential written into a file the process read opens nothing
//!   once that process is gone.
//! - **An extension process** presents nothing: the connection is the identity. Before it is
//!   started, its subject is bound to the product's end of the stream it is handed
//!   ([`ChildBindings::bind_child`]); what comes over that stream is that extension's, and nothing
//!   else can come over it — the process listens on nothing, and no other process holds the other
//!   end. The binding goes when the stream does: the process exited, the stream closed, the handle
//!   was dropped.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};

use harnix_contract::s1::ExtId;
use harnix_contract::sync::lock;
use harnix_contract::Subject;

/// The engine credentials that are live, and whose each is.
#[derive(Default)]
pub struct EngineCredentials {
    live: Mutex<HashMap<String, String>>,
}

impl EngineCredentials {
    /// A credential for the daemon about to be spawned for `instance`, admitted from now on.
    pub fn issue_engine(&self, instance: &str) -> String {
        let token = format!("{:032x}{:032x}", rand(), rand());
        lock(&self.live).insert(token.clone(), instance.to_string());
        token
    }

    /// The daemon that held it has gone.
    pub fn revoke(&self, token: &str) {
        lock(&self.live).remove(token);
    }

    /// Who presents `token`, when it is a live engine credential. A map lookup: not a timing
    /// oracle worth defending against from this machine, where a tool endpoint is served.
    pub fn admit(&self, token: &str) -> Option<Subject> {
        if token.is_empty() {
            return None;
        }
        lock(&self.live).get(token).map(|instance| Subject::Engine { instance: instance.clone() })
    }
}

/// The extension processes started so far that are still bound: one binding per stream handed out.
#[derive(Default)]
pub struct ChildBindings {
    live: Mutex<HashMap<u64, ExtId>>,
    next: AtomicU64,
}

impl ChildBindings {
    /// Bind the subject of the extension `id` to the stream about to be handed to its process:
    /// what comes over the product's end is `Extension{id}`'s for as long as the binding is held.
    pub fn bind_child(self: &Arc<Self>, id: &ExtId) -> ChildSubject {
        let key = self.next.fetch_add(1, Ordering::Relaxed);
        lock(&self.live).insert(key, id.clone());
        ChildSubject { bindings: Arc::downgrade(self), key, subject: Subject::extension(id.clone()) }
    }

    /// How many streams are bound to `id` now: one per running process of it, at most.
    pub fn bound(&self, id: &ExtId) -> usize {
        lock(&self.live).values().filter(|bound| *bound == id).count()
    }
}

/// One extension process's subject, bound to the stream it was handed. Dropped, it is revoked.
pub struct ChildSubject {
    bindings: Weak<ChildBindings>,
    key: u64,
    subject: Subject,
}

impl ChildSubject {
    /// Who calls over the stream it is bound to.
    pub fn subject(&self) -> &Subject {
        &self.subject
    }
}

impl Drop for ChildSubject {
    fn drop(&mut self) {
        if let Some(bindings) = self.bindings.upgrade() {
            lock(&bindings.live).remove(&self.key);
        }
    }
}

/// 128 bits from the operating system's generator.
fn rand() -> u128 {
    let mut bytes = [0u8; 16];
    ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut bytes).expect("the system's random source");
    u128::from_le_bytes(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_credential_admits_its_engine_until_revoked() {
        let engines = EngineCredentials::default();
        let token = engines.issue_engine("side");
        assert_eq!(engines.admit(&token), Some(Subject::Engine { instance: "side".into() }));
        assert_ne!(engines.issue_engine("side"), token, "one per daemon");
        engines.revoke(&token);
        assert_eq!(engines.admit(&token), None);
        assert_eq!(engines.admit(""), None);
    }

    #[test]
    fn a_child_is_its_extension_until_its_binding_goes() {
        let bindings = Arc::new(ChildBindings::default());
        let id = ExtId::package("acme.notes").unwrap();
        let first = bindings.bind_child(&id);
        assert_eq!(first.subject(), &Subject::extension(id.clone()));
        let second = bindings.bind_child(&id);
        assert_eq!(bindings.bound(&id), 2);
        drop(first);
        assert_eq!(bindings.bound(&id), 1);
        drop(second);
        assert_eq!(bindings.bound(&id), 0);
    }
}
