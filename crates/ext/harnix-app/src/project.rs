//! A session's project, read-only: the right column's files tab and its preview.
//!
//! Fenced in the session's own project root, as the engine has it — not `$HOME`, not the projects
//! directory: the files tab shows what the conversation works on. A path is relative to that root
//! and may not climb out of it (`..`, an absolute path), and what it names is resolved through
//! every symlink before it is read, so a link in the project pointing at `/etc` is no way out.
//! The caller claims the session like any session call, and `engine.session.get` with that claim
//! is what checks it may see it.

use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};

use harnix_ext::{codes, Call, RpcError, MAIN_INSTANCE};
use serde_json::{json, Value};

use crate::{require_session_id, require_str, App};

/// Directories a listing leaves out unless asked for all: heavy, generated, or the VCS's own.
const HEAVY: &[&str] = &[".git", "node_modules", "target"];
/// The most entries one listing answers; the rest are counted.
const MAX_ENTRIES: usize = 2000;
/// The largest text a read answers whole; past it, the preview says so instead.
pub(crate) const MAX_TEXT_BYTES: u64 = 1024 * 1024;
/// The largest image a read answers (base64 makes it a third bigger on the wire).
pub(crate) const MAX_IMAGE_BYTES: u64 = 5 * 1024 * 1024;
/// How much of a file is looked at to tell text from binary.
const SNIFF_BYTES: usize = 8192;

impl App {
    /// `files.list {session_id, path?, all?}`: one directory of the session's project.
    pub(crate) async fn files_list(&self, call: &Call, params: Value) -> Result<Value, RpcError> {
        let root = session_root(call, &params).await?;
        let path = params.get("path").and_then(Value::as_str).unwrap_or("").to_string();
        let all = params.get("all").and_then(Value::as_bool).unwrap_or(false);
        tokio::task::spawn_blocking(move || {
            let dir = within(&root, &path)?;
            if !dir.is_dir() {
                return Err(RpcError::invalid_params(format!("not a directory: {path}")));
            }
            Ok(listing(&root, &dir, all))
        })
        .await
        .map_err(|e| RpcError::new(codes::INTERNAL_ERROR, e.to_string()))?
    }

    /// `files.read {session_id, path}`: one file of the session's project, as a preview takes it.
    pub(crate) async fn files_read(&self, call: &Call, params: Value) -> Result<Value, RpcError> {
        let root = session_root(call, &params).await?;
        let path = require_str(&params, "path")?;
        tokio::task::spawn_blocking(move || {
            let file = within(&root, &path)?;
            if !file.is_file() {
                return Err(RpcError::invalid_params(format!("not a file: {path}")));
            }
            read(&root, &file)
        })
        .await
        .map_err(|e| RpcError::new(codes::INTERNAL_ERROR, e.to_string()))?
    }
}

/// The session's project root, canonical: what the reads are fenced in.
async fn session_root(call: &Call, params: &Value) -> Result<PathBuf, RpcError> {
    let session_id = require_session_id(params)?;
    let instance = params.get("instance").cloned().unwrap_or(json!(MAIN_INSTANCE));
    let mut target = json!({"instance": instance, "session_id": session_id});
    if let Some(space) = params.get("space").filter(|s| !s.is_null()) {
        target["space"] = space.clone();
    }
    let info = call.call("engine.session.get", target).await?;
    let root = info
        .get("project_root")
        .and_then(Value::as_str)
        .filter(|r| !r.is_empty())
        .ok_or_else(|| RpcError::invalid_params("the session runs in no project"))?;
    Path::new(root)
        .canonicalize()
        .map_err(|e| RpcError::invalid_params(format!("the session's project cannot be read: {e}")))
}

/// `path` inside `root`, resolved. Relative and climbing nowhere, by its words alone first — so a
/// path outside is refused the same whether or not it exists — then by where its links lead.
pub(crate) fn within(root: &Path, path: &str) -> Result<PathBuf, RpcError> {
    let outside = || RpcError::new(codes::REFUSED, format!("outside the session's project: {path}"));
    let given = Path::new(path);
    if given.components().any(|c| !matches!(c, Component::Normal(_) | Component::CurDir)) {
        return Err(outside());
    }
    let real = root
        .join(given)
        .canonicalize()
        .map_err(|_| RpcError::missing(format!("no such file in the project: {path}")))?;
    if !real.starts_with(root) {
        return Err(outside());
    }
    Ok(real)
}

/// A path relative to the root, `/`-separated, as the interface addresses it.
fn relative(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .map(|p| p.components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/"))
        .unwrap_or_default()
}

/// One directory's entries, directories first. Unless `all`, what `.gitignore` leaves out and the
/// heavy directories are not listed, only counted (`hidden`).
pub(crate) fn listing(root: &Path, dir: &Path, all: bool) -> Value {
    let kept: Option<HashSet<std::ffi::OsString>> = (!all).then(|| {
        ignore::WalkBuilder::new(dir)
            .max_depth(Some(1))
            .hidden(false)
            .parents(true)
            .require_git(false)
            .build()
            .flatten()
            .filter(|e| e.depth() == 1)
            .map(|e| e.file_name().to_os_string())
            .collect()
    });
    let mut entries: Vec<(bool, String, Value)> = Vec::new();
    let mut hidden = 0usize;
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let name = entry.file_name();
        let shown = name.to_string_lossy().to_string();
        if let Some(kept) = &kept {
            if HEAVY.contains(&shown.as_str()) || !kept.contains(&name) {
                hidden += 1;
                continue;
            }
        }
        let link = entry.file_type().map(|t| t.is_symlink()).unwrap_or(false);
        // Followed, so a link to a directory opens like one; where it leads is checked on opening.
        let meta = entry.path().metadata().ok();
        let is_dir = meta.as_ref().is_some_and(|m| m.is_dir());
        let mut row = json!({"name": shown, "path": relative(root, &dir.join(&name)), "dir": is_dir});
        if link {
            row["link"] = json!(true);
        }
        if let Some(m) = meta.filter(|m| m.is_file()) {
            row["size"] = json!(m.len());
        }
        entries.push((!is_dir, shown.to_lowercase(), row));
    }
    entries.sort_by(|a, b| (a.0, &a.1).cmp(&(b.0, &b.1)));
    let truncated = entries.len() > MAX_ENTRIES;
    entries.truncate(MAX_ENTRIES);
    json!({
        "root": root.display().to_string(),
        "path": relative(root, dir),
        "entries": entries.into_iter().map(|(_, _, row)| row).collect::<Vec<_>>(),
        "hidden": hidden,
        "truncated": truncated,
    })
}

/// The media type of an image a preview shows, by its extension.
fn image_type(path: &Path) -> Option<&'static str> {
    let ext = path.extension()?.to_string_lossy().to_lowercase();
    Some(match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "ico" => "image/x-icon",
        // Drawn as an `<img>`, where its scripts do not run.
        "svg" => "image/svg+xml",
        _ => return None,
    })
}

/// A file as a preview takes it: `text`, an `image` (base64), or — when it is neither, or too
/// large to send — `binary` or `too_large`, with its size, for the preview to say so.
pub(crate) fn read(root: &Path, file: &Path) -> Result<Value, RpcError> {
    let unreadable = |e: std::io::Error| RpcError::new(codes::INTERNAL_ERROR, format!("unreadable: {e}"));
    let size = file.metadata().map_err(unreadable)?.len();
    let mut answer = json!({"path": relative(root, file), "size": size});
    if let Some(mime) = image_type(file) {
        if size > MAX_IMAGE_BYTES {
            answer["kind"] = json!("too_large");
        } else {
            use base64::Engine as _;
            let bytes = std::fs::read(file).map_err(unreadable)?;
            answer["kind"] = json!("image");
            answer["mime"] = json!(mime);
            answer["data"] = json!(base64::engine::general_purpose::STANDARD.encode(&bytes));
        }
        return Ok(answer);
    }
    if size > MAX_TEXT_BYTES {
        answer["kind"] = json!("too_large");
        return Ok(answer);
    }
    let bytes = std::fs::read(file).map_err(unreadable)?;
    let looks_binary = bytes[..bytes.len().min(SNIFF_BYTES)].contains(&0);
    match String::from_utf8(bytes) {
        Ok(text) if !looks_binary => {
            answer["kind"] = json!("text");
            answer["text"] = json!(text);
        }
        _ => answer["kind"] = json!("binary"),
    }
    Ok(answer)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::create_dir_all(root.join("node_modules/x")).unwrap();
        std::fs::create_dir_all(root.join("target")).unwrap();
        std::fs::create_dir_all(root.join(".git")).unwrap();
        std::fs::create_dir_all(root.join("dist")).unwrap();
        std::fs::write(root.join(".gitignore"), "dist/\n*.log\n").unwrap();
        std::fs::write(root.join("README.md"), "# hi\n").unwrap();
        std::fs::write(root.join("run.log"), "x").unwrap();
        std::fs::write(root.join("src/a.rs"), "fn main() {}\n").unwrap();
        (dir, root)
    }

    fn names(listed: &Value) -> Vec<String> {
        listed["entries"].as_array().unwrap().iter().map(|e| e["name"].as_str().unwrap().to_string()).collect()
    }

    #[test]
    fn a_path_may_not_climb_out_of_the_project_whether_or_not_it_exists() {
        let (_d, root) = project();
        assert!(within(&root, "src/a.rs").is_ok());
        assert!(within(&root, "").is_ok());
        for bad in ["../x", "src/../../x", "/etc/passwd", "../../../../etc/passwd"] {
            let e = within(&root, bad).unwrap_err();
            assert_eq!(e.code, codes::REFUSED, "{bad}");
        }
        assert_eq!(within(&root, "nope.txt").unwrap_err().code, codes::NOT_FOUND);
    }

    #[cfg(unix)]
    #[test]
    fn a_link_out_of_the_project_is_no_way_out() {
        let (_d, root) = project();
        let elsewhere = tempfile::tempdir().unwrap();
        std::fs::write(elsewhere.path().join("secret"), "s").unwrap();
        std::os::unix::fs::symlink(elsewhere.path(), root.join("out")).unwrap();
        std::os::unix::fs::symlink(root.join("src"), root.join("in")).unwrap();
        assert_eq!(within(&root, "out/secret").unwrap_err().code, codes::REFUSED);
        assert_eq!(within(&root, "out").unwrap_err().code, codes::REFUSED);
        assert!(within(&root, "in/a.rs").is_ok());
        let listed = listing(&root, &root, true);
        let out = listed["entries"].as_array().unwrap().iter().find(|e| e["name"] == "out").unwrap();
        assert_eq!((out["link"].as_bool(), out["dir"].as_bool()), (Some(true), Some(true)));
    }

    #[test]
    fn a_listing_hides_the_heavy_and_the_ignored_unless_asked_for_all() {
        let (_d, root) = project();
        let listed = listing(&root, &root, false);
        assert_eq!(names(&listed), ["src", ".gitignore", "README.md"]);
        assert_eq!(listed["hidden"], 5);
        assert_eq!(listed["path"], "");
        let all = listing(&root, &root, true);
        assert_eq!(names(&all), [".git", "dist", "node_modules", "src", "target", ".gitignore", "README.md", "run.log"]);
        assert_eq!(all["hidden"], 0);
        let src = listing(&root, &root.join("src"), false);
        assert_eq!(src["path"], "src");
        assert_eq!(src["entries"][0], json!({"name": "a.rs", "path": "src/a.rs", "dir": false, "size": 13}));
    }

    #[test]
    fn a_read_is_text_an_image_or_says_why_it_is_not() {
        let (_d, root) = project();
        let text = read(&root, &root.join("src/a.rs")).unwrap();
        assert_eq!((text["kind"].as_str(), text["text"].as_str(), text["path"].as_str()), (Some("text"), Some("fn main() {}\n"), Some("src/a.rs")));
        std::fs::write(root.join("blob.dat"), [0u8, 1, 2, 3]).unwrap();
        assert_eq!(read(&root, &root.join("blob.dat")).unwrap()["kind"], "binary");
        std::fs::write(root.join("p.png"), [137u8, 80, 78, 71]).unwrap();
        let image = read(&root, &root.join("p.png")).unwrap();
        assert_eq!((image["kind"].as_str(), image["mime"].as_str(), image["data"].as_str()), (Some("image"), Some("image/png"), Some("iVBORw==")));
        let big = std::fs::File::create(root.join("big.txt")).unwrap();
        big.set_len(MAX_TEXT_BYTES + 1).unwrap();
        let too = read(&root, &root.join("big.txt")).unwrap();
        assert_eq!((too["kind"].as_str(), too["size"].as_u64()), (Some("too_large"), Some(MAX_TEXT_BYTES + 1)));
        assert!(too.get("text").is_none());
    }
}
