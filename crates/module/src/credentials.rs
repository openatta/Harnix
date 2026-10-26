//! Where a module's secrets come from.
//!
//! A profile never holds a secret, only a reference to one:
//!
//! ```toml
//! [modules.watchlist]
//! quote_api_key = { credential = "helm/quote-api" }
//! ```
//!
//! The application decides what answers a reference — the system keychain,
//! a vault, the environment — by handing `App::builder().credentials(..)` a
//! [`Credentials`]. The default reads the environment.

use base::interface::credentials::Secret;
use serde::Deserialize;

/// A reference to a secret, as written in a profile.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct CredentialRef {
    pub credential: String,
}

pub trait Credentials: Send + Sync {
    /// The secret this reference names, or why there is none.
    fn get(&self, reference: &str) -> Result<Secret, String>;
}

/// `helm/quote-api` → `$HARNIX_CREDENTIAL_HELM_QUOTE_API`.
pub struct EnvCredentials;

impl EnvCredentials {
    pub fn variable(reference: &str) -> String {
        let name: String = reference
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_uppercase() } else { '_' })
            .collect();
        format!("HARNIX_CREDENTIAL_{name}")
    }
}

impl Credentials for EnvCredentials {
    fn get(&self, reference: &str) -> Result<Secret, String> {
        let var = Self::variable(reference);
        match std::env::var(&var) {
            Ok(value) if !value.is_empty() => Ok(Secret::new(value)),
            _ => Err(format!("no credential `{reference}` (looked for ${var})")),
        }
    }
}
