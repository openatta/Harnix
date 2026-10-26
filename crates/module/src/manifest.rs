//! What a module says about itself.

/// A module's identity and what it discloses.
///
/// `grants` are methods beyond what a device may call that this module needs
/// — the settings module writing a provider, which no device may do
/// directly. They are applied to the authorization table at registration and
/// shown on the diagnostics page: a module is trusted code, and the grant is
/// a disclosure, not a sandbox.
#[derive(Debug, Clone)]
pub struct Manifest {
    pub id: String,
    pub version: String,
    pub disclosure: String,
    pub grants: Vec<String>,
}

impl Manifest {
    pub fn new(id: impl Into<String>, version: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            version: version.into(),
            disclosure: String::new(),
            grants: Vec::new(),
        }
    }

    pub fn disclosure(mut self, text: impl Into<String>) -> Self {
        self.disclosure = text.into();
        self
    }

    pub fn grant(mut self, method: impl Into<String>) -> Self {
        self.grants.push(method.into());
        self
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
