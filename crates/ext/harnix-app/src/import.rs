//! `harnix-app.sessions.importUpload` and `harnix-app.sessions.import`: a session bundle in.
//!
//! The inverse of the archive export. The zip is too large for a frame, so it travels the
//! large-payload side channel: `importUpload` grants a one-shot upload (the page sends the bytes
//! to the returned URL), and `import` hands the file the grant landed to the engine
//! (`harnix.session.import`), which installs it as new sessions in the claimed space. A device
//! names a grant by its token, never a path: the only file this method opens is one it granted.
//!
//! A bundle's project is the engine's to place: the bundle's own when it exists on this machine,
//! the caller's `project_root` when one is given, no project otherwise (the hub says so in the
//! answer's `warnings`).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

use harnix_ext::{Call, RpcError};
use serde_json::{json, Value};

use crate::{require_str, App};

/// Ceiling on an uploaded bundle: a session with its sub-agents and images is megabytes; this is
/// for the one that carries a long recording.
pub(crate) const MAX_BUNDLE_BYTES: usize = 256 * 1024 * 1024;

/// Grants handed out and not yet spent: token → the file the upload lands at.
#[derive(Default)]
pub(crate) struct Grants(Mutex<HashMap<String, PathBuf>>);

impl Grants {
    fn note(&self, token: String, path: PathBuf) {
        let mut all = self.0.lock().unwrap_or_else(|e| e.into_inner());
        // A grant nobody used is not kept for ever: the table holds the latest few.
        if all.len() >= 16 {
            if let Some(oldest) = all.keys().next().cloned() {
                all.remove(&oldest);
            }
        }
        all.insert(token, path);
    }

    fn take(&self, token: &str) -> Option<PathBuf> {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).remove(token)
    }
}

impl App {
    /// A one-shot upload for a bundle called `name`.
    pub(crate) fn import_upload(&self, call: &Call, params: &Value) -> Result<Value, RpcError> {
        let name = require_str(params, "name")?;
        if !name.to_ascii_lowercase().ends_with(".zip") {
            return Err(RpcError::invalid_params("a session bundle is a .zip"));
        }
        let grant = call.ext().bulk().grant_named(&name, MAX_BUNDLE_BYTES)?;
        self.grants.note(grant.token().to_string(), grant.path().to_path_buf());
        Ok(json!({
            "token": grant.token(),
            "url": format!("/upload?token={}", grant.token()),
            "max_bytes": grant.max_bytes(),
            "name": name,
        }))
    }

    /// The uploaded bundle under `token` as new sessions in the claimed space.
    pub(crate) async fn import(&self, call: &Call, params: Value) -> Result<Value, RpcError> {
        let token = require_str(&params, "token")?;
        let path = self
            .grants
            .take(&token)
            .ok_or_else(|| RpcError::invalid_params("no such upload: grant one with sessions.importUpload and send the file first"))?;
        let mut ask = json!({"path": path});
        for key in ["space", "instance", "project_root"] {
            if let Some(value) = params.get(key) {
                ask[key] = value.clone();
            }
        }
        let result = call.call("harnix.session.import", ask).await;
        let _ = std::fs::remove_file(&path);
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_grant_is_spent_once_and_the_table_stays_small() {
        let grants = Grants::default();
        grants.note("a".into(), PathBuf::from("/x/a.zip"));
        assert_eq!(grants.take("a"), Some(PathBuf::from("/x/a.zip")));
        assert_eq!(grants.take("a"), None, "spent");
        assert_eq!(grants.take("never-granted"), None);
        for i in 0..40 {
            grants.note(format!("t{i}"), PathBuf::from(format!("/x/{i}.zip")));
        }
        assert!(grants.0.lock().unwrap().len() <= 16);
    }
}
