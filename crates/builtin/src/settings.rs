//! The settings plane: read the three tiers, write one field into one tier.
//!
//! Agix exposes `config.get` (read, per tier, redacted) and
//! `config.reload`, but no generic write — the supported way to change a
//! setting is to edit the tier's `settings.json`, which is exactly what
//! `config.reload` exists for and what the engine's `WatchHub` already
//! watches. So Harnix writes the file and asks the engine to re-read it.
//!
//! Two rules make that safe to expose to a browser:
//!
//! - **A whitelist, not a passthrough.** Only the fields in [`FIELDS`] can be
//!   written, each with a declared kind that the value is checked against. A
//!   settings file is a program's behaviour; "set any key to any JSON" is not
//!   a feature, it is a remote configuration channel.
//! - **Read, modify, write the whole file.** Hand edits are preserved because
//!   the file is re-read immediately before each change, and replaced
//!   atomically.
//!
//! Providers are written the same way, into the global tier, and checked
//! against the engine's own `ProviderConfig` first. Not through
//! `config.setProvider`: that writes the project tier of the engine's working
//! directory, which is wherever the process happened to be started — a
//! provider added from the settings page would be gone the next time it was
//! started from somewhere else, and a desktop app has no working directory
//! worth writing to. `config.reload` then rebuilds the router.

use std::path::PathBuf;

use harnix_contract::{codes, RpcError};
use serde_json::{json, Map, Value};

use crate::{require_str, Builtin};

/// What a field accepts. Anything else is refused before a file is touched.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Kind {
    Text,
    Count,
    Flag,
    Choice(&'static [&'static str]),
}

pub(crate) struct Field {
    /// Dotted path into settings.json.
    pub key: &'static str,
    pub kind: Kind,
}

/// The settings a person changes from a UI. Deliberately short: every entry
/// here is a promise that writing it does something predictable.
pub(crate) const FIELDS: &[Field] = &[
    Field {
        key: "model.model_name",
        kind: Kind::Text,
    },
    Field {
        key: "model.max_tokens",
        kind: Kind::Count,
    },
    Field {
        key: "permission_mode",
        kind: Kind::Choice(&["default", "acceptEdits", "plan", "bypassPermissions"]),
    },
    Field {
        key: "memory_enabled",
        kind: Kind::Flag,
    },
    Field {
        key: "allow_client_permission_override",
        kind: Kind::Flag,
    },
];

const TIERS: &[&str] = &["global", "scene", "project"];

impl Builtin {
    /// Every editable field, its effective value, and where each tier stands.
    pub(crate) async fn settings_describe(&self, params: Value) -> Result<Value, RpcError> {
        // `config.get` needs a scene and refuses without one. Falling back to
        // the engine's own default rather than sending a request that cannot
        // succeed: a settings page opened with no session is the ordinary
        // case, and it used to answer every field `null` — a write made and
        // then read back as nothing, with the error swallowed on the way.
        let scene = params
            .get("scene")
            .and_then(Value::as_str)
            .map(str::to_string)
            .or_else(|| self.system().active_scenes.first().cloned());
        let project_root = params
            .get("project_root")
            .and_then(Value::as_str)
            .map(str::to_string);

        let mut raw = Map::new();
        let mut problems: Vec<Value> = Vec::new();
        for tier in TIERS {
            let mut request = json!({"tier": tier});
            if let Some(scene) = &scene {
                request["scene"] = json!(scene);
            }
            if let Some(root) = &project_root {
                request["project_root"] = json!(root);
            }
            // A tier that genuinely has no file answers `null`; a call that
            // *failed* is a different thing and is reported rather than
            // flattened into the same answer. The two used to be
            // indistinguishable, which is how a page of nulls looked like an
            // empty configuration instead of a broken request.
            let settings = match self.call("settings", "config.get", request).await {
                Ok(value) => value.get("settings").cloned().unwrap_or(Value::Null),
                Err(e) => {
                    tracing::warn!(tier, error = %e.message, "settings tier could not be read");
                    problems.push(json!({"tier": tier, "error": e.message}));
                    Value::Null
                }
            };
            raw.insert((*tier).to_string(), settings);
        }

        let mut effective_request = json!({"tier": "effective"});
        if let Some(scene) = &scene {
            effective_request["scene"] = json!(scene);
        }
        if let Some(root) = &project_root {
            effective_request["project_root"] = json!(root);
        }
        let effective = self.call("settings", "config.get", effective_request)
            .await
            .ok()
            .and_then(|value| value.get("settings").cloned())
            .unwrap_or(Value::Null);

        let fields: Vec<Value> = FIELDS
            .iter()
            .map(|field| {
                let per_tier: Map<String, Value> = TIERS
                    .iter()
                    .map(|tier| {
                        let value = raw
                            .get(*tier)
                            .and_then(|settings| dig(settings, field.key))
                            .cloned()
                            .unwrap_or(Value::Null);
                        ((*tier).to_string(), value)
                    })
                    .collect();

                // What the files say, highest tier wins. Computed here rather
                // than taken from `config.get {tier:"effective"}` because that
                // answer is memoized per (project, scene) and
                // `apply_reloaded_settings` only swaps the pool's own pair —
                // so for every other pair it keeps serving the value from
                // before the reload, and a settings page that showed it would
                // report that a write it just made did nothing. The engine's
                // own answer rides along as `engine` for diagnostics.
                let (source, from_files) = TIERS
                    .iter()
                    .rev()
                    .find_map(|tier| {
                        per_tier
                            .get(*tier)
                            .filter(|value| !value.is_null())
                            .map(|value| (json!(tier), value.clone()))
                    })
                    .unwrap_or((Value::Null, Value::Null));

                json!({
                    "key": field.key,
                    "kind": kind_name(field.kind),
                    "options": match field.kind {
                        Kind::Choice(options) => json!(options),
                        _ => Value::Null,
                    },
                    "effective": from_files,
                    "source": source,
                    "engine": dig(&effective, field.key).cloned().unwrap_or(Value::Null),
                    "tiers": per_tier,
                })
            })
            .collect();

        let providers = self.call("settings", "config.getProvider", json!({}))
            .await
            .unwrap_or(json!({}));

        Ok(json!({
            "fields": fields,
            "scene": scene,
            // Empty in the ordinary case. Not empty means a tier could not be
            // read, and the page can say so instead of showing its nulls as
            // if they were the configuration.
            "problems": problems,
            "paths": {
                "global": self.tier_path("global", scene.as_deref(), project_root.as_deref())
                    .map(|p| p.display().to_string()),
                "scene": self.tier_path("scene", scene.as_deref(), project_root.as_deref())
                    .map(|p| p.display().to_string()),
                "project": self.tier_path("project", scene.as_deref(), project_root.as_deref())
                    .map(|p| p.display().to_string()),
            },
            "providers": providers,
        }))
    }

    /// Write one whitelisted field into one tier, then make the engine re-read.
    ///
    /// A `null` value removes the key, which is how a tier gives a setting
    /// back to the tier below it.
    pub(crate) async fn settings_set(&self, params: Value) -> Result<Value, RpcError> {
        let tier = require_str(&params, "tier")?;
        let key = require_str(&params, "key")?;
        let value = params.get("value").cloned().unwrap_or(Value::Null);
        // The same default as `describe`, for the same reason: a write with
        // no scene would otherwise resolve no tier path and be refused for a
        // reason that has nothing to do with what was asked.
        let default_scene = self.system().active_scenes.first().cloned();
        let scene = params
            .get("scene")
            .and_then(Value::as_str)
            .or(default_scene.as_deref());
        let project_root = params.get("project_root").and_then(Value::as_str);

        let Some(field) = FIELDS.iter().find(|f| f.key == key) else {
            return Err(RpcError::new(
                codes::INVALID_PARAMS,
                format!("`{key}` is not an editable setting"),
            ));
        };
        if !value.is_null() && !accepts(field.kind, &value) {
            return Err(RpcError::new(
                codes::INVALID_PARAMS,
                format!("`{key}` does not accept that value"),
            ));
        }
        let Some(path) = self.tier_path(&tier, scene, project_root) else {
            return Err(RpcError::new(
                codes::INVALID_PARAMS,
                format!("no `{tier}` tier for this session"),
            ));
        };

        let mut document = read_document(&path)?;
        set_path(&mut document, &key, value);
        write_document(&path, &document)?;

        let reloaded = self.call("settings", "config.reload", json!({}))
            .await
            .is_ok();
        let mut describe = self.settings_describe(params.clone()).await?;
        if let Some(object) = describe.as_object_mut() {
            object.insert("written".into(), json!(path.display().to_string()));
            object.insert("reloaded".into(), json!(reloaded));
        }
        Ok(describe)
    }

    /// Add, edit or remove a provider — credentials included — in the global
    /// tier, then make the engine re-read.
    ///
    /// `config` is a patch: a key that is absent keeps its value (which is how
    /// an edit leaves the stored API key alone), `null` or blank removes it. With no
    /// default left, the first provider becomes the default: a router with
    /// providers and no default has nothing to send a turn to.
    pub(crate) async fn settings_set_provider(&self, params: Value) -> Result<Value, RpcError> {
        let provider_id = require_str(&params, "provider_id")?;
        if !valid_provider_id(&provider_id) {
            return Err(RpcError::invalid_params(
                "provider_id: letters, digits, `-`, `_` and `.`",
            ));
        }
        let Some(path) = self.tier_path("global", None, None) else {
            return Err(RpcError::new(codes::INTERNAL_ERROR, "no global tier"));
        };
        let mut document = read_document(&path)?;
        patch_providers(
            &mut document,
            &provider_id,
            params.get("delete").and_then(Value::as_bool).unwrap_or(false),
            params.get("config"),
            params.get("default_provider").and_then(Value::as_str),
        )?;
        write_document(&path, &document)?;

        let reloaded = self.call("settings", "config.reload", json!({})).await;
        let providers = self.call("settings", "config.getProvider", json!({}))
            .await
            .unwrap_or(json!({}));
        Ok(json!({
            "written": path.display().to_string(),
            "reloaded": reloaded.is_ok(),
            "providers": providers,
        }))
    }

    /// The settings.json a tier writes to, if that tier exists here.
    fn tier_path(
        &self,
        tier: &str,
        scene: Option<&str>,
        project_root: Option<&str>,
    ) -> Option<PathBuf> {
        let root = self.system().data_root;
        match tier {
            "global" => Some(root.join("settings.json")),
            "scene" => scene.map(|scene| root.join("scenes").join(scene).join("settings.json")),
            "project" => project_root.map(|p| PathBuf::from(p).join(".agix").join("settings.json")),
            _ => None,
        }
    }
}

fn kind_name(kind: Kind) -> &'static str {
    match kind {
        Kind::Text => "text",
        Kind::Count => "count",
        Kind::Flag => "flag",
        Kind::Choice(_) => "choice",
    }
}

fn accepts(kind: Kind, value: &Value) -> bool {
    match kind {
        Kind::Text => value.as_str().is_some_and(|s| !s.trim().is_empty()),
        Kind::Count => value.as_u64().is_some_and(|n| n > 0 && n <= 1_000_000),
        Kind::Flag => value.is_boolean(),
        Kind::Choice(options) => value.as_str().is_some_and(|s| options.contains(&s)),
    }
}

/// Read a dotted path out of a settings document.
fn dig<'a>(document: &'a Value, key: &str) -> Option<&'a Value> {
    let mut cursor = document;
    for segment in key.split('.') {
        cursor = cursor.get(segment)?;
    }
    Some(cursor)
}

/// Write a dotted path into a settings document, creating objects on the way
/// and removing the key when the value is null.
fn set_path(document: &mut Value, key: &str, value: Value) {
    let segments: Vec<&str> = key.split('.').collect();
    if !document.is_object() {
        *document = json!({});
    }
    let mut cursor = document;
    for segment in &segments[..segments.len() - 1] {
        if !cursor.get(*segment).map(Value::is_object).unwrap_or(false) {
            cursor[*segment] = json!({});
        }
        cursor = cursor.get_mut(*segment).expect("just inserted");
    }
    let last = segments[segments.len() - 1];
    match value {
        Value::Null => {
            if let Some(object) = cursor.as_object_mut() {
                object.remove(last);
            }
        }
        value => cursor[last] = value,
    }
}

const PROVIDER_KEYS: &[&str] = &[
    "api_type",
    "base_url",
    "api_key",
    "api_key_env",
    "default_model",
    "models",
];

fn valid_provider_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// Apply one provider change to a settings document, refusing what the engine
/// would not load.
fn patch_providers(
    document: &mut Value,
    provider_id: &str,
    delete: bool,
    config: Option<&Value>,
    default_provider: Option<&str>,
) -> Result<(), RpcError> {
    if !document.is_object() {
        *document = json!({});
    }
    if !document.get("providers").is_some_and(Value::is_object) {
        document["providers"] = json!({});
    }
    let providers = document["providers"].as_object_mut().expect("just made an object");
    if delete {
        providers.remove(provider_id);
    } else if let Some(config) = config {
        let Some(patch) = config.as_object() else {
            return Err(RpcError::invalid_params("config must be an object"));
        };
        let mut entry = providers
            .get(provider_id)
            .cloned()
            .filter(Value::is_object)
            .unwrap_or_else(|| json!({}));
        for (key, value) in patch {
            if !PROVIDER_KEYS.contains(&key.as_str()) {
                return Err(RpcError::invalid_params(format!(
                    "config.{key} is not a provider setting"
                )));
            }
            let blank = value.is_null() || value.as_str().is_some_and(|s| s.trim().is_empty());
            let object = entry.as_object_mut().expect("an object");
            if blank {
                object.remove(key);
            } else {
                object.insert(key.clone(), value.clone());
            }
        }
        match entry.get("api_type").and_then(Value::as_str) {
            Some("anthropic" | "openai_compatible") => {}
            _ => {
                return Err(RpcError::invalid_params(
                    "config.api_type: `anthropic` or `openai_compatible`",
                ))
            }
        }
        serde_json::from_value::<base::provider::ProviderConfig>(entry.clone())
            .map_err(|e| RpcError::invalid_params(format!("invalid provider config: {e}")))?;
        providers.insert(provider_id.to_string(), entry);
    }

    let first = providers.keys().next().cloned();
    let current = document
        .get("default_provider")
        .and_then(Value::as_str)
        .map(str::to_string);
    let wanted = default_provider.map(str::to_string).or(current);
    let providers = document["providers"].as_object().expect("an object");
    let default = match wanted {
        Some(id) if providers.contains_key(&id) => Some(id),
        Some(id) if default_provider.is_some() => {
            return Err(RpcError::invalid_params(format!(
                "default_provider `{id}` is not configured"
            )))
        }
        _ => first,
    };
    let object = document.as_object_mut().expect("an object");
    match default {
        Some(id) => {
            object.insert("default_provider".into(), json!(id));
        }
        None => {
            object.remove("default_provider");
        }
    }
    Ok(())
}

/// A tier's settings.json, or an empty document when the file is not there.
fn read_document(path: &std::path::Path) -> Result<Value, RpcError> {
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| {
            RpcError::new(
                codes::INTERNAL_ERROR,
                format!("{} is not valid JSON: {e}", path.display()),
            )
        }),
        Err(_) => Ok(json!({})),
    }
}

/// Replace a tier's settings.json atomically.
fn write_document(path: &std::path::Path, document: &Value) -> Result<(), RpcError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(io_error)?;
    }
    let temp = path.with_extension("json.tmp");
    let body = serde_json::to_vec_pretty(document)
        .map_err(|e| RpcError::new(codes::INTERNAL_ERROR, e.to_string()))?;
    std::fs::write(&temp, &body)
        .and_then(|()| std::fs::rename(&temp, path))
        .map_err(io_error)
}

fn io_error(e: std::io::Error) -> RpcError {
    RpcError::new(codes::INTERNAL_ERROR, format!("could not write: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_nested_write_creates_its_parents_and_keeps_siblings() {
        let mut document = json!({"model": {"max_tokens": 2000}, "memory_enabled": true});
        set_path(&mut document, "model.model_name", json!("deepseek-v4"));
        assert_eq!(document["model"]["model_name"], json!("deepseek-v4"));
        assert_eq!(document["model"]["max_tokens"], json!(2000));
        assert_eq!(document["memory_enabled"], json!(true));
    }

    #[test]
    fn null_gives_the_setting_back_to_the_tier_below() {
        let mut document = json!({"permission_mode": "plan"});
        set_path(&mut document, "permission_mode", Value::Null);
        assert!(document.get("permission_mode").is_none());
    }

    #[test]
    fn a_provider_edit_keeps_what_it_leaves_out_and_the_first_is_the_default() {
        let mut document = json!({"memory_enabled": true});
        let config = json!({"api_type": "anthropic", "api_key": "k1", "default_model": "m1"});
        patch_providers(&mut document, "a", false, Some(&config), None).unwrap();
        assert_eq!(document["default_provider"], json!("a"));
        let edit = json!({"api_type": "anthropic", "base_url": "", "default_model": "m2"});
        patch_providers(&mut document, "a", false, Some(&edit), None).unwrap();
        assert_eq!(document["providers"]["a"]["api_key"], json!("k1"));
        assert_eq!(document["providers"]["a"]["default_model"], json!("m2"));
        assert_eq!(document["memory_enabled"], json!(true));
    }

    #[test]
    fn deleting_the_default_hands_it_to_another_provider() {
        let mut document = json!({});
        let anthropic = json!({"api_type": "anthropic"});
        let openai = json!({"api_type": "openai_compatible", "base_url": "https://api.openai.com/v1"});
        patch_providers(&mut document, "a", false, Some(&anthropic), None).unwrap();
        patch_providers(&mut document, "b", false, Some(&openai), Some("b")).unwrap();
        assert_eq!(document["default_provider"], json!("b"));
        patch_providers(&mut document, "b", true, None, None).unwrap();
        assert_eq!(document["default_provider"], json!("a"));
        patch_providers(&mut document, "a", true, None, None).unwrap();
        assert!(document.get("default_provider").is_none());
    }

    #[test]
    fn a_provider_the_engine_would_not_load_is_refused() {
        let mut document = json!({});
        assert!(patch_providers(&mut document, "a", false, Some(&json!({"api_type": "gemini"})), None).is_err());
        assert!(patch_providers(&mut document, "a", false, Some(&json!({"api_type": "anthropic", "shell": "x"})), None).is_err());
        assert!(patch_providers(&mut document, "a", false, Some(&json!({"api_type": "anthropic"})), Some("zz")).is_err());
        assert!(!valid_provider_id("a b"));
    }

    #[test]
    fn kinds_refuse_what_they_do_not_mean() {
        assert!(accepts(Kind::Count, &json!(4000)));
        assert!(!accepts(Kind::Count, &json!(0)));
        assert!(!accepts(Kind::Count, &json!("4000")));
        assert!(accepts(Kind::Choice(&["plan"]), &json!("plan")));
        assert!(!accepts(Kind::Choice(&["plan"]), &json!("yolo")));
        assert!(!accepts(Kind::Text, &json!("   ")));
    }
}
