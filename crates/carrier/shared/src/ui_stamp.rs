//! The watcher of a package's interface files; what the page is told of the interface itself is
//! [`crate::frames`].

use std::path::Path;

/// A stamp of everything under a package's `ui/`: each file's path, size and modification time.
/// Read on every tick for a package whose files change under it (unsigned, in developer mode), so
/// it is a watcher that needs no thread: a file saved there changes it.
pub fn ui_stamp(dir: &Path) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    let mut stack = vec![dir.to_path_buf()];
    let mut files = Vec::new();
    while let Some(at) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&at) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            match entry.metadata() {
                Ok(meta) if meta.is_dir() => stack.push(path),
                Ok(meta) => files.push((path, meta.len(), meta.modified().ok())),
                Err(_) => {}
            }
        }
    }
    files.sort();
    files.hash(&mut hasher);
    hasher.finish()
}
