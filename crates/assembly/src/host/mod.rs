//! The extension host: what turns the extensions a product lists into a running process.
//!
//! An extension comes as a descriptor whatever made it: a native extension's `register`, or a
//! package's descriptor registered while the process runs ([`dynamic`]). The host checks the
//! grants, installs each descriptor's rows in the route table, its topics and spaces in the hub,
//! its routes in the transport, its engine contribution and templates in the instances; seats it
//! — hands its declarations the kernel's services, through which its context reaches the process
//! ([`seat`]) — and drives its lifecycle ([`lifecycle`]). An extension that fails at any step is
//! disabled with everything it declared, and the rest go on; the diagnostics and
//! `harnix.extensions` say which and why ([`status`]).

pub(crate) mod contribution;
pub(crate) mod dynamic;
pub(crate) mod lifecycle;
pub(crate) mod routes;
pub(crate) mod seat;
pub(crate) mod spawn;
pub mod status;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use harnix_contract::s1::{Capability, ExtId};
use harnix_ext::{ErasedExtension, ExtensionDescriptor, Manifest};
use harnix_kernel::Owner;
use harnix_router::RouteTable;
use serde_json::Value;

pub use status::{ExtensionStatus, MethodStatus};

/// One extension the product added, with what it is granted.
pub(crate) struct Entry {
    pub ext: Box<dyn ErasedExtension>,
    pub granted: Vec<Capability>,
}

/// One extension, through every step of the start.
pub(crate) struct Record {
    pub ext: Box<dyn ErasedExtension>,
    pub manifest: Manifest,
    /// `None` for an id that is not one: refused at registration.
    pub id: Option<ExtId>,
    pub descriptor: Option<ExtensionDescriptor>,
    /// Its section of the profile, as written.
    pub config: Value,
    pub data_dir: PathBuf,
    pub enabled: Arc<AtomicBool>,
    /// Whether its rows went in the table. One refused before they did — a second extension with
    /// an id already taken — has nothing to withdraw, and the rows under that id are the other's.
    pub installed: bool,
    pub status: ExtensionStatus,
}
impl Record {
    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::Acquire)
    }

    /// Its rows' owner in the route table: asked only once its id was checked.
    pub fn owner(&self) -> Owner {
        Owner::extension(self.id.clone().expect("an extension's id is checked first"))
    }

    pub fn disable(&mut self, phase: &'static str, reason: String, table: &mut RouteTable) {
        if self.installed {
            table.withdraw(&self.owner());
        }
        self.disabled(phase, reason);
    }

    /// [`Record::disable`] once the table is the router's, which withdraws its rows itself.
    pub fn disabled(&mut self, phase: &'static str, reason: String) {
        tracing::error!(extension = %self.manifest.id(), phase, %reason, "extension disabled");
        self.enabled.store(false, Ordering::Release);
        self.status.enabled = false;
        self.status.phase = Some(phase);
        self.status.reason = Some(reason);
    }
}
