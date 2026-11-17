//! A space, as an extension declares it (concept §5.4): an id, the instance its new sessions land
//! on, and the scene they start in. What a space looks like is the interface's; which sessions
//! belong to it is the hub's ledger. The kernel knows no space by name: every one is an extension's.

use serde::{Deserialize, Serialize};

/// Where a space's new sessions land when the caller names no instance.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case", tag = "kind", content = "id")]
pub enum DefaultInstance {
    /// A fixed instance, `main` unless said otherwise.
    Fixed(String),
    /// Built from this template the first time the space needs it, then reused; the instance
    /// belongs to the space.
    Template(String),
}

impl Default for DefaultInstance {
    fn default() -> Self {
        DefaultInstance::Fixed(crate::MAIN_INSTANCE.to_string())
    }
}

impl DefaultInstance {
    /// `main`, `ci`, `template:research`.
    pub fn parse(text: &str) -> Self {
        match text.strip_prefix("template:") {
            Some(id) => DefaultInstance::Template(id.to_string()),
            None => DefaultInstance::Fixed(text.to_string()),
        }
    }
}

/// The text [`DefaultInstance::parse`] reads: `main`, `template:research`.
impl std::fmt::Display for DefaultInstance {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DefaultInstance::Fixed(id) => f.write_str(id),
            DefaultInstance::Template(template) => write!(f, "template:{template}"),
        }
    }
}

/// A space an extension declares: built with [`SpaceDecl::new`] and its builders; the fields are
/// read by the hub.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct SpaceDecl {
    pub id: String,
    #[serde(default)]
    pub instance: DefaultInstance,
    /// The scene a new session starts in; the instance's default when absent.
    #[serde(default)]
    pub scene: Option<String>,
    /// A device's call claiming this space may watch and act on a session of any space; each
    /// time it reaches across one, that is audited. A guard against mistakes, like every claim —
    /// the power to read or change another space's sessions is a capability's — and a native
    /// extension's to declare only.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub cross_space: bool,
    /// The space a device's call that claims none is checked as, and where a session with no
    /// recorded space belongs. At most one in a process.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub default_space: bool,
    /// A page of its own, an overview with no conversation: no session is created, imported or
    /// moved into it, and it lists none.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub no_sessions: bool,
}

impl SpaceDecl {
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            instance: DefaultInstance::default(),
            scene: None,
            cross_space: false,
            default_space: false,
            no_sessions: false,
        }
    }

    pub fn instance(mut self, instance: &str) -> Self {
        self.instance = DefaultInstance::parse(instance);
        self
    }

    pub fn scene(mut self, scene: impl Into<String>) -> Self {
        self.scene = Some(scene.into());
        self
    }

    /// See [`SpaceDecl::cross_space`](#structfield.cross_space).
    pub fn cross_space(mut self) -> Self {
        self.cross_space = true;
        self
    }

    /// See [`SpaceDecl::default_space`](#structfield.default_space).
    pub fn default_space(mut self) -> Self {
        self.default_space = true;
        self
    }

    /// See [`SpaceDecl::no_sessions`](#structfield.no_sessions).
    pub fn no_sessions(mut self) -> Self {
        self.no_sessions = true;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_default_instance_prints_as_it_is_parsed() {
        for text in ["main", "ci", "template:research"] {
            assert_eq!(DefaultInstance::parse(text).to_string(), text);
        }
    }
}
