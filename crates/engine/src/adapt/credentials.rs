//! A [`CredentialSource`] as Agix's.

use std::sync::Arc;

use base::interface::credentials::{CredentialSource as AgixSource, Secret as AgixSecret};
use base::provider::ProviderConfig;
use harnix_engine_api::CredentialSource;

pub(crate) struct AgixCredentials(pub(crate) Arc<dyn CredentialSource>);

impl AgixSource for AgixCredentials {
    fn api_key(&self, provider_id: &str, _config: &ProviderConfig) -> Result<AgixSecret, String> {
        self.0.get(provider_id).map(|secret| AgixSecret::new(secret.expose()))
    }
}

#[cfg(test)]
mod tests {
    use crate::adapt::{agix_source, assert_covers, impl_methods, trait_methods};

    #[test]
    fn every_method_of_agixs_credential_source_is_written() {
        let agix = trait_methods(&agix_source("crates/core/src/interface/credentials.rs"), "pub trait CredentialSource:");
        let ours = impl_methods(include_str!("credentials.rs"), "impl AgixSource for AgixCredentials");
        assert_covers(agix, ours, &["api_key"], &[]);
    }
}
