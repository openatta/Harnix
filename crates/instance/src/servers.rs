//! What 0.4.3 wrote into a spawned daemon's settings so it reached the packages' tools over MCP,
//! taken back out: those tools now reach it over its own socket, registered by this process.
//!
//! Harnix recorded which `mcp_servers` entries it wrote, in `<data root>/.harnix-servers.json`;
//! only those go, and the record with them. An entry the instance configures itself stays. From
//! 0.4.4 on nothing writes them, so a later version drops this.

use std::path::Path;

use serde_json::Value;

use crate::inherit::{read, write_private};

/// The record of what was written, beside the instance's own settings.
pub(crate) const RECORD: &str = ".harnix-servers.json";

/// Take out of the instance's settings the MCP servers 0.4.3 wrote there. `Ok(true)` when the
/// settings changed. Nothing to do, and nothing touched, when there is no record.
pub(crate) fn forget(instance_root: &Path) -> std::io::Result<bool> {
    let record_path = instance_root.join(RECORD);
    if !record_path.exists() {
        return Ok(false);
    }
    // The instance's own tier is written by its engine too, under this lock.
    let _held = harnix_contract::dir_lock::DirLock::acquire(instance_root)?;
    let settings_path = instance_root.join("settings.json");
    let record = read(&record_path)?;
    let mut settings = read(&settings_path)?;
    let written: Vec<&str> = record
        .get("mcp_servers")
        .and_then(Value::as_array)
        .map(|names| names.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();

    let mut changed = false;
    if let Some(object) = settings.as_object_mut() {
        if let Some(entries) = object.get_mut("mcp_servers").and_then(Value::as_object_mut) {
            for name in &written {
                changed |= entries.remove(*name).is_some();
            }
            // An empty section says nothing an absent one does not.
            if entries.is_empty() {
                object.remove("mcp_servers");
                changed = true;
            }
        }
    }
    if changed {
        write_private(&settings_path, &settings)?;
    }
    std::fs::remove_file(&record_path)?;
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn settings(root: &Path) -> Value {
        serde_json::from_slice(&std::fs::read(root.join("settings.json")).unwrap()).unwrap()
    }

    fn written(root: &Path, settings: Value, record: &[&str]) {
        std::fs::write(root.join("settings.json"), settings.to_string()).unwrap();
        std::fs::write(root.join(RECORD), json!({"mcp_servers": record}).to_string()).unwrap();
    }

    #[test]
    fn only_what_was_written_goes_and_the_record_with_it() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        written(
            root,
            json!({
                "memory_enabled": true,
                "mcp_servers": {
                    "ext-acme_notes": {"type": "streamable_http", "url": "http://127.0.0.1:1/ext/acme.notes/mcp"},
                    "mine": {"type": "stdio", "command": "x"},
                },
            }),
            &["ext-acme_notes", "ext-gone"],
        );
        assert!(forget(root).unwrap());
        let s = settings(root);
        assert!(s["mcp_servers"].get("ext-acme_notes").is_none());
        assert_eq!(s["mcp_servers"]["mine"]["command"], json!("x"), "the instance's own entry went");
        assert_eq!(s["memory_enabled"], json!(true));
        assert!(!root.join(RECORD).exists());
        // Once is enough: a second time finds no record and touches nothing.
        let before = std::fs::read(root.join("settings.json")).unwrap();
        assert!(!forget(root).unwrap());
        assert_eq!(std::fs::read(root.join("settings.json")).unwrap(), before);
    }

    #[test]
    fn a_section_left_empty_goes_too() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        written(root, json!({"mcp_servers": {"ext-acme_notes": {"type": "streamable_http", "url": "x"}}}), &["ext-acme_notes"]);
        assert!(forget(root).unwrap());
        assert!(settings(root).get("mcp_servers").is_none());
        assert!(!root.join(RECORD).exists());
    }

    #[test]
    fn without_a_record_nothing_is_written() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        assert!(!forget(root).unwrap());
        assert!(!root.join("settings.json").exists());

        let own = json!({"mcp_servers": {"ext-acme_notes": {"type": "stdio", "command": "own"}}}).to_string();
        std::fs::write(root.join("settings.json"), &own).unwrap();
        assert!(!forget(root).unwrap());
        assert_eq!(std::fs::read_to_string(root.join("settings.json")).unwrap(), own);
    }
}
