//! The field table: every setting the settings page edits, declared once.
//!
//! A row says what the key is, what it accepts, its default, which tiers may write it, which
//! section of the page shows it, the text keys its label and hint have, and whether a value of it
//! is a risk the page confirms first. `settings.describe` hands the declarations to the page,
//! which draws its forms from them; `settings.write` checks every value against them. Adding a
//! setting is adding a row here (and its text in the interface's dictionaries).
//!
//! A whitelist, not a passthrough: a settings file is a program's behaviour, and "set any key to
//! any JSON" is a remote configuration channel, not a feature.

use serde_json::{json, Map, Value};

/// What a field accepts. Anything else is refused before a file is touched.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Type {
    /// `true` or `false`.
    Toggle,
    /// One of the options.
    Select(&'static [&'static str]),
    /// A whole number within the bounds, both included.
    Number { min: u64, max: u64 },
    /// Text that is not blank.
    Text,
    /// Text that is not blank and is never read back: `describe` says only whether it is set.
    Secret,
    /// Strings, each not blank.
    #[allow(dead_code)] // no field of the five is one; the page draws it all the same
    List,
    /// Rows of the columns; a column marked required must be in every row, no other key may be.
    #[allow(dead_code)] // as `List`
    Rules(&'static [Column]),
    /// Where a task's model calls go: a provider's id (its default model), or
    /// `{"provider": id, "model": name}` — the two forms a `task_models` entry takes.
    Route,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Column {
    pub key: &'static str,
    /// `None`: text that is not blank. `Some`: one of the options.
    pub options: Option<&'static [&'static str]>,
    pub required: bool,
}

/// What a field is when no tier writes it, as far as the page is told. `None` when it depends on
/// how the engine was started (the model, its token budget), which the page has no way to say.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Default {
    None,
    Bool(bool),
    /// A number's; none of the fields so far has one the page is told.
    #[allow(dead_code)]
    Int(u64),
    Str(&'static str),
}

/// A value the page confirms before writing: `values` empty is any value.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Risk {
    pub values: &'static [&'static str],
    /// The text key of what the confirmation says.
    pub message: &'static str,
}

/// Values the page offers for a field that only the configured providers know: `describe` fills
/// them in for the (project, scene) it reads. An offer, not a limit: a name not offered is accepted.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Offer {
    None,
    /// Every model a provider names, its default and its allowed ones.
    Models,
}

pub(crate) const TIERS: &[&str] = &["global", "scene", "project"];

#[derive(Debug, Clone, Copy)]
pub(crate) struct Field {
    /// Dotted path into settings.json.
    pub key: &'static str,
    pub ty: Type,
    pub default: Default,
    /// The tiers that may write it, of [`TIERS`].
    pub tiers: &'static [&'static str],
    /// The section of the settings page it is drawn in.
    pub section: &'static str,
    /// Text keys. `None` for the label: `settings.field.<key>`.
    pub label: Option<&'static str>,
    pub hint: Option<&'static str>,
    pub risk: Option<Risk>,
    pub offer: Offer,
}

impl Field {
    /// A field every tier may write, with no default the page is told, its label by its key.
    pub(crate) const fn new(key: &'static str, ty: Type, section: &'static str) -> Self {
        Field { key, ty, default: Default::None, tiers: TIERS, section, label: None, hint: None, risk: None, offer: Offer::None }
    }
    pub(crate) const fn default(self, default: Default) -> Self {
        Field { default, ..self }
    }
    #[allow(dead_code)]
    pub(crate) const fn tiers(self, tiers: &'static [&'static str]) -> Self {
        Field { tiers, ..self }
    }
    pub(crate) const fn hint(self, hint: &'static str) -> Self {
        Field { hint: Some(hint), ..self }
    }
    pub(crate) const fn risk(self, values: &'static [&'static str], message: &'static str) -> Self {
        Field { risk: Some(Risk { values, message }), ..self }
    }
    pub(crate) const fn offer(self, offer: Offer) -> Self {
        Field { offer, ..self }
    }
}

/// The settings a person changes from the page. Every entry is a promise that writing it does
/// something predictable; the order is the order a section draws them in.
pub(crate) const FIELDS: &[Field] = &[
    Field::new("permission_mode", Type::Select(&["default", "acceptEdits", "plan", "bypassPermissions"]), "engine")
        .default(Default::Str("default"))
        .risk(&["bypassPermissions"], "settings.risk.bypassPermissions"),
    Field::new("memory_enabled", Type::Toggle, "engine").default(Default::Bool(true)),
    Field::new("allow_client_permission_override", Type::Toggle, "engine").default(Default::Bool(false)),
    Field::new("model.model_name", Type::Text, "models").offer(Offer::Models),
    Field::new("model.max_tokens", Type::Number { min: 1, max: 1_000_000 }, "models"),
    // models: the engine's own keys (agix/docs/schemas/settings.schema.json). Its defaults depend
    // on how it was started, so none is said here; unset, `effort` is the endpoint's to choose.
    // Effort and thinking apply to every session: the engine takes neither per session.
    Field::new("model.effort", Type::Select(&["low", "medium", "high", "xhigh", "max"]), "models")
        .hint("settings.hint.model.effort"),
    Field::new("model.thinking_mode", Type::Select(&["auto", "off", "on"]), "models")
        .hint("settings.hint.model.thinking_mode"),
    Field::new("model.max_output_tokens", Type::Number { min: 1, max: 1_000_000 }, "models")
        .hint("settings.hint.model.max_output_tokens"),
    Field::new("model.fallback_model", Type::Text, "models")
        .offer(Offer::Models)
        .hint("settings.hint.model.fallback_model"),
    // `main` is the default provider and the model a session picks: not a row of its own.
    Field::new("task_models.subagent", Type::Route, "task-models").hint("settings.hint.task_models"),
    Field::new("task_models.compact", Type::Route, "task-models").hint("settings.hint.task_models"),
    Field::new("task_models.memory", Type::Route, "task-models").hint("settings.hint.task_models"),
    Field::new("task_models.title", Type::Route, "task-models").hint("settings.hint.task_models"),
    Field::new("task_models.web_search", Type::Route, "task-models").hint("settings.hint.task_models"),
    Field::new("task_models.web_fetch", Type::Route, "task-models").hint("settings.hint.task_models.web_fetch"),
];

pub(crate) fn find(key: &str) -> Option<&'static Field> {
    FIELDS.iter().find(|f| f.key == key)
}

/// Whether `value` is one the field accepts; why not when it is not. `null` is not a value here:
/// it is how a tier gives a setting back, and the caller takes it before asking.
pub(crate) fn check(ty: Type, value: &Value) -> Result<(), String> {
    let not_blank = |v: &Value| v.as_str().is_some_and(|s| !s.trim().is_empty());
    match ty {
        Type::Toggle if value.is_boolean() => Ok(()),
        Type::Toggle => Err("true or false".into()),
        Type::Select(options) if value.as_str().is_some_and(|s| options.contains(&s)) => Ok(()),
        Type::Select(options) => Err(format!("one of {}", options.join(", "))),
        Type::Number { min, max } if value.as_u64().is_some_and(|n| n >= min && n <= max) => Ok(()),
        Type::Number { min, max } => Err(format!("a whole number from {min} to {max}")),
        Type::Text | Type::Secret if not_blank(value) => Ok(()),
        Type::Text | Type::Secret => Err("text that is not blank".into()),
        Type::List => match value.as_array() {
            Some(items) if items.iter().all(not_blank) => Ok(()),
            _ => Err("a list of text, none of it blank".into()),
        },
        Type::Rules(columns) => {
            let Some(rows) = value.as_array() else { return Err("a list of rows".into()) };
            for (i, row) in rows.iter().enumerate() {
                let Some(row) = row.as_object() else { return Err(format!("row {}: not a row", i + 1)) };
                if let Some(extra) = row.keys().find(|k| !columns.iter().any(|c| c.key == k.as_str())) {
                    return Err(format!("row {}: `{extra}` is not a column", i + 1));
                }
                for column in columns {
                    match row.get(column.key) {
                        None | Some(Value::Null) if column.required => {
                            return Err(format!("row {}: `{}` is required", i + 1, column.key))
                        }
                        None | Some(Value::Null) => {}
                        Some(cell) => {
                            let fits = match column.options {
                                Some(options) => cell.as_str().is_some_and(|s| options.contains(&s)),
                                None => not_blank(cell),
                            };
                            if !fits {
                                return Err(format!("row {}: `{}` does not accept that", i + 1, column.key));
                            }
                        }
                    }
                }
            }
            Ok(())
        }
        Type::Route => match value {
            Value::String(_) if not_blank(value) => Ok(()),
            Value::Object(route) => {
                let provider_named = route.get("provider").is_some_and(not_blank);
                let model_fits = route.get("model").is_none_or(|m| m.is_null() || not_blank(m));
                let only_these = route.keys().all(|k| k == "provider" || k == "model");
                if provider_named && model_fits && only_these {
                    Ok(())
                } else {
                    Err("a provider, and optionally one of its models".into())
                }
            }
            _ => Err("a provider, and optionally one of its models".into()),
        },
    }
}

/// The provider a route sends to, when it names one.
pub(crate) fn route_provider(value: &Value) -> Option<&str> {
    match value {
        Value::String(s) => Some(s.as_str()),
        Value::Object(route) => route.get("provider").and_then(Value::as_str),
        _ => None,
    }
}

/// What the page offers for `field`, from the providers `engine.config.getProvider` answered: the
/// models they name (`suggestions`) for a field that offers them, the providers and their models
/// (`routes`) for a route. Empty for any other field.
pub(crate) fn offers(field: &Field, providers: &Value) -> Map<String, Value> {
    let configured = providers.get("providers").and_then(Value::as_object);
    let models_of = |provider: &Value| -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        let named = provider.get("default_model").into_iter().chain(provider.get("models").and_then(Value::as_array).into_iter().flatten());
        for model in named.filter_map(Value::as_str).filter(|m| !m.trim().is_empty()) {
            if !out.iter().any(|m| m == model) {
                out.push(model.to_string());
            }
        }
        out
    };
    let mut out = Map::new();
    if field.ty == Type::Route {
        let routes: Vec<Value> = configured
            .into_iter()
            .flatten()
            .map(|(id, provider)| json!({"provider": id, "default_model": provider.get("default_model").cloned().unwrap_or(Value::Null), "models": models_of(provider)}))
            .collect();
        out.insert("routes".into(), json!(routes));
    }
    if field.offer == Offer::Models {
        let mut all: Vec<String> = Vec::new();
        for model in configured.into_iter().flatten().flat_map(|(_, provider)| models_of(provider)) {
            if !all.contains(&model) {
                all.push(model);
            }
        }
        out.insert("suggestions".into(), json!(all));
    }
    out
}

/// The name of a type as the page reads it.
fn type_name(ty: Type) -> &'static str {
    match ty {
        Type::Toggle => "toggle",
        Type::Select(_) => "select",
        Type::Number { .. } => "number",
        Type::Text => "text",
        Type::Secret => "secret",
        Type::List => "list",
        Type::Rules(_) => "rules",
        Type::Route => "route",
    }
}

/// What `kind` said before there were types, kept for whoever reads it.
fn kind_name(ty: Type) -> &'static str {
    match ty {
        Type::Toggle => "flag",
        Type::Select(_) => "choice",
        Type::Number { .. } => "count",
        other => type_name(other),
    }
}

fn default_value(default: Default) -> Value {
    match default {
        Default::None => Value::Null,
        Default::Bool(b) => json!(b),
        Default::Int(n) => json!(n),
        Default::Str(s) => json!(s),
    }
}

/// A field's declaration, as `describe` gives it: everything the page needs to draw its control.
pub(crate) fn declaration(field: &Field) -> Map<String, Value> {
    let mut out = Map::new();
    out.insert("key".into(), json!(field.key));
    out.insert("type".into(), json!(type_name(field.ty)));
    out.insert("kind".into(), json!(kind_name(field.ty)));
    out.insert(
        "options".into(),
        match field.ty {
            Type::Select(options) => json!(options),
            _ => Value::Null,
        },
    );
    if let Type::Number { min, max } = field.ty {
        out.insert("min".into(), json!(min));
        out.insert("max".into(), json!(max));
    }
    if let Type::Rules(columns) = field.ty {
        let columns: Vec<Value> = columns
            .iter()
            .map(|c| {
                json!({
                    "key": c.key,
                    "type": if c.options.is_some() { "select" } else { "text" },
                    "options": c.options,
                    "required": c.required,
                })
            })
            .collect();
        out.insert("columns".into(), json!(columns));
    }
    if field.default != Default::None {
        out.insert("default".into(), default_value(field.default));
    }
    out.insert("writable".into(), json!(field.tiers));
    out.insert("section".into(), json!(field.section));
    out.insert("label".into(), json!(field.label.map_or_else(|| format!("settings.field.{}", field.key), str::to_string)));
    if let Some(hint) = field.hint {
        out.insert("hint".into(), json!(hint));
    }
    if let Some(risk) = field.risk {
        out.insert("risk".into(), json!({"values": risk.values, "message": risk.message}));
    }
    out
}

/// A secret as the page may see it: whether it is set, and its last four characters when it is
/// long enough for those not to give it away. The engine redacts some itself; what it redacted is
/// left as it is.
pub(crate) fn redact(value: &Value) -> Value {
    match value {
        Value::Null => Value::Null,
        Value::String(s) if s.starts_with("***") => value.clone(),
        Value::String(s) if s.chars().count() >= 12 => {
            let tail: String = s.chars().rev().take(4).collect::<Vec<_>>().into_iter().rev().collect();
            json!(format!("***{tail}"))
        }
        _ => json!("***"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RULES: Type = Type::Rules(&[
        Column { key: "tool", options: None, required: true },
        Column { key: "action", options: Some(&["allow", "deny", "ask"]), required: true },
        Column { key: "match", options: Some(&["exact"]), required: false },
    ]);

    #[test]
    fn every_type_refuses_what_it_does_not_mean() {
        assert!(check(Type::Toggle, &json!(true)).is_ok());
        assert!(check(Type::Toggle, &json!("true")).is_err());
        assert!(check(Type::Select(&["plan"]), &json!("plan")).is_ok());
        assert!(check(Type::Select(&["plan"]), &json!("yolo")).is_err());
        let count = Type::Number { min: 1, max: 1_000_000 };
        assert!(check(count, &json!(4000)).is_ok());
        assert!(check(count, &json!(0)).is_err());
        assert!(check(count, &json!(1_000_001)).is_err());
        assert!(check(count, &json!("4000")).is_err());
        assert!(check(count, &json!(4.5)).is_err());
        assert!(check(Type::Text, &json!("   ")).is_err());
        assert!(check(Type::Text, &json!("deepseek")).is_ok());
        assert!(check(Type::Secret, &json!("")).is_err());
        assert!(check(Type::List, &json!(["a", "b"])).is_ok());
        assert!(check(Type::List, &json!(["a", " "])).is_err());
        assert!(check(Type::List, &json!("a")).is_err());
        assert!(check(RULES, &json!([{"tool": "Bash(git status)", "action": "allow"}])).is_ok());
        assert!(check(RULES, &json!([{"tool": "Bash", "action": "allow", "match": "exact"}])).is_ok());
        assert!(check(RULES, &json!([{"tool": "Bash", "action": "maybe"}])).is_err());
        assert!(check(RULES, &json!([{"action": "deny"}])).is_err());
        assert!(check(RULES, &json!([{"tool": "Bash", "action": "ask", "shell": "x"}])).is_err());
    }

    #[test]
    fn a_declaration_says_what_the_page_draws() {
        let mode = declaration(find("permission_mode").unwrap());
        assert_eq!(mode["type"], json!("select"));
        assert_eq!(mode["kind"], json!("choice"));
        assert_eq!(mode["default"], json!("default"));
        assert_eq!(mode["writable"], json!(["global", "scene", "project"]));
        assert_eq!(mode["label"], json!("settings.field.permission_mode"));
        assert_eq!(mode["risk"]["values"], json!(["bypassPermissions"]));
        let tokens = declaration(find("model.max_tokens").unwrap());
        assert_eq!((&tokens["type"], &tokens["kind"], &tokens["min"]), (&json!("number"), &json!("count"), &json!(1)));
        assert!(tokens.get("default").is_none(), "the engine's start-up budget is not the page's to say");
        let rules = declaration(&Field::new("permission_rules", RULES, "permissions").tiers(&["global"]).hint("h"));
        assert_eq!(rules["columns"][1]["options"], json!(["allow", "deny", "ask"]));
        assert_eq!((&rules["writable"], &rules["hint"]), (&json!(["global"]), &json!("h")));
    }

    #[test]
    fn the_five_fields_are_the_ones_there_were() {
        let keys: Vec<&str> = FIELDS.iter().map(|f| f.key).collect();
        for key in ["permission_mode", "memory_enabled", "allow_client_permission_override", "model.model_name", "model.max_tokens"] {
            assert!(keys.contains(&key), "{key} is gone");
        }
        assert!(keys.iter().enumerate().all(|(i, k)| !keys[..i].contains(k)), "a key is declared twice");
    }

    #[test]
    fn a_route_is_a_provider_or_a_provider_and_its_model() {
        assert!(check(Type::Route, &json!("deepseek")).is_ok());
        assert!(check(Type::Route, &json!({"provider": "deepseek"})).is_ok());
        assert!(check(Type::Route, &json!({"provider": "deepseek", "model": "deepseek-chat"})).is_ok());
        assert!(check(Type::Route, &json!({"provider": "deepseek", "model": null})).is_ok());
        assert!(check(Type::Route, &json!(" ")).is_err());
        assert!(check(Type::Route, &json!({"model": "m"})).is_err());
        assert!(check(Type::Route, &json!({"provider": "a", "model": ""})).is_err());
        assert!(check(Type::Route, &json!({"provider": "a", "base_url": "http://x"})).is_err());
        assert!(check(Type::Route, &json!(3)).is_err());
        assert_eq!(route_provider(&json!({"provider": "a", "model": "m"})), Some("a"));
        assert_eq!(route_provider(&json!("b")), Some("b"));
    }

    #[test]
    fn the_model_fields_are_the_engines_keys() {
        // Each one a key the engine reads (agix/docs/schemas/settings.schema.json), none of them
        // with a default the page could know.
        for key in ["model.effort", "model.thinking_mode", "model.max_output_tokens", "model.fallback_model"] {
            let field = find(key).unwrap_or_else(|| panic!("{key} is not declared"));
            assert_eq!((field.section, field.default), ("models", Default::None), "{key}");
        }
        assert_eq!(find("model.effort").unwrap().ty, Type::Select(&["low", "medium", "high", "xhigh", "max"]));
        for task in ["subagent", "compact", "memory", "title", "web_search", "web_fetch"] {
            let field = find(&format!("task_models.{task}")).unwrap_or_else(|| panic!("{task} is not routed"));
            assert_eq!((field.ty, field.section), (Type::Route, "task-models"));
        }
        assert!(find("task_models.main").is_none(), "main is the default provider's");
        assert!(check(Type::Select(&["auto", "off", "on"]), &json!("on")).is_ok());
    }

    #[test]
    fn the_configured_models_are_offered() {
        let providers = json!({"providers": {
            "a": {"default_model": "a-1", "models": ["a-1", "a-2"]},
            "b": {"default_model": null, "models": ["a-2", "b-1"]},
        }});
        let fallback = offers(find("model.fallback_model").unwrap(), &providers);
        assert_eq!(fallback["suggestions"], json!(["a-1", "a-2", "b-1"]));
        assert!(fallback.get("routes").is_none());
        let title = offers(find("task_models.title").unwrap(), &providers);
        assert_eq!(title["routes"], json!([
            {"provider": "a", "default_model": "a-1", "models": ["a-1", "a-2"]},
            {"provider": "b", "default_model": null, "models": ["a-2", "b-1"]},
        ]));
        // Nothing configured: nothing offered, and the field still takes a name typed in.
        assert_eq!(offers(find("model.fallback_model").unwrap(), &json!({}))["suggestions"], json!([]));
        assert!(offers(find("memory_enabled").unwrap(), &providers).is_empty());
        let route = declaration(find("task_models.web_fetch").unwrap());
        assert_eq!((&route["type"], &route["hint"]), (&json!("route"), &json!("settings.hint.task_models.web_fetch")));
    }

    #[test]
    fn a_secret_is_never_read_back() {
        assert_eq!(redact(&json!("sk-0123456789abcd")), json!("***abcd"));
        assert_eq!(redact(&json!("short")), json!("***"));
        assert_eq!(redact(&json!("***abcd")), json!("***abcd"));
        assert_eq!(redact(&Value::Null), Value::Null);
    }
}
