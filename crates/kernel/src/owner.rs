//! Who owns a name: what installs it, answers it and takes it back out.

use harnix_contract::ExtId;

/// The part of the skeleton that owns a kernel row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum KernelPart {
    Hub,
    Instances,
    Assembly,
    Router,
}

/// A row's owner. Rows are installed and withdrawn by owner, all of one owner's at once.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Owner {
    Kernel(KernelPart),
    /// What passes through to the engine under its own name.
    Engine,
    /// An extension, by id: a compiled-in module, or a runtime application.
    Extension(ExtId),
}

impl Owner {
    pub fn extension(id: ExtId) -> Self {
        Owner::Extension(id)
    }

    /// The extension's id, when an extension owns it.
    pub fn extension_id(&self) -> Option<&ExtId> {
        match self {
            Owner::Extension(id) => Some(id),
            _ => None,
        }
    }

    /// How the method table names it. The assembly's and the router's rows go by the names the
    /// table has always been published under (`kernel:app`, `kernel:authz`), so that what a
    /// client was told about a row does not move when the code behind it does.
    pub fn label(&self) -> String {
        match self {
            Owner::Kernel(KernelPart::Hub) => "kernel:hub".into(),
            Owner::Kernel(KernelPart::Instances) => "kernel:instances".into(),
            Owner::Kernel(KernelPart::Assembly) => "kernel:app".into(),
            Owner::Kernel(KernelPart::Router) => "kernel:authz".into(),
            Owner::Engine => "engine".into(),
            Owner::Extension(id) => format!("ext:{id}"),
        }
    }
}
