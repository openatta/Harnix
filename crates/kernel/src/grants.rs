//! What each extension was granted, and how far it is trusted: written by whoever registers
//! extensions, read where a call is decided.
//!
//! Capabilities are checked in three places only: the router (a row's requirement), the services
//! an extension's context hands out, and the instance registry's owner checks. All three read a
//! [`GrantBook`]; nothing decides by an extension's id.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::RwLock;

use harnix_contract::s1::{Capability, ExtId, Trust};
use harnix_contract::sync::{read, write};
use harnix_contract::Subject;

/// The grants, read-only.
pub trait GrantBook: Send + Sync {
    /// Whether `ext` holds `cap`.
    fn holds(&self, ext: &ExtId, cap: &Capability) -> bool;
    /// How far `ext` is trusted; `None` for an id nothing registered.
    fn trust(&self, ext: &ExtId) -> Option<Trust>;
    /// What a page of `ext`'s in a frame may call besides `ext`'s own names; `None` when it has
    /// no framed interface, and then a frame of it calls nothing.
    fn frame_calls(&self, ext: &ExtId) -> Option<BTreeSet<String>>;
}

/// Whether `subject` holds `cap`: the kernel holds every capability, a device none.
pub fn holds(grants: &dyn GrantBook, subject: &Subject, cap: &Capability) -> bool {
    match subject {
        Subject::Kernel => true,
        Subject::Extension { id } => grants.holds(id, cap),
        _ => false,
    }
}

struct Grant {
    caps: BTreeSet<Capability>,
    trust: Trust,
}

/// The grants, written. Only the extension host writes it.
#[derive(Default)]
pub struct GrantTable {
    grants: RwLock<BTreeMap<ExtId, Grant>>,
    /// Kept apart from the grants: a package's interface is there while it is installed and
    /// enabled, whether or not its backend is registered at the moment.
    frames: RwLock<BTreeMap<ExtId, BTreeSet<String>>>,
}

impl GrantTable {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register `ext`: what it holds and how far it is trusted. Replaces what it had.
    pub fn set(&self, ext: ExtId, caps: impl IntoIterator<Item = Capability>, trust: Trust) {
        let grant = Grant {
            caps: caps.into_iter().collect(),
            trust,
        };
        write(&self.grants).insert(ext, grant);
    }

    /// What a page of `ext`'s in a frame may call besides its own names; `None` takes its framed
    /// interface away.
    pub fn set_frame(&self, ext: &ExtId, calls: Option<BTreeSet<String>>) {
        match calls {
            Some(calls) => write(&self.frames).insert(ext.clone(), calls),
            None => write(&self.frames).remove(ext),
        };
    }

    /// Forget `ext`: it holds nothing any more. Its framed interface is [`GrantTable::set_frame`]'s.
    pub fn remove(&self, ext: &ExtId) {
        write(&self.grants).remove(ext);
    }

    /// What `ext` holds, sorted.
    pub fn capabilities(&self, ext: &ExtId) -> Vec<Capability> {
        read(&self.grants)
            .get(ext)
            .map(|g| g.caps.iter().cloned().collect())
            .unwrap_or_default()
    }
}

impl GrantBook for GrantTable {
    fn holds(&self, ext: &ExtId, cap: &Capability) -> bool {
        read(&self.grants).get(ext).is_some_and(|g| g.caps.contains(cap))
    }

    fn trust(&self, ext: &ExtId) -> Option<Trust> {
        read(&self.grants).get(ext).map(|g| g.trust.clone())
    }

    fn frame_calls(&self, ext: &ExtId) -> Option<BTreeSet<String>> {
        read(&self.frames).get(ext).cloned()
    }
}

#[cfg(test)]
mod tests {
    use harnix_contract::s1::{caps, PackageCarrier};

    use super::*;

    #[test]
    fn what_is_held_is_what_was_granted_and_the_kernel_holds_everything() {
        let table = GrantTable::new();
        let system = ExtId::native("system").unwrap();
        table.set(system.clone(), [caps::ENGINE_INSPECT], Trust::Origin);
        assert!(holds(&table, &Subject::extension(system.clone()), &caps::ENGINE_INSPECT));
        assert!(!holds(&table, &Subject::extension(system.clone()), &caps::ENGINE_CONTROL));
        assert!(holds(&table, &Subject::Kernel, &caps::ENGINE_CONTROL));
        assert!(!holds(&table, &Subject::device("d"), &caps::ENGINE_INSPECT));

        let pkg = ExtId::package("acme.notes").unwrap();
        table.set(pkg.clone(), [], Trust::consent(["harnix.send".to_string()], PackageCarrier::Wasm));
        assert!(table.trust(&pkg).unwrap().is_consent());
        assert_eq!(table.frame_calls(&pkg), None);
        table.set_frame(&pkg, Some(["harnix.sessions".to_string()].into()));
        assert!(table.frame_calls(&pkg).unwrap().contains("harnix.sessions"));
        table.remove(&pkg);
        assert_eq!(table.trust(&pkg), None);
        assert!(table.frame_calls(&pkg).is_some(), "its interface outlives its backend");
        table.set_frame(&pkg, None);
        assert_eq!(table.frame_calls(&pkg), None);
    }
}
