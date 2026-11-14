//! Where an audit entry ends up.
//!
//! `harnix_authz::Audit` keeps a bounded ring so the diagnostics page can show
//! recent decisions, and a ring dies with the process. concept §6.5 asks for
//! something else: decisions written into the **session timeline**, in order
//! with every other entry, surviving load, fork and resume.
//!
//! Agix already has exactly that seam, and reusing it rather than
//! building a second log buys one specific thing: **an unknown namespace is
//! inert, never an error.** The kernel never parses an extension entry's
//! payload, so a session whose log contains entries from something that is no
//! longer installed still loads, forks and resumes unchanged. A separate
//! audit file would have had to re-earn that, and would have been a second
//! place where "what happened in this session" is written down.
//!
//! # Not every decision
//!
//! An allow on a read-only method, several times a second, would bury the
//! entries that matter under entries nobody will ever read — and a log nobody
//! reads is the same as no log. What lands here is what someone would go
//! looking for afterwards: a refusal, a device paired or revoked, an
//! extension installed. Routine allows stay in the ring.

use harnix_authz::{AuditEntry, AuditSink};
use harnix_engine::timeline::Timeline;

/// The namespace these entries are filed under. One key, opaque to the
/// engine, and the thing that makes them skippable by anything that does not
/// know what they are.
const NS: &str = "harnix.audit";

pub struct TimelineAudit {
    /// Which session an entry belongs to.
    ///
    /// Authorization decides about a *subject and a method*; it has no idea
    /// which conversation was open, and giving it one would be exactly the
    /// leak the layering test forbids. So entries are filed under the session
    /// named in the call's own parameters when there is one, and dropped into
    /// the ring alone when there is not — a device being paired belongs to no
    /// conversation, and inventing one for it would be worse than not having
    /// it in a transcript.
    timeline: Timeline,
}

impl TimelineAudit {
    pub fn new(timeline: Timeline) -> Self {
        Self { timeline }
    }
}

impl AuditSink for TimelineAudit {
    fn append(&self, entry: &AuditEntry) {
        // Only what someone would come looking for.
        if entry.decision == "allow" && !is_consequential(&entry.method) {
            return;
        }
        // A session id that does not parse is one that was never real — a typo in a call that
        // was refused, most likely. There is nowhere to file that, and inventing somewhere would
        // be worse than the ring. The write is fire and forget: an audit write must never be able
        // to fail a call, and it must never be able to slow one down either.
        let Some(session) = entry.session.as_deref() else {
            return;
        };
        let payload = serde_json::json!({
            "subject": entry.subject,
            "method": entry.method,
            "reason": entry.reason,
        });
        self.timeline.append(session, NS, entry.decision, payload);
    }
}

/// Allows worth a timeline entry: the ones that change what this process is
/// or what it may reach.
fn is_consequential(method: &str) -> bool {
    // The system space acts on sessions that are not its own: every act is worth going back for.
    (method.starts_with("system.") && method != "system.process")
        || method.starts_with("observability.")
        || matches!(
            method,
            "plugins.install"
            | "plugins.uninstall"
            | "plugins.enable"
            | "plugins.disable"
            // Picks up whatever is in the directory now, including a package
            // put there by hand — so it can change the active set without any
            // other call having been made.
            | "plugins.reload"
            | "devices.pair.complete"
            | "devices.revoke"
            | "settings.set"
            | "settings.setProvider"
            // Switching a package on or off changes what this process serves.
            | "packages.enable"
            | "packages.disable"
            // Installing, updating or removing one changes what code this process runs.
            | "packages.install"
            | "packages.installFile"
            | "packages.update"
            | "packages.uninstall"
            // Deleting what one keeps cannot be undone.
            | "packages.clearData"
            | "harnix.session.delete"
            | "harnix.instances.create"
            | "harnix.instances.stop"
            | "harnix.instances.restart"
            // Reloads every instance inheriting `main`'s providers with what was just written.
            | "harnix.instances.inheritProviders"
            // The engine's settings and plugins, written by an extension holding the capability
            // (a device reaches none of these): where model traffic goes, what code is loaded.
            | "engine.config.set"
            | "engine.config.update"
            | "engine.config.setProvider"
            | "engine.config.reload"
            | "engine.plugin.install"
            | "engine.plugin.uninstall"
            | "engine.plugin.enable"
            | "engine.plugin.disable"
            | "engine.plugin.reload"
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The rule that keeps the timeline readable: a refusal is always worth a
    /// line, a routine allow never is.
    #[test]
    fn only_consequential_allows_are_written() {
        assert!(is_consequential("devices.revoke"));
        assert!(is_consequential("plugins.install"));
        assert!(!is_consequential("harnix.sessions.list"));
        assert!(!is_consequential("engine.session.history"));
    }

    /// Every extension lifecycle call is consequential, not just installing: disabling and
    /// uninstalling a package must leave a trace too.
    #[test]
    fn the_whole_extension_lifecycle_is_audited() {
        // The lifecycle is exactly the calls that change what is installed; `upload` hands over a
        // file and `list` reads.
        for prefix in ["plugins."] {
            for call in ["install", "uninstall", "enable", "disable", "reload"] {
                let method = format!("{prefix}{call}");
                assert!(is_consequential(&method), "{method} changes what is installed and is not audited");
            }
            for call in ["upload", "list"] {
                assert!(!is_consequential(&format!("{prefix}{call}")));
            }
        }
        for call in ["install", "installFile", "update", "uninstall", "enable", "disable", "clearData"] {
            let method = format!("packages.{call}");
            assert!(is_consequential(&method), "{method} changes what is installed and is not audited");
        }
        // The engine's own lifecycle, which only an extension holding `engine.plugins` reaches:
        // the `plugins` extension's call is one entry, the engine's under it another.
        for call in ["install", "uninstall", "enable", "disable", "reload"] {
            let method = format!("engine.plugin.{call}");
            assert!(is_consequential(&method), "{method} changes what is installed and is not audited");
        }
        assert!(!is_consequential("engine.plugin.list"));
    }

    /// What an extension writes into the engine's settings with its capability, and the reload
    /// that carries a provider to the instances inheriting it.
    #[test]
    fn writes_to_the_engines_settings_are_audited() {
        for method in [
            "engine.config.set",
            "engine.config.update",
            "engine.config.setProvider",
            "engine.config.reload",
            "harnix.instances.inheritProviders",
        ] {
            assert!(is_consequential(method), "{method}");
        }
        assert!(!is_consequential("engine.config.get"));
        assert!(!is_consequential("engine.config.getProvider"));
    }
}
