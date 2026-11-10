//! A space, as the backend knows it (concept §5.4): an id, the instance its new sessions land on, and
//! the scene they start in. What a space looks like is the interface's; which sessions belong to it
//! is the hub's ledger.

use serde::{Deserialize, Serialize};

/// The session space: the conversations, and the projects they run in. Sessions with no recorded
/// space belong here — every one created before spaces existed.
pub const SESSIONS_SPACE: &str = "sessions";
/// The projects view: interface only. Its sessions are the session space's, listed by project.
pub const PROJECTS_SPACE: &str = "projects";

/// Where a space's new sessions land when the caller names no instance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case", tag = "kind", content = "id")]
pub enum DefaultInstance {
    /// A fixed instance, `system` unless said otherwise.
    Fixed(String),
    /// Built from this template the first time the space needs it, then reused; the instance
    /// belongs to the space.
    Template(String),
}

impl Default for DefaultInstance {
    fn default() -> Self {
        DefaultInstance::Fixed(crate::SYSTEM_INSTANCE.to_string())
    }
}

impl DefaultInstance {
    /// `system`, `ci`, `template:research`.
    pub fn parse(text: &str) -> Self {
        match text.strip_prefix("template:") {
            Some(id) => DefaultInstance::Template(id.to_string()),
            None => DefaultInstance::Fixed(text.to_string()),
        }
    }
}

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
}

impl SpaceDecl {
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            instance: DefaultInstance::default(),
            scene: None,
            cross_space: false,
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
}
