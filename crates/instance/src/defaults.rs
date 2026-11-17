//! What Harnix's instances start with where the engine's own default is not Harnix's.
//!
//! One thing so far: telemetry is off. The engine records a transcript of every session's calls,
//! costs and permission decisions to its data root by default; a product that does not say so to
//! the person using it should not start with that on. The default is written into the instance's
//! global settings tier, once, when the tier does not say: a person's own choice (on, or a
//! deliberate off) is never overwritten, and it is visible and editable on the settings page like
//! any other value. A tier the person later clears is written again at the next start.
//!
//! Into every data root Harnix owns: `main`'s engine root (Harnix's own, under its state directory
//! unless `--engine-dir` names another), and the instances it spawns or builds from a template.
//! Not an attached daemon's, which is its owner's.

use std::path::Path;

use serde_json::{json, Value};

/// Write `telemetry_enabled: false` into the global tier of the instance at `data_root` unless it
/// has a value there. Everything else in the file is left as it was.
pub(crate) fn telemetry_off(data_root: &Path) -> std::io::Result<()> {
    // The engine writes this file under the same lock: the read and the write are one hold.
    let _held = harnix_contract::dir_lock::DirLock::acquire(data_root)?;
    let path = data_root.join("settings.json");
    let mut settings: Value = match std::fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str(&text)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, format!("{}: {e}", path.display())))?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => json!({}),
        Err(e) => return Err(e),
    };
    let Some(object) = settings.as_object_mut() else { return Ok(()) };
    if object.get("telemetry_enabled").is_some_and(|v| !v.is_null()) {
        return Ok(());
    }
    object.insert("telemetry_enabled".into(), json!(false));
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(&settings).unwrap_or_default())?;
    std::fs::rename(&tmp, &path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(path: &Path) -> Value {
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    #[test]
    fn a_fresh_instance_starts_with_telemetry_off() {
        let dir = tempfile::tempdir().unwrap();
        telemetry_off(dir.path()).unwrap();
        assert_eq!(read(&dir.path().join("settings.json")), json!({"telemetry_enabled": false}));
    }

    #[test]
    fn a_choice_the_tier_has_is_kept_and_the_rest_of_the_file_with_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, r#"{"telemetry_enabled":true,"memory_enabled":false}"#).unwrap();
        telemetry_off(dir.path()).unwrap();
        assert_eq!(read(&path), json!({"telemetry_enabled": true, "memory_enabled": false}));
        std::fs::write(&path, r#"{"memory_enabled":false}"#).unwrap();
        telemetry_off(dir.path()).unwrap();
        assert_eq!(read(&path), json!({"memory_enabled": false, "telemetry_enabled": false}));
    }

    #[test]
    fn a_file_that_is_not_json_is_not_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, "{ not json").unwrap();
        assert!(telemetry_off(dir.path()).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{ not json");
    }
}
