//! Who a row is open to.
//!
//! Data, judged elsewhere (`harnix_authz::decide`): a row says how far it is open, and the one
//! place that decides reads it. Nothing is open unless a constructor or a `with_*` says so. The
//! kernel reaches every row but what the hub owns.

use std::collections::BTreeSet;

use serde::Serialize;

use super::{Capability, ExtId};

/// Which extensions a row is open to by its own declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Reach {
    Any,
    Only(BTreeSet<ExtId>),
}

/// What a row acts on, for how far a call to it is narrowed: a package, and a page in a frame,
/// reach only their own space and its sessions.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RowScope {
    /// Nothing of a space or a session.
    #[default]
    Open,
    /// One space: listing its sessions, creating one in it.
    Space,
    /// One session.
    Session,
}

impl RowScope {
    pub fn as_str(self) -> &'static str {
        match self {
            RowScope::Open => "open",
            RowScope::Space => "space",
            RowScope::Session => "session",
        }
    }
}

/// How far one row is open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Access {
    device: bool,
    extensions: Reach,
    /// What an extension must hold besides being let in. A device holds nothing.
    requires: Option<Capability>,
    /// Devices are let in whatever an extension must hold: what a person decides about their own
    /// session, which an extension does only when granted.
    device_free: bool,
    scope: RowScope,
    /// Why it is refused to whoever it is not open to, when that is on purpose.
    refused: Option<&'static str>,
    /// Refused to everyone, the kernel included, whatever is held: the hub's own.
    hub_owned: bool,
}

impl Access {
    fn closed() -> Self {
        Self {
            device: false,
            extensions: Reach::Only(BTreeSet::new()),
            requires: None,
            device_free: false,
            scope: RowScope::Open,
            refused: None,
            hub_owned: false,
        }
    }

    /// The kernel alone, until a `with_*` says otherwise.
    pub fn kernel() -> Self {
        Self::closed()
    }

    /// Any device, and the kernel: a person, through an interface. No extension, acting as
    /// itself, is let in unless a `with_*` says so — what a person decides is theirs alone.
    pub fn device() -> Self {
        Self::closed().with_device()
    }

    /// Any extension, and the kernel.
    pub fn extensions() -> Self {
        Self::closed().with_extensions()
    }

    /// One extension, by id, and the kernel: for the rare row one extension serves another alone.
    pub fn only(id: ExtId) -> Self {
        Self::closed().with_extension(id)
    }

    /// The kernel, and an extension holding `cap`. Never a device.
    pub fn capability(cap: Capability) -> Self {
        Self::extensions().requiring(cap)
    }

    /// Any device, the kernel, and an extension holding `cap`: a person decides it for themselves,
    /// an extension only when granted.
    pub fn device_or_capability(cap: Capability) -> Self {
        Self { device_free: true, ..Self::capability(cap).with_device() }
    }

    /// The kernel alone, with a reason everyone else is told.
    pub fn refused(why: &'static str) -> Self {
        Self::closed().refusing(why)
    }

    /// Nobody, the kernel included, and nothing held opens it: the hub owns it, and reaching it
    /// would be a way around the hub.
    pub fn hub_owned(why: &'static str) -> Self {
        Self {
            hub_owned: true,
            ..Self::refused(why)
        }
    }

    pub fn with_device(mut self) -> Self {
        self.device = true;
        self
    }

    pub fn with_extensions(mut self) -> Self {
        self.extensions = Reach::Any;
        self
    }

    /// Open to one more extension, by id.
    pub fn with_extension(mut self, id: ExtId) -> Self {
        if let Reach::Only(ids) = &mut self.extensions {
            ids.insert(id);
        }
        self
    }

    pub fn requiring(mut self, cap: Capability) -> Self {
        self.requires = Some(cap);
        self
    }

    pub fn refusing(mut self, why: &'static str) -> Self {
        self.refused = Some(why);
        self
    }

    pub fn scoped(mut self, scope: RowScope) -> Self {
        self.scope = scope;
        self
    }

    pub fn opens_devices(&self) -> bool {
        self.device && (self.requires.is_none() || self.device_free)
    }

    /// Whether every extension is let in by the row's own declaration.
    pub fn opens_every_extension(&self) -> bool {
        self.extensions == Reach::Any
    }

    /// Whether `ext` is let in by the row's declaration. Not whether it may call: a capability the
    /// row requires, and a row the hub owns, are the deciding layer's to apply.
    pub fn opens_extension(&self, ext: &ExtId) -> bool {
        match &self.extensions {
            Reach::Any => true,
            Reach::Only(ids) => ids.contains(ext),
        }
    }

    pub fn requirement(&self) -> Option<&Capability> {
        self.requires.as_ref()
    }

    pub fn scope(&self) -> RowScope {
        self.scope
    }

    pub fn refusal(&self) -> Option<&'static str> {
        self.refused
    }

    pub fn is_hub_owned(&self) -> bool {
        self.hub_owned
    }

    /// Every extension let in by declaration, sorted: `*` for every extension.
    pub fn extensions_listed(&self) -> Vec<String> {
        match &self.extensions {
            Reach::Any => vec!["*".into()],
            Reach::Only(ids) => ids.iter().map(ToString::to_string).collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::s1::caps;

    #[test]
    fn a_capability_row_is_never_a_devices() {
        let row = Access::capability(caps::ENGINE_INSPECT);
        assert!(!row.opens_devices());
        assert!(row.opens_every_extension());
        assert_eq!(row.requirement(), Some(&caps::ENGINE_INSPECT));
        assert!(!Access::device().requiring(caps::ENGINE_EXPORT).opens_devices());
        assert_eq!(Access::device().scope(), RowScope::Open);
        // Unless the row says a device decides it for itself.
        let either = Access::device_or_capability(caps::ENGINE_CONTROL);
        assert!(either.opens_devices() && either.opens_every_extension());
        assert_eq!(either.requirement(), Some(&caps::ENGINE_CONTROL));
        // A device's row is no extension's unless it says so.
        assert!(Access::device().opens_devices());
        assert!(!Access::device().opens_every_extension());
        assert!(Access::device().with_extensions().opens_every_extension());
        assert_eq!(Access::device().scoped(RowScope::Session).scope().as_str(), "session");
    }
}
