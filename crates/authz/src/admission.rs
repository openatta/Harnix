//! The credentials this process issues to engines it spawned: what a spawned daemon presents
//! when it calls back over HTTP (a package's tool endpoint), admitted as `Subject::Engine`.
//!
//! One per daemon process, issued before it is spawned and revoked when it goes: a credential
//! written into a file the process read opens nothing once that process is gone.

use std::collections::HashMap;
use std::sync::Mutex;

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
}
