//! What an earlier release left in an engine's data root.

use std::path::Path;

/// What marked a skill a package copied into an instance's skills directory, before skills were
/// listed to sessions from the package's own directory.
const SKILL_MARK: &str = ".harnix-app";

/// Remove the skills a package copied into `data_root`'s skills directory: left there they are
/// seen in every session, and shadow a listed skill of the same name. Only a subdirectory with
/// the mark goes; a skill of the user's stays. Done before the engine reads the directory, and
/// harmless to do again.
pub fn strip_package_skills(data_root: &Path) {
    let dir = data_root.join("skills");
    let Ok(entries) = std::fs::read_dir(&dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.join(SKILL_MARK).is_file() {
            if let Err(e) = std::fs::remove_dir_all(&path) {
                tracing::warn!(path = %path.display(), error = %e, "a skill an earlier release copied in was not removed");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_marked_skills_go() {
        let root = tempfile::tempdir().unwrap();
        let skills = root.path().join("skills");
        for (name, marked) in [("theirs", true), ("mine", false)] {
            std::fs::create_dir_all(skills.join(name)).unwrap();
            std::fs::write(skills.join(name).join("SKILL.md"), "x").unwrap();
            if marked {
                std::fs::write(skills.join(name).join(SKILL_MARK), "acme").unwrap();
            }
        }
        strip_package_skills(root.path());
        assert!(!skills.join("theirs").exists());
        assert!(skills.join("mine").join("SKILL.md").exists());
        strip_package_skills(root.path());
        strip_package_skills(&root.path().join("nowhere"));
    }
}
