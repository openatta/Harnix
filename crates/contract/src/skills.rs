//! A package's skills, as the host component that runs it lists them to the hub.

/// One package's skills directory, and the spaces it declared: what the hub lists to a session
/// as its host skill directories. No space declared: every space's.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct SkillSource {
    /// The package.
    pub ext: String,
    /// `<state>/ext/<id>/skills`: always this path, whether or not it exists yet.
    pub dir: std::path::PathBuf,
    pub spaces: Vec<String>,
}

impl SkillSource {
    pub fn new(ext: impl Into<String>, dir: impl Into<std::path::PathBuf>, spaces: Vec<String>) -> Self {
        Self { ext: ext.into(), dir: dir.into(), spaces }
    }
}
