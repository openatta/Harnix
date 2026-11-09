//! What an application's files take: the `${data}` half of its `storage_mb`.
//!
//! The quota is one number for everything the application keeps — its key-value store and the
//! files it writes under `/data` — because "16 MB of storage" is what the installer showed, not
//! "16 MB, plus whatever it writes to disk". The key-value store is accounted on every write; the
//! files are written through WASI, out of sight, so they are measured instead: when the VM starts
//! and after every call that could have written them, off the async threads. An application over
//! its quota has its key-value writes refused, and its instances get `/data` with its files
//! read-only — it may still delete them, which is how it makes room. That includes the resident
//! backend, which would otherwise keep the writable `/data` it started with: a backend whose
//! files cross the quota is replaced, as a timed-out one is, and the instance it gets has the
//! files read-only (and the same again when it is back under, to write again).

use std::path::Path;

/// More entries than this and the walk stops counting: a directory that large is over any quota a
/// manifest may ask for, and measuring it must not be the slow part of a call.
const MAX_ENTRIES: usize = 100_000;

/// Bytes in regular files under `root`, symlinks not followed. Zero when it does not exist; `None`
/// when it has more than [`MAX_ENTRIES`] entries — over the quota, whatever the bytes come to, or
/// many small files would make the walk cheaper the more of them there are.
pub(crate) fn dir_bytes(root: &Path) -> Option<u64> {
    dir_bytes_within(root, MAX_ENTRIES)
}

fn dir_bytes_within(root: &Path, max_entries: usize) -> Option<u64> {
    let mut total = 0u64;
    let mut seen = 0usize;
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            seen += 1;
            if seen > max_entries {
                return None;
            }
            let Ok(kind) = entry.file_type() else { continue };
            if kind.is_dir() {
                stack.push(entry.path());
            } else if kind.is_file() {
                total += entry.metadata().map(|m| m.len()).unwrap_or(0);
            }
        }
    }
    Some(total)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn files_are_counted_and_links_are_not_followed() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("data");
        std::fs::create_dir_all(root.join("a/b")).unwrap();
        std::fs::write(root.join("one"), vec![0u8; 100]).unwrap();
        std::fs::write(root.join("a/b/two"), vec![0u8; 50]).unwrap();
        let outside = tmp.path().join("outside");
        std::fs::write(&outside, vec![0u8; 10_000]).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&outside, root.join("link")).unwrap();
        assert_eq!(dir_bytes(&root), Some(150));
        assert_eq!(dir_bytes(&tmp.path().join("absent")), Some(0));
    }

    #[test]
    fn too_many_entries_is_no_total_rather_than_a_partial_one() {
        let tmp = tempfile::tempdir().unwrap();
        for n in 0..5 {
            std::fs::write(tmp.path().join(format!("f{n}")), b"x").unwrap();
        }
        assert_eq!(dir_bytes_within(tmp.path(), 5), Some(5));
        assert_eq!(dir_bytes_within(tmp.path(), 4), None);
    }
}
