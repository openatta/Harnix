//! A space, as the backend knows it (design §8): an id, the instance its new sessions land on, and
//! the scene they start in. What a space looks like is the interface's; which sessions belong to it
//! is the hub's ledger.

use serde::{Deserialize, Serialize};

/// The session space: the workspaces and their conversations. Sessions with no recorded space
/// belong here — every one created before spaces existed.
pub const SESSIONS_SPACE: &str = "sessions";
/// The overview, with its own general session.
pub const HOME_SPACE: &str = "home";
/// The debugging face: sees and acts on every space's sessions, and every time it reaches across
/// a space that is audited (D17).
pub const SYSTEM_SPACE: &str = "system";

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
}

impl SpaceDecl {
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            instance: DefaultInstance::default(),
            scene: None,
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
}
