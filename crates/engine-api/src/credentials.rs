//! Provider keys a provider's settings leave unset.

/// Where the key of a configured provider comes from when its settings name none.
///
/// Asked while the engine turns provider settings into clients — at start, and when a provider is
/// changed — so it answers at once: a source that reaches a vault resolves and caches first.
pub trait CredentialSource: Send + Sync {
    /// The key for the provider `provider_id`, or why there is none.
    fn get(&self, provider_id: &str) -> Result<Secret, String>;
}

/// A credential. It prints as nothing.
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
