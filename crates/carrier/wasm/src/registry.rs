//! What is running: the packages whose tools are on offer and whose interface is served, read by
//! the tool provider (`tools.rs`), the routes (`routes.rs`) and the interface entries
//! (`frames.rs`). A package is in it exactly while it runs.

use std::collections::BTreeMap;
use std::ops::Deref;
use std::sync::{Arc, OnceLock, RwLock, Weak};

use harnix_carrier_shared::sync::{read, write};
use harnix_carrier_shared::{Loaded, Roster};
use harnix_wasm_port::{Control, Offer};

/// What is running, shared with the tool provider and the routes.
#[derive(Default)]
pub struct Registry {
    roster: Roster,
    /// What the host offered, by package: read when it is loaded.
    offered: RwLock<BTreeMap<String, Offer>>,
    control: OnceLock<Weak<dyn Control>>,
    /// Each package's `[capabilities] net`, from the moment it starts: the hosts its HTTP may
    /// reach, decided here.
    nets: RwLock<BTreeMap<String, Vec<String>>>,
}

/// The roster (`loaded`, `get`, `progress`, `listen`, …) is the shared one's.
impl Deref for Registry {
    type Target = Roster;

    fn deref(&self) -> &Roster {
        &self.roster
    }
}

impl Registry {
    pub(crate) fn insert(&self, loaded: Arc<Loaded>) {
        self.roster.insert(loaded);
    }

    pub(crate) fn remove(&self, id: &str) -> Option<Arc<Loaded>> {
        self.roster.remove(id)
    }

    pub(crate) fn set_net(&self, id: &str, net: Option<Vec<String>>) {
        match net {
            Some(net) => write(&self.nets).insert(id.to_string(), net),
            None => write(&self.nets).remove(id),
        };
    }

    pub(crate) fn net(&self, id: &str) -> Option<Vec<String>> {
        read(&self.nets).get(id).cloned()
    }

    /// The host that runs what is listed here: what tool calls and hooks go to.
    pub fn control(&self) -> Option<Arc<dyn Control>> {
        self.control.get().and_then(Weak::upgrade)
    }

    pub(crate) fn lend(&self, control: Weak<dyn Control>) {
        let _ = self.control.set(control);
    }

    /// The host offered a package's tools (or, with none, took them back).
    pub(crate) fn offer(&self, offer: Offer) {
        let mut offered = write(&self.offered);
        if offer.component {
            offered.insert(offer.app.clone(), offer);
        } else {
            offered.remove(&offer.app);
        }
    }

    pub(crate) fn offered(&self, id: &str) -> Option<Offer> {
        read(&self.offered).get(id).cloned()
    }
}
