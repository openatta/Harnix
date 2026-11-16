//! The engine's own reaping of what its commands leave behind. Ending the processes left in a
//! daemon's session is `harnix_supervise::reap`.

/// Turn on the engine's own reaping for the instance at `data_root`: `sandbox.reap_process_groups`
/// in its global settings tier, which every project's settings are layered over. Written to the
/// file rather than set in memory because the engine reads each project's settings from disk, and
/// a `config.reload` reads them all again. Everything else in the file is left as it was.
pub(crate) fn engine_reaps(data_root: &std::path::Path) -> std::io::Result<()> {
    use serde_json::{json, Value};
    // The engine writes this file under the same lock: the read and the write are one hold.
    let _held = harnix_contract::dir_lock::DirLock::acquire(data_root)?;
    let path = data_root.join("settings.json");
    let mut settings: Value = match std::fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str(&text)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, format!("{}: {e}", path.display())))?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => json!({}),
        Err(e) => return Err(e),
    };
    if !settings.is_object() {
        settings = json!({});
    }
    if settings.pointer("/sandbox/reap_process_groups") == Some(&Value::Bool(true)) {
        return Ok(());
    }
    if !settings.get("sandbox").is_some_and(Value::is_object) {
        settings["sandbox"] = json!({});
    }
    settings["sandbox"]["reap_process_groups"] = json!(true);
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(&settings).unwrap_or_default())?;
    std::fs::rename(&tmp, &path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_engine_s_reaping_is_turned_on_in_the_global_tier_and_nothing_else_moves() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, r#"{"model":"m","sandbox":{"subprocess_env":{"mode":"allowlist"}}}"#).unwrap();
        engine_reaps(dir.path()).unwrap();
        let written: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(written["sandbox"]["reap_process_groups"], true);
        assert_eq!(written["sandbox"]["subprocess_env"]["mode"], "allowlist");
        assert_eq!(written["model"], "m");
        let fresh = tempfile::tempdir().unwrap();
        engine_reaps(fresh.path()).unwrap();
        let written: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(fresh.path().join("settings.json")).unwrap()).unwrap();
        assert_eq!(written, serde_json::json!({"sandbox": {"reap_process_groups": true}}));
    }
}
