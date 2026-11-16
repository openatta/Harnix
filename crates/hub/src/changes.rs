//! What a turn wrote, kept in the session's log.
//!
//! The engine says which files a tool call wrote beside its result, and nowhere else: its
//! transcript keeps the words the model read, not the diffs. A page that refreshes, or opens the
//! session later, would find the changes gone. So when a turn settles the hub files them as one
//! entry (`harnix.changes` / `turn`) in the session's log, through the same seam the audit uses:
//! the engine never parses an entry, so a log with these in it loads, forks and resumes as before,
//! and an interface that does not know them skips them. The interface draws it as the changes card
//! (harnix-app.md).
//!
//! One entry per turn that wrote anything, the files merged by path in the order first written,
//! bounded: the log is read whole when a session opens, so a turn that rewrote a tree cannot make
//! every later open pay for it.

use harnix_engine_link::events::FileWrite;
use serde_json::{json, Value};

/// The namespace the entries are filed under: the hub's own, as the audit's `harnix.audit` is the
/// assembly's.
pub(crate) const NS: &str = "harnix.changes";
/// The one event in it.
pub(crate) const EVENT: &str = "turn";

/// The most files one entry lists; the rest are counted (`more`).
pub(crate) const MAX_FILES: usize = 200;
/// The most diff text one entry carries, over all its files; past it a file keeps its counts and
/// says `omitted: "too_large"`.
pub(crate) const MAX_DIFF_BYTES: usize = 512 * 1024;

/// One file, as the turn left it: its writes taken together.
struct Merged {
    path: String,
    created: bool,
    added: u64,
    removed: u64,
    bytes: Option<u64>,
    diffs: Vec<String>,
    truncated: bool,
    omitted: Option<String>,
}

/// The entry's payload for a turn that wrote `wrote`; `None` when it wrote nothing.
///
/// A file written twice is one row: created if its first write created it, the line counts
/// summed, its size the last one known, the diffs one after another (each a unified diff with its
/// own header, so read in order they replay the turn). A write without a diff leaves the file
/// without one — half a story told as the whole would be wrong.
pub(crate) fn payload(turn_id: &str, wrote: &[FileWrite]) -> Option<Value> {
    if wrote.is_empty() {
        return None;
    }
    let mut files: Vec<Merged> = Vec::new();
    for w in wrote {
        match files.iter_mut().find(|f| f.path == w.path) {
            Some(f) => {
                f.added += w.added;
                f.removed += w.removed;
                f.bytes = w.bytes.or(f.bytes);
                f.truncated |= w.truncated;
                match (&w.diff, &f.omitted) {
                    (Some(diff), None) => f.diffs.push(diff.clone()),
                    (Some(_), Some(_)) => {}
                    (None, _) => {
                        f.diffs.clear();
                        f.omitted = Some(w.omitted.clone().unwrap_or_else(|| "too_large".into()));
                    }
                }
            }
            None => files.push(Merged {
                path: w.path.clone(),
                created: w.created,
                added: w.added,
                removed: w.removed,
                bytes: w.bytes,
                diffs: w.diff.iter().cloned().collect(),
                truncated: w.truncated,
                omitted: if w.diff.is_some() { None } else { Some(w.omitted.clone().unwrap_or_else(|| "too_large".into())) },
            }),
        }
    }
    let more = files.len().saturating_sub(MAX_FILES);
    files.truncate(MAX_FILES);
    let (added, removed) = files.iter().fold((0, 0), |(a, r), f| (a + f.added, r + f.removed));
    let mut budget = MAX_DIFF_BYTES;
    let rows: Vec<Value> = files
        .into_iter()
        .map(|f| {
            let mut row = json!({
                "path": f.path,
                "op": if f.created { "create" } else { "modify" },
                "added": f.added,
                "removed": f.removed,
            });
            if let Some(bytes) = f.bytes {
                row["bytes"] = json!(bytes);
            }
            let diff = f.diffs.join("\n");
            match f.omitted {
                Some(why) => row["omitted"] = json!(why),
                None if diff.len() > budget => row["omitted"] = json!("too_large"),
                None => {
                    budget -= diff.len();
                    row["diff"] = json!(diff);
                    if f.truncated {
                        row["truncated"] = json!(true);
                    }
                }
            }
            row
        })
        .collect();
    let mut payload = json!({"turn_id": turn_id, "files": rows, "added": added, "removed": removed});
    if more > 0 {
        payload["more"] = json!(more);
    }
    Some(payload)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &str, created: bool, diff: Option<&str>) -> FileWrite {
        FileWrite::new(path, created).lines(2, 1).bytes(Some(10)).diff(diff.map(str::to_string), false, None)
    }

    #[test]
    fn a_turn_that_wrote_nothing_files_nothing() {
        assert_eq!(payload("t1", &[]), None);
    }

    #[test]
    fn a_file_written_twice_is_one_row_with_both_diffs() {
        let wrote = [write("/w/a.rs", true, Some("d1")), write("/w/b.rs", false, Some("d2")), write("/w/a.rs", false, Some("d3"))];
        let p = payload("t1", &wrote).unwrap();
        assert_eq!(p["turn_id"], "t1");
        assert_eq!((p["added"].as_u64(), p["removed"].as_u64()), (Some(6), Some(3)));
        let files = p["files"].as_array().unwrap();
        assert_eq!(files.len(), 2);
        assert_eq!(files[0], json!({"path": "/w/a.rs", "op": "create", "added": 4, "removed": 2, "bytes": 10, "diff": "d1\nd3"}));
        assert_eq!(files[1]["op"], "modify");
        assert!(p.get("more").is_none());
    }

    #[test]
    fn a_write_without_a_diff_leaves_the_file_without_one_and_says_why() {
        let binary = FileWrite::new("/w/a.bin", false).diff(None, false, Some("binary".into()));
        let p = payload("t", &[write("/w/a.bin", false, Some("d1")), binary]).unwrap();
        assert_eq!(p["files"][0]["omitted"], "binary");
        assert!(p["files"][0].get("diff").is_none());
        let redacted = FileWrite::new("/w/c", false).diff(None, false, None);
        assert_eq!(payload("t", &[redacted]).unwrap()["files"][0]["omitted"], "too_large");
    }

    #[test]
    fn the_entry_is_bounded_in_files_and_in_diff_text() {
        let big = "x".repeat(MAX_DIFF_BYTES / 2 + 1);
        let wrote: Vec<FileWrite> = (0..MAX_FILES + 5).map(|i| write(&format!("/w/{i}"), false, Some(&big))).collect();
        let p = payload("t", &wrote).unwrap();
        let files = p["files"].as_array().unwrap();
        assert_eq!((files.len(), p["more"].as_u64()), (MAX_FILES, Some(5)));
        assert!(files[0].get("diff").is_some());
        assert_eq!(files[1]["omitted"], "too_large");
        assert_eq!(files[1]["added"], 2, "a file past the budget keeps its counts");
    }
}
