//! Step 2: every extension registered, in order — the built-ins first, then the product's.

use std::sync::{Arc, OnceLock};

use harnix_authz::Devices;
use harnix_contract::s1::caps;
use harnix_kernel::{Access, GrantTable, KernelPart, Owner};
use harnix_router::{Row, RouteTable};
use harnix_builtin::Builtin;
use harnix_ext::Extension;
use harnix_module::ErasedModule;

use super::state_files::{load_devices, DEVICES_FILE};
use super::wiring::AppsWiring;
use crate::pairing;
use crate::gate::Behind;
use crate::host::{lifecycle, Added, Entry, Namespace, Record};
use crate::kernel_rows;
use crate::paths::Roots;
use crate::profile::Profile;

/// What registering produced: the extensions' records and the route table they declared into,
/// what the kernel's rows in it are answered by once that is built, and what serving needs
/// later.
pub(super) struct Registered {
    pub records: Vec<Record>,
    pub table: RouteTable,
    pub behind: Behind,
    pub held: Held,
    /// What each extension was granted, and how far each is trusted.
    pub grants: Arc<GrantTable>,
    /// Whether anything may register extensions while the process runs: then the spaces can
    /// change after assembly, and there is a topic to watch them on.
    pub dynamic: bool,
}

/// What the built-ins hold that serving needs.
pub(crate) struct Held {
    /// The built-ins, when the application has them: the bulk store.
    pub builtin: Option<Arc<Builtin>>,
    /// The devices paired before, read back: pairing carries trust across once, not once per
    /// start.
    pub paired: Arc<Devices>,
    /// The transport's channels, once it exists (step 6): what revoking a device ends.
    pub channels: Arc<OnceLock<harnix_transport::Channels>>,
    /// Pairing, listing and revoking devices: what `devices.manage` hands an extension.
    pub devices: Arc<pairing::DeviceBook>,
}

/// The built-ins are granted what they require: `with_builtins` is the product asking for them
/// as Harnix ships them.
fn granted_as_required(module: Box<dyn ErasedModule>) -> Entry {
    let granted = module.manifest().requires;
    Entry {
        added: Added::Module { module, namespace: Namespace::Kernel },
        granted,
    }
}

/// The same for an extension: Harnix's own, which the product takes as they ship.
fn extension_as_required(extension: impl Extension) -> Entry {
    let granted = extension.manifest().required().to_vec();
    Entry { added: Added::Extension(Box::new(extension)), granted }
}

pub(super) fn register(
    builtins: bool,
    product: Vec<Entry>,
    profile: &Profile,
    roots: &Roots,
    apps: &AppsWiring,
) -> anyhow::Result<Registered> {
    let paired = Arc::new(load_devices(&roots.state.join(DEVICES_FILE))?);
    let channels: Arc<OnceLock<harnix_transport::Channels>> = Arc::new(OnceLock::new());
    // Revoking is wired to the transport's session registry, because
    // ending a device's channels needs the layer that knows there are
    // channels. Nothing above transport ever learns there were.
    let disconnect = {
        let channels = channels.clone();
        Arc::new(move |id: &str| {
            let ended = channels.get().map_or(0, |c| c.revoke_device(id));
            tracing::info!(device = id, channels = ended, "revoked; its channels are closed");
        })
    };
    let book = Arc::new(pairing::DeviceBook::new(paired.clone(), disconnect));
    let mut entries = Vec::new();
    let builtin = if builtins {
        let builtin = Builtin::new(roots.state.clone(), roots.projects.clone())?;
        for module in builtin.modules_with(
            profile.shell.keeps(harnix_builtin::SYSTEM_SPACE),
            profile.shell.model_switch,
        ) {
            entries.push(granted_as_required(Box::new(module)));
        }
        // Runtime applications' install records and switches: only when this process hosts them.
        if profile.apps.enabled {
            let module = apps.apps_module(
                harnix_builtin::apps::AppsModule::new(roots.state.join("apps")).dev(profile.apps.dev),
            );
            entries.push(granted_as_required(Box::new(module)));
        }
        entries.push(extension_as_required(harnix_ext_settings::Settings));
        entries.push(extension_as_required(harnix_ext_plugins::Plugins::default()));
        entries.push(extension_as_required(harnix_ext_observability::Observability));
        entries.push(extension_as_required(harnix_ext_devices::Devices));
        Some(builtin)
    } else {
        None
    };
    entries.extend(product);
    let behind = Behind::default();
    let mut table = kernel_rows::table(&behind);
    apps.rows(&mut table)?;
    let grants = Arc::new(GrantTable::new());
    let records = lifecycle::register(entries, &mut table, &grants, &profile.ext, &profile.extensions.off, &roots.state)?;
    // What changes the list of spaces after assembly is an extension registered while the process
    // runs; only when something may do that is there anything to watch.
    let dynamic = profile.apps.enabled
        || records.iter().any(|r| r.is_enabled() && r.status.granted.iter().any(|c| c == caps::EXTENSIONS_MANAGE.as_str()));
    if dynamic {
        let spaces = Row::topic(harnix_hub::SPACES_TOPIC, Access::device());
        table
            .install(&Owner::Kernel(KernelPart::Hub), vec![spaces])
            .map_err(|e| anyhow::anyhow!("{e}"))?;
    }
    crate::aliases::install(&mut table)?;
    Ok(Registered {
        records,
        table,
        grants,
        dynamic,
        behind,
        held: Held {
            builtin,
            paired,
            channels,
            devices: book,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roots(dir: &std::path::Path) -> Roots {
        Roots {
            engine: dir.join("engine"),
            projects: dir.join("projects"),
            state: dir.join("state"),
        }
    }

    fn ids(registered: &Registered) -> Vec<String> {
        registered.records.iter().map(|r| r.manifest.id().to_string()).collect()
    }

    /// The built-ins register before the application's modules, the devices extension last of
    /// them; the `apps` module only where the profile enables runtime applications.
    #[test]
    fn the_built_ins_register_first_and_apps_only_when_enabled() {
        let dir = tempfile::tempdir().unwrap();
        let roots = roots(dir.path());
        std::fs::create_dir_all(&roots.state).unwrap();
        let mut profile = Profile::default();

        let registered = register(true, Vec::new(), &profile, &roots, &AppsWiring::default()).unwrap();
        let listed = ids(&registered);
        assert!(registered.held.builtin.is_some());
        assert!(!listed.iter().any(|id| id == harnix_builtin::apps::MODULE_ID), "{listed:?}");
        assert_eq!(listed.last().map(String::as_str), Some("devices"), "{listed:?}");
        assert!(registered.records.iter().all(|r| r.is_enabled()), "{listed:?}");

        profile.apps.enabled = true;
        let registered = register(true, Vec::new(), &profile, &roots, &AppsWiring::default()).unwrap();
        assert!(ids(&registered).iter().any(|id| id == harnix_builtin::apps::MODULE_ID));
    }

    /// Without the built-ins there is no bulk store, no devices module, and nothing registered
    /// but the application's own modules.
    #[test]
    fn without_the_built_ins_nothing_is_registered_but_the_application() {
        let dir = tempfile::tempdir().unwrap();
        let roots = roots(dir.path());
        std::fs::create_dir_all(&roots.state).unwrap();
        let registered =
            register(false, Vec::new(), &Profile::default(), &roots, &AppsWiring::default()).unwrap();
        assert!(registered.held.builtin.is_none());
        assert!(registered.records.is_empty());
    }
}
