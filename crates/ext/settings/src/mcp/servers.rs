//! MCP servers in a tier's files: what a form gives turned into the engine's configuration, the
//! files read back as the page sees them, and the changes the engine has no method for (removing
//! one, switching one off and on).
//!
//! A server is two halves, as the engine writes it (`engine.mcp.addServer`): its `env` and `headers` in the
//! tier's `settings.local.json` (0600, never committed), the rest in its `settings.json`; the two
//! merged are the server. Both are read and written under the tier's directory lock.
//!
//! What a form may say of a value it was given in secret: the engine's environment variable it is
//! read from (`var`, written as `${NAME}`), a new value (`secret`, written with `$` as `$$` so the
//! engine does not expand it), or `keep` — the one already written, which the page never saw.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use harnix_ext::{codes, Rpc, RpcError};
use serde::Deserialize;
use serde_json::{json, Map, Value};

use crate::wire::{McpAuthStatus, McpOAuth, McpServer, McpStatus, McpValue};

/// A server's two halves: what goes to settings.json, and to settings.local.json.
pub(crate) type Halves = (Map<String, Value>, Map<String, Value>);

/// The key every tier's servers are under.
pub(crate) const SERVERS: &str = "mcp_servers";
/// The file beside a tier's settings.json that holds what is secret.
pub(crate) const LOCAL_FILE: &str = "settings.local.json";

/// What a form sets; every other key of a server already written (its `oauth` block, its time
/// limits) is kept as it is.
const FORM_KEYS: &[&str] = &["type", "command", "args", "env", "url", "headers"];
/// The keys that go to `settings.local.json`.
const SECRET_KEYS: &[&str] = &["env", "headers"];

const MAX_NAME: usize = 64;
const MAX_ARGS: usize = 64;
const MAX_ENTRIES: usize = 64;
const MAX_TEXT: usize = 8192;

/// A server as a form gives it.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    /// `stdio`, `http` or `sse`.
    pub transport: String,
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, Given>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub headers: BTreeMap<String, Given>,
}

/// One value given in secret: exactly one of the three.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Given {
    #[serde(default)]
    pub var: Option<String>,
    #[serde(default)]
    pub secret: Option<String>,
    #[serde(default)]
    pub keep: bool,
}

pub(crate) fn invalid(message: impl Into<String>) -> RpcError {
    RpcError::invalid_params(message)
}

/// A name a new server may have: letters, digits, `-` and `_`, starting with a letter or digit, and
/// no `__` — the engine names a server's tools `mcp__<server>__<tool>`, and a permission rule names
/// them by that prefix.
pub(crate) fn check_name(name: &str) -> Rpc<()> {
    let ok = !name.is_empty()
        && name.len() <= MAX_NAME
        && name.chars().next().is_some_and(|c| c.is_ascii_alphanumeric())
        && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
        && !name.contains("__");
    if ok {
        Ok(())
    } else {
        Err(invalid(format!(
            "`{name}` is not a server name: up to {MAX_NAME} letters, digits, `-` and `_`, starting with a letter or digit, without `__`"
        )))
    }
}

fn env_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && name.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn header_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= 256 && name.chars().all(|c| c.is_ascii_alphanumeric() || "!#$%&'*+.^_`|~-".contains(c))
}

fn text(what: &str, value: &str) -> Rpc<()> {
    if value.len() > MAX_TEXT {
        return Err(invalid(format!("{what} is longer than {MAX_TEXT} bytes")));
    }
    if value.contains('\0') {
        return Err(invalid(format!("{what} has a NUL in it")));
    }
    Ok(())
}

/// The engine's `type` for a form's transport.
fn engine_type(transport: &str) -> Rpc<&'static str> {
    match transport {
        "stdio" => Ok("stdio"),
        "http" => Ok("streamable_http"),
        "sse" => Ok("sse"),
        other => Err(invalid(format!("`{other}` is not a transport: stdio, http or sse"))),
    }
}

/// A form's transport for the engine's `type`; one the engine does not have, as written.
pub(crate) fn transport_of(engine_type: Option<&str>) -> String {
    match engine_type {
        None | Some("stdio") => "stdio".into(),
        Some("streamable_http") => "http".into(),
        Some("sse") => "sse".into(),
        Some("web_socket" | "websocket") => "websocket".into(),
        Some(other) => other.into(),
    }
}

/// The server a form describes, as its two halves: what goes into settings.json, and the `env` /
/// `headers` that go into settings.local.json. `before` is the server as written already (the two
/// halves merged), whose other keys are kept and whose values `keep` refers to. In the project tier
/// a value cannot come from the engine's environment: the engine does not let a project's server
/// read it.
pub(crate) fn build(input: &Input, tier: &str, before: Option<&Map<String, Value>>) -> Rpc<Halves> {
    let ty = engine_type(&input.transport)?;
    let stdio = ty == "stdio";
    let mut public: Map<String, Value> = before
        .map(|b| b.iter().filter(|(k, _)| !FORM_KEYS.contains(&k.as_str())).map(|(k, v)| (k.clone(), v.clone())).collect())
        .unwrap_or_default();
    // An `oauth` block is a remote server's; a server that is now a process has none.
    if stdio {
        public.remove("oauth");
    }
    public.insert("type".into(), json!(ty));
    if stdio {
        let command = input.command.as_deref().map(str::trim).unwrap_or_default();
        if command.is_empty() {
            return Err(invalid("a stdio server needs a command"));
        }
        text("the command", command)?;
        if input.args.len() > MAX_ARGS {
            return Err(invalid(format!("more than {MAX_ARGS} arguments")));
        }
        for arg in &input.args {
            text("an argument", arg)?;
        }
        if input.url.as_deref().is_some_and(|u| !u.trim().is_empty()) || !input.headers.is_empty() {
            return Err(invalid("a stdio server has no url and no headers"));
        }
        public.insert("command".into(), json!(command));
        public.insert("args".into(), json!(input.args));
    } else {
        let url = input.url.as_deref().map(str::trim).unwrap_or_default();
        check_url(url)?;
        if input.command.as_deref().is_some_and(|c| !c.trim().is_empty()) || !input.args.is_empty() || !input.env.is_empty() {
            return Err(invalid("a remote server has no command, arguments or environment: give it a url and headers"));
        }
        public.insert("url".into(), json!(url));
    }
    let mut secret = Map::new();
    let previous = |key: &str, name: &str| before.and_then(|b| b.get(key)).and_then(|m| m.get(name)).and_then(Value::as_str).map(str::to_string);
    for (key, given, valid, what) in [
        ("env", &input.env, env_name as fn(&str) -> bool, "an environment variable"),
        ("headers", &input.headers, header_name as fn(&str) -> bool, "a header"),
    ] {
        if given.len() > MAX_ENTRIES {
            return Err(invalid(format!("more than {MAX_ENTRIES} entries in {key}")));
        }
        let mut values = Map::new();
        for (name, value) in given {
            if !valid(name) {
                return Err(invalid(format!("`{name}` is not the name of {what}")));
            }
            let written = match (value.var.as_deref(), value.secret.as_deref(), value.keep) {
                (Some(var), None, false) => {
                    if tier == "project" {
                        return Err(invalid(format!(
                            "{key}.{name}: a project's server cannot read the engine's environment; give the value, or declare the server in the global tier"
                        )));
                    }
                    if !env_name(var) {
                        return Err(invalid(format!("{key}.{name}: `{var}` is not the name of an environment variable")));
                    }
                    format!("${{{var}}}")
                }
                (None, Some(secret), false) => {
                    text(&format!("{key}.{name}"), secret)?;
                    if key == "headers" && secret.contains(['\r', '\n']) {
                        return Err(invalid(format!("headers.{name} has a line break in it")));
                    }
                    secret.replace('$', "$$")
                }
                (None, None, true) => previous(key, name).ok_or_else(|| invalid(format!("{key}.{name}: there is no value written to keep")))?,
                _ => return Err(invalid(format!("{key}.{name}: give one of var, secret or keep"))),
            };
            values.insert(name.clone(), json!(written));
        }
        if !values.is_empty() {
            secret.insert(key.into(), Value::Object(values));
        }
    }
    Ok((public, secret))
}

/// A remote server's address: `http` or `https`, with a host, nothing else.
pub(crate) fn check_url(url: &str) -> Rpc<()> {
    if url.is_empty() {
        return Err(invalid("a remote server needs a url"));
    }
    text("the url", url)?;
    let parsed = url::Url::parse(url).map_err(|e| invalid(format!("`{url}` is not a URL: {e}")))?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none_or(str::is_empty) {
        return Err(invalid(format!("`{url}`: an MCP server is reached over http or https")));
    }
    Ok(())
}

/// A value read back: the variable it names when it is exactly `$NAME` or `${NAME}`, else a value
/// given in secret, of which only the last four characters are shown (none of a short one).
pub(crate) fn shown_value(raw: &Value) -> McpValue {
    let text = raw.as_str().unwrap_or_default();
    let var = text
        .strip_prefix("${")
        .and_then(|rest| rest.strip_suffix('}'))
        .or_else(|| text.strip_prefix('$'))
        .filter(|name| env_name(name));
    match var {
        Some(name) => McpValue { kind: "env".into(), var: Some(name.into()), masked: None },
        None => {
            let plain = text.replace("$$", "$");
            let masked = if plain.chars().count() >= 12 {
                let tail: String = plain.chars().rev().take(4).collect::<Vec<_>>().into_iter().rev().collect();
                format!("***{tail}")
            } else {
                "***".into()
            };
            McpValue { kind: "secret".into(), var: None, masked: Some(masked) }
        }
    }
}

/// One server as the page sees it: its two halves merged, every secret value masked.
pub(crate) fn view(name: &str, tier: &str, enabled: bool, server: &Map<String, Value>) -> McpServer {
    let values = |key: &str| -> BTreeMap<String, McpValue> {
        server.get(key).and_then(Value::as_object).map(|m| m.iter().map(|(k, v)| (k.clone(), shown_value(v))).collect()).unwrap_or_default()
    };
    let oauth = server.get("oauth").and_then(Value::as_object);
    let mut unsupported = Vec::new();
    if oauth.is_some_and(|o| o.contains_key("private_key_jwt")) {
        unsupported.push("private_key_jwt".to_string());
    }
    let str_of = |v: Option<&Value>| v.and_then(Value::as_str).map(str::to_string);
    McpServer {
        name: name.into(),
        tier: tier.into(),
        enabled,
        transport: transport_of(server.get("type").and_then(Value::as_str)),
        command: str_of(server.get("command")),
        args: server
            .get("args")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect())
            .unwrap_or_default(),
        url: str_of(server.get("url")),
        env: values("env"),
        headers: values("headers"),
        oauth: oauth.map(|o| McpOAuth {
            grant: str_of(o.get("grant")).unwrap_or_else(|| "authorization_code".into()),
            client_id: str_of(o.get("client_id")),
            issuer: str_of(o.get("issuer")),
            scopes: o.get("scopes").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default(),
        }),
        overridden_by: None,
        unsupported,
        status: None,
    }
}

/// What the engine says of one server (an element of `engine.mcp.status`'s `servers`).
pub(crate) fn status_of(entry: &Value) -> McpStatus {
    let str_of = |v: Option<&Value>| v.and_then(Value::as_str).map(str::to_string);
    let auth = entry.get("auth");
    McpStatus {
        state: str_of(entry.get("state")).unwrap_or_else(|| "connected".into()),
        tool_count: entry.get("tool_count").and_then(Value::as_u64).unwrap_or(0),
        error: str_of(entry.get("error")),
        limited: str_of(entry.get("limited")),
        auth: McpAuthStatus {
            state: str_of(auth.and_then(|a| a.get("state"))).unwrap_or_else(|| "none".into()),
            issuer: str_of(auth.and_then(|a| a.get("issuer"))),
            scopes: auth
                .and_then(|a| a.get("scopes"))
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect())
                .unwrap_or_default(),
            expires_at: str_of(auth.and_then(|a| a.get("expires_at"))),
            error: str_of(auth.and_then(|a| a.get("error"))),
        },
    }
}

/// The settings.local.json beside a tier's settings.json.
pub(crate) fn local_of(settings: &Path) -> PathBuf {
    settings.with_file_name(LOCAL_FILE)
}

/// A tier's two documents.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Tier {
    pub settings: Value,
    pub local: Value,
}

impl Tier {
    /// Its servers, each one's two halves merged (the local one over the other, as the engine
    /// merges them), by name.
    pub(crate) fn servers(&self) -> BTreeMap<String, Map<String, Value>> {
        let mut out: BTreeMap<String, Map<String, Value>> = BTreeMap::new();
        for doc in [&self.settings, &self.local] {
            let Some(servers) = doc.get(SERVERS).and_then(Value::as_object) else { continue };
            for (name, server) in servers {
                let Some(server) = server.as_object() else { continue };
                let merged = out.entry(name.clone()).or_default();
                for (key, value) in server {
                    merged.insert(key.clone(), value.clone());
                }
            }
        }
        out
    }

    pub(crate) fn declares(&self, name: &str) -> bool {
        [&self.settings, &self.local].iter().any(|doc| doc.get(SERVERS).and_then(|s| s.get(name)).is_some())
    }

    /// Take a server out of both documents: its two halves, `None` for a half it did not have.
    pub(crate) fn take(&mut self, name: &str) -> (Option<Value>, Option<Value>) {
        let out = |doc: &mut Value| {
            let servers = doc.get_mut(SERVERS)?.as_object_mut()?;
            let taken = servers.remove(name);
            if servers.is_empty() {
                if let Some(object) = doc.as_object_mut() {
                    object.remove(SERVERS);
                }
            }
            taken
        };
        let public = out(&mut self.settings);
        let secret = out(&mut self.local);
        (public, secret)
    }

    /// Put a server in, as two halves; the local half only when it has anything, and nothing of
    /// it left behind when it has not (a server is replaced whole).
    pub(crate) fn put(&mut self, name: &str, public: Map<String, Value>, secret: Map<String, Value>) {
        self.take(name);
        servers_of(&mut self.settings).insert(name.into(), Value::Object(public));
        if !secret.is_empty() {
            servers_of(&mut self.local).insert(name.into(), Value::Object(secret));
        }
    }
}

fn servers_of(doc: &mut Value) -> &mut Map<String, Value> {
    if !doc.is_object() {
        *doc = json!({});
    }
    let object = doc.as_object_mut().expect("an object just now");
    let servers = object.entry(SERVERS).or_insert_with(|| json!({}));
    if !servers.is_object() {
        *servers = json!({});
    }
    servers.as_object_mut().expect("an object just now")
}

/// A merged server split back into what goes to settings.json and what to settings.local.json.
pub(crate) fn split(server: &Map<String, Value>) -> Halves {
    let (secret, public): (Map<String, Value>, Map<String, Value>) =
        server.iter().map(|(k, v)| (k.clone(), v.clone())).partition(|(k, _)| SECRET_KEYS.contains(&k.as_str()));
    (public, secret)
}

/// Read a tier's two files, change them, and write back the ones that changed (the local one
/// first, created 0600), all under the tier's directory lock.
pub(crate) fn edit<T>(settings: &Path, change: impl FnOnce(&mut Tier) -> Rpc<T>) -> Rpc<T> {
    let local = local_of(settings);
    let _held = crate::lock_tier(settings)?;
    let before = Tier { settings: crate::read_document(settings)?, local: crate::read_document(&local)? };
    let mut tier = before.clone();
    let out = change(&mut tier)?;
    if tier.local != before.local {
        write_private(&local, &tier.local)?;
    }
    if tier.settings != before.settings {
        crate::write_document(settings, &tier.settings)?;
    }
    Ok(out)
}

/// Read a tier's two files. Not under the lock, which would make the tier's directory: listing a
/// project's servers leaves no `.harnix/` behind. Each file is replaced whole when written, so a
/// read sees one version of it.
pub(crate) fn read(settings: &Path) -> Rpc<Tier> {
    Ok(Tier { settings: crate::read_document(settings)?, local: crate::read_document(&local_of(settings))? })
}

/// Replace a file atomically, readable by its owner alone.
pub(crate) fn write_private(path: &Path, document: &Value) -> Rpc<()> {
    let io = |e: std::io::Error| RpcError::new(codes::INTERNAL_ERROR, format!("could not write {}: {e}", path.display()));
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(io)?;
    }
    let temp = path.with_extension("json.tmp");
    let body = serde_json::to_vec_pretty(document).map_err(|e| RpcError::new(codes::INTERNAL_ERROR, e.to_string()))?;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    {
        use std::io::Write as _;
        let mut file = options.open(&temp).map_err(io)?;
        file.write_all(&body).map_err(io)?;
    }
    std::fs::rename(&temp, path).map_err(io)
}

/// Servers switched off, kept by Harnix where the engine does not read them: in the extension's own
/// data, not beside the tier — a project's `.harnix/` may be committed, and a server's secrets go with
/// it. One file, 0600: `{"<a tier's settings.json>": {"<server>": {"settings": …, "local": …}}}`.
pub(crate) struct Aside {
    path: PathBuf,
}

impl Aside {
    pub(crate) fn new(path: PathBuf) -> Self {
        Self { path }
    }

    fn read(&self) -> Rpc<Map<String, Value>> {
        Ok(crate::read_document(&self.path)?.as_object().cloned().unwrap_or_default())
    }

    /// One change to the file at a time in this process; it is this extension's alone.
    fn changing() -> std::sync::MutexGuard<'static, ()> {
        static CHANGING: std::sync::Mutex<()> = std::sync::Mutex::new(());
        CHANGING.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// The servers switched off in the tier whose settings.json is `tier`, merged as [`Tier::servers`] merges.
    pub(crate) fn of(&self, tier: &Path) -> Rpc<BTreeMap<String, Map<String, Value>>> {
        let all = self.read()?;
        let Some(servers) = all.get(&key(tier)).and_then(Value::as_object) else { return Ok(BTreeMap::new()) };
        Ok(servers
            .iter()
            .map(|(name, kept)| {
                let mut merged = kept.get("settings").and_then(Value::as_object).cloned().unwrap_or_default();
                if let Some(local) = kept.get("local").and_then(Value::as_object) {
                    merged.extend(local.clone());
                }
                (name.clone(), merged)
            })
            .collect())
    }

    pub(crate) fn holds(&self, tier: &Path, name: &str) -> Rpc<bool> {
        Ok(self.read()?.get(&key(tier)).and_then(|s| s.get(name)).is_some())
    }

    /// Keep a server's two halves aside.
    pub(crate) fn keep(&self, tier: &Path, name: &str, public: Option<Value>, secret: Option<Value>) -> Rpc<()> {
        let _one = Self::changing();
        let mut all = self.read()?;
        let servers = all.entry(key(tier)).or_insert_with(|| json!({}));
        if !servers.is_object() {
            *servers = json!({});
        }
        servers
            .as_object_mut()
            .expect("an object just now")
            .insert(name.into(), json!({"settings": public.unwrap_or(json!({})), "local": secret}));
        write_private(&self.path, &Value::Object(all))
    }

    /// Take a server's two halves back out.
    pub(crate) fn take(&self, tier: &Path, name: &str) -> Rpc<Option<Halves>> {
        let _one = Self::changing();
        let mut all = self.read()?;
        let k = key(tier);
        let Some(servers) = all.get_mut(&k).and_then(Value::as_object_mut) else { return Ok(None) };
        let Some(kept) = servers.remove(name) else { return Ok(None) };
        if servers.is_empty() {
            all.remove(&k);
        }
        write_private(&self.path, &Value::Object(all))?;
        let half = |key: &str| kept.get(key).and_then(Value::as_object).cloned().unwrap_or_default();
        Ok(Some((half("settings"), half("local"))))
    }
}

fn key(tier: &Path) -> String {
    tier.display().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(v: Value) -> Input {
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn a_name_is_letters_digits_dashes_and_underscores_without_a_double_underscore() {
        for good in ["fs", "github", "my-server", "a_b", "9lives"] {
            assert!(check_name(good).is_ok(), "{good}");
        }
        for bad in ["", "-x", "_x", "a__b", "a.b", "a b", "ü", &"x".repeat(65)] {
            assert!(check_name(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_stdio_server_is_a_command_its_arguments_and_its_environment_kept_apart() {
        let given = input(json!({
            "transport": "stdio", "command": " npx ", "args": ["-y", "@x/server"],
            "env": {"TOKEN": {"secret": "s3cr$t-value"}, "HOME_DIR": {"var": "HOME"}},
        }));
        let (public, secret) = build(&given, "global", None).unwrap();
        assert_eq!(Value::Object(public), json!({"type": "stdio", "command": "npx", "args": ["-y", "@x/server"]}));
        // A literal `$` is written so the engine does not expand it; a variable is named.
        assert_eq!(Value::Object(secret), json!({"env": {"TOKEN": "s3cr$$t-value", "HOME_DIR": "${HOME}"}}));
    }

    #[test]
    fn a_remote_server_is_a_url_and_headers_and_keeps_what_the_form_does_not_set() {
        let before = json!({"type": "streamable_http", "url": "https://old.example/mcp", "oauth": {"client_id": "c"}, "tool_timeout_ms": 5000,
                            "headers": {"Authorization": "Bearer old-token-1234"}});
        let given = input(json!({"transport": "http", "url": "https://mcp.example/mcp", "headers": {"Authorization": {"keep": true}, "X-Team": {"secret": "t"}}}));
        let (public, secret) = build(&given, "global", before.as_object()).unwrap();
        assert_eq!(Value::Object(public), json!({"type": "streamable_http", "url": "https://mcp.example/mcp", "oauth": {"client_id": "c"}, "tool_timeout_ms": 5000}));
        assert_eq!(Value::Object(secret), json!({"headers": {"Authorization": "Bearer old-token-1234", "X-Team": "t"}}));
        // An sse server too; and turned into a process, its `oauth` is no longer one it has.
        let (public, _) = build(&input(json!({"transport": "sse", "url": "http://127.0.0.1:9/sse"})), "global", None).unwrap();
        assert_eq!(public["type"], json!("sse"));
        let (public, _) = build(&input(json!({"transport": "stdio", "command": "x"})), "global", before.as_object()).unwrap();
        assert!(!public.contains_key("oauth") && !public.contains_key("url"));
    }

    #[test]
    fn what_a_form_says_wrong_is_refused_by_name() {
        let refused = |v: Value, tier: &str| build(&input(v), tier, None).unwrap_err().message;
        assert!(refused(json!({"transport": "pigeon"}), "global").contains("not a transport"));
        assert!(refused(json!({"transport": "stdio", "command": "  "}), "global").contains("needs a command"));
        assert!(refused(json!({"transport": "http"}), "global").contains("needs a url"));
        assert!(refused(json!({"transport": "http", "url": "ftp://x/y"}), "global").contains("http or https"));
        assert!(refused(json!({"transport": "http", "url": "not a url"}), "global").contains("not a URL"));
        assert!(refused(json!({"transport": "stdio", "command": "x", "url": "https://x"}), "global").contains("no url"));
        assert!(refused(json!({"transport": "http", "url": "https://x", "env": {"A": {"secret": "b"}}}), "global").contains("no command"));
        assert!(refused(json!({"transport": "stdio", "command": "x", "env": {"1A": {"secret": "b"}}}), "global").contains("environment variable"));
        assert!(refused(json!({"transport": "stdio", "command": "x", "env": {"A": {"secret": "b", "var": "C"}}}), "global").contains("one of"));
        assert!(refused(json!({"transport": "stdio", "command": "x", "env": {"A": {}}}), "global").contains("one of"));
        assert!(refused(json!({"transport": "stdio", "command": "x", "env": {"A": {"var": "not-a-var"}}}), "global").contains("not the name"));
        assert!(refused(json!({"transport": "stdio", "command": "x", "env": {"A": {"keep": true}}}), "global").contains("no value written"));
        assert!(refused(json!({"transport": "http", "url": "https://x", "headers": {"Bad Name": {"secret": "v"}}}), "global").contains("a header"));
        assert!(refused(json!({"transport": "http", "url": "https://x", "headers": {"X": {"secret": "a\r\nInjected: 1"}}}), "global").contains("line break"));
        assert!(serde_json::from_value::<Input>(json!({"transport": "stdio", "cmd": "x"})).is_err(), "an unknown key is not ignored");
    }

    #[test]
    fn a_projects_server_takes_no_value_from_the_engines_environment() {
        let given = json!({"transport": "stdio", "command": "x", "env": {"TOKEN": {"var": "GITHUB_TOKEN"}}});
        let e = build(&input(given.clone()), "project", None).unwrap_err();
        assert!(e.message.contains("cannot read the engine's environment"), "{}", e.message);
        assert!(build(&input(given), "global", None).is_ok());
        assert!(build(&input(json!({"transport": "stdio", "command": "x", "env": {"TOKEN": {"secret": "v"}}})), "project", None).is_ok());
    }

    #[test]
    fn a_value_read_back_is_the_variable_it_names_or_masked() {
        assert_eq!(shown_value(&json!("${GITHUB_TOKEN}")), McpValue { kind: "env".into(), var: Some("GITHUB_TOKEN".into()), masked: None });
        assert_eq!(shown_value(&json!("$HOME")).var.as_deref(), Some("HOME"));
        let long = shown_value(&json!("Bearer sk-0123456789abcd"));
        assert_eq!((long.kind.as_str(), long.masked.as_deref()), ("secret", Some("***abcd")));
        assert_eq!(shown_value(&json!("short")).masked.as_deref(), Some("***"));
        // A value with a variable inside it is still a value, and the escape is not shown.
        assert_eq!(shown_value(&json!("pre-${X}-post-xxxxxx")).kind, "secret");
        assert_eq!(shown_value(&json!("0123456789a$$")).masked.as_deref(), Some("***89a$"));
    }

    #[test]
    fn a_server_is_viewed_merged_with_no_secret_in_it() {
        let server = json!({"type": "streamable_http", "url": "https://x/mcp", "oauth": {"client_id": "c", "private_key_jwt": {"key_file": "/k.pem"}},
                            "headers": {"Authorization": "Bearer sk-0123456789abcd"}});
        let shown = view("docs", "global", true, server.as_object().unwrap());
        let text = serde_json::to_string(&shown).unwrap();
        assert!(!text.contains("0123456789"), "{text}");
        assert_eq!(shown.transport, "http");
        assert_eq!(shown.unsupported, ["private_key_jwt"]);
        assert_eq!(shown.oauth.unwrap().grant, "authorization_code");
        assert_eq!(view("x", "global", true, json!({"type": "web_socket", "url": "wss://x"}).as_object().unwrap()).transport, "websocket");
    }

    #[test]
    fn a_tier_is_edited_in_its_two_files_and_a_server_replaced_whole() {
        let dir = tempfile::tempdir().unwrap();
        let settings = dir.path().join("settings.json");
        std::fs::write(&settings, r#"{"model": {"max_tokens": 10}, "mcp_servers": {"old": {"type": "stdio", "command": "o"}}}"#).unwrap();
        std::fs::write(local_of(&settings), r#"{"providers": {"p": {"api_key": "k"}}, "mcp_servers": {"fs": {"env": {"A": "stale"}}}}"#).unwrap();
        let given = input(json!({"transport": "stdio", "command": "fs-server", "env": {"T": {"secret": "v"}}}));
        edit(&settings, |tier| {
            let (public, secret) = build(&given, "global", tier.servers().get("fs"))?;
            tier.put("fs", public, secret);
            Ok(())
        })
        .unwrap();
        let tier = read(&settings).unwrap();
        assert_eq!(tier.settings["model"]["max_tokens"], json!(10), "the rest of the file is kept");
        assert_eq!(tier.settings["mcp_servers"]["fs"], json!({"type": "stdio", "command": "fs-server", "args": []}));
        assert_eq!(tier.local["mcp_servers"]["fs"], json!({"env": {"T": "v"}}), "the stale half went with the server");
        assert_eq!(tier.local["providers"]["p"]["api_key"], json!("k"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(local_of(&settings)).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
        // Taken out: from both files, and the key goes with the last server.
        edit(&settings, |tier| {
            tier.take("fs");
            tier.take("old");
            Ok(())
        })
        .unwrap();
        let tier = read(&settings).unwrap();
        assert!(tier.settings.get("mcp_servers").is_none() && tier.local.get("mcp_servers").is_none(), "{tier:?}");
    }

    #[test]
    fn a_server_switched_off_is_kept_aside_with_its_secrets_and_comes_back_as_it_was() {
        let dir = tempfile::tempdir().unwrap();
        let settings = dir.path().join("tier").join("settings.json");
        std::fs::create_dir_all(settings.parent().unwrap()).unwrap();
        std::fs::write(&settings, r#"{"mcp_servers": {"fs": {"type": "stdio", "command": "x"}}}"#).unwrap();
        std::fs::write(local_of(&settings), r#"{"mcp_servers": {"fs": {"env": {"T": "secret-value"}}}}"#).unwrap();
        let aside = Aside::new(dir.path().join("data").join("mcp-aside.json"));
        let (public, secret) = edit(&settings, |tier| Ok(tier.take("fs"))).unwrap();
        aside.keep(&settings, "fs", public, secret).unwrap();
        assert!(!read(&settings).unwrap().declares("fs"));
        assert!(aside.holds(&settings, "fs").unwrap());
        assert_eq!(aside.of(&settings).unwrap()["fs"], json!({"type": "stdio", "command": "x", "env": {"T": "secret-value"}}).as_object().unwrap().clone());
        let (public, secret) = aside.take(&settings, "fs").unwrap().unwrap();
        edit(&settings, |tier| {
            tier.put("fs", public, secret);
            Ok(())
        })
        .unwrap();
        let tier = read(&settings).unwrap();
        assert_eq!(tier.local["mcp_servers"]["fs"]["env"]["T"], json!("secret-value"));
        assert!(!aside.holds(&settings, "fs").unwrap());
        assert!(aside.take(&settings, "fs").unwrap().is_none());
    }

    #[test]
    fn reading_a_tier_that_has_no_files_makes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let settings = dir.path().join("project").join(".harnix").join("settings.json");
        assert_eq!(read(&settings).unwrap(), Tier { settings: json!({}), local: json!({}) });
        assert!(!dir.path().join("project").exists(), "a read made the tier's directory");
    }

    #[test]
    fn a_merged_server_splits_back_into_its_two_halves() {
        let merged = json!({"type": "stdio", "command": "x", "env": {"A": "b"}});
        let (public, secret) = split(merged.as_object().unwrap());
        assert_eq!(Value::Object(public), json!({"type": "stdio", "command": "x"}));
        assert_eq!(Value::Object(secret), json!({"env": {"A": "b"}}));
    }

    #[test]
    fn the_engines_status_is_read_without_a_token() {
        let status = status_of(&json!({"name": "docs", "transport": "streamable_http", "tool_count": 0, "state": "auth_required",
                                       "auth": {"state": "required", "scopes": ["files:read"]}}));
        assert_eq!((status.state.as_str(), status.auth.state.as_str()), ("auth_required", "required"));
        assert_eq!(status.auth.scopes, ["files:read"]);
    }
}
