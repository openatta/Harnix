//! Providers an instance inherits from `system` (ipdrun-11).
//!
//! The settings page writes providers into the `system` instance's global tier; an instance built
//! from a template has a data root, and so a global tier, of its own, and saw none of them. A
//! template that says `inherit_providers` has `system`'s providers merged into its instances' global
//! tier — when an instance is built, and every time the settings page changes a provider — and the
//! instance reloads.
//!
//! Merged into the global tier because Agix has three tiers (global, scene, project) and no fourth
//! to stack a read-only "upstream" under them. So Harnix records which provider ids it wrote, in
//! `<data_root>/.inherited-providers.json`, and only ever adds, changes or removes those: a
//! provider the instance configures itself under the same id is the instance's, and stays. The
//! default provider likewise: set from `system`'s only when the instance has none of its own.

use std::path::Path;

use serde_json::{json, Map, Value};

/// The record of what was inherited, beside the instance's own settings.
pub(crate) const RECORD: &str = ".inherited-providers.json";

/// Bring one instance's global tier in line with `system`'s providers. `Ok(true)` when it changed.
pub(crate) fn merge(system_root: &Path, instance_root: &Path) -> std::io::Result<bool> {
    let upstream = read(&system_root.join("settings.json"))?;
    let settings_path = instance_root.join("settings.json");
    let record_path = instance_root.join(RECORD);
    let mut settings = read(&settings_path)?;
    let record = read(&record_path)?;

    let theirs: Map<String, Value> = upstream
        .get("providers")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let their_default = upstream.get("default_provider").and_then(Value::as_str).map(str::to_string);
    let owned: Vec<String> = record
        .get("providers")
        .and_then(Value::as_array)
        .map(|ids| ids.iter().filter_map(Value::as_str).map(str::to_string).collect())
        .unwrap_or_default();
    let owned_default = record.get("default_provider").and_then(Value::as_bool).unwrap_or(false);
    if theirs.is_empty() && owned.is_empty() && !owned_default {
        return Ok(false);
    }

    let before = settings.clone();
    if !settings.is_object() {
        settings = json!({});
    }
    if !settings.get("providers").is_some_and(Value::is_object) {
        settings["providers"] = json!({});
    }
    let providers = settings["providers"].as_object_mut().expect("an object");
    for id in &owned {
        if !theirs.contains_key(id) {
            providers.remove(id);
        }
    }
    let mut now_owned = Vec::new();
    for (id, config) in &theirs {
        if providers.contains_key(id) && !owned.contains(id) {
            continue;
        }
        providers.insert(id.clone(), config.clone());
        now_owned.push(id.clone());
    }
    let usable_default = their_default.filter(|id| providers.contains_key(id));
    let object = settings.as_object_mut().expect("an object");
    let own_default = object.contains_key("default_provider") && !owned_default;
    let now_owned_default = if own_default {
        false
    } else if let Some(id) = usable_default {
        object.insert("default_provider".into(), json!(id));
        true
    } else {
        object.remove("default_provider");
        false
    };
    if settings.get("providers").and_then(Value::as_object).is_some_and(Map::is_empty)
        && before.get("providers").is_none()
    {
        settings.as_object_mut().expect("an object").remove("providers");
    }

    let changed = settings != before;
    if changed {
        write_private(&settings_path, &settings)?;
    }
    write_private(
        &record_path,
        &json!({"providers": now_owned, "default_provider": now_owned_default}),
    )?;
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

/// Replaced whole, readable by its owner only: it holds credentials.
fn write_private(path: &Path, value: &Value) -> std::io::Result<()> {
    use std::io::Write as _;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let temp = path.with_extension("json.tmp");
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let body = serde_json::to_vec_pretty(value).map_err(std::io::Error::other)?;
    options.open(&temp)?.write_all(&body)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&temp, std::fs::Permissions::from_mode(0o600))?;
    }
    std::fs::rename(&temp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, value: Value) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, value.to_string()).unwrap();
    }

    fn settings(root: &Path) -> Value {
        serde_json::from_slice(&std::fs::read(root.join("settings.json")).unwrap()).unwrap()
    }

    #[test]
    fn inherited_providers_follow_system_and_the_instances_own_stay() {
        let dir = tempfile::tempdir().unwrap();
        let (system, instance) = (dir.path().join("system"), dir.path().join("instance"));
        write(&instance.join("settings.json"), json!({
            "memory_enabled": true,
            "providers": {"mine": {"api_type": "anthropic"}, "shared": {"api_type": "anthropic", "base_url": "own"}},
        }));
        write(&system.join("settings.json"), json!({
            "providers": {"a": {"api_type": "anthropic", "api_key": "k"}, "shared": {"api_type": "anthropic", "base_url": "theirs"}},
            "default_provider": "a",
        }));
        assert!(merge(&system, &instance).unwrap());
        let s = settings(&instance);
        assert_eq!(s["providers"]["a"]["api_key"], json!("k"));
        assert_eq!(s["providers"]["shared"]["base_url"], json!("own"), "the instance's own provider was overwritten");
        assert!(s["providers"].get("mine").is_some());
        assert_eq!(s["default_provider"], json!("a"));
        assert_eq!(s["memory_enabled"], json!(true));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(instance.join("settings.json")).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }

        // Deleted upstream: gone here too, the instance's own untouched.
        write(&system.join("settings.json"), json!({"providers": {}}));
        assert!(merge(&system, &instance).unwrap());
        let s = settings(&instance);
        assert!(s["providers"].get("a").is_none());
        assert!(s["providers"].get("shared").is_some() && s["providers"].get("mine").is_some());
        assert!(s.get("default_provider").is_none());
        assert!(!merge(&system, &instance).unwrap(), "nothing to change, and it changed");
    }

    #[test]
    fn an_instance_with_a_default_of_its_own_keeps_it() {
        let dir = tempfile::tempdir().unwrap();
        let (system, instance) = (dir.path().join("system"), dir.path().join("instance"));
        write(&instance.join("settings.json"), json!({"providers": {"mine": {"api_type": "anthropic"}}, "default_provider": "mine"}));
        write(&system.join("settings.json"), json!({"providers": {"a": {"api_type": "anthropic"}}, "default_provider": "a"}));
        merge(&system, &instance).unwrap();
        assert_eq!(settings(&instance)["default_provider"], json!("mine"));
    }

    #[test]
    fn nothing_to_inherit_touches_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let (system, instance) = (dir.path().join("system"), dir.path().join("instance"));
        assert!(!merge(&system, &instance).unwrap());
        assert!(!instance.join("settings.json").exists());
    }
}
