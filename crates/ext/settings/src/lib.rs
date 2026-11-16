//! The settings plane: read the three tiers, write fields into one tier.
//!
//! Agix exposes `engine.config.get` (read, per tier, redacted) and `engine.config.reload`, but no
//! write for settings other than providers — the supported way to change one is to edit the tier's
//! `settings.json`, which is exactly what `engine.config.reload` exists for and what the engine's
//! watcher already watches. So this writes the file and asks the engine to re-read it.
//!
//! Three rules make that safe to expose to a browser:
//!
//! - **A whitelist, not a passthrough.** Only the fields in the table ([`fields::FIELDS`]) can be
//!   written, each with a declared type that the value is checked against, and only into the
//!   tiers the field allows.
//! - **Read, modify, write the whole file.** Hand edits are preserved because the file is
//!   re-read immediately before each change, under the tier's directory lock (the one the engine
//!   takes), and replaced atomically.
//! - **A revision.** `describe` says one for the tiers it read; a write that names it is refused
//!   with `CONFLICT` when a tier's file changed since, so a page does not write over what it never
//!   saw.
//!
//! Providers are written the same way, into the global tier. The engine's `engine.config.setProvider`
//! writes the project tier of a working directory, which is wherever the process happened to be
//! started — a provider added from the settings page would be gone the next time it was started
//! from somewhere else — so the engine is asked only what it is good at here: whether it reads
//! the entry. The file is written, the engine reloads, and `routing` in the answer says; a
//! refusal puts the file back as it was.

use std::path::{Path, PathBuf};

use harnix_ext::{caps, codes, Access, Call, Extension, Manifest, Registrar, Rpc, RpcError, MAIN_INSTANCE};
use harnix_host_api::SettingsTiers;
use serde_json::{json, Map, Value};

mod dir_lock;
mod fields;

use fields::{Type, FIELDS, TIERS};

#[derive(Default)]
pub struct Settings;

#[async_trait::async_trait]
impl Extension for Settings {
    type Config = ();

    fn manifest(&self) -> Manifest {
        Manifest::new("settings", env!("CARGO_PKG_VERSION"))
            .title("Settings")
            .disclosure("读写引擎的设置三层；写 provider 的凭据与端点")
            // Reading the settings, reloading them, and — `harnix.instances.inheritProviders` —
            // handing a changed provider to the instances whose template inherits `main`'s.
            .requires([caps::ENGINE_CONFIG_READ, caps::ENGINE_CONFIG_WRITE])
    }

    fn register(&self, r: &mut Registrar<'_, ()>) {
        r.method("describe")
            .access(Access::device())
            .describe("every editable field, its effective value, and where each tier stands")
            .raw(|call, params| async move { describe(&call, params).await });
        r.method("set")
            .access(Access::device())
            .describe("write one whitelisted field into one tier, then make the engine re-read")
            .raw(|call, params| async move { set(&call, params).await });
        r.method("write")
            .access(Access::device())
            .describe("write several fields into one tier as one change, checked whole and against a revision")
            .raw(|call, params| async move { write(&call, params).await });
        r.method("setProvider")
            .access(Access::device())
            .describe("add, edit or remove a provider, credentials included, in the global tier")
            .raw(|call, params| async move { set_provider(&call, params).await });
    }
}

fn require_str(params: &Value, key: &str) -> Rpc<String> {
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
fn tier_path(call: &Call, tier: &str, scene: Option<&str>, project_root: Option<&str>) -> Rpc<Option<PathBuf>> {
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
        "project" => project_root.map(|p| PathBuf::from(p).join(".agix").join("settings.json")),
        _ => None,
    })
}

/// The scene a call is about: the one it names, else the engine's first active one.
///
/// `engine.config.get` needs a scene and refuses without one, and a write with no scene would
/// resolve no tier path. A settings page opened with no session is the ordinary case, and it used
/// to answer every field `null` — a write made and then read back as nothing.
fn scene_of(call: &Call, params: &Value) -> Rpc<Option<String>> {
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
    // Taken before the tiers are read: a file that changes while they are is a newer revision than
    // the one said, and a write naming this one is refused rather than let through.
    let paths = tier_paths(call, scene.as_deref(), project_root.as_deref())?;
    let revision = revision_of(&paths);

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
        // A tier that genuinely has no file answers `null`; a call that *failed* is a different
        // thing and is reported rather than flattened into the same answer. The two used to be
        // indistinguishable, which is how a page of nulls looked like an empty configuration
        // instead of a broken request.
        let settings = match call.call("engine.config.get", request).await {
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
            let engine = effective.as_ref().and_then(|settings| dig(settings, field.key));
            field_view(field, per_tier, engine)
        })
        .collect();

    let providers = call.call("engine.config.getProvider", json!({})).await.unwrap_or(json!({}));
    let shown = |path: &Option<PathBuf>| path.as_ref().map(|p| p.display().to_string());

    Ok(json!({
        "fields": fields,
        "scene": scene,
        // What `write` compares against: the three tiers' files as they were when this was read.
        "revision": revision,
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

/// One field as the page gets it: its declaration, then where it stands. `effective` is the
/// engine's value (`None`: it could not be asked, and the files' highest tier stands in). `source`
/// is the highest tier that wrote the field; `withheld` says the project tier wrote a value the
/// engine does not use, which is what an untrusted project's looser setting looks like; `invalid`
/// names the tiers whose value the field does not accept (written by hand), and why.
fn field_view(field: &fields::Field, mut per_tier: Map<String, Value>, engine: Option<&Value>) -> Value {
    let invalid: Map<String, Value> = per_tier
        .iter()
        .filter(|(_, value)| !value.is_null())
        .filter_map(|(tier, value)| fields::check(field.ty, value).err().map(|why| (tier.clone(), json!(why))))
        .collect();
    let written = TIERS
        .iter()
        .rev()
        .find_map(|tier| per_tier.get(*tier).filter(|value| !value.is_null()).map(|value| (*tier, value.clone())));
    let source = written.as_ref().map_or(Value::Null, |(tier, _)| json!(tier));
    let mut effective = match engine {
        Some(value) => value.clone(),
        None => written.map_or(Value::Null, |(_, value)| value),
    };
    let project = per_tier.get("project").filter(|value| !value.is_null());
    let withheld = engine.is_some() && project.is_some_and(|value| *value != effective);
    if field.ty == Type::Secret {
        effective = fields::redact(&effective);
        for value in per_tier.values_mut() {
            *value = fields::redact(value);
        }
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

/// Write one whitelisted field into one tier, then make the engine re-read.
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
/// `CONFLICT` when a tier's file changed since.
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
    let Some(path) = tier_path(call, &tier, scene.as_deref(), project_root)? else {
        return Err(RpcError::new(codes::INVALID_PARAMS, format!("no `{tier}` tier for this session")));
    };
    let paths = tier_paths(call, scene.as_deref(), project_root)?;
    write_fields(&path, &changes, revision.as_deref(), &paths)?;

    // Re-read the pair `describe` asks about next: the engine keeps its settings per (project,
    // scene), and a reload names the one it drops.
    let mut reload = json!({});
    if let Some(scene) = &scene {
        reload["scene"] = json!(scene);
    }
    if let Some(root) = project_root {
        reload["project_root"] = json!(root);
    }
    let reloaded = call.call("engine.config.reload", reload).await.is_ok();
    let mut described = describe(call, params.clone()).await?;
    if let Some(object) = described.as_object_mut() {
        object.insert("written".into(), json!(path.display().to_string()));
        object.insert("reloaded".into(), json!(reloaded));
    }
    Ok(described)
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

/// The tiers' files as one token: what a write compares against what `describe` said. A file that
/// is not there counts as such, so creating one is a change too.
fn revision_of(paths: &[Option<PathBuf>]) -> String {
    // FNV-1a: stable across runs and builds, unlike the standard library's hasher, and enough to
    // tell one version of a small file from another.
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let mut eat = |bytes: &[u8]| {
        for b in bytes {
            hash ^= u64::from(*b);
            hash = hash.wrapping_mul(0x0100_0000_01b3);
        }
    };
    for path in paths {
        match path.as_deref().map(std::fs::read) {
            Some(Ok(bytes)) => {
                eat(b"file:");
                eat(&bytes);
            }
            Some(Err(_)) => eat(b"none"),
            None => eat(b"no tier"),
        }
        eat(b"\0");
    }
    format!("{hash:016x}")
}

/// Write `changes` into the settings.json at `path`, keeping everything else in it, under the
/// tier's directory lock; with `revision`, only when the tiers (`paths`) are still as it says.
fn write_fields(path: &Path, changes: &Map<String, Value>, revision: Option<&str>, paths: &[Option<PathBuf>]) -> Rpc<()> {
    let _held = lock_tier(path)?;
    if let Some(expected) = revision {
        let now = revision_of(paths);
        if now != expected {
            return Err(RpcError::with_data(
                codes::CONFLICT,
                "the settings changed since they were read: read them again",
                json!({"revision": now}),
            ));
        }
    }
    let mut document = read_document(path)?;
    for (key, value) in changes {
        set_path(&mut document, key, value.clone());
    }
    write_document(path, &document)
}

/// Add, edit or remove a provider — credentials included — in the global tier, then make the
/// engine re-read.
///
/// `config` is a patch: a key that is absent keeps its value (which is how an edit leaves the
/// stored API key alone), `null` or blank removes it. With no default left, the first provider
/// becomes the default: a router with providers and no default has nothing to send a turn to.
async fn set_provider(call: &Call, params: Value) -> Rpc<Value> {
    let provider_id = require_str(&params, "provider_id")?;
    if !valid_provider_id(&provider_id) {
        return Err(RpcError::invalid_params("provider_id: letters, digits, `-`, `_` and `.`"));
    }
    let delete = params.get("delete").and_then(Value::as_bool).unwrap_or(false);
    let Some(path) = tier_path(call, "global", None, None)? else {
        return Err(RpcError::new(codes::INTERNAL_ERROR, "no global tier"));
    };
    let before = {
        // Read, patch and write under the tier's directory lock, which the engine takes too.
        let _held = lock_tier(&path)?;
        let before = std::fs::read(&path).ok();
        let mut document = read_document(&path)?;
        patch_providers(
            &mut document,
            &provider_id,
            delete,
            params.get("config"),
            params.get("default_provider").and_then(Value::as_str),
        )?;
        write_document(&path, &document)?;
        before
    };

    let reloaded = call.call("engine.config.reload", json!({})).await;
    // What the engine reads is the entry as patched, not the patch: it says whether it can. A
    // router it could not rebuild (`router_rebuilt: false`) is not a refusal; the shape not
    // parsing (`routing.ok: false`), or the reload failing, is.
    if !delete {
        let refusal = match &reloaded {
            Ok(answer) => routing_refusal(answer),
            Err(e) => Some(e.message.clone()),
        };
        if let Some(why) = refusal {
            {
                let _held = lock_tier(&path)?;
                restore(&path, before.as_deref())?;
            }
            if let Err(e) = call.call("engine.config.reload", json!({})).await {
                tracing::warn!(error = %e.message, "the engine did not reload after a provider was put back");
            }
            return Err(RpcError::invalid_params(why));
        }
    }
    let providers = call.call("engine.config.getProvider", json!({})).await.unwrap_or(json!({}));
    // And to every instance whose template inherits `main`'s providers.
    let inherited = match call.call("harnix.instances.inheritProviders", json!({})).await {
        Ok(answer) => answer.get("reloaded").cloned().unwrap_or(json!([])),
        Err(e) => {
            tracing::warn!(error = %e.message, "the instances inheriting providers were not updated");
            json!([])
        }
    };
    Ok(json!({
        "written": path.display().to_string(),
        "reloaded": reloaded.is_ok(),
        "reloaded_instances": inherited,
        "providers": providers,
    }))
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

/// Apply one provider change to a settings document, refusing what a settings page may not write.
/// Whether the engine can read the entry is its own to say: see `set_provider`.
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
            Some("anthropic" | "openai_compatible" | "openai_responses") => {}
            _ => {
                return Err(RpcError::invalid_params(
                    "config.api_type: `anthropic`, `openai_compatible` or `openai_responses`",
                ))
            }
        }
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

/// Set one key in a tier's settings.json, keeping everything else in it.
#[cfg(test)]
fn set_in_file(path: &Path, key: &str, value: Value) -> Result<(), RpcError> {
    let mut changes = Map::new();
    changes.insert(key.to_string(), value);
    write_fields(path, &changes, None, &[])
}

/// A tier's settings.json, or an empty document when the file is not there.
///
/// Only *not there* is empty. Every caller writes the document back, so a
/// read that failed for any other reason — permissions, a disk error —
/// taken as empty would have been written back as a tier holding the one key
/// just set, its providers and credentials gone.
fn read_document(path: &std::path::Path) -> Result<Value, RpcError> {
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
fn lock_tier(path: &std::path::Path) -> Result<dir_lock::DirLock, RpcError> {
    let dir = path.parent().unwrap_or(std::path::Path::new("."));
    dir_lock::DirLock::acquire(dir).map_err(io_error)
}

/// Why the engine did not accept what `engine.config.reload` re-read, from its answer: `routing.ok` is
/// whether the configuration parses. `None` when it did.
fn routing_refusal(answer: &Value) -> Option<String> {
    let routing = answer.get("routing")?;
    if routing.get("ok").and_then(Value::as_bool) != Some(false) {
        return None;
    }
    Some(
        routing
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("the engine could not read the settings")
            .to_string(),
    )
}

/// Put a tier's settings.json back as it was: its bytes, or no file when there was none.
fn restore(path: &std::path::Path, before: Option<&[u8]>) -> Result<(), RpcError> {
    match before {
        Some(bytes) => std::fs::write(path, bytes).map_err(io_error),
        None => match std::fs::remove_file(path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(io_error(e)),
            _ => Ok(()),
        },
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
    fn only_a_routing_that_does_not_parse_is_a_refusal() {
        let ok = json!({"routing": {"ok": true, "router_rebuilt": false, "router_error": "no client"}});
        assert_eq!(routing_refusal(&ok), None);
        let bad = json!({"routing": {"ok": false, "error": "providers.x: bad"}});
        assert_eq!(routing_refusal(&bad).as_deref(), Some("providers.x: bad"));
    }

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
        let responses = json!({"api_type": "openai_responses", "base_url": "https://api.openai.com/v1"});
        patch_providers(&mut document, "r", false, Some(&responses), None).unwrap();
        assert!(patch_providers(&mut document, "a", false, Some(&json!({"api_type": "anthropic", "shell": "x"})), None).is_err());
        assert!(patch_providers(&mut document, "a", false, Some(&json!({"api_type": "anthropic"})), Some("zz")).is_err());
        assert!(!valid_provider_id("a b"));
    }

    #[test]
    fn a_field_is_the_engines_value_and_says_when_the_project_tier_was_not_taken() {
        let field = &FIELDS[0];
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

    /// A writer waits for whoever holds the tier's directory, the engine included.
    #[test]
    fn a_write_waits_while_the_tier_s_directory_is_held() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let held = dir_lock::DirLock::acquire(dir.path()).unwrap();
        let writer = {
            let path = path.clone();
            std::thread::spawn(move || set_in_file(&path, "model.max_tokens", json!(4000)).unwrap())
        };
        std::thread::sleep(std::time::Duration::from_millis(200));
        assert!(!path.exists(), "written while another writer held the tier");
        drop(held);
        writer.join().unwrap();
        assert_eq!(read_document(&path).unwrap(), json!({"model": {"max_tokens": 4000}}));
    }

    /// A tier that cannot be read is an error, and the file is left as it
    /// was — not replaced by a document holding only the key being set.
    #[test]
    fn an_unreadable_tier_is_not_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let original = r#"{"providers":{"a":{"api_type":"anthropic","api_key":"k"}}}"#;
        std::fs::write(&path, original).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000)).unwrap();
            // Root reads it anyway; then there is nothing to show here.
            if std::fs::read(&path).is_err() {
                assert!(set_in_file(&path, "model.max_tokens", json!(4000)).is_err());
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
                assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
            } else {
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
            }
        }
        // A read that fails for another reason than "not there" — here, a directory where the
        // file should be — is an error too, wherever this runs.
        let not_a_file = dir.path().join("scene");
        std::fs::create_dir(&not_a_file).unwrap();
        assert!(read_document(&not_a_file).is_err());
        // And a file that is not there is an empty tier, written with the one key.
        let fresh = dir.path().join("fresh.json");
        set_in_file(&fresh, "model.max_tokens", json!(4000)).unwrap();
        assert_eq!(read_document(&fresh).unwrap(), json!({"model": {"max_tokens": 4000}}));
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
    fn a_write_against_a_revision_that_moved_on_is_a_conflict() {
        let dir = tempfile::tempdir().unwrap();
        let global = dir.path().join("global").join("settings.json");
        let project = dir.path().join("project").join("settings.json");
        let paths = vec![Some(global.clone()), None, Some(project.clone())];
        let read = revision_of(&paths);
        assert_eq!(read, revision_of(&paths), "the same files are the same revision");

        // Someone else writes the project tier after the page read the settings.
        std::fs::create_dir_all(project.parent().unwrap()).unwrap();
        std::fs::write(&project, r#"{"permission_mode":"plan"}"#).unwrap();
        let mut changes = Map::new();
        changes.insert("model.max_tokens".into(), json!(4000));
        changes.insert("memory_enabled".into(), json!(false));
        let e = write_fields(&global, &changes, Some(&read), &paths).unwrap_err();
        assert_eq!(e.code, codes::CONFLICT);
        assert!(!global.exists(), "written over a revision that moved on");

        // Read again, written as one.
        let again = e.data.unwrap()["revision"].as_str().unwrap().to_string();
        assert_eq!(again, revision_of(&paths));
        write_fields(&global, &changes, Some(&again), &paths).unwrap();
        assert_eq!(read_document(&global).unwrap(), json!({"model": {"max_tokens": 4000}, "memory_enabled": false}));
        assert_ne!(revision_of(&paths), again, "a write is a new revision");
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
