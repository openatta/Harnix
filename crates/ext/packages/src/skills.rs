//! A package's skills, at `<state>/ext/<id>/skills/`: the directory the hub lists to the sessions
//! that see the package (`harnix_hub`'s `skills.rs`).
//!
//! The directory is written here, from the version unpacked, and is the engine's to read: it
//! watches the directory itself. So it is never replaced as a whole — what moves is what is in it.

use std::path::Path;

use harnix_host_api::SkillSource;
use harnix_package::acquire::internal;
use harnix_package::package;

use crate::records::InstallRecord;
use crate::State;

pub(crate) const SKILLS: &str = "skills";
const NEW: &str = "skills.new";
const OLD: &str = "skills.old";

type Rpc<T> = harnix_ext::Rpc<T>;

fn io(path: &Path, e: std::io::Error) -> harnix_ext::RpcError {
    internal(format!("{}: {e}", path.display()))
}

fn remove(path: &Path) -> Rpc<()> {
    match std::fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(io(path, e)),
    }
}

/// The package's skills, from the version unpacked at `root`, put at `<app_dir>/skills`. Written
/// beside it first and moved in entry by entry: a skill directory the engine watches stays the
/// same directory (the watch follows it, on Linux by its inode, so a directory renamed over it
/// would go unwatched).
pub(crate) fn sync(app_dir: &Path, root: &Path) -> Rpc<()> {
    let target = app_dir.join(SKILLS);
    let (new, old) = (app_dir.join(NEW), app_dir.join(OLD));
    // What an interrupted run left.
    remove(&new)?;
    remove(&old)?;
    let inspected = package::inspect_dir(root).map_err(|e| internal(format!("{}: {e}", root.display())))?;
    if inspected.skills.is_empty() {
        return remove(&target);
    }
    let source = root.join("agent").join(SKILLS);
    for name in &inspected.skills {
        let from = source.join(name);
        let mut files = Vec::new();
        package::walk(&from, &from, &mut files).map_err(|e| io(&from, e))?;
        // A skill that is an empty directory is still a skill directory.
        std::fs::create_dir_all(new.join(name)).map_err(|e| io(&new, e))?;
        for (relative, path) in files {
            let to = new.join(name).join(&relative);
            if let Some(parent) = to.parent() {
                std::fs::create_dir_all(parent).map_err(|e| io(parent, e))?;
            }
            std::fs::copy(&path, &to).map_err(|e| io(&to, e))?;
        }
    }
    if !target.is_dir() {
        return std::fs::rename(&new, &target).map_err(|e| io(&target, e));
    }
    std::fs::create_dir_all(&old).map_err(|e| io(&old, e))?;
    for entry in std::fs::read_dir(&target).map_err(|e| io(&target, e))?.flatten() {
        std::fs::rename(entry.path(), old.join(entry.file_name())).map_err(|e| io(&entry.path(), e))?;
    }
    for entry in std::fs::read_dir(&new).map_err(|e| io(&new, e))?.flatten() {
        std::fs::rename(entry.path(), target.join(entry.file_name())).map_err(|e| io(&entry.path(), e))?;
    }
    remove(&old)?;
    remove(&new)
}

/// Whether the package has skills here.
pub(crate) fn has(app_dir: &Path) -> bool {
    app_dir.join(SKILLS).is_dir()
}

/// The skills of the packages that are switched on, and the spaces each declared, as the hub is
/// told them: replaces the last. A package switched off lists none.
pub(crate) fn publish(state: &State) {
    let (records, _) = crate::records::read_all(&state.dir);
    let sources = records.iter().filter(|r| r.enabled).filter_map(|r| source(state, r)).collect();
    state.host.skill_sources(sources);
}

fn source(state: &State, record: &InstallRecord) -> Option<SkillSource> {
    let app_dir = state.package_dir(&record.id);
    if !has(&app_dir) {
        return None;
    }
    let manifest = harnix_package::acquire::manifest_in(&app_dir.join(&record.version)).ok()?;
    let spaces = manifest.spaces.iter().map(|s| s.id.clone()).collect();
    Some(SkillSource::new(record.id.clone(), app_dir.join(SKILLS), spaces))
}

/// At start: a package whose skills are not at their place (one installed before they were, or
/// whose last write was cut short) has them written from its files.
pub(crate) fn settle(state: &State, record: &InstallRecord) {
    let app_dir = state.package_dir(&record.id);
    let root = app_dir.join(&record.version);
    let wanted = package::inspect_dir(&root).map(|i| !i.skills.is_empty()).unwrap_or(false);
    if wanted != has(&app_dir) || app_dir.join("skills.new").exists() || app_dir.join("skills.old").exists() {
        if let Err(e) = sync(&app_dir, &root) {
            tracing::warn!(package = %record.id, error = %e.message, "the package's skills were not written");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch() -> tempfile::TempDir {
        // Not the system's temporary directory: the engine refuses skills there.
        let base = std::env::current_exe().unwrap().ancestors().nth(3).unwrap().join("tmp");
        std::fs::create_dir_all(&base).unwrap();
        tempfile::tempdir_in(base).unwrap()
    }

    fn package(root: &Path, skills: &[(&str, &str)]) {
        std::fs::create_dir_all(root).unwrap();
        std::fs::write(
            root.join("harnix-ext.toml"),
            "[app]\nid = \"acme.notes\"\nversion = \"1.0.0\"\nharnix_api = 2\ntitle = \"Notes\"\npublisher = \"acme\"\n",
        )
        .unwrap();
        std::fs::create_dir_all(root.join("agent")).unwrap();
        std::fs::write(root.join("agent/plugin.toml"), "[plugin]\nname = \"acme-notes\"\nversion = \"1.0.0\"\n").unwrap();
        for (name, text) in skills {
            let dir = root.join("agent/skills").join(name);
            std::fs::create_dir_all(dir.join("files")).unwrap();
            std::fs::write(dir.join("SKILL.md"), text).unwrap();
            std::fs::write(dir.join("files/note.txt"), "x").unwrap();
        }
    }

    fn read(dir: &Path, name: &str) -> String {
        std::fs::read_to_string(dir.join(SKILLS).join(name).join("SKILL.md")).unwrap()
    }

    #[cfg(unix)]
    fn inode(path: &Path) -> u64 {
        use std::os::unix::fs::MetadataExt;
        std::fs::metadata(path).unwrap().ino()
    }

    #[test]
    fn the_first_write_puts_the_skills_in_place() {
        let tmp = scratch();
        let (app, root) = (tmp.path().join("app"), tmp.path().join("app/1.0.0"));
        package(&root, &[("tour", "one"), ("guide", "two")]);
        sync(&app, &root).unwrap();
        assert_eq!(read(&app, "tour"), "one");
        assert!(app.join(SKILLS).join("guide/files/note.txt").is_file());
        assert!(!app.join(NEW).exists() && !app.join(OLD).exists());
    }

    #[test]
    fn an_upgrade_replaces_what_is_in_the_directory_and_not_the_directory() {
        let tmp = scratch();
        let (app, root) = (tmp.path().join("app"), tmp.path().join("app/1.0.0"));
        package(&root, &[("tour", "one"), ("gone", "bye")]);
        sync(&app, &root).unwrap();
        #[cfg(unix)]
        let before = inode(&app.join(SKILLS));
        let next = tmp.path().join("app/1.1.0");
        package(&next, &[("tour", "newer"), ("added", "new")]);
        sync(&app, &next).unwrap();
        assert_eq!(read(&app, "tour"), "newer");
        assert!(app.join(SKILLS).join("added").is_dir() && !app.join(SKILLS).join("gone").exists());
        #[cfg(unix)]
        assert_eq!(inode(&app.join(SKILLS)), before);
    }

    #[test]
    fn a_package_without_skills_has_none_and_what_was_cut_short_is_cleared() {
        let tmp = scratch();
        let (app, root) = (tmp.path().join("app"), tmp.path().join("app/1.0.0"));
        package(&root, &[("tour", "one")]);
        sync(&app, &root).unwrap();
        std::fs::create_dir_all(app.join(NEW).join("half")).unwrap();
        std::fs::create_dir_all(app.join(OLD).join("half")).unwrap();
        let bare = tmp.path().join("app/2.0.0");
        package(&bare, &[]);
        sync(&app, &bare).unwrap();
        assert!(!has(&app) && !app.join(NEW).exists() && !app.join(OLD).exists());
        // And again from the first, with the leftovers of a run that was cut short in the way.
        std::fs::create_dir_all(app.join(NEW).join("half")).unwrap();
        sync(&app, &root).unwrap();
        assert_eq!(read(&app, "tour"), "one");
        assert!(!app.join(NEW).exists());
    }
}
