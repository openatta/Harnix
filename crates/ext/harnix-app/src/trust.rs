//! Project trust: whether a project's own settings are in force — its hooks, MCP servers, looser
//! permissions, providers, agent types and skills. An untrusted project's settings can only
//! tighten; the engine says which of them wait on trust, and keeps the trust itself in its
//! global settings (`docs/harnix-app.md` §3).
//!
//! A device holds no capability: these methods are its way to the engine's `project.*`, which
//! this extension reaches with `engine.control`. It trusts only a project the device can see — one
//! added as a project here, or one a listed session runs in — so a page cannot be talked into
//! trusting any directory on the disk by naming it.
//!
//! What each answer adds to the engine's status, for every reader alike (the projects space, a
//! session's context tab, a project's own flows): `items` (each key that waits on trust, with its
//! category and the name it carries), `categories` (the kinds among them, in [`CATEGORIES`]
//! order), and `withheld` (the items not in force now: all of them while the project is
//! untrusted, none otherwise). The topic `harnix-app.trust` carries the last status read or
//! written of each project, so another page sees a change made here.

use std::collections::BTreeMap;
use std::path::PathBuf;

use harnix_ext::{Call, RpcError};
use serde_json::{json, Map, Value};

use crate::{require_session_id, require_str, App};

/// The topic each project's last known status is published on: `harnix-app.trust`.
pub(crate) const TOPIC: &str = "trust";

/// The kinds of thing trust lets in, in the order a page lists them.
pub(crate) const CATEGORIES: &[&str] =
    &["hooks", "mcp", "permissions", "sandbox", "models", "telemetry", "plugins", "agents", "skills", "other"];

/// The kind a key the engine says waits on trust is, by its first segment.
pub(crate) fn category(key: &str) -> &'static str {
    let head = key.split(['.', '/', '(']).next().unwrap_or(key);
    match head {
        "hooks_config" | "scripts" => "hooks",
        "mcp_servers" => "mcp",
        "permission_mode"
        | "allow_client_permission_override"
        | "disable_skill_shell_execution"
        | "permission_rules" => "permissions",
        "sandbox" => "sandbox",
        "providers" | "default_provider" | "task_models" | "model" => "models",
        "telemetry_url" | "recorder" | "instruction_file" => "telemetry",
        "plugins" => "plugins",
        "agents" => "agents",
        "skills" => "skills",
        _ => "other",
    }
}

/// The one thing a key names, when it names one: the server of `mcp_servers.github`, the type of
/// `agents/reviewer`, the rule of `permission_rules.allow(Bash(git:*))`.
fn named(key: &str) -> Option<String> {
    let at = key.find(['.', '/'])?;
    let rest = &key[at + 1..];
    let rest = rest.strip_prefix("allow(").and_then(|r| r.strip_suffix(')')).unwrap_or(rest);
    (!rest.is_empty()).then(|| rest.to_string())
}

/// The engine's status of a project, with what this extension adds to it (the module's head).
pub(crate) fn described(mut status: Value) -> Value {
    let keys: Vec<String> = status
        .get("requires_trust")
        .and_then(Value::as_array)
        .map(|keys| keys.iter().filter_map(Value::as_str).map(str::to_string).collect())
        .unwrap_or_default();
    let items: Vec<Value> =
        keys.iter().map(|key| json!({"key": key, "category": category(key), "name": named(key)})).collect();
    let categories: Vec<&str> =
        CATEGORIES.iter().copied().filter(|c| keys.iter().any(|key| category(key) == *c)).collect();
    let untrusted = status.get("trust").and_then(Value::as_str) == Some("untrusted");
    let withheld = if untrusted { items.clone() } else { Vec::new() };
    if let Some(object) = status.as_object_mut() {
        object.insert("items".into(), json!(items));
        object.insert("categories".into(), json!(categories));
        object.insert("withheld".into(), json!(withheld));
    }
    status
}

/// A session with no project: nothing of its own to trust.
fn no_project() -> Value {
    json!({
        "project_root": null, "trust": "none", "matched": null, "trust_source": null, "default": null,
        "requires_trust": [], "items": [], "categories": [], "withheld": [],
    })
}

/// A directory as compared here: its real path when it has one, else as written without a
/// trailing `/`. The engine keeps a trusted root resolved, so a link to a project is the project.
fn same_dir(path: &str) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| PathBuf::from(path.trim_end_matches('/')))
}

/// Whether `root` is one of `known`, as directories.
pub(crate) fn among<'a>(root: &str, known: impl IntoIterator<Item = &'a str>) -> bool {
    let wanted = same_dir(root);
    known.into_iter().any(|k| same_dir(k) == wanted)
}

/// The call's instance, when it names one: passed on so the engine asked is that one.
fn with_instance(params: &Value, mut call: Value) -> Value {
    if let Some(instance) = params.get("instance").filter(|v| v.is_string()) {
        call["instance"] = instance.clone();
    }
    call
}

impl App {
    /// `projects.status {project_root, instance?, scene?}`.
    pub(crate) async fn project_status(&self, call: &Call, params: Value) -> Result<Value, RpcError> {
        let root = require_str(&params, "project_root")?;
        self.must_see(call, &root).await?;
        let mut ask = with_instance(&params, json!({"project_root": root}));
        if let Some(scene) = params.get("scene").filter(|v| v.is_string()) {
            ask["scene"] = scene.clone();
        }
        let status = described(call.call("engine.project.status", ask).await?);
        self.heard(call, &status).await;
        Ok(status)
    }

    /// `projects.trust {project_root, trusted, instance?}`: the status once written, and
    /// `written_to`. No longer trusting a project takes away the entry that trusts this directory
    /// alone; a glob or the default may still trust it, which the answer says (`matched`).
    pub(crate) async fn project_trust(&self, call: &Call, params: Value) -> Result<Value, RpcError> {
        let root = require_str(&params, "project_root")?;
        let Some(trusted) = params.get("trusted").and_then(Value::as_bool) else {
            return Err(RpcError::invalid_params("missing trusted"));
        };
        self.must_see(call, &root).await?;
        let ask = with_instance(&params, json!({"project_root": root, "trusted": trusted}));
        let status = described(call.call("engine.project.trust", ask).await?);
        self.heard(call, &status).await;
        Ok(status)
    }

    /// `sessions.projectStatus {instance?, session_id, space?}`: the status of the project the
    /// session runs in, under its scene, with `session_id`. A session with no project answers
    /// `trust: "none"` and nothing withheld.
    pub(crate) async fn session_project_status(&self, call: &Call, params: Value) -> Result<Value, RpcError> {
        let session = require_session_id(&params)?;
        let info = call.call("engine.session.get", with_instance(&params, json!({"session_id": session}))).await?;
        let root = info.get("project_root").and_then(Value::as_str).filter(|r| !r.is_empty());
        let mut status = match root {
            None => no_project(),
            Some(root) => {
                let mut ask = with_instance(&params, json!({"project_root": root}));
                if let Some(scene) = info.get("scene").filter(|v| v.is_string()) {
                    ask["scene"] = scene.clone();
                }
                let status = described(call.call("engine.project.status", ask).await?);
                self.heard(call, &status).await;
                status
            }
        };
        status["session_id"] = json!(session);
        Ok(status)
    }

    /// Refuse a project the device cannot see: neither added here nor where a listed session runs.
    pub(crate) async fn must_see(&self, call: &Call, root: &str) -> Result<(), RpcError> {
        let added: Vec<String> = {
            let store = self.store.lock().await;
            store.state().workspaces.iter().filter_map(|w| w.path.clone()).collect()
        };
        if among(root, added.iter().map(String::as_str)) {
            return Ok(());
        }
        let listed = call.call("engine.session.list", json!({"status": "all", "limit": 200})).await?;
        let roots: Vec<&str> = listed
            .get("sessions")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|s| s.get("project_root").and_then(Value::as_str))
            .collect();
        if among(root, roots) {
            return Ok(());
        }
        Err(RpcError::refused(format!("`{root}` is not a project here: add it as a project first")))
    }

    /// Keep a project's status as last heard, and publish the lot when it changed.
    async fn heard(&self, call: &Call, status: &Value) {
        let Some(root) = status.get("project_root").and_then(Value::as_str) else {
            return;
        };
        let projects = {
            let mut known = self.trust.lock().await;
            if known.get(root) == Some(status) {
                return;
            }
            known.insert(root.to_string(), status.clone());
            known.clone()
        };
        if let Err(e) = call.ext().topics().publish(TOPIC, json!({}), topic_value(projects)).await {
            tracing::warn!(error = %e.message, "the projects' trust was not published");
        }
    }
}

/// The topic's value: each project's last known status, by its root.
fn topic_value(projects: BTreeMap<String, Value>) -> Value {
    json!({"projects": projects.into_iter().collect::<Map<String, Value>>()})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_key_the_engine_lists_falls_in_one_category() {
        for (key, kind) in [
            ("hooks_config", "hooks"),
            ("scripts", "hooks"),
            ("mcp_servers.github", "mcp"),
            ("permission_mode", "permissions"),
            ("allow_client_permission_override", "permissions"),
            ("disable_skill_shell_execution", "permissions"),
            ("permission_rules.allow(Bash(git:*))", "permissions"),
            ("sandbox.network_mode", "sandbox"),
            ("sandbox.cache_directories.cargo", "sandbox"),
            ("providers.local", "models"),
            ("default_provider", "models"),
            ("task_models.title", "models"),
            ("model.base_url", "models"),
            ("model.auth_token", "models"),
            ("telemetry_url", "telemetry"),
            ("recorder", "telemetry"),
            ("instruction_file", "telemetry"),
            ("plugins.enabled", "plugins"),
            ("agents/reviewer", "agents"),
            ("skills/deploy", "skills"),
            ("something_new", "other"),
        ] {
            assert_eq!(category(key), kind, "{key}");
            assert!(CATEGORIES.contains(&kind));
        }
    }

    #[test]
    fn a_key_says_the_one_thing_it_names() {
        assert_eq!(named("mcp_servers.github").as_deref(), Some("github"));
        assert_eq!(named("agents/reviewer").as_deref(), Some("reviewer"));
        assert_eq!(named("permission_rules.allow(Bash(git:*))").as_deref(), Some("Bash(git:*)"));
        assert_eq!(named("sandbox.cache_directories.cargo").as_deref(), Some("cache_directories.cargo"));
        assert_eq!(named("hooks_config"), None);
    }

    #[test]
    fn an_untrusted_project_has_everything_withheld_and_a_trusted_one_nothing() {
        let status = |trust: &str| json!({"project_root": "/p", "trust": trust, "requires_trust": ["skills/deploy", "hooks_config", "mcp_servers.github"]});
        let untrusted = described(status("untrusted"));
        assert_eq!(untrusted["categories"], json!(["hooks", "mcp", "skills"]));
        assert_eq!(untrusted["withheld"].as_array().unwrap().len(), 3);
        assert_eq!(untrusted["items"][2], json!({"key": "mcp_servers.github", "category": "mcp", "name": "github"}));
        // The engine's own fields are kept as it said them.
        assert_eq!(untrusted["requires_trust"], status("untrusted")["requires_trust"]);

        let trusted = described(status("trusted"));
        assert_eq!(trusted["withheld"], json!([]));
        assert_eq!(trusted["items"].as_array().unwrap().len(), 3);
        // No project tier of its own: nothing to hold back.
        assert_eq!(described(json!({"trust": "none", "requires_trust": []}))["withheld"], json!([]));
        assert_eq!(no_project()["withheld"], json!([]));
    }

    #[test]
    fn a_project_is_known_however_its_path_is_written() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_str().unwrap().to_string();
        assert!(among(&format!("{root}/"), [root.as_str()]));
        // A link to a project is that project: the engine trusts the resolved directory.
        #[cfg(unix)]
        {
            let link = dir.path().with_extension("link");
            std::os::unix::fs::symlink(dir.path(), &link).unwrap();
            assert!(among(link.to_str().unwrap(), [root.as_str()]));
            std::fs::remove_file(&link).unwrap();
        }
        assert!(!among("/somewhere/else", [root.as_str()]));
        // One that is not there is compared as written.
        assert!(among("/not/there/", ["/not/there"]));
    }

    #[test]
    fn the_topic_is_every_project_by_its_root() {
        let mut projects = BTreeMap::new();
        projects.insert("/a".to_string(), json!({"trust": "trusted"}));
        assert_eq!(topic_value(projects), json!({"projects": {"/a": {"trust": "trusted"}}}));
    }
}
