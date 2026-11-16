//! Where an extension's secrets come from.
//!
//! A profile never holds a secret, only a reference to one:
//!
//! ```toml
//! [ext.watchlist]
//! quote_api_key = { credential = "helm/quote-api" }
//! ```
//!
//! The product decides what answers a reference — the system keychain, a vault, the
//! environment — by handing `App::builder().credentials(..)` a [`Credentials`]. The default reads
//! the environment.

use serde::Deserialize;

/// A credential, as the product's credential lookup hands it out. It prints as nothing.
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn expose(&self) -> &str {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret(…)")
    }
}

/// A reference to a secret, as written in a profile.
#[non_exhaustive]
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct CredentialRef {
    pub credential: String,
}

impl CredentialRef {
    pub fn new(credential: impl Into<String>) -> Self {
        Self { credential: credential.into() }
    }
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
