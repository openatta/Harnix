//! Step 2: every extension registered, in order — the built-ins first, then the product's.

use std::sync::{Arc, OnceLock};

use harnix_authz::Devices;
use harnix_contract::s1::caps;
use harnix_kernel::{Access, GrantTable, KernelPart, Owner};
use harnix_router::{Row, RouteTable};
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

/// What registering holds that serving needs.
pub(crate) struct Held {
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
    // Runtime applications' install records and switches: only when this process hosts them.
    if builtins && profile.apps.enabled {
        let module = apps.apps_module(
            harnix_builtin::apps::AppsModule::new(roots.state.join("apps")).dev(profile.apps.dev),
        );
        entries.push(granted_as_required(Box::new(module)));
    }
    entries.extend(product);
    let behind = Behind::default();
    let mut table = kernel_rows::table(&behind);
    if profile.shell.model_switch {
        let (owner, row) = kernel_rows::hub_row(harnix_hub::Hub::model_switch_row(&behind.hub));
        table.install(&owner, vec![row]).map_err(|e| anyhow::anyhow!("{e}"))?;
    }
    apps.rows(&mut table)?;
    let grants = Arc::new(GrantTable::new());
    let records = lifecycle::register(entries, &mut table, &grants, &profile.ext, &profile.extensions.off, &roots.state)?;
    // Whether anything may register extensions while the process runs, and so change the spaces
    // after assembly.
    let dynamic = profile.apps.enabled
        || records.iter().any(|r| r.is_enabled() && r.status.granted.iter().any(|c| c == caps::EXTENSIONS_MANAGE.as_str()));
    // The spaces are every extension's: a client learns them from this topic, whoever is linked.
    let spaces = Row::topic(harnix_hub::SPACES_TOPIC, Access::device());
    table.install(&Owner::Kernel(KernelPart::Hub), vec![spaces]).map_err(|e| anyhow::anyhow!("{e}"))?;
    Ok(Registered {
        records,
        table,
        grants,
        dynamic,
        behind,
        held: Held {
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

    /// The `apps` module registers only where the profile enables runtime applications, and before
    /// the application's own.
    #[test]
    fn apps_registers_only_when_enabled() {
        let dir = tempfile::tempdir().unwrap();
        let roots = roots(dir.path());
        std::fs::create_dir_all(&roots.state).unwrap();
        let mut profile = Profile::default();

        let registered = register(true, Vec::new(), &profile, &roots, &AppsWiring::default()).unwrap();
        assert!(ids(&registered).is_empty());

        profile.apps.enabled = true;
        let registered = register(true, Vec::new(), &profile, &roots, &AppsWiring::default()).unwrap();
        assert_eq!(ids(&registered), [harnix_builtin::apps::MODULE_ID]);
        let registered = register(false, Vec::new(), &profile, &roots, &AppsWiring::default()).unwrap();
        assert!(ids(&registered).is_empty());
    }

    /// `harnix.session.setModel` is in the table only with `[shell] model_switch`.
    #[test]
    fn the_model_switch_is_a_row_only_when_written() {
        let dir = tempfile::tempdir().unwrap();
        let roots = roots(dir.path());
        std::fs::create_dir_all(&roots.state).unwrap();
        let mut profile = Profile::default();
        let registered = register(true, Vec::new(), &profile, &roots, &AppsWiring::default()).unwrap();
        assert!(registered.table.row("harnix.session.setModel").is_none());
        profile.shell.model_switch = true;
        let registered = register(true, Vec::new(), &profile, &roots, &AppsWiring::default()).unwrap();
        assert!(registered.table.row("harnix.session.setModel").is_some());
    }
}
