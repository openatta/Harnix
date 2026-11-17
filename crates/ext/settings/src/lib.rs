//! The settings plane: read the three tiers, write fields into one tier.
//!
//! Both through the engine: `engine.config.get` reads a tier (redacted, with its revision), and
//! `engine.config.set` writes keys into one — under the tier's directory lock, checked by the
//! engine's own loader, all or nothing, with the engine re-reading what it wrote. What stays here is
//! what the settings page promises beyond that:
//!
//! - **A whitelist, not a passthrough.** Only the fields in the table ([`fields::FIELDS`]) can be
//!   written, each with a declared type that the value is checked against, and only into the
//!   tiers the field allows.
//! - **A revision.** `describe` says one for the tiers it read; a write that names it hands the
//!   engine the written tier's as `if_revision`, and is refused with `CONFLICT` when that tier
//!   changed since, so a page does not write over what it never saw.
//!
//! Providers are the engine's to write too: `engine.config.setProvider` takes the global tier and a
//! `dry_run`, and puts the key in the `settings.local.json` beside it ([`providers`]).
//!
//! MCP servers are the page's too ([`mcp`]): added, changed, removed, switched off and signed in to,
//! in the tier chosen.

use std::path::PathBuf;

use harnix_ext::{caps, codes, Access, Call, Extension, ExtensionContext, Manifest, Registrar, Rpc, RpcError, MAIN_INSTANCE};
use harnix_host_api::SettingsTiers;
use serde_json::{json, Map, Value};

mod dir_lock;
mod fields;
mod mcp;
mod providers;
pub mod wire;

use fields::{Type, FIELDS, TIERS};

#[derive(Default)]
pub struct Settings;

#[async_trait::async_trait]
impl Extension for Settings {
    type Config = ();

    fn manifest(&self) -> Manifest {
        Manifest::new("settings", env!("CARGO_PKG_VERSION"))
            .title("Settings")
            .disclosure(
                "读写引擎的设置三层；写 provider 的凭据与端点；替设备加、改、删、停用 MCP 服务器（能起子进程的工具）并登录远端的；\
                 读会话下一次请求里 MCP 的工具与指令",
            )
            // Reading the settings, reloading them, and — `harnix.instances.inheritProviders` —
            // handing a changed provider to the instances whose template inherits `main`'s.
            .requires([caps::ENGINE_CONFIG_READ, caps::ENGINE_CONFIG_WRITE])
            // A project's MCP servers and every sign-in go through the engine (`engine.mcp`); the MCP
            // part of a session's next call is read off `inspectContext` (`engine.inspect`). Without
            // them the MCP section lists and edits the global and scene tiers only.
            .optional([caps::ENGINE_MCP, caps::ENGINE_INSPECT])
    }

    fn register(&self, r: &mut Registrar<'_, ()>) {
        r.method("describe")
            .access(Access::device())
            .describe("every editable field, its effective value, and where each tier stands")
            .raw(|call, params| async move { describe(&call, params).await });
        r.method("set")
            .access(Access::device())
            .describe("write one whitelisted field into one tier, through the engine")
            .raw(|call, params| async move { set(&call, params).await });
        r.method("write")
            .access(Access::device())
            .describe("write several fields into one tier as one change, through the engine, checked whole and against a revision; `dry_run` only checks")
            .raw(|call, params| async move { write(&call, params).await });
        r.method("setProvider")
            .access(Access::device())
            .describe("add, edit or remove a provider, credentials included, in the global tier")
            .raw(|call, params| async move { set_provider(&call, params).await });
        mcp::register(r);
    }

    async fn start(&self, cx: ExtensionContext) -> anyhow::Result<()> {
        cx.provide(mcp::McpState::default());
        Ok(())
    }
}

pub(crate) fn require_str(params: &Value, key: &str) -> Rpc<String> {
    params
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| RpcError::invalid_params(format!("missing {key}")))
}

/// The instance whose settings these are.
fn main_engine(call: &Call) -> Rpc<harnix_ext::EngineView> {
    call.ext()
        .engine(MAIN_INSTANCE)
        .ok_or_else(|| RpcError::new(codes::INTERNAL_ERROR, "the engine has not started yet"))
}

/// The settings.json a tier writes to, if that tier exists here. The global and scene tiers are
/// under the engine's data root, which the host says; the project tier is the project's own.
pub(crate) fn tier_path(call: &Call, tier: &str, scene: Option<&str>, project_root: Option<&str>) -> Rpc<Option<PathBuf>> {
    let files = call.ext().service::<SettingsTiers>()?;
    let under_main = |path: Option<PathBuf>| {
        path.ok_or_else(|| RpcError::new(codes::INTERNAL_ERROR, "the engine has not started yet"))
    };
    Ok(match tier {
        "global" => Some(under_main(files.global(MAIN_INSTANCE))?),
        "scene" => match scene {
            Some(scene) => Some(under_main(files.scene(MAIN_INSTANCE, scene))?),
            None => None,
        },
        "project" => project_root.map(|p| PathBuf::from(p).join(harnix_ext::engine::PROJECT_DIR).join("settings.json")),
        _ => None,
    })
}

/// The scene a call is about: the one it names, else the engine's first active one.
///
/// `engine.config.get` needs a scene and refuses without one, and a write with no scene would
/// resolve no tier path. A settings page opened with no session is the ordinary case, and it used
/// to answer every field `null` — a write made and then read back as nothing.
pub(crate) fn scene_of(call: &Call, params: &Value) -> Rpc<Option<String>> {
    let default_scene = main_engine(call)?.active_scenes().first().cloned();
    Ok(params.get("scene").and_then(Value::as_str).map(str::to_string).or(default_scene))
}

/// The settings.json of every tier there is for this (scene, project), in [`TIERS`]' order.
fn tier_paths(call: &Call, scene: Option<&str>, project_root: Option<&str>) -> Rpc<Vec<Option<PathBuf>>> {
    TIERS.iter().map(|tier| tier_path(call, tier, scene, project_root)).collect()
}

/// Every editable field, its effective value, and where each tier stands.
async fn describe(call: &Call, params: Value) -> Rpc<Value> {
    let scene = scene_of(call, &params)?;
    let project_root = params.get("project_root").and_then(Value::as_str).map(str::to_string);
    let paths = tier_paths(call, scene.as_deref(), project_root.as_deref())?;

    let mut raw = Map::new();
    let mut revisions = Map::new();
    let mut problems: Vec<Value> = Vec::new();
    for tier in TIERS {
        let mut request = locate(scene.as_deref(), project_root.as_deref());
        request["tier"] = json!(tier);
        // A tier that genuinely has no file answers `null`; a call that *failed* is a different
        // thing and is reported rather than flattened into the same answer. The two used to be
        // indistinguishable, which is how a page of nulls looked like an empty configuration
        // instead of a broken request.
        let settings = match call.call("engine.config.get", request).await {
            Ok(value) => {
                // Each tier's settings and its revision come in the one answer: what a write
                // compares against is what was shown.
                if let Some(revision) = value.get("revision").filter(|r| r.is_string()) {
                    revisions.insert((*tier).to_string(), revision.clone());
                }
                value.get("settings").cloned().unwrap_or(Value::Null)
            }
            Err(e) => {
                tracing::warn!(tier, error = %e.message, "settings tier could not be read");
                problems.push(json!({"tier": tier, "error": e.message}));
                Value::Null
            }
        };
        raw.insert((*tier).to_string(), settings);
    }

    let mut effective_request = locate(scene.as_deref(), project_root.as_deref());
    effective_request["tier"] = json!("effective");
    // What the engine runs with: its layers merged, and what an untrusted project may not loosen
    // already taken off. Re-read for this (project, scene) by `set` before it is asked.
    let effective = match call.call("engine.config.get", effective_request).await {
        Ok(value) => value.get("settings").cloned(),
        Err(e) => {
            tracing::warn!(error = %e.message, "the effective settings could not be read");
            problems.push(json!({"tier": "effective", "error": e.message}));
            None
        }
    };

    // The providers of this (project, scene): what a model field offers, and a route may name.
    let providers = providers::redacted(call.call("engine.config.getProvider", locate(scene.as_deref(), project_root.as_deref())).await.unwrap_or(json!({})));
    let providers = providers::with_unanswered(providers, effective.as_ref());

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
            let engine = engine_value(field, effective.as_ref());
            let mut view = field_view(field, per_tier, engine.as_ref());
            if let Some(view) = view.as_object_mut() {
                view.extend(fields::offers(field, &providers));
            }
            view
        })
        .collect();

    let shown = |path: &Option<PathBuf>| path.as_ref().map(|p| p.display().to_string());

    Ok(json!({
        "fields": fields,
        "scene": scene,
        // What `write` compares against: each tier's revision, as the engine said it with the values.
        "revision": revision_token(&revisions),
        // Empty in the ordinary case. Not empty means a tier could not be read, and the page can
        // say so instead of showing its nulls as if they were the configuration.
        "problems": problems,
        "paths": {
            "global": shown(&paths[0]),
            "scene": shown(&paths[1]),
            "project": shown(&paths[2]),
        },
        "providers": providers,
    }))
}

/// The `scene` / `project_root` an engine call is located by, each only when there is one: the
/// engine refuses `null` for either.
pub(crate) fn locate(scene: Option<&str>, project_root: Option<&str>) -> Value {
    let mut at = json!({});
    if let Some(scene) = scene {
        at["scene"] = json!(scene);
    }
    if let Some(root) = project_root {
        at["project_root"] = json!(root);
    }
    at
}

/// A field's value in the engine's effective settings (`None`: they could not be read). The engine
/// leaves out a list it has no rows of, so an additive field it does not name has none: an
/// untrusted project's only `allow` rule, withheld, leaves no `permission_rules` at all.
fn engine_value(field: &fields::Field, effective: Option<&Value>) -> Option<Value> {
    let settings = effective?;
    match dig(settings, field.key) {
        Some(value) => Some(value.clone()),
        None if field.additive => Some(json!([])),
        None => None,
    }
}

/// One field as the page gets it: its declaration, then where it stands. `effective` is the
/// engine's value (`None`: it could not be asked, and the files' highest tier stands in). `source`
/// is the highest tier that wrote the field; `withheld` says the project tier wrote a value the
/// engine does not use, which is what an untrusted project's looser setting looks like; `invalid`
/// names the tiers whose value the field does not accept (written by hand), and why.
///
/// An additive field's value is every tier's rows together, so the project tier's is withheld when
/// a row of it is not among the engine's (an untrusted project's `allow` rules are dropped).
fn field_view(field: &fields::Field, mut per_tier: Map<String, Value>, engine: Option<&Value>) -> Value {
    let invalid: Map<String, Value> = per_tier
        .iter()
        .filter(|(_, value)| !value.is_null())
        // A value shown and not written has nothing to be wrong about.
        .filter(|_| !matches!(field.ty, Type::Json | Type::Counts))
        .filter_map(|(tier, value)| fields::check(field.ty, value).err().map(|why| (tier.clone(), json!(why))))
        .collect();
    let written = TIERS
        .iter()
        .rev()
        .find_map(|tier| per_tier.get(*tier).filter(|value| !value.is_null()).map(|value| (*tier, value.clone())));
    let source = written.as_ref().map_or(Value::Null, |(tier, _)| json!(tier));
    let mut effective = match engine {
        Some(value) => value.clone(),
        None if field.additive => {
            let rows: Vec<Value> = TIERS
                .iter()
                .filter_map(|tier| per_tier.get(*tier).and_then(Value::as_array))
                .flatten()
                .cloned()
                .collect();
            if rows.is_empty() && written.is_none() { Value::Null } else { Value::Array(rows) }
        }
        None => written.map_or(Value::Null, |(_, value)| value),
    };
    let project = per_tier.get("project").filter(|value| !value.is_null());
    let withheld = engine.is_some()
        && project.is_some_and(|value| match (field.additive, value.as_array()) {
            (true, Some(rows)) => {
                let taken = effective.as_array().map(Vec::as_slice).unwrap_or_default();
                let same = |a: &Value, b: &Value| match field.ty {
                    Type::Rules(columns) => fields::same_row(columns, a, b),
                    _ => a == b,
                };
                rows.iter().any(|row| !taken.iter().any(|t| same(row, t)))
            }
            _ => *value != effective,
        });
    effective = fields::shown(field.ty, &effective);
    for value in per_tier.values_mut() {
        *value = fields::shown(field.ty, value);
    }
    let mut view = fields::declaration(field);
    view.insert("effective".into(), effective);
    view.insert("source".into(), source);
    view.insert("withheld".into(), json!(withheld));
    view.insert("tiers".into(), Value::Object(per_tier));
    if !invalid.is_empty() {
        view.insert("invalid".into(), Value::Object(invalid));
    }
    Value::Object(view)
}

/// Write one whitelisted field into one tier.
///
/// A `null` value removes the key, which is how a tier gives a setting back to the tier below it.
async fn set(call: &Call, params: Value) -> Rpc<Value> {
    let key = require_str(&params, "key")?;
    let value = params.get("value").cloned().unwrap_or(Value::Null);
    let mut changes = Map::new();
    changes.insert(key, value);
    apply(call, params, changes, None).await
}

/// Write several fields into one tier as one change: every value is checked first, and nothing is
/// written unless all of them are accepted. With `revision` (what `describe` said), refused with
/// `CONFLICT` when the tier written changed since. With `dry_run`, the engine checks the change and
/// writes nothing: what it would refuse is in the answer's `rejected`.
async fn write(call: &Call, params: Value) -> Rpc<Value> {
    let Some(changes) = params.get("changes").and_then(Value::as_object).cloned() else {
        return Err(RpcError::invalid_params("missing changes: an object of key → value"));
    };
    let revision = params.get("revision").and_then(Value::as_str).map(str::to_string);
    apply(call, params, changes, revision).await
}

async fn apply(call: &Call, params: Value, changes: Map<String, Value>, revision: Option<String>) -> Rpc<Value> {
    let tier = require_str(&params, "tier")?;
    let scene = scene_of(call, &params)?;
    let project_root = params.get("project_root").and_then(Value::as_str);

    if !TIERS.contains(&tier.as_str()) {
        return Err(RpcError::invalid_params(format!("`{tier}` is not a tier: global, scene or project")));
    }
    check_changes(&tier, &changes)?;
    if changes.iter().any(|(key, value)| !value.is_null() && fields::find(key).is_some_and(|f| f.ty == Type::Route)) {
        let providers = call.call("engine.config.getProvider", locate(scene.as_deref(), project_root)).await?;
        check_routes(&changes, &providers)?;
    }
    if (tier == "scene" && scene.is_none()) || (tier == "project" && project_root.is_none()) {
        return Err(RpcError::new(codes::INVALID_PARAMS, format!("no `{tier}` tier for this session")));
    }
    let dry_run = params.get("dry_run").and_then(Value::as_bool).unwrap_or(false);
    let mut request = locate(scene.as_deref(), project_root);
    request["tier"] = json!(tier);
    request["changes"] = Value::Object(changes);
    if dry_run {
        request["dry_run"] = json!(true);
    }
    if let Some(token) = &revision {
        match tier_revision(token, &tier) {
            Ok(Some(expected)) => request["if_revision"] = json!(expected),
            // The page did not read this tier: there is nothing it could have written over.
            Ok(None) => {}
            Err(()) => return Err(conflict(call, scene.as_deref(), project_root).await),
        }
    }
    let written = match call.call("engine.config.set", request).await {
        Ok(written) => written,
        Err(e) if e.code == SETTINGS_CONFLICT => return Err(conflict(call, scene.as_deref(), project_root).await),
        Err(e) => return Err(refused_by_key(e)),
    };

    let mut described = describe(call, params.clone()).await?;
    if let Some(object) = described.as_object_mut() {
        object.insert("written".into(), written.get("written_to").cloned().unwrap_or(Value::Null));
        // The engine re-reads what it wrote as part of the write.
        object.insert("reloaded".into(), json!(!dry_run));
        if dry_run {
            object.insert("dry_run".into(), json!(true));
            object.insert("rejected".into(), written.get("rejected").cloned().unwrap_or(json!([])));
        }
        if let Some(overridden) = written.get("overridden").filter(|o| o.as_array().is_some_and(|a| !a.is_empty())) {
            object.insert("overridden".into(), overridden.clone());
        }
    }
    Ok(described)
}

/// The engine's code for a write whose `if_revision` is not the tier's any more (daemon protocol §9).
const SETTINGS_CONFLICT: i32 = -32028;

/// `CONFLICT`, with the revision to write against now: every tier read again.
async fn conflict(call: &Call, scene: Option<&str>, project_root: Option<&str>) -> RpcError {
    let mut revisions = Map::new();
    for tier in TIERS {
        let mut request = locate(scene, project_root);
        request["tier"] = json!(tier);
        if let Some(revision) = call.call("engine.config.get", request).await.ok().and_then(|v| v.get("revision").cloned()).filter(Value::is_string) {
            revisions.insert((*tier).to_string(), revision);
        }
    }
    RpcError::with_data(
        codes::CONFLICT,
        "the settings changed since they were read: read them again",
        json!({"revision": revision_token(&revisions)}),
    )
}

/// The engine's refusal of keys (`data.rejected`: `{key, kind, reason}` each) as the page marks
/// refused values (`data.errors`: `{key, message}` each), the rest of it as it was; any other error
/// as it came.
fn refused_by_key(mut e: RpcError) -> RpcError {
    let errors: Option<Vec<Value>> = e.data.as_ref().and_then(|d| d.get("rejected")).and_then(Value::as_array).map(|rejected| {
        rejected
            .iter()
            .map(|r| {
                let key = r.get("key").and_then(Value::as_str).unwrap_or_default();
                let reason = r.get("reason").and_then(Value::as_str).or_else(|| r.get("kind").and_then(Value::as_str)).unwrap_or("refused");
                json!({"key": key, "message": format!("`{key}`: {reason}")})
            })
            .collect()
    });
    if let (Some(errors), Some(Value::Object(data))) = (errors, e.data.as_mut()) {
        data.insert("errors".into(), json!(errors));
    }
    e
}

/// The revision `describe` says: each tier's, as the engine gave it, in one token the page hands
/// back unread.
fn revision_token(revisions: &Map<String, Value>) -> String {
    Value::Object(revisions.clone()).to_string()
}

/// The written tier's revision in a token `describe` said; `Ok(None)` when it read no revision for
/// that tier, `Err` when the token is not one it says.
fn tier_revision(token: &str, tier: &str) -> Result<Option<String>, ()> {
    let revisions: Map<String, Value> = serde_json::from_str(token).map_err(|_| ())?;
    match revisions.get(tier) {
        None => Ok(None),
        Some(Value::String(revision)) => Ok(Some(revision.clone())),
        Some(_) => Err(()),
    }
}

/// Every change checked against the table, all of them at once: refused with each one that is
/// not accepted named in `data.errors`, so a form can mark them all and not only the first.
fn check_changes(tier: &str, changes: &Map<String, Value>) -> Rpc<()> {
    let mut errors: Vec<(String, String)> = Vec::new();
    for (key, value) in changes {
        let Some(field) = fields::find(key) else {
            errors.push((key.clone(), format!("`{key}` is not an editable setting")));
            continue;
        };
        if !field.tiers.contains(&tier) {
            errors.push((key.clone(), format!("`{key}` is not set in the `{tier}` tier")));
            continue;
        }
        if value.is_null() {
            continue;
        }
        if let Err(why) = fields::check(field.ty, value) {
            errors.push((key.clone(), format!("`{key}` does not accept that value: {why}")));
        }
    }
    if errors.is_empty() {
        return Ok(());
    }
    let message = errors.iter().map(|(_, m)| m.as_str()).collect::<Vec<_>>().join("; ");
    let data: Vec<Value> = errors.iter().map(|(key, message)| json!({"key": key, "message": message})).collect();
    Err(RpcError::with_data(codes::INVALID_PARAMS, message, json!({"errors": data})))
}

/// Every route in `changes` names a provider this (project, scene) has: one that does not is
/// refused, named as `check_changes` names a value, rather than written for the engine to fall
/// back from.
fn check_routes(changes: &Map<String, Value>, providers: &Value) -> Rpc<()> {
    let configured = providers.get("providers").and_then(Value::as_object);
    let errors: Vec<(String, String)> = changes
        .iter()
        .filter(|(key, _)| fields::find(key).is_some_and(|f| f.ty == Type::Route))
        .filter_map(|(key, value)| {
            let provider = fields::route_provider(value)?;
            let known = configured.is_some_and(|p| p.contains_key(provider));
            (!known).then(|| (key.clone(), format!("`{key}`: `{provider}` is not a configured provider")))
        })
        .collect();
    if errors.is_empty() {
        return Ok(());
    }
    let message = errors.iter().map(|(_, m)| m.as_str()).collect::<Vec<_>>().join("; ");
    let data: Vec<Value> = errors.iter().map(|(key, message)| json!({"key": key, "message": message})).collect();
    Err(RpcError::with_data(codes::INVALID_PARAMS, message, json!({"errors": data})))
}

/// Add, edit or remove a provider — credentials included — in the global tier, through the
/// engine.
async fn set_provider(call: &Call, params: Value) -> Rpc<Value> {
    let scene = scene_of(call, &params)?;
    providers::set_provider(call, scene.as_deref(), &params).await
}

/// Read a dotted path out of a settings document.
fn dig<'a>(document: &'a Value, key: &str) -> Option<&'a Value> {
    let mut cursor = document;
    for segment in key.split('.') {
        cursor = cursor.get(segment)?;
    }
    Some(cursor)
}

/// A tier's settings.json, or an empty document when the file is not there.
///
/// Only *not there* is empty. Every caller writes the document back, so a
/// read that failed for any other reason — permissions, a disk error —
/// taken as empty would have been written back as a tier holding the one key
/// just set, its providers and credentials gone.
pub(crate) fn read_document(path: &std::path::Path) -> Result<Value, RpcError> {
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| {
            RpcError::new(
                codes::INTERNAL_ERROR,
                format!("{} is not valid JSON: {e}", path.display()),
            )
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(json!({})),
        Err(e) => Err(RpcError::new(
            codes::INTERNAL_ERROR,
            format!("could not read {}: {e}", path.display()),
        )),
    }
}

/// The directory lock of the tier a settings file is in, the one Agix's writers take.
pub(crate) fn lock_tier(path: &std::path::Path) -> Result<dir_lock::DirLock, RpcError> {
    let dir = path.parent().unwrap_or(std::path::Path::new("."));
    dir_lock::DirLock::acquire(dir).map_err(io_error)
}

/// Replace a tier's settings.json atomically.
pub(crate) fn write_document(path: &std::path::Path, document: &Value) -> Result<(), RpcError> {
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
    fn a_field_is_the_engines_value_and_says_when_the_project_tier_was_not_taken() {
        let field = fields::find("permission_mode").unwrap();
        let tiers = |global: Value, project: Value| {
            let mut m = Map::new();
            m.insert("global".into(), global);
            m.insert("scene".into(), Value::Null);
            m.insert("project".into(), project);
            m
        };
        // Nothing written: the engine's default shows, from no tier.
        let unset = field_view(field, tiers(Value::Null, Value::Null), Some(&json!("default")));
        assert_eq!((&unset["effective"], &unset["source"], &unset["withheld"]), (&json!("default"), &Value::Null, &json!(false)));
        // The project wrote it and the engine took it.
        let taken = field_view(field, tiers(json!("a"), json!("b")), Some(&json!("b")));
        assert_eq!((&taken["source"], &taken["withheld"]), (&json!("project"), &json!(false)));
        // The project wrote it and the engine did not.
        let withheld = field_view(field, tiers(json!("a"), json!("b")), Some(&json!("a")));
        assert_eq!((&withheld["effective"], &withheld["source"], &withheld["withheld"]), (&json!("a"), &json!("project"), &json!(true)));
        // The engine could not be asked: the files stand in, and nothing is called withheld.
        let blind = field_view(field, tiers(json!("a"), json!("b")), None);
        assert_eq!((&blind["effective"], &blind["withheld"]), (&json!("b"), &json!(false)));
    }

    #[test]
    fn every_change_that_is_not_accepted_is_named_and_none_is_written() {
        let mut changes = Map::new();
        changes.insert("model.max_tokens".into(), json!("many"));
        changes.insert("make.believe".into(), json!(1));
        changes.insert("memory_enabled".into(), json!(false));
        changes.insert("permission_mode".into(), Value::Null);
        let e = check_changes("global", &changes).unwrap_err();
        assert_eq!(e.code, codes::INVALID_PARAMS);
        let errors = e.data.as_ref().unwrap()["errors"].as_array().unwrap().clone();
        let keys: Vec<&str> = errors.iter().map(|e| e["key"].as_str().unwrap()).collect();
        assert_eq!(keys, ["make.believe", "model.max_tokens"]);
        assert!(e.message.contains("is not an editable setting") && e.message.contains("does not accept that value"));
        changes.remove("make.believe");
        changes.insert("model.max_tokens".into(), json!(4000));
        assert!(check_changes("project", &changes).is_ok());
    }

    #[test]
    fn a_write_names_the_revision_of_the_tier_it_writes() {
        let mut revisions = Map::new();
        revisions.insert("global".into(), json!("r1:9f2c"));
        revisions.insert("project".into(), json!("r1:0a1b"));
        let token = revision_token(&revisions);
        assert_eq!(tier_revision(&token, "global"), Ok(Some("r1:9f2c".into())));
        assert_eq!(tier_revision(&token, "project"), Ok(Some("r1:0a1b".into())));
        // A tier that was not read: nothing to compare against.
        assert_eq!(tier_revision(&token, "scene"), Ok(None));
        // Not a token `describe` says: the page reads the settings again.
        assert!(tier_revision("0123456789abcdef", "global").is_err());
        assert!(tier_revision(r#"{"global": 3}"#, "global").is_err());
    }

    #[test]
    fn what_the_engine_refused_is_named_by_key_and_the_rest_kept() {
        let refused = RpcError::with_data(
            codes::INVALID_PARAMS,
            "1 key rejected",
            json!({"rejected": [{"key": "permission_mode", "kind": "invalid_value", "reason": "`yolo` is not a mode"}, {"key": "x", "kind": "unknown_key"}]}),
        );
        let e = refused_by_key(refused);
        assert_eq!(e.code, codes::INVALID_PARAMS);
        let data = e.data.unwrap();
        assert_eq!(data["errors"], json!([{"key": "permission_mode", "message": "`permission_mode`: `yolo` is not a mode"}, {"key": "x", "message": "`x`: unknown_key"}]));
        assert_eq!(data["rejected"].as_array().unwrap().len(), 2);
        // Any other refusal goes on as it came: a locked tier, a project that is not there.
        let locked = refused_by_key(RpcError::new(-32026, "the settings directory /x is locked by another process; nothing was written"));
        assert_eq!((locked.code, locked.data.is_none()), (-32026, true));
    }

    #[test]
    fn a_value_written_by_hand_that_the_field_does_not_accept_is_said() {
        let field = fields::find("model.max_tokens").unwrap();
        let mut tiers = Map::new();
        tiers.insert("global".into(), json!(4000));
        tiers.insert("scene".into(), json!("lots"));
        tiers.insert("project".into(), Value::Null);
        let view = field_view(field, tiers, Some(&json!(4000)));
        assert_eq!(view["type"], json!("number"));
        assert!(view["invalid"]["scene"].as_str().unwrap().contains("whole number"));
        assert!(view["invalid"].get("global").is_none());
    }

    #[test]
    fn a_route_to_a_provider_that_is_not_configured_is_refused_by_name() {
        let providers = json!({"providers": {"deepseek": {}, "anthropic": {}}});
        let mut changes = Map::new();
        changes.insert("task_models.title".into(), json!("deepseek"));
        changes.insert("task_models.compact".into(), json!({"provider": "anthropic", "model": "m"}));
        changes.insert("task_models.memory".into(), Value::Null);
        changes.insert("memory_enabled".into(), json!(true));
        assert!(check_routes(&changes, &providers).is_ok());
        changes.insert("task_models.web_search".into(), json!({"provider": "gone"}));
        let e = check_routes(&changes, &providers).unwrap_err();
        assert_eq!(e.code, codes::INVALID_PARAMS);
        assert_eq!(e.data.unwrap()["errors"], json!([{"key": "task_models.web_search", "message": "`task_models.web_search`: `gone` is not a configured provider"}]));
        // No providers to be read: every route is to one that is not there.
        assert!(check_routes(&changes, &json!({})).is_err());
    }

    #[test]
    fn an_engine_call_is_located_only_by_what_there_is() {
        assert_eq!(locate(None, None), json!({}));
        assert_eq!(locate(Some("code"), Some("/p")), json!({"scene": "code", "project_root": "/p"}));
    }

    #[test]
    fn an_advanced_value_is_shown_with_its_source_and_nothing_is_wrong_with_it() {
        let field = fields::find("hooks_config").unwrap();
        let mut tiers = Map::new();
        tiers.insert("global".into(), json!({"PreToolUse": [{"type": "command", "command": "secret-tool"}]}));
        tiers.insert("scene".into(), Value::Null);
        tiers.insert("project".into(), Value::Null);
        let view = field_view(field, tiers, Some(&json!({"PreToolUse": [{"type": "command", "command": "secret-tool"}]})));
        assert_eq!((&view["effective"], &view["source"]), (&json!({"PreToolUse": 1}), &json!("global")));
        assert!(view.get("invalid").is_none());
        assert!(!view.to_string().contains("secret-tool"));
        // Not written from the page, whatever tier is asked.
        let mut changes = Map::new();
        changes.insert("team".into(), json!({"enabled": true}));
        assert!(check_changes("global", &changes).is_err());
        changes.clear();
        changes.insert("telemetry_enabled".into(), json!(true));
        assert!(check_changes("global", &changes).is_ok());
        assert!(check_changes("project", &changes).is_err(), "a project has no say in telemetry");
    }

    #[test]
    fn an_additive_field_is_every_tier_s_rows_and_withheld_when_a_project_row_is_dropped() {
        let field = fields::find("permission_rules").unwrap();
        let deny = json!({"tool": "Bash(rm:*)", "action": "deny"});
        let allow = json!({"tool": "Bash(ls)", "action": "allow"});
        let tiers = |global: Value, project: Value| {
            let mut m = Map::new();
            m.insert("global".into(), global);
            m.insert("scene".into(), Value::Null);
            m.insert("project".into(), project);
            m
        };
        // Trusted: the engine has both tiers' rows, and nothing is withheld.
        let both = json!([deny.clone(), allow.clone()]);
        let view = field_view(field, tiers(json!([deny.clone()]), json!([allow.clone()])), Some(&both));
        assert_eq!((&view["effective"], &view["source"], &view["withheld"]), (&both, &json!("project"), &json!(false)));
        assert_eq!(view["tiers"]["global"], json!([deny.clone()]), "a tier's own rows are what it edits");
        // The engine leaves a rule's `match` out when it is `pattern`: still the same rule.
        let spelled = json!({"tool": "Bash(ls)", "action": "allow", "match": "pattern"});
        let view = field_view(field, tiers(json!([deny.clone()]), json!([spelled])), Some(&both));
        assert_eq!(view["withheld"], json!(false));
        // Untrusted: the project's allow is not among the engine's.
        let view = field_view(field, tiers(json!([deny.clone()]), json!([allow.clone()])), Some(&json!([deny.clone()])));
        assert_eq!(view["withheld"], json!(true));
        // Untrusted with no other rows: the engine names no rules at all, which is none of them.
        let engine = engine_value(field, Some(&json!({"permission_mode": "default"})));
        let view = field_view(field, tiers(Value::Null, json!([allow.clone()])), engine.as_ref());
        assert_eq!((&view["effective"], &view["withheld"]), (&json!([]), &json!(true)));
        assert_eq!(engine_value(fields::find("permission_mode").unwrap(), Some(&json!({}))), None);
        // The engine could not be asked: the tiers' rows together stand in.
        let view = field_view(field, tiers(json!([deny.clone()]), json!([allow.clone()])), None);
        assert_eq!((&view["effective"], &view["withheld"]), (&both, &json!(false)));
        let view = field_view(field, tiers(Value::Null, Value::Null), None);
        assert_eq!(view["effective"], Value::Null);
    }

    #[test]
    fn a_secret_field_shows_only_that_it_is_set() {
        let field = fields::Field::new("web.api_key", Type::Secret, "web");
        let mut tiers = Map::new();
        tiers.insert("global".into(), json!("sk-0123456789abcd"));
        tiers.insert("scene".into(), Value::Null);
        tiers.insert("project".into(), Value::Null);
        let view = field_view(&field, tiers, Some(&json!("sk-0123456789abcd")));
        assert!(!view.to_string().contains("0123456789"), "{view}");
        assert_eq!(view["effective"], json!("***abcd"));
        assert_eq!(view["source"], json!("global"));
    }
}
