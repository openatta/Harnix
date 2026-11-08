//! MCP servers this process offers the daemons it spawns (applications, design §5.1).
//!
//! A runtime application runs in one VM, in this process. An instance built in this process reaches
//! its tools directly; a daemon this process spawned is another process, and reaches them over MCP
//! instead — an endpoint per application, served by this process, with a token of its own. What
//! goes into the daemon is a set of `mcp_servers` entries in its global tier: written when it is
//! built, and again (with a `config.reload`) whenever the set changes — an application enabled,
//! disabled, installed or removed — or this process learns where it listens.
//!
//! Written the way inherited providers are (`inherit.rs`): Harnix records which entries it wrote,
//! in `<data_root>/.harnix-servers.json`, and only ever adds, changes or removes those. An entry the
//! instance configures itself under the same name is the instance's, and stays.

use std::path::Path;

use serde_json::{json, Map, Value};

/// The record of what was written, beside the instance's own settings.
pub(crate) const RECORD: &str = ".harnix-servers.json";

/// Bring one instance's `mcp_servers` in line with `servers`. `Ok(true)` when it changed.
pub(crate) fn merge(instance_root: &Path, servers: &Map<String, Value>) -> std::io::Result<bool> {
    let settings_path = instance_root.join("settings.json");
    let record_path = instance_root.join(RECORD);
    let mut settings = read(&settings_path)?;
    let record = read(&record_path)?;
    let owned: Vec<String> = record
        .get("mcp_servers")
        .and_then(Value::as_array)
        .map(|ids| ids.iter().filter_map(Value::as_str).map(str::to_string).collect())
        .unwrap_or_default();
    if servers.is_empty() && owned.is_empty() {
        return Ok(false);
    }

    let before = settings.clone();
    if !settings.is_object() {
        settings = json!({});
    }
    if !settings.get("mcp_servers").is_some_and(Value::is_object) {
        settings["mcp_servers"] = json!({});
    }
    let entries = settings["mcp_servers"].as_object_mut().expect("an object");
    for name in &owned {
        if !servers.contains_key(name) {
            entries.remove(name);
        }
    }
    let mut now_owned = Vec::new();
    for (name, config) in servers {
        if entries.contains_key(name) && !owned.contains(name) {
            tracing::warn!(server = %name, "the instance configures an MCP server of this name itself; left as it is");
            continue;
        }
        entries.insert(name.clone(), config.clone());
        now_owned.push(name.clone());
    }
    // An empty section says nothing an absent one does not.
    if entries.is_empty() {
        settings.as_object_mut().expect("an object").remove("mcp_servers");
    }

    let changed = settings != before;
    if changed {
        crate::inherit::write_private(&settings_path, &settings)?;
    }
    crate::inherit::write_private(&record_path, &json!({"mcp_servers": now_owned}))?;
    Ok(changed)
}

fn read(path: &Path) -> std::io::Result<Value> {
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| {
            std::io::Error::new(std::io::ErrorKind::InvalidData, format!("{}: {e}", path.display()))
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(json!({})),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(root: &Path) -> Value {
        serde_json::from_slice(&std::fs::read(root.join("settings.json")).unwrap()).unwrap()
    }

    fn servers(names: &[&str]) -> Map<String, Value> {
        names
            .iter()
            .map(|n| (n.to_string(), json!({"type": "remote", "url": format!("x/{n}")})))
            .collect()
    }

    #[test]
    fn written_entries_follow_the_set_and_the_instances_own_stay() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(
            root.join("settings.json"),
            json!({"memory_enabled": true, "mcp_servers": {"mine": {"type": "stdio", "command": "x"}, "taken": {"type": "stdio", "command": "own"}}})
                .to_string(),
        )
        .unwrap();
        assert!(merge(root, &servers(&["app-a", "taken"])).unwrap());
        let s = settings(root);
        assert_eq!(s["mcp_servers"]["app-a"]["url"], json!("x/app-a"));
        assert_eq!(s["mcp_servers"]["taken"]["command"], json!("own"), "the instance's own entry was replaced");
        assert!(s["mcp_servers"].get("mine").is_some());
        assert_eq!(s["memory_enabled"], json!(true));
        assert!(!merge(root, &servers(&["app-a", "taken"])).unwrap(), "nothing changed, and it changed");

        // Withdrawn: gone, and only what was written.
        assert!(merge(root, &servers(&[])).unwrap());
        let s = settings(root);
        assert!(s["mcp_servers"].get("app-a").is_none());
        assert!(s["mcp_servers"].get("mine").is_some() && s["mcp_servers"].get("taken").is_some());
    }

    #[test]
    fn nothing_to_offer_touches_nothing() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!merge(dir.path(), &Map::new()).unwrap());
        assert!(!dir.path().join("settings.json").exists());
    }

    #[test]
    fn a_fresh_instance_gets_the_section_and_loses_it_again() {
        let dir = tempfile::tempdir().unwrap();
        assert!(merge(dir.path(), &servers(&["app-a"])).unwrap());
        assert!(settings(dir.path())["mcp_servers"].get("app-a").is_some());
        assert!(merge(dir.path(), &Map::new()).unwrap());
        assert!(settings(dir.path()).get("mcp_servers").is_none());
    }
}
