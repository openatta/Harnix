//! What an extension says about itself.

use harnix_contract::s1::{Capability, CapabilitySpec};

/// An extension's identity, what it discloses, and the privileges it needs.
///
/// Privilege is capabilities, granted by the product where it adds the extension, never taken:
/// `requires` must all be granted or the process refuses to start; `optional` may be, and the
/// extension does less without them (it reads [`GrantsView`](crate::GrantsView)); nothing else
/// may be — a privilege granted and not asked for is one nobody sees. `defines` are capabilities
/// of its own, named `<its id>.<name>`, that its rows may require and the product may grant to
/// other extensions. All of it is on the diagnostics page and in `harnix.extensions`.
#[derive(Debug, Clone)]
pub struct Manifest {
    id: String,
    version: String,
    title: String,
    disclosure: String,
    requires: Vec<Capability>,
    optional: Vec<Capability>,
    defines: Vec<CapabilitySpec>,
}

impl Manifest {
    /// A native extension's id is lowercase letters, digits and `-`, starting with a letter; it
    /// names its methods (`<id>.<name>`), its routes, its directory and its profile section.
    pub fn new(id: &str, version: &str) -> Self {
        Self {
            id: id.to_string(),
            version: version.to_string(),
            title: String::new(),
            disclosure: String::new(),
            requires: Vec::new(),
            optional: Vec::new(),
            defines: Vec::new(),
        }
    }

    /// What it is called where it is listed.
    pub fn title(mut self, title: &str) -> Self {
        self.title = title.to_string();
        self
    }

    /// What it does, and on whose behalf: the paragraph the diagnostics page and a consent show.
    pub fn disclosure(mut self, text: &str) -> Self {
        self.disclosure = text.to_string();
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

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn version(&self) -> &str {
        &self.version
    }

    /// Its title; its id when it gave none.
    pub fn title_text(&self) -> &str {
        if self.title.is_empty() {
            &self.id
        } else {
            &self.title
        }
    }

    pub fn disclosure_text(&self) -> &str {
        &self.disclosure
    }

    /// What it must be granted.
    pub fn required(&self) -> &[Capability] {
        &self.requires
    }

    /// What it may be granted.
    pub fn optionals(&self) -> &[Capability] {
        &self.optional
    }

    /// What it defines.
    pub fn defined(&self) -> &[CapabilitySpec] {
        &self.defines
    }
}
