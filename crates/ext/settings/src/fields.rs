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
    #[allow(dead_code)] // no field is one; the page draws it all the same
    Secret,
    /// Strings, each not blank.
    List,
    /// Rows of the columns; a column marked required must be in every row, no other key may be.
    Rules(&'static [Column]),
    /// Where a task's model calls go: a provider's id (its default model), or
    /// `{"provider": id, "model": name}` — the two forms a `task_models` entry takes.
    Route,
    /// Any JSON, shown and never written: the page says where a value stands and where its file
    /// is. A field of it is declared with no writable tier.
    Json,
    /// As `Json`, with an object of lists (hooks by event) shown as how many entries each has:
    /// what a hook runs may carry a credential, and the page has no business showing it.
    Counts,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Column {
    pub key: &'static str,
    /// `None`: text that is not blank. `Some`: one of the options; an optional one left out is its
    /// first.
    pub options: Option<&'static [&'static str]>,
    pub required: bool,
    /// The text key of its heading; the key itself when none.
    pub label: Option<&'static str>,
    /// What a text cell must be, beyond not blank.
    pub check: Option<Cell>,
}

/// A text cell's own shape, checked the same way on the page.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Cell {
    /// A permission rule's tool: `Name` or `Name(content)`, the name `__`-separated segments.
    Tool,
}

impl Column {
    /// Text that is not blank, optional.
    pub(crate) const fn text(key: &'static str) -> Self {
        Column { key, options: None, required: false, label: None, check: None }
    }
    /// One of the options, optional.
    pub(crate) const fn select(key: &'static str, options: &'static [&'static str]) -> Self {
        Column { options: Some(options), ..Column::text(key) }
    }
    pub(crate) const fn required(self) -> Self {
        Column { required: true, ..self }
    }
    pub(crate) const fn label(self, label: &'static str) -> Self {
        Column { label: Some(label), ..self }
    }
    pub(crate) const fn check(self, check: Cell) -> Self {
        Column { check: Some(check), ..self }
    }
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
    /// The prefix of its options' text keys (`<prefix>.<option>`); `None`: shown as they are.
    pub option_labels: Option<&'static str>,
    /// Every tier's value applies side by side instead of the highest one standing for the rest:
    /// a tier's own rows are what it edits, and the engine's value is all of them together.
    pub additive: bool,
}

impl Field {
    /// A field every tier may write, with no default the page is told, its label by its key.
    pub(crate) const fn new(key: &'static str, ty: Type, section: &'static str) -> Self {
        Field {
            key,
            ty,
            default: Default::None,
            tiers: TIERS,
            section,
            label: None,
            hint: None,
            risk: None,
            option_labels: None,
            additive: false,
            offer: Offer::None,
        }
    }
    pub(crate) const fn default(self, default: Default) -> Self {
        Field { default, ..self }
    }
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
    pub(crate) const fn option_labels(self, prefix: &'static str) -> Self {
        Field { option_labels: Some(prefix), ..self }
    }
    pub(crate) const fn additive(self) -> Self {
        Field { additive: true, ..self }
    }
}

/// The tiers a field is written in when only the global one may: what the engine reads from there
/// alone, or what a project has no say in.
const GLOBAL: &[&str] = &["global"];
/// No tier: a field the page shows and the engine's file is edited for.
const READ_ONLY: &[&str] = &[];

/// The settings a person changes from the page. Every entry is a promise that writing it does
/// something predictable; the order is the order a section draws them in.
pub(crate) const FIELDS: &[Field] = &[
    Field::new("memory_enabled", Type::Toggle, "engine").default(Default::Bool(true)),
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
    // Every `task_models` route is here, the web's two included; `main` is the default provider and
    // the model a session picks: not a row of its own.
    Field::new("task_models.subagent", Type::Route, "task-models").hint("settings.hint.task_models"),
    Field::new("task_models.compact", Type::Route, "task-models").hint("settings.hint.task_models"),
    Field::new("task_models.memory", Type::Route, "task-models").hint("settings.hint.task_models"),
    Field::new("task_models.title", Type::Route, "task-models").hint("settings.hint.task_models"),
    // Unset, these two are not the default provider's: search is the session's own model, and a
    // fetched page is read by no model at all.
    Field::new("task_models.web_search", Type::Route, "task-models").hint("settings.hint.task_models.web_search"),
    Field::new("task_models.web_fetch", Type::Route, "task-models").hint("settings.hint.task_models.web_fetch"),
    // permissions, sandbox, network
    Field::new("permission_mode", Type::Select(&["default", "acceptEdits", "plan", "dontAsk", "bypassPermissions"]), "permissions")
        .default(Default::Str("default"))
        .option_labels("settings.option.permission_mode")
        .hint("settings.hint.permission_mode")
        .risk(&["bypassPermissions"], "settings.risk.bypassPermissions"),
    Field::new("allow_client_permission_override", Type::Toggle, "permissions").default(Default::Bool(false)),
    // Every tier's rules apply together, and a project's `allow` counts only once it is trusted.
    Field::new("permission_rules", Type::Rules(PERMISSION_RULE), "permissions")
        .additive()
        .hint("settings.hint.permission_rules"),
    Field::new("sandbox.network_mode", Type::Select(&["unrestricted", "deny_all", "allowlist", "proxy"]), "sandbox")
        .default(Default::Str("unrestricted"))
        .option_labels("settings.option.sandbox.network_mode")
        .hint("settings.hint.sandbox.network_mode"),
    // The string form only (`127.0.0.1:3128`); one with `socks` or `no_proxy` is written in the file.
    Field::new("sandbox.egress_proxy", Type::Text, "sandbox").hint("settings.hint.sandbox.egress_proxy"),
    Field::new("sandbox.additional_directories", Type::List, "sandbox").hint("settings.hint.sandbox.additional_directories"),
    Field::new("sandbox.read_directories", Type::List, "sandbox").hint("settings.hint.sandbox.read_directories"),
    Field::new("sandbox.require_enforcement", Type::Toggle, "sandbox")
        .default(Default::Bool(false))
        .hint("settings.hint.sandbox.require_enforcement"),
    // The engine reads it from the global tier only, at start.
    Field::new("network.use_system_proxy", Type::Toggle, "network")
        .default(Default::Bool(true))
        .tiers(GLOBAL)
        .hint("settings.hint.network.use_system_proxy"),
    Field::new("sandbox.allowed_domains", Type::List, "network").hint("settings.hint.sandbox.allowed_domains"),
    // general & display
    // What the engine answers in; the shell's language choice writes it (`auto` gives it back).
    Field::new("language", Type::Text, "general").tiers(GLOBAL),
    // privacy & advanced
    // Harnix's instances, `main` among them, start with it off in the global tier
    // (docs/harnix-app.md, settings); the engine's own default is on, so a cleared value reads as on
    // until the next start.
    Field::new("telemetry_enabled", Type::Toggle, "privacy")
        .default(Default::Bool(false))
        .tiers(GLOBAL)
        .hint("settings.hint.telemetry_enabled"),
    Field::new("telemetry_url", Type::Text, "privacy").tiers(GLOBAL).hint("settings.hint.telemetry_url"),
    Field::new("compaction", Type::Json, "advanced").tiers(READ_ONLY),
    Field::new("execution", Type::Json, "advanced").tiers(READ_ONLY),
    Field::new("hooks_config", Type::Counts, "advanced").tiers(READ_ONLY),
    Field::new("team", Type::Json, "advanced").tiers(READ_ONLY),
    Field::new("checkpoints", Type::Json, "advanced").tiers(READ_ONLY),
];

/// A `permission_rules` entry as the engine reads it: `{tool, action, match?}`, the path, command
/// or URL pattern inside the tool's parentheses (`Bash(git push:*)`, `Edit(src/**)`).
const PERMISSION_RULE: &[Column] = &[
    Column::text("tool").required().label("settings.column.permission_rules.tool").check(Cell::Tool),
    Column::select("action", &["allow", "ask", "deny"]).required().label("settings.column.permission_rules.action"),
    // `exact`: what "allow every time" keeps; the engine reads it on an allow only.
    Column::select("match", &["pattern", "exact"]).label("settings.column.permission_rules.match"),
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
        Type::Json | Type::Counts => Err("not written from here: edit the settings file".into()),
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
                            if let (Some(Cell::Tool), Some(text)) = (column.check, cell.as_str()) {
                                tool_pattern(text).map_err(|why| format!("row {}: `{}` {why}", i + 1, column.key))?;
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
        // A `models` entry is an id, or an object with its id and the model's limits.
        let ids = named.filter_map(|m| m.as_str().or_else(|| m.get("id")?.as_str()));
        for model in ids.filter(|m| !m.trim().is_empty()) {
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

/// Whether two rows of the columns say the same: an optional select left out is its first option,
/// which is how the engine writes a default back (a rule's `match` is `pattern` unless it says not).
pub(crate) fn same_row(columns: &[Column], a: &Value, b: &Value) -> bool {
    let cell = |row: &Value, column: &Column| match (row.get(column.key), column.options) {
        (None | Some(Value::Null), Some(options)) if !column.required => options.first().map(|o| json!(o)),
        (cell, _) => cell.filter(|c| !c.is_null()).cloned(),
    };
    let known = |row: &Value| row.as_object().is_some_and(|o| o.keys().all(|k| columns.iter().any(|c| c.key == k.as_str())));
    if !known(a) || !known(b) {
        return a == b;
    }
    columns.iter().all(|column| cell(a, column) == cell(b, column))
}

/// Whether `text` is a tool the engine's rules can name: `Name` or `Name(content)`, the name one
/// or more `__`-separated segments (`mcp__github` covers that server's tools). A name has no
/// wildcard: a rule names a prefix, and `mcp__github__*` would match no tool at all.
pub(crate) fn tool_pattern(text: &str) -> Result<(), String> {
    let text = text.trim();
    let (name, content) = match text.find('(') {
        Some(open) if text.ends_with(')') => (&text[..open], Some(&text[open + 1..text.len() - 1])),
        Some(_) => return Err("opens a `(` it does not close at the end".into()),
        None if text.contains(')') => return Err("closes a `)` it did not open".into()),
        None => (text, None),
    };
    let name = name.trim();
    if name.contains('*') {
        return Err("has a wildcard in the tool's name: write the prefix (`mcp__github`) for all its tools".into());
    }
    let segment = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'));
    if name.is_empty() || !name.split("__").all(segment) {
        return Err(format!("`{name}` is not a tool's name: letters, digits, `-`, `_`, segments joined by `__`"));
    }
    if content.is_some_and(|c| c.trim().is_empty()) {
        return Err("has `()` with nothing in it: leave them out for every call of the tool".into());
    }
    Ok(())
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
        Type::Json | Type::Counts => "json",
    }
}

/// A value as the page may see it: a secret redacted, hooks counted, anything else as it is.
pub(crate) fn shown(ty: Type, value: &Value) -> Value {
    match ty {
        Type::Secret => redact(value),
        Type::Counts => match value {
            Value::Object(events) => Value::Object(
                events
                    .iter()
                    .map(|(event, hooks)| (event.clone(), json!(hooks.as_array().map_or(1, Vec::len))))
                    .collect(),
            ),
            other => other.clone(),
        },
        _ => value.clone(),
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
    if let (Type::Select(options), Some(prefix)) = (field.ty, field.option_labels) {
        let labels: Map<String, Value> = options.iter().map(|o| ((*o).to_string(), json!(format!("{prefix}.{o}")))).collect();
        out.insert("optionLabels".into(), Value::Object(labels));
    }
    if let Type::Number { min, max } = field.ty {
        out.insert("min".into(), json!(min));
        out.insert("max".into(), json!(max));
    }
    if let Type::Rules(columns) = field.ty {
        let columns: Vec<Value> = columns
            .iter()
            .map(|c| {
                let mut column = json!({
                    "key": c.key,
                    "type": if c.options.is_some() { "select" } else { "text" },
                    "options": c.options,
                    "required": c.required,
                });
                if let Some(label) = c.label {
                    column["label"] = json!(label);
                }
                if let Some(Cell::Tool) = c.check {
                    column["check"] = json!("tool");
                }
                column
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
    if field.additive {
        out.insert("additive".into(), json!(true));
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
        Column::text("tool").required(),
        Column::select("action", &["allow", "deny", "ask"]).required(),
        Column::select("match", &["exact"]),
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
    fn every_key_is_declared_once() {
        let mut keys: Vec<&str> = FIELDS.iter().map(|f| f.key).collect();
        let all = keys.len();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), all, "a key declared twice");
        for key in ["permission_mode", "memory_enabled", "allow_client_permission_override", "model.model_name", "model.max_tokens"] {
            assert_eq!(find(key).map(|f| f.tiers), Some(TIERS), "{key}");
        }
    }

    #[test]
    fn a_rule_names_a_tool_the_engine_can_match() {
        for good in ["Bash", "Bash(git push:*)", "Edit(src/**)", "Read(~/.ssh/**)", "WebFetch(https://*.example.com/**)", "mcp__github", "plugin__github-tools__open_pr", "Bash(echo (hi))"] {
            assert!(tool_pattern(good).is_ok(), "{good}");
        }
        for bad in ["", "  ", "Bash(git", "Bash)", "(git status)", "Bash()", "mcp__github__*", "*", "mcp__", "__x", "Bash Tool", "Bash (ls"] {
            assert!(tool_pattern(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn permission_rules_take_the_engine_s_shape_and_refuse_a_tool_it_cannot_match() {
        let rules = find("permission_rules").unwrap();
        assert!(check(rules.ty, &json!([{"tool": "Bash(git push:*)", "action": "deny"}, {"tool": "Edit(src/**)", "action": "allow", "match": "pattern"}])).is_ok());
        assert!(check(rules.ty, &json!([{"tool": "Bash(ls)", "action": "allow", "match": "exact"}])).is_ok());
        let e = check(rules.ty, &json!([{"tool": "Bash", "action": "allow"}, {"tool": "mcp__x__*", "action": "deny"}])).unwrap_err();
        assert!(e.starts_with("row 2: `tool`") && e.contains("wildcard"), "{e}");
        assert!(check(rules.ty, &json!([{"tool": "Bash", "effect": "allow"}])).is_err(), "the engine's key is `action`");
        assert!(check(rules.ty, &json!([{"tool": "Bash", "action": "maybe"}])).is_err());
        let view = declaration(rules);
        assert_eq!(view["additive"], json!(true));
        assert_eq!(view["columns"][0]["check"], json!("tool"));
        assert_eq!(view["columns"][1]["options"], json!(["allow", "ask", "deny"]));
        assert_eq!(view["columns"][1]["label"], json!("settings.column.permission_rules.action"));
        assert!(view["columns"][2].get("check").is_none());
    }

    #[test]
    fn a_row_left_at_an_optional_select_s_first_option_is_the_same_row() {
        let Type::Rules(columns) = find("permission_rules").unwrap().ty else { panic!() };
        let bare = json!({"tool": "Bash(ls)", "action": "allow"});
        assert!(same_row(columns, &bare, &json!({"tool": "Bash(ls)", "action": "allow", "match": "pattern"})));
        assert!(!same_row(columns, &bare, &json!({"tool": "Bash(ls)", "action": "allow", "match": "exact"})));
        assert!(!same_row(columns, &bare, &json!({"tool": "Bash(ls)", "action": "deny"})));
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
        assert!(check(Type::Route, &json!({"provider": "a", "model": " "})).is_err());
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
        // Every route in one section, the web's two included.
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
            "b": {"default_model": null, "models": ["a-2", {"id": "b-1", "context_window": 128000}]},
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
    fn the_permissions_sandbox_and_network_fields_are_the_engine_s_keys() {
        let mode = declaration(find("permission_mode").unwrap());
        assert_eq!(mode["section"], json!("permissions"));
        assert!(mode["options"].as_array().unwrap().contains(&json!("dontAsk")));
        assert_eq!(mode["optionLabels"]["bypassPermissions"], json!("settings.option.permission_mode.bypassPermissions"));
        assert_eq!(find("allow_client_permission_override").unwrap().section, "permissions");
        assert_eq!(declaration(find("sandbox.network_mode").unwrap())["default"], json!("unrestricted"));
        assert_eq!(declaration(find("network.use_system_proxy").unwrap())["writable"], json!(["global"]));
        for key in ["sandbox.additional_directories", "sandbox.read_directories", "sandbox.allowed_domains"] {
            assert_eq!(declaration(find(key).unwrap())["type"], json!("list"), "{key}");
        }
        assert!(check(find("sandbox.network_mode").unwrap().ty, &json!("allowlist")).is_ok());
        assert!(check(find("sandbox.network_mode").unwrap().ty, &json!("none")).is_err());
    }

    #[test]
    fn general_and_privacy_fields_are_global_and_advanced_ones_are_read_only() {
        let language = find("language").unwrap();
        assert_eq!((language.section, language.tiers), ("general", GLOBAL));
        let telemetry = declaration(find("telemetry_enabled").unwrap());
        assert_eq!((&telemetry["section"], &telemetry["default"]), (&json!("privacy"), &json!(false)));
        assert_eq!(telemetry["hint"], json!("settings.hint.telemetry_enabled"));
        assert!(check(find("telemetry_url").unwrap().ty, &json!("http://localhost:4318")).is_ok());
        for key in ["compaction", "execution", "hooks_config", "team", "checkpoints"] {
            let field = find(key).unwrap();
            assert_eq!((field.section, field.tiers), ("advanced", READ_ONLY), "{key}");
            let declared = declaration(field);
            assert_eq!((&declared["type"], &declared["writable"]), (&json!("json"), &json!([])), "{key}");
            assert!(check(field.ty, &json!({})).is_err(), "{key} is not written from the page");
        }
    }

    #[test]
    fn hooks_are_shown_as_counts_and_never_as_what_they_run() {
        let hooks = json!({"PreToolUse": [{"type": "http", "url": "https://h", "headers": {"Authorization": "Bearer abc"}}, {"type": "command", "command": "x"}], "SessionStart": []});
        let shown = shown(Type::Counts, &hooks);
        assert_eq!(shown, json!({"PreToolUse": 2, "SessionStart": 0}));
        assert!(!shown.to_string().contains("abc"));
        let execution = json!({"max_agent_depth": 3});
        assert_eq!(super::shown(Type::Json, &execution), execution);
    }

    #[test]
    fn a_secret_is_never_read_back() {
        assert_eq!(redact(&json!("sk-0123456789abcd")), json!("***abcd"));
        assert_eq!(redact(&json!("short")), json!("***"));
        assert_eq!(redact(&json!("***abcd")), json!("***abcd"));
        assert_eq!(redact(&Value::Null), Value::Null);
    }
}
