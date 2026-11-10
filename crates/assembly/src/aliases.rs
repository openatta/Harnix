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
    // The kernel's: a title is the engine's name for a session; the composer's model switch; a
    // device's upload.
    ("harnix.sessions.rename", "harnix.session.rename"),
    ("harnix.sessions.setModel", "harnix.session.setModel"),
    ("harnix.upload.begin", "harnix.bulk.upload"),
    // harnix-app
    ("harnix.workspaces.list", "harnix-app.workspaces.list"),
    ("harnix.workspaces.create", "harnix-app.workspaces.create"),
    ("harnix.workspaces.update", "harnix-app.workspaces.update"),
    ("harnix.workspaces.reorder", "harnix-app.workspaces.reorder"),
    ("harnix.workspaces.remove", "harnix-app.workspaces.remove"),
    ("harnix.sessions.archive", "harnix-app.sessions.archive"),
    ("harnix.sessions.export", "harnix-app.sessions.export"),
    ("harnix.prefs.set", "harnix-app.prefs.set"),
    ("harnix.search", "harnix-app.search"),
    ("harnix.listDirectory", "harnix-app.listDirectory"),
    ("harnix.recentProjects", "harnix-app.recentProjects"),
    ("harnix.files", "harnix-app.files"),
    ("harnix.projects.create", "harnix-app.projects.create"),
    // system
    ("harnix.process", "system.process"),
    ("harnix.system.context", "system.context"),
    ("harnix.system.messages.drop", "system.messages.drop"),
    ("harnix.system.messages.edit", "system.messages.edit"),
    ("harnix.system.tasks.stop", "system.tasks.stop"),
    ("harnix.system.tasks.message", "system.tasks.message"),
    ("harnix.system.tasks.output", "system.tasks.output"),
    ("harnix.system.compact", "system.compact"),
    ("harnix.system.clear", "system.clear"),
    ("harnix.system.model", "system.model"),
    ("harnix.system.rename", "system.rename"),
    ("harnix.system.goal", "system.goal"),
    ("harnix.system.goal.set", "system.goal.set"),
    ("harnix.system.goal.update", "system.goal.update"),
    ("harnix.system.instances.stop", "system.instances.stop"),
    ("harnix.system.instances.restart", "system.instances.restart"),
    ("harnix.system.instances.create", "system.instances.create"),
    ("harnix.system.instances.templates", "system.instances.templates"),
    // flow
    ("app.flow.list", "flow.list"),
    ("app.flow.get", "flow.get"),
    ("app.flow.history", "flow.history"),
    ("app.flow.defs", "flow.defs"),
    ("app.flow.def", "flow.def"),
    ("app.flow.schema", "flow.schema"),
    ("app.flow.catalog", "flow.catalog"),
    ("app.flow.validate", "flow.validate"),
    ("app.flow.save", "flow.save"),
    ("app.flow.delete", "flow.delete"),
    ("app.flow.parts", "flow.parts"),
    ("app.flow.part", "flow.part"),
    ("app.flow.part.save", "flow.part.save"),
    ("app.flow.part.delete", "flow.part.delete"),
    ("app.flow.hooks", "flow.hooks"),
    ("app.flow.focus", "flow.focus"),
    ("app.flow.session", "flow.session"),
    ("app.flow.sessions", "flow.sessions"),
    ("app.flow.guide", "flow.guide"),
    ("app.flow.pause", "flow.pause"),
    ("app.flow.resume", "flow.resume"),
    ("app.flow.dry_run.start", "flow.dry_run.start"),
    ("app.flow.dry_run.step", "flow.dry_run.step"),
    ("app.flow.dry_run.run", "flow.dry_run.run"),
    ("app.flow.dry_run.list", "flow.dry_run.list"),
    ("app.flow.start", "flow.start"),
    ("app.flow.cancel", "flow.cancel"),
    ("app.flow.proposal", "flow.proposal"),
    ("app.flow.claim", "flow.claim"),
    ("app.flow.heartbeat", "flow.heartbeat"),
    ("app.flow.submit", "flow.submit"),
    ("app.flow.fail", "flow.fail"),
    ("app.flow.retry", "flow.retry"),
    ("app.flow.confirm", "flow.confirm"),
    ("app.flow.force", "flow.force"),
    ("app.flow.rework", "flow.rework"),
    ("app.flow.release", "flow.release"),
];

/// Old topic → new topic: decided as the new one's row, and watched in the hub by the old name,
/// told under it.
const TOPICS: &[(&str, &str)] = &[
    ("harnix.process", "system.process"),
    ("app.flow.runs", "flow.runs"),
    ("app.flow.run", "flow.run"),
    ("app.flow.defs", "flow.defs"),
    ("app.flow.focus", "flow.focus"),
    ("app.flow.dry_runs", "flow.dry_runs"),
];

/// Every alias whose new name is in the table: one whose extension is not linked here, or is
/// switched off, has nothing to answer as.
pub(crate) fn install(table: &mut RouteTable) -> anyhow::Result<()> {
    for (old, new) in ALIASES {
        if table.row(new).is_some() {
            table.alias(old, new).map_err(|e| anyhow::anyhow!("the alias `{old}` → `{new}`: {e}"))?;
        }
    }
    for (old, new) in topics(table) {
        let (old, new) = (harnix_router::topic(old), harnix_router::topic(new));
        table.alias(&old, &new).map_err(|e| anyhow::anyhow!("the alias `{old}` → `{new}`: {e}"))?;
    }
    Ok(())
}

/// The moving topics whose new name is in the table: what the hub is told to watch by both names.
pub(crate) fn topics(table: &RouteTable) -> Vec<(&'static str, &'static str)> {
    TOPICS.iter().copied().filter(|(_, new)| table.row(&harnix_router::topic(new)).is_some()).collect()
}

/// Which of the shell's own spaces it draws, as it reads them from `hello` until it activates its
/// interface packages by the extensions there: the spaces the enabled extensions declare, and the
/// projects view beside the session space.
pub(crate) fn shell_spaces(declared: &[String]) -> Vec<String> {
    let mut spaces = Vec::new();
    for id in declared {
        spaces.push(id.clone());
        if id == "sessions" {
            spaces.push("projects".to_string());
        }
    }
    spaces
}
