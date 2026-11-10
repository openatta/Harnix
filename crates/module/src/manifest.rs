//! What a module says about itself.

use harnix_contract::s1::{Capability, CapabilitySpec};

/// A module's identity, what it discloses, and the privileges it needs.
///
/// Privilege is capabilities, granted by the application where it adds the module, never taken:
/// `requires` must all be granted or the process refuses to start; `optional` may be, and the
/// module does less without them; nothing else may be (a privilege granted and not asked for is
/// one nobody sees). `defines` are capabilities of its own (`<id>.<name>`) its rows may require
/// and the application may grant to other modules. All of it is on the diagnostics page.
#[derive(Debug, Clone)]
pub struct Manifest {
    pub id: String,
    pub version: String,
    pub disclosure: String,
    pub requires: Vec<Capability>,
    pub optional: Vec<Capability>,
    pub defines: Vec<CapabilitySpec>,
}

impl Manifest {
    pub fn new(id: impl Into<String>, version: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            version: version.into(),
            disclosure: String::new(),
            requires: Vec::new(),
            optional: Vec::new(),
            defines: Vec::new(),
        }
    }

    pub fn disclosure(mut self, text: impl Into<String>) -> Self {
        self.disclosure = text.into();
        self
    }

    pub fn requires(mut self, caps: impl IntoIterator<Item = Capability>) -> Self {
        self.requires.extend(caps);
        self
    }

    pub fn optional(mut self, caps: impl IntoIterator<Item = Capability>) -> Self {
        self.optional.extend(caps);
        self
    }

    /// A capability of its own, named `<its id>.<name>`.
    pub fn defines(mut self, spec: CapabilitySpec) -> Self {
        self.defines.push(spec);
        self
    }

    /// The same, as the extension interface says it.
    pub fn to_ext(&self) -> harnix_ext::Manifest {
        let mut manifest = harnix_ext::Manifest::new(&self.id, &self.version)
            .disclosure(&self.disclosure)
            .requires(self.requires.iter().cloned())
            .optional(self.optional.iter().cloned());
        for spec in &self.defines {
            manifest = manifest.defines(spec.clone());
        }
        manifest
    }
}

/// Whether an id can name a module: lowercase letters, digits and `-`,
/// starting with a letter. It becomes a method prefix, a route segment and a
/// directory name, and all three should mean the same thing.
pub(crate) fn valid_id(id: &str) -> bool {
    let mut chars = id.chars();
    matches!(chars.next(), Some('a'..='z'))
        && chars.all(|c| matches!(c, 'a'..='z' | '0'..='9' | '-'))
}

#[doc(hidden)]
pub fn check_id(id: &str) -> Result<(), String> {
    if valid_id(id) {
        Ok(())
    } else {
        Err(format!(
            "`{id}` is not a module id: lowercase letters, digits and `-`, starting with a letter"
        ))
    }
}
