//! What the `apps` feature adds to starting a process: the runtime applications run in.

use std::collections::BTreeMap;
use std::sync::Arc;

/// The topic a running application's tool calls report their progress on (`host.progress`).
pub(super) const APPS_PROGRESS_TOPIC: &str = "harnix.apps.progress";

/// What issues and revokes the credential each daemon this process spawns presents at a runtime
/// application's MCP endpoint: the application runtime, which serves them.
pub(super) struct DaemonTokens(pub(super) Arc<harnix_apps::Registry>);

impl harnix_instance::ServerTokens for DaemonTokens {
    fn issue(&self) -> String {
        self.0.issue_daemon_token()
    }
    fn revoke(&self, token: &str) {
        self.0.revoke_daemon_token(token)
    }
}

/// What the application runtime verifies packages with: the store's key built in (or, for a test
/// application, the ones it was assembled with), and the profile's trusted publishers. A key that
/// does not read stops the start, naming it: a typo here would otherwise quietly trust nobody.
pub(super) fn signing_keys(
    apps: &harnix_instance::profile::AppsProfile,
    store: Option<&[String]>,
) -> anyhow::Result<harnix_apps::signature::Keys> {
    use harnix_apps::signature::PublicKey;
    let store = match store {
        Some(keys) => keys
            .iter()
            .map(|k| PublicKey::parse(k).map_err(anyhow::Error::msg))
            .collect::<anyhow::Result<Vec<_>>>()?,
        None => harnix_apps::store::store_keys(),
    };
    let mut trusted = BTreeMap::new();
    for (publisher, keys) in &apps.trusted_publishers {
        let keys = keys
            .iter()
            .map(|k| {
                PublicKey::parse(k)
                    .map_err(|e| anyhow::anyhow!("[apps.trusted_publishers] {publisher}: {e}"))
            })
            .collect::<anyhow::Result<Vec<_>>>()?;
        trusted.insert(publisher.clone(), keys);
    }
    Ok(harnix_apps::signature::Keys { store, trusted })
}
