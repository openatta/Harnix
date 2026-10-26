//! The kernel's half of the authorization table.
//!
//! Reading this file answers "what can a client call" for everything the
//! kernel itself answers. Modules add their own rows at registration (their
//! methods, topics and routes, and the grants their manifests disclose); the
//! diagnostics page lists the whole table. Everything not named anywhere is
//! refused — the table is an allow-list, so a new method is unreachable until
//! someone writes it down (§3.4).

use harnix_authz::{MethodTable, Reach, ENGINE_REFUSALS, HUB_OWNED};

/// What the application layer answers itself, above the hub.
pub const APP_METHODS: &[&str] = &["harnix.contributions", "harnix.diagnostics"];

/// Looking into a session and changing its context: the kernel's, and a
/// module's by grant. A device reaches them through the system space's own
/// methods, which say what they are doing (design §9.3).
pub const INSPECTION: &[&str] = &[
    "session.tasks",
    "session.stopTask",
    "session.inspectContext",
    "session.dropMessages",
    "session.editMessage",
];

pub fn table() -> MethodTable {
    let mut table = MethodTable::new()
        // Engine methods a built-in module calls on a person's behalf and no
        // device calls directly: reading settings, managing packages. The
        // kernel's, and granted by name to the module whose manifest asks.
        // Reaching Agix's own `plugin.install` and friends is not opened to
        // devices, because changing what is installed changes what this host
        // serves, and going around the host would leave a disabled package's
        // interface module still being served with nothing to notice it.
        // First, so that one a device may call anyway (`plugin.list`) keeps
        // the device's reach below.
        .allow_all(&harnix_builtin::engine_methods(), Reach::Kernel)
        // The hub's own semantics. A device drives its sessions.
        .allow_all(harnix_hub::HUB_METHODS, Reach::Device)
        // Engine methods that pass through untouched.
        .allow_all(harnix_hub::PASSTHROUGH, Reach::Device)
        // Interrupting, closing and deleting are intercepted by the hub but
        // are still the device's to ask for.
        .allow_all(
            &["session.interrupt", "session.close", "session.delete"],
            Reach::Device,
        )
        .allow_all(APP_METHODS, Reach::Device)
        .allow_all(INSPECTION, Reach::Kernel);

    // Answered by the authorizer itself, and reachable by anyone that got
    // this far: a subject may always ask what it may do.
    table = table.allow(harnix_authz::REACHABLE, Reach::Device);

    // The instance tree, as whole values. Reading it is a device's: it is
    // what the system space draws (design §6.3, §9).
    for (name, _) in harnix_hub::HUB_TOPICS {
        table = table.allow(&harnix_authz::topic(name), Reach::Device);
    }

    // Methods that exist and are refused, with the reason in the error.
    // Refused explicitly rather than by omission: "unknown method" would read
    // as a bug in Harnix to whoever hit it, and these are deliberate.
    for (method, reason) in ENGINE_REFUSALS {
        table = table.allow(method, Reach::Refused(reason));
    }
    for (method, reason) in HUB_OWNED {
        table = table.allow(method, Reach::HubOwned(reason));
    }
    table
}

#[cfg(test)]
mod tests {
    use harnix_contract::Subject;

    use super::*;

    fn device() -> Subject {
        Subject::Device { id: "test".into() }
    }

    /// The whole point of an allow-list: a name nobody wrote down is refused,
    /// and adding a method is a decision rather than an accident.
    #[test]
    fn an_unlisted_method_is_refused() {
        let table = table();
        assert!(!table.reachable(&device()).contains(&"session.somethingNew"));
    }

    /// The engine trusts whoever reaches its dispatch. These would hand that
    /// trust to a client (§3.4), so they are refused with a reason rather
    /// than left unimplemented.
    #[test]
    fn credential_and_endpoint_methods_are_refused() {
        let table = table();
        for (method, _) in ENGINE_REFUSALS.iter().chain(HUB_OWNED) {
            assert!(
                !table.reachable(&device()).contains(method),
                "{method} reachable by a device"
            );
        }
    }

    /// The engine's package lifecycle is not a device's to reach directly,
    /// because reaching it would skip the re-read that keeps what is served in
    /// step with what is installed. `mcp.addServer` configures a
    /// subprocess-spawning tool with no manifest and no disclosure at all.
    #[test]
    fn the_engine_package_lifecycle_and_mcp_are_not_reachable() {
        let table = table();
        let reachable = table.reachable(&device());
        for method in [
            "plugin.install",
            "plugin.uninstall",
            "plugin.enable",
            "plugin.disable",
            "mcp.addServer",
        ] {
            assert!(!reachable.contains(&method), "{method} reachable by a device");
        }
    }

    /// What the hub owns is never granted, not even to the kernel's own
    /// modules: each one is a way around the hub.
    #[test]
    fn the_hubs_own_methods_cannot_be_granted() {
        let mut table = table();
        for (method, _) in HUB_OWNED {
            assert!(table.grant(method, "anything").is_err(), "{method} was grantable");
        }
    }
}
