//! The names that are moving, while their callers move with them.
//!
//! Each old name answers as the row of its new one: decided, reached and answered as the new one
//! is, the audit keeping the name the caller used, and the method table listing the pair apart.
//! Filled as Harnix's own extensions take names under their ids; emptied, and this file deleted,
//! once every caller says the new names.

use harnix_router::RouteTable;

/// Old name → new name.
const ALIASES: &[(&str, &str)] = &[
    ("harnix.settings.describe", "settings.describe"),
    ("harnix.settings.set", "settings.set"),
    ("harnix.settings.setProvider", "settings.setProvider"),
    ("harnix.plugins.upload", "plugins.upload"),
    ("harnix.plugins.install", "plugins.install"),
    ("harnix.plugins.uninstall", "plugins.uninstall"),
    ("harnix.plugins.enable", "plugins.enable"),
    ("harnix.plugins.disable", "plugins.disable"),
    ("harnix.plugins.reload", "plugins.reload"),
    ("harnix.plugins.list", "plugins.list"),
    ("harnix.system.trace", "observability.trace"),
    ("harnix.system.trace.call", "observability.trace.call"),
    ("harnix.system.turns", "observability.turns"),
    ("harnix.system.usage", "observability.usage"),
    // The two names of one method: the system space's tab and the earlier request envelope.
    ("harnix.system.calls", "observability.calls"),
    ("harnix.requestHeaders", "observability.calls"),
    ("harnix.devices.list", "devices.list"),
    ("harnix.devices.pair.begin", "devices.pair.begin"),
    ("harnix.devices.pair.complete", "devices.pair.complete"),
    ("harnix.devices.revoke", "devices.revoke"),
];

/// Every alias whose new name is in the table: one whose extension is not linked here, or is
/// switched off, has nothing to answer as.
pub(crate) fn install(table: &mut RouteTable) -> anyhow::Result<()> {
    for (old, new) in ALIASES {
        if table.row(new).is_some() {
            table.alias(old, new).map_err(|e| anyhow::anyhow!("the alias `{old}` → `{new}`: {e}"))?;
        }
    }
    Ok(())
}
