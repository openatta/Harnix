//! Directories and projects: the file-facing methods.

use std::path::{Path, PathBuf};

use harnix_ext::{codes, Call, RpcError};
use serde_json::{json, Value};

use crate::project::session_root;
use crate::store::{new_workspace_id, Workspace};
use crate::{home_dir, require_str, App};

impl App {
    /// Files under a project root, for the composer's `@` menu.
    ///
    /// The root is a session's project (`session_id`, claimed like any session call), or one the
    /// caller names that is a project here — added as one, or where a listed session runs. Any
    /// other directory is refused: a mention menu that walks wherever it is pointed is a file
    /// browser for the whole disk. A page in a frame names a session.
    ///
    /// Walks with `ignore`, so `.gitignore` and friends apply — a mention menu
    /// full of `node_modules` is a mention menu nobody uses. Bounded by a
    /// result cap and a file cap, and matched on the path as typed (substring,
    /// case-insensitive) rather than fuzzily: a wrong file quietly attached is
    /// worse than one more keystroke.
    pub(crate) async fn files(&self, call: &Call, params: Value) -> Result<Value, RpcError> {
        const MAX_RESULTS: usize = 30;

        let root = match asked(&params, call.on_behalf_of().is_some())? {
            Asked::Session => session_root(call, &params).await?,
            Asked::Project(root) => {
                self.must_see(call, &root).await?;
                Path::new(&root)
                    .canonicalize()
                    .map_err(|e| RpcError::invalid_params(format!("the project cannot be read: {e}")))?
            }
        };
        let query = params
            .get("query")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_lowercase();
        let limit = params
            .get("limit")
            .and_then(Value::as_u64)
            .unwrap_or(MAX_RESULTS as u64)
            .min(MAX_RESULTS as u64) as usize;

        let root_display = root.display().to_string();
        let files = tokio::task::spawn_blocking(move || walk(&root, &query, limit))
            .await
            .map_err(|e| RpcError::new(codes::INTERNAL_ERROR, e.to_string()))?;

        Ok(json!({
            "project_root": root_display,
            "files": files
                .into_iter()
                .map(|(path, size)| json!({"path": path, "size": size}))
                .collect::<Vec<_>>(),
        }))
    }

    /// One directory level, for the project picker.
    ///
    /// Opens on the projects root. Browsing is allowed inside `$HOME` **or**
    /// inside the projects root — the operator named that root, so it is
    /// trusted even when it sits outside the home directory (a deployment
    /// keeping projects on another volume is the ordinary case). Anything
    /// else is refused: a browser asking for `/` is what the fence is for.
    pub(crate) async fn list_directory(&self, params: Value) -> Result<Value, RpcError> {
        let home =
            home_dir().ok_or_else(|| RpcError::new(codes::INTERNAL_ERROR, "no home directory"))?;
        // Compared with canonical paths below, so canonical itself (`/var` is `/private/var`).
        let home = home.canonicalize().unwrap_or(home);
        let target = match params.get("path").and_then(Value::as_str) {
            Some(p) if !p.is_empty() => PathBuf::from(p),
            // The projects directory, made here if no project has made it yet: a picker that
            // opened on home before the first project would start somewhere projects are not.
            // Home only when it cannot be made.
            _ if std::fs::create_dir_all(&self.projects_root).is_ok() => self.projects_root.clone(),
            _ => home.clone(),
        };
        let projects = self
            .projects_root
            .canonicalize()
            .unwrap_or_else(|_| self.projects_root.clone());
        let target = fenced(&target, &home, &projects)?;
        let mut entries: Vec<Value> = Vec::new();
        let read = std::fs::read_dir(&target)
            .map_err(|e| RpcError::new(codes::INVALID_PARAMS, format!("unreadable: {e}")))?;
        for entry in read.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
            if !is_dir {
                continue; // a project root is a directory; files are noise here
            }
            entries.push(json!({"name": name, "hidden": name.starts_with('.')}));
        }
        entries.sort_by(|a, b| {
            a["name"]
                .as_str()
                .unwrap_or("")
                .cmp(b["name"].as_str().unwrap_or(""))
        });
        let anchor = if target.starts_with(&projects) && !projects.starts_with(&home) {
            projects.as_path()
        } else {
            home.as_path()
        };
        let mut breadcrumbs: Vec<Value> = Vec::new();
        let mut walk = target.as_path();
        loop {
            breadcrumbs.push(json!({
                "name": walk.file_name().map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| walk.display().to_string()),
                "path": walk.display().to_string(),
            }));
            if walk == anchor {
                break;
            }
            match walk.parent() {
                Some(p) if p.starts_with(anchor) || p == anchor => walk = p,
                _ => break,
            }
        }
        breadcrumbs.reverse();
        Ok(json!({
            "path": target.display().to_string(),
            "home": home.display().to_string(),
            "projects_root": self.projects_root.display().to_string(),
            "entries": entries,
            "breadcrumbs": breadcrumbs,
        }))
    }

    /// Create a project directory under the projects root.
    ///
    /// One segment, no separators: this makes a project, it does not place a
    /// directory anywhere the caller names. The workspace is registered in the
    /// same call, because a project the sidebar cannot show is not one the
    /// user asked for.
    pub(crate) async fn projects_create(&self, params: Value) -> Result<Value, RpcError> {
        let name = require_str(&params, "name")?.trim().to_string();
        let invalid = name.is_empty()
            || name.contains('/')
            || name.contains('\\')
            || name.starts_with('.')
            || name == ".."
            || name.len() > 128;
        if invalid {
            return Err(RpcError::new(
                codes::INVALID_PARAMS,
                "a project name is one path segment, not a path",
            ));
        }
        let path = self.projects_root.join(&name);
        if path.exists() {
            return Err(RpcError::new(
                codes::INVALID_PARAMS,
                format!("already exists: {}", path.display()),
            ));
        }
        std::fs::create_dir_all(&path)
            .map_err(|e| RpcError::new(codes::INTERNAL_ERROR, format!("could not create: {e}")))?;

        let display = path.display().to_string();
        let mut store = self.store.lock().await;
        let workspace = store.update(|state| {
            state.workspaces.push(Workspace {
                id: new_workspace_id(),
                path: Some(display.clone()),
                title: name.clone(),
                collapsed: false,
                session_order: Vec::new(),
            });
            state.workspaces.last().cloned()
        });
        Ok(json!({"path": display, "workspace": workspace}))
    }

    /// Projects that already have sessions, most recently active first — the
    /// list a new-session dialog should offer before it offers a file browser.
    pub(crate) async fn recent_projects(&self, call: &Call) -> Result<Value, RpcError> {
        let listed = call.call("engine.session.list", json!({"status": "all", "limit": 200})).await?;
        let mut seen: Vec<(String, String)> = Vec::new();
        if let Some(sessions) = listed.get("sessions").and_then(Value::as_array) {
            for s in sessions {
                let Some(root) = s.get("project_root").and_then(Value::as_str) else {
                    continue;
                };
                let last = s
                    .get("last_active")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                match seen.iter_mut().find(|(r, _)| r == root) {
                    Some((_, at)) if *at < last => *at = last,
                    Some(_) => {}
                    None => seen.push((root.to_string(), last)),
                }
            }
        }
        seen.sort_by(|a, b| b.1.cmp(&a.1));
        let cwd = std::env::current_dir()
            .ok()
            .map(|p| p.display().to_string());
        Ok(json!({
            "cwd": cwd,
            "projects_root": self.projects_root.display().to_string(),
            "projects": seen.into_iter()
                .map(|(root, at)| json!({"project_root": root, "last_active": at}))
                .collect::<Vec<_>>(),
        }))
    }
}

/// Which root a `files` call asks for, before it is checked.
#[derive(Debug, PartialEq, Eq)]
enum Asked {
    /// The project of the session in `session_id`.
    Session,
    /// A directory named in `project_root`: only a project here.
    Project(String),
}

/// `session_id` when there is one, whatever else is said; else `project_root`, but not from a page
/// in a frame, which reaches a project only through a session of its own space.
fn asked(params: &Value, framed: bool) -> Result<Asked, RpcError> {
    if params.get("session_id").is_some_and(|v| !v.is_null()) {
        return Ok(Asked::Session);
    }
    match params.get("project_root").and_then(Value::as_str).filter(|r| !r.is_empty()) {
        Some(_) if framed => Err(RpcError::refused("a page in a frame names a session, not a directory")),
        Some(root) => Ok(Asked::Project(root.to_string())),
        None => Err(RpcError::invalid_params("missing session_id or project_root")),
    }
}

/// The files under `root` whose path holds `query`, at most `limit`, shallow ones first. Links are
/// not followed, so a link in the project to elsewhere offers nothing from there.
fn walk(root: &Path, query: &str, limit: usize) -> Vec<(String, u64)> {
    const MAX_VISITED: usize = 20_000;
    let mut out: Vec<(String, u64)> = Vec::new();
    let mut visited = 0usize;
    for entry in ignore::WalkBuilder::new(root).hidden(true).follow_links(false).build().flatten() {
        visited += 1;
        if visited > MAX_VISITED || out.len() >= limit {
            break;
        }
        if !entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
            continue;
        }
        let Ok(relative) = entry.path().strip_prefix(root) else {
            continue;
        };
        let path = relative.to_string_lossy().to_string();
        if !query.is_empty() && !path.to_lowercase().contains(query) {
            continue;
        }
        let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
        out.push((path, size));
    }
    // Shallow paths first: the file someone means is rarely the deepest.
    out.sort_by_key(|(path, _)| (path.matches('/').count(), path.len()));
    out
}

/// The picker's `target`, resolved through `..` and every link, when that lands in `home` or in
/// `projects` (both canonical). One that cannot be resolved is not listed: compared as written,
/// `<home>/../x` would pass as inside home.
fn fenced(target: &Path, home: &Path, projects: &Path) -> Result<PathBuf, RpcError> {
    let real = target
        .canonicalize()
        .map_err(|e| RpcError::new(codes::INVALID_PARAMS, format!("unreadable: {e}")))?;
    if !real.starts_with(home) && !real.starts_with(projects) {
        return Err(RpcError::new(codes::INVALID_PARAMS, "path is outside the home and projects directories"));
    }
    Ok(real)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_menu_names_a_session_or_a_project_and_nothing_else() {
        assert_eq!(asked(&json!({"session_id": "s", "project_root": "/etc"}), false).unwrap(), Asked::Session);
        assert_eq!(asked(&json!({"session_id": "s"}), true).unwrap(), Asked::Session);
        assert_eq!(asked(&json!({"project_root": "/p"}), false).unwrap(), Asked::Project("/p".into()));
        // Nothing named is not the projects directory, nor wherever the process stands.
        assert_eq!(asked(&json!({"query": "x"}), false).unwrap_err().code, codes::INVALID_PARAMS);
        assert_eq!(asked(&json!({"project_root": "", "session_id": null}), false).unwrap_err().code, codes::INVALID_PARAMS);
        assert_eq!(asked(&json!({"project_root": "/p"}), true).unwrap_err().code, codes::REFUSED);
    }

    #[cfg(unix)]
    #[test]
    fn a_link_out_of_the_project_offers_nothing_from_there() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("p");
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/main.rs"), "x").unwrap();
        let elsewhere = dir.path().join("elsewhere");
        std::fs::create_dir_all(&elsewhere).unwrap();
        std::fs::write(elsewhere.join("secret.txt"), "s").unwrap();
        std::os::unix::fs::symlink(&elsewhere, root.join("out")).unwrap();
        std::os::unix::fs::symlink(elsewhere.join("secret.txt"), root.join("secret.txt")).unwrap();
        let paths: Vec<String> = walk(&root, "", 30).into_iter().map(|(p, _)| p).collect();
        assert_eq!(paths, ["src/main.rs"]);
        assert!(walk(&root, "secret", 30).is_empty());
    }

    #[test]
    fn the_picker_is_fenced_by_where_a_path_leads_not_how_it_is_written() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().canonicalize().unwrap();
        let (home, projects, outside) = (base.join("home"), base.join("projects"), base.join("outside"));
        for d in [&home, &projects, &outside] {
            std::fs::create_dir_all(d.join("sub")).unwrap();
        }
        assert_eq!(fenced(&home.join("sub"), &home, &projects).unwrap(), home.join("sub"));
        assert_eq!(fenced(&projects.join("sub"), &home, &projects).unwrap(), projects.join("sub"));
        // `..` out of the fence, to a directory there or one that is not.
        for bad in [home.join("../outside"), home.join("sub/../../outside/sub"), home.join("../nowhere"), outside.clone()] {
            assert_eq!(fenced(&bad, &home, &projects).unwrap_err().code, codes::INVALID_PARAMS, "{}", bad.display());
        }
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&outside, home.join("out")).unwrap();
            assert!(fenced(&home.join("out"), &home, &projects).is_err());
            assert!(fenced(&home.join("out/sub"), &home, &projects).is_err());
            // A link that stays inside is where it leads.
            std::os::unix::fs::symlink(projects.join("sub"), home.join("in")).unwrap();
            assert_eq!(fenced(&home.join("in"), &home, &projects).unwrap(), projects.join("sub"));
        }
    }
}
