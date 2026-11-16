//! Providers, written through the engine: `engine.config.setProvider` with `tier: "global"`, tried
//! first with `dry_run` and written only when the engine says the result routes.
//!
//! The engine writes the global tier's `settings.json` under its own directory lock, and the key
//! into the `settings.local.json` beside it (0600, never committed). What stays here is what the
//! settings page promises beyond the engine's patch:
//!
//! - **Which keys a page may write**: the ones in [`PROVIDER_KEYS`].
//! - **Blank is "remove"**: a blank string or `null` gives the key up. The engine merges a patch
//!   key by key and would keep `""` as a value (an empty `base_url` is not "the default one"), so
//!   blank goes to it as `null`, which the engine reads as unset; `models`, a list there, as `[]`,
//!   which is the same "no filter".
//! - **There is always a default while there are providers**: a router with providers and no
//!   default has nothing to send a turn to. Deleting the default hands it to the first left; with
//!   no default (or one that is gone) the first becomes it. The engine changes `default_provider`
//!   only when told, so it is told.
//! - **A key is never read back**: the engine answers `api_key` as `***` and its last four
//!   characters; what goes on to the page is redacted again here ([`redacted`]), so a key the
//!   engine let through whole would still not reach it.

use std::collections::BTreeSet;

use harnix_ext::{Rpc, RpcError};
use serde_json::{json, Map, Value};

/// What the provider writes need of the engine: its calls, so a test can stand in for it.
#[async_trait::async_trait]
pub(crate) trait Engine: Send + Sync {
    async fn call(&self, method: &str, params: Value) -> Rpc<Value>;
}

#[async_trait::async_trait]
impl Engine for harnix_ext::Call {
    async fn call(&self, method: &str, params: Value) -> Rpc<Value> {
        harnix_ext::Call::call(self, method, params).await
    }
}

pub(crate) const PROVIDER_KEYS: &[&str] = &[
    "api_type",
    "base_url",
    "api_key",
    "api_key_env",
    "default_model",
    "models",
    // OpenAI-compatible endpoints' thinking: how it is switched, and whether its reasoning is
    // sent back within a tool loop (DeepSeek's thinking mode needs it).
    "thinking_toggle",
    "reasoning_passback",
];

/// The fields an OpenAI-compatible endpoint is told to switch thinking by.
pub(crate) const THINKING_TOGGLES: &[&str] = &["deepseek", "enable_thinking", "chat_template_kwargs"];

pub(crate) fn valid_provider_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// The page's patch as the engine takes it: only provider keys, blank as `null` (`[]` for `models`).
pub(crate) fn engine_config(config: &Value) -> Rpc<Value> {
    let Some(patch) = config.as_object() else {
        return Err(RpcError::invalid_params("config must be an object"));
    };
    let mut out = Map::new();
    for (key, value) in patch {
        if !PROVIDER_KEYS.contains(&key.as_str()) {
            return Err(RpcError::invalid_params(format!("config.{key} is not a provider setting")));
        }
        let blank = value.is_null() || value.as_str().is_some_and(|s| s.trim().is_empty());
        let value = match (blank, key.as_str()) {
            (true, "models") => json!([]),
            // A flag the engine reads as `false` when it is not there.
            (true, "reasoning_passback") => json!(false),
            (true, _) => Value::Null,
            (false, "reasoning_passback") if !value.is_boolean() => {
                return Err(RpcError::invalid_params("config.reasoning_passback: true or false"));
            }
            (false, "thinking_toggle") if !value.as_str().is_some_and(|s| THINKING_TOGGLES.contains(&s)) => {
                return Err(RpcError::invalid_params(format!("config.thinking_toggle: one of {}", THINKING_TOGGLES.join(", "))));
            }
            (false, _) => value.clone(),
        };
        out.insert(key.clone(), value);
    }
    Ok(Value::Object(out))
}

/// `engine.config.getProvider`'s answer with every provider's `api_key` as the page may see it:
/// `***` and the last four characters at most.
pub(crate) fn redacted(mut answer: Value) -> Value {
    if let Some(providers) = answer.get_mut("providers").and_then(Value::as_object_mut) {
        for provider in providers.values_mut() {
            if let Some(key) = provider.get_mut("api_key") {
                *key = crate::fields::redact(key);
            }
        }
    }
    answer
}

/// The `default_provider` to send with this change, if any: the one asked for, which must be
/// configured once the change is made; else, when the current default is not (deleted, or never
/// set), the first provider left. `None` when the current one stands, or none is left.
pub(crate) fn default_after(
    configured: &BTreeSet<String>,
    current: Option<&str>,
    provider_id: &str,
    delete: bool,
    adds: bool,
    asked: Option<&str>,
) -> Rpc<Option<String>> {
    let mut after = configured.clone();
    if delete {
        after.remove(provider_id);
    } else if adds {
        after.insert(provider_id.to_string());
    }
    if let Some(id) = asked {
        if !after.contains(id) {
            return Err(RpcError::invalid_params(format!("default_provider `{id}` is not configured")));
        }
        return Ok(Some(id.to_string()));
    }
    match current {
        Some(id) if after.contains(id) => Ok(None),
        _ => Ok(after.into_iter().next()),
    }
}

/// Add, edit or remove a provider in the global tier; `scene` is the one the global tier is read
/// through (`engine.config.get` asks for one).
pub(crate) async fn set_provider(engine: &dyn Engine, scene: Option<&str>, params: &Value) -> Rpc<Value> {
    let provider_id = params
        .get("provider_id")
        .and_then(Value::as_str)
        .ok_or_else(|| RpcError::invalid_params("missing provider_id"))?;
    if !valid_provider_id(provider_id) {
        return Err(RpcError::invalid_params("provider_id: letters, digits, `-`, `_` and `.`"));
    }
    let delete = params.get("delete").and_then(Value::as_bool).unwrap_or(false);
    let config = match params.get("config").filter(|_| !delete) {
        Some(config) => Some(engine_config(config)?),
        None => None,
    };

    // What the global tier has now: its own file, not the merged view, since that is the one
    // written.
    let mut read = json!({"tier": "global"});
    if let Some(scene) = scene {
        read["scene"] = json!(scene);
    }
    let global = engine.call("engine.config.get", read).await?;
    let settings = global.get("settings").cloned().unwrap_or(Value::Null);
    let configured: BTreeSet<String> = settings
        .get("providers")
        .and_then(Value::as_object)
        .map(|p| p.keys().cloned().collect())
        .unwrap_or_default();
    let current = settings.get("default_provider").and_then(Value::as_str);
    let default = default_after(
        &configured,
        current,
        provider_id,
        delete,
        config.is_some(),
        params.get("default_provider").and_then(Value::as_str),
    )?;

    let mut request = json!({"tier": "global", "provider_id": provider_id});
    if delete {
        request["delete"] = json!(true);
    }
    if let Some(config) = config {
        request["config"] = config;
    }
    if let Some(id) = default {
        request["default_provider"] = json!(id);
    }
    // Tried first: the engine merges the change with every tier, builds the clients, and refuses
    // what would not route, having written nothing. A delete is not tried, as it never was: taking a
    // provider away is how a configuration that does not route is mended.
    if !delete {
        let mut dry = request.clone();
        dry["dry_run"] = json!(true);
        engine.call("engine.config.setProvider", dry).await?;
    }
    request["dry_run"] = json!(false);
    let written = engine.call("engine.config.setProvider", request).await?;

    let providers = redacted(engine.call("engine.config.getProvider", json!({})).await.unwrap_or(json!({})));
    // And to every instance whose template inherits `main`'s providers.
    let inherited = match engine.call("harnix.instances.inheritProviders", json!({})).await {
        Ok(answer) => answer.get("reloaded").cloned().unwrap_or(json!([])),
        Err(e) => {
            tracing::warn!(error = %e.message, "the instances inheriting providers were not updated");
            json!([])
        }
    };
    Ok(json!({
        "written": written.get("written_to").cloned().unwrap_or(Value::Null),
        // The `settings.local.json` the key went to, when one was written.
        "secrets_written": written.get("secrets_written_to").cloned().unwrap_or(Value::Null),
        // The engine re-reads its own settings as part of the write.
        "reloaded": true,
        "routing": written.get("routing").cloned().unwrap_or(Value::Null),
        "reloaded_instances": inherited,
        "providers": providers,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// An engine that answers from what it is given and remembers what it was asked.
    struct Fake {
        global: Value,
        refuse_dry_run: Option<&'static str>,
        calls: Mutex<Vec<(String, Value)>>,
    }

    impl Fake {
        fn new(global: Value) -> Self {
            Fake { global, refuse_dry_run: None, calls: Mutex::new(Vec::new()) }
        }
        fn writes(&self) -> Vec<Value> {
            let calls = self.calls.lock().unwrap();
            calls.iter().filter(|(m, _)| m == "engine.config.setProvider").map(|(_, p)| p.clone()).collect()
        }
    }

    #[async_trait::async_trait]
    impl Engine for Fake {
        async fn call(&self, method: &str, params: Value) -> Rpc<Value> {
            self.calls.lock().unwrap().push((method.to_string(), params.clone()));
            match method {
                "engine.config.get" => Ok(json!({"tier": "global", "settings": self.global})),
                "engine.config.setProvider" if params["dry_run"] == json!(true) => match self.refuse_dry_run {
                    Some(why) => Err(RpcError::invalid_params(why)),
                    None => Ok(json!({"dry_run": true, "written_to": "/g/settings.json", "routing": {"ok": true}})),
                },
                "engine.config.setProvider" => {
                    Ok(json!({"dry_run": false, "written_to": "/g/settings.json", "routing": {"ok": true}}))
                }
                "engine.config.getProvider" => Ok(json!({"providers": {}})),
                "harnix.instances.inheritProviders" => Ok(json!({"reloaded": ["heir"]})),
                other => panic!("not an engine call here: {other}"),
            }
        }
    }

    #[tokio::test]
    async fn a_provider_is_tried_then_written_into_the_global_tier() {
        let engine = Fake::new(Value::Null);
        let params = json!({"provider_id": "a", "config": {"api_type": "anthropic", "api_key": "k", "default_model": "m"}});
        let answer = set_provider(&engine, Some("code"), &params).await.unwrap();
        let writes = engine.writes();
        assert_eq!(writes.len(), 2);
        assert_eq!((&writes[0]["dry_run"], &writes[1]["dry_run"]), (&json!(true), &json!(false)));
        for write in &writes {
            assert_eq!(write["tier"], json!("global"));
            assert_eq!(write["config"]["api_key"], json!("k"));
            // The first provider is the default.
            assert_eq!(write["default_provider"], json!("a"));
        }
        assert_eq!(answer["written"], json!("/g/settings.json"));
        assert_eq!(answer["reloaded_instances"], json!(["heir"]));
        // The global tier was read through a scene: the engine asks for one.
        let calls = engine.calls.lock().unwrap();
        assert_eq!(calls[0], ("engine.config.get".to_string(), json!({"tier": "global", "scene": "code"})));
    }

    #[tokio::test]
    async fn what_the_dry_run_refuses_is_not_written() {
        let mut engine = Fake::new(json!({"providers": {"a": {}}, "default_provider": "a"}));
        engine.refuse_dry_run = Some("provider 'b': unrecognized api_type 'gemini'");
        let e = set_provider(&engine, None, &json!({"provider_id": "b", "config": {"api_type": "gemini"}})).await.unwrap_err();
        assert!(e.message.contains("api_type"), "{}", e.message);
        let writes = engine.writes();
        assert_eq!(writes.len(), 1, "written after a refused dry run");
        assert_eq!(writes[0]["dry_run"], json!(true));
        // The current default stands, so none was sent.
        assert!(writes[0].get("default_provider").is_none());
    }

    #[tokio::test]
    async fn deleting_the_default_hands_it_on_and_is_not_tried_first() {
        let engine = Fake::new(json!({"providers": {"a": {}, "b": {}}, "default_provider": "b"}));
        set_provider(&engine, None, &json!({"provider_id": "b", "delete": true})).await.unwrap();
        let writes = engine.writes();
        assert_eq!(writes.len(), 1);
        assert_eq!((&writes[0]["delete"], &writes[0]["default_provider"]), (&json!(true), &json!("a")));
        assert!(writes[0].get("config").is_none());
    }

    #[tokio::test]
    async fn a_default_that_is_not_configured_is_refused_before_the_engine_is_asked() {
        let engine = Fake::new(json!({"providers": {"a": {}}}));
        let e = set_provider(&engine, None, &json!({"provider_id": "a", "default_provider": "zz"})).await.unwrap_err();
        assert!(e.message.contains("zz"));
        assert!(engine.writes().is_empty());
        assert!(set_provider(&engine, None, &json!({"provider_id": "a b"})).await.is_err());
        let e = set_provider(&engine, None, &json!({"provider_id": "a", "config": {"shell": "x"}})).await.unwrap_err();
        assert!(e.message.contains("config.shell"));
    }

    #[test]
    fn blank_goes_to_the_engine_as_unset() {
        let config = engine_config(&json!({"api_type": "anthropic", "base_url": "  ", "api_key_env": null, "models": null, "default_model": "m"})).unwrap();
        assert_eq!(config, json!({"api_type": "anthropic", "base_url": null, "api_key_env": null, "models": [], "default_model": "m"}));
        // A key left out is not in the patch, which is how an edit keeps the stored key.
        assert!(config.get("api_key").is_none());
    }

    #[test]
    fn thinking_is_switched_by_a_field_the_engine_knows() {
        let config = engine_config(&json!({"thinking_toggle": "deepseek", "reasoning_passback": true})).unwrap();
        assert_eq!(config, json!({"thinking_toggle": "deepseek", "reasoning_passback": true}));
        // Blank gives them up: no toggle sent, the reasoning not passed back.
        let config = engine_config(&json!({"thinking_toggle": "", "reasoning_passback": null})).unwrap();
        assert_eq!(config, json!({"thinking_toggle": null, "reasoning_passback": false}));
        assert!(engine_config(&json!({"thinking_toggle": "always"})).unwrap_err().message.contains("deepseek"));
        assert!(engine_config(&json!({"reasoning_passback": "yes"})).is_err());
    }

    #[test]
    fn a_key_goes_to_the_page_as_its_last_four_at_most() {
        let answer = redacted(json!({"providers": {
            "whole": {"api_key": "sk-0123456789abcd", "api_key_env": "DEEPSEEK_API_KEY"},
            "engine": {"api_key": "***wxyz"},
            "short": {"api_key": "abc"},
            "env": {"api_key": null, "api_key_env": "OPENAI_API_KEY"},
        }, "default_provider": "whole"}));
        let providers = &answer["providers"];
        assert_eq!(providers["whole"]["api_key"], json!("***abcd"));
        assert_eq!(providers["engine"]["api_key"], json!("***wxyz"));
        assert_eq!(providers["short"]["api_key"], json!("***"));
        assert_eq!(providers["env"]["api_key"], Value::Null);
        // The name of the variable is not a secret: the page says where the key comes from.
        assert_eq!(providers["env"]["api_key_env"], json!("OPENAI_API_KEY"));
        assert_eq!(answer["default_provider"], json!("whole"));
        assert!(!answer.to_string().contains("0123456789"));
    }

    #[test]
    fn there_is_a_default_while_there_are_providers() {
        let ids = |list: &[&str]| list.iter().map(|s| s.to_string()).collect::<BTreeSet<_>>();
        // The first one added is the default.
        assert_eq!(default_after(&ids(&[]), None, "a", false, true, None).unwrap().as_deref(), Some("a"));
        // An edit leaves the default where it is.
        assert_eq!(default_after(&ids(&["a", "b"]), Some("a"), "b", false, true, None).unwrap(), None);
        // A default that is gone is replaced by the first.
        assert_eq!(default_after(&ids(&["a", "b"]), Some("x"), "b", false, true, None).unwrap().as_deref(), Some("a"));
        // The last one deleted leaves nothing to name.
        assert_eq!(default_after(&ids(&["a"]), Some("a"), "a", true, false, None).unwrap(), None);
        // Asking for one that is being deleted is refused.
        assert!(default_after(&ids(&["a", "b"]), Some("a"), "b", true, false, Some("b")).is_err());
        // Making one the default without a config: it must be there already.
        assert_eq!(default_after(&ids(&["a", "b"]), Some("a"), "b", false, false, Some("b")).unwrap().as_deref(), Some("b"));
        assert!(default_after(&ids(&["a"]), Some("a"), "c", false, false, Some("c")).is_err());
    }
}
