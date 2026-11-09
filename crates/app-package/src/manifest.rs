//! `harnix-app.toml`: what an application is, and what it asks for.
//!
//! Read strictly. Everything an application may have is a section or a field below; anything else
//! is refused with its name, because a manifest is also what the installer shows a person, and a
//! line nothing reads is a line that person was shown for nothing — or a capability somebody
//! believes was granted.
//!
//! ```toml
//! [app]
//! id = "acme.showcase"          # <publisher>.<name>; dot-separated lowercase segments
//! version = "0.1.0"
//! harnix_api = 1                # the application interface's major version
//! title = "Showcase"            # optional
//! publisher = "acme"            # the id's first segment
//! description = "…"             # optional
//!
//! [capabilities]                # nothing unless written
//! net = ["api.example.com"]     # hosts `http-request` may reach, exactly
//! env = ["SHOWCASE_TOKEN"]      # what `secret` may read
//! storage_mb = 1                # the key-value store's ceiling
//! hub = ["session.create", "topic:harnix.sessions.status"]
//!                               # what it may call (methods) and subscribe to (`topic:<name>`) as
//!                               # `module:<id>`; its own `app.<id>.*` needs no entry
//! fs = ["${data}", "${project}"]# ${data} read-write; ${project} read-only, tool calls only
//!
//! [[component]]                 # at most one
//! name = "tools"                # wasm/<name>.<engine fingerprint>.cwasm
//! world = "harnix:app/tools-app@0.1.0"   # or harnix:app/backend-app@0.1.0
//! memory_mb = 64
//! timeout_ms = 30000
//!
//! [[method]]                    # app.<id>.<name>, answered by the backend (`backend-app` only)
//! name = "notes.add"
//! subject = "device"            # who may call it: device (the default), module, kernel, or a list
//! description = "…"             # optional
//! [method.params]               # optional: JSON Schema the params are checked against
//! type = "object"
//! required = ["text"]
//!
//! [[topic]]                     # app.<id>.<name>, published by the backend (`backend-app` only)
//! name = "notes"
//!
//! [[space]]                     # a backend space: where its sessions land
//! id = "acme.showcase"          # the application's id, or `<id>.<name>`
//! scene = "harnix"              # optional: the scene a new session starts in
//! instance = "system"           # optional
//!
//! [config]                      # host.config(): defaults, set over by the profile
//! greeting = "hello"
//!
//! [[ui]]                        # ES modules under ui/, as a package's [[ui]]: run in the page,
//! point = "home.card"           # a trusted publisher's only
//! module = "showcase.js"
//! api_version = 2
//!
//! [interface]                   # the interface in isolated frames (`surface.rs`)
//! hub = ["harnix.sessions"]     # beyond app.<id>.*, what a frame may call
//!
//! [[surface]]                   # a region of the shell a frame fills
//! point = "space"
//! space = "acme.showcase"
//! page = "frame/board.html"     # under ui/
//!
//! [agent]                       # agent/ (an Agix package) goes into these instances
//! install_into = ["system"]
//!
//! [[inline.tool_row]]           # what the shell draws for it inline, from here alone: tool rows,
//! tool = "echo"                 #   cards in the flow, commands, how its space groups sessions
//! title = "Echo · {input.text}" #   (`inline.rs`)
//! ```

use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::Value;

use crate::inline::Inline;
pub use crate::surface::{Interface, Surface, Title};

/// The application interface this build implements: `[app] harnix_api`.
pub const HARNIX_API: u32 = 1;

/// The file name, at the package's root.
pub const MANIFEST: &str = "harnix-app.toml";

/// The sections a manifest may have.
const SECTIONS: &[&str] = &[
    "app",
    "capabilities",
    "component",
    "config",
    "ui",
    "agent",
    "method",
    "topic",
    "space",
    "inline",
    "interface",
    "surface",
];

/// The capabilities there are.
const CAPABILITIES: &[&str] = &["net", "env", "storage_mb", "hub", "fs"];

#[derive(Debug, Clone, PartialEq)]
pub struct Manifest {
    pub app: App,
    pub capabilities: Capabilities,
    pub component: Option<Component>,
    /// `[config]`, as JSON.
    pub config: Value,
    pub ui: Vec<UiEntry>,
    pub agent: Agent,
    pub methods: Vec<Method>,
    pub topics: Vec<Topic>,
    pub spaces: Vec<Space>,
    /// `[inline]`: what the shell draws for it, declared.
    pub inline: Inline,
    /// `[interface]`: how the interface runs in frames, and what they may reach.
    pub interface: Interface,
    /// `[[surface]]`: the regions of the shell its frames fill.
    pub surfaces: Vec<Surface>,
}

/// One `[[method]]`: `app.<id>.<name>`, answered by the backend's `handle`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Method {
    pub name: String,
    /// Who may call it: `device`, `module`, `kernel`, or several. The application itself always
    /// may (its tools reach their backend that way).
    #[serde(default = "default_subject")]
    pub subject: Subjects,
    #[serde(default)]
    pub description: Option<String>,
    /// JSON Schema its params are checked against before the backend sees them, written as TOML.
    #[serde(default)]
    pub params: Option<toml::Value>,
}

/// `subject = "device"` or `subject = ["device", "module"]`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum Subjects {
    One(String),
    Many(Vec<String>),
}

impl Subjects {
    pub fn names(&self) -> Vec<&str> {
        match self {
            Subjects::One(s) => vec![s.as_str()],
            Subjects::Many(v) => v.iter().map(String::as_str).collect(),
        }
    }
}

fn default_subject() -> Subjects {
    Subjects::One("device".into())
}

/// Who a method may be declared for.
pub const SUBJECTS: &[&str] = &["device", "module", "kernel"];

/// One `[[topic]]`: `app.<id>.<name>`, whole values the backend publishes (`host.publish`).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Topic {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
}

/// One `[[space]]`: a backend space, where the sessions created in it land. What it looks like is
/// the interface module's (a `space` contribution of the same id).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Space {
    /// The application's id, or `<id>.<name>`.
    pub id: String,
    #[serde(default)]
    pub scene: Option<String>,
    #[serde(default)]
    pub instance: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct App {
    pub id: String,
    pub version: String,
    pub harnix_api: u32,
    #[serde(default)]
    pub title: Option<String>,
    pub publisher: String,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Capabilities {
    #[serde(default)]
    pub net: Vec<String>,
    #[serde(default)]
    pub env: Vec<String>,
    #[serde(default)]
    pub storage_mb: u32,
    #[serde(default)]
    pub hub: Vec<String>,
    #[serde(default)]
    pub fs: Vec<String>,
}

/// The two worlds a component may implement, by the names a manifest writes.
pub const WORLDS: &[&str] = &["harnix:app/tools-app@0.1.0", "harnix:app/backend-app@0.1.0"];

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Component {
    pub name: String,
    pub world: String,
    #[serde(default = "default_memory_mb")]
    pub memory_mb: u32,
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
}

fn default_memory_mb() -> u32 {
    64
}

fn default_timeout_ms() -> u64 {
    30_000
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UiEntry {
    pub point: String,
    /// Under the package's `ui/`.
    pub module: String,
    pub api_version: u32,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Agent {
    /// `system`, or the id of an instance that exists when the application is installed.
    #[serde(default = "default_install_into")]
    pub install_into: Vec<String>,
}

impl Default for Agent {
    fn default() -> Self {
        Self {
            install_into: default_install_into(),
        }
    }
}

fn default_install_into() -> Vec<String> {
    vec!["system".to_string()]
}

/// What the manifest says, in a sentence for whoever is installing it.
pub type Error = String;

impl Manifest {
    pub fn parse(text: &str) -> Result<Self, Error> {
        let table: toml::Table = text.parse().map_err(|e: toml::de::Error| {
            format!("{MANIFEST} is not valid TOML: {}", e.message())
        })?;
        for key in table.keys() {
            if !SECTIONS.contains(&key.as_str()) {
                return Err(format!(
                    "{MANIFEST}: unknown section `{key}` (an application has: {})",
                    SECTIONS.join(", ")
                ));
            }
        }
        if let Some(caps) = table.get("capabilities") {
            let caps = caps
                .as_table()
                .ok_or_else(|| format!("{MANIFEST}: `capabilities` is a table"))?;
            for key in caps.keys() {
                if !CAPABILITIES.contains(&key.as_str()) {
                    return Err(format!(
                        "{MANIFEST}: unknown capability `{key}` — an application may ask for {} and nothing else",
                        CAPABILITIES.join(", ")
                    ));
                }
            }
        }

        let section = |name: &str| -> Option<toml::Value> { table.get(name).cloned() };
        let app: App = match section("app") {
            Some(v) => v.try_into().map_err(|e| field_error("app", e))?,
            None => return Err(format!("{MANIFEST}: `[app]` is missing")),
        };
        let capabilities: Capabilities = match section("capabilities") {
            Some(v) => v.try_into().map_err(|e| field_error("capabilities", e))?,
            None => Capabilities::default(),
        };
        let components: Vec<Component> = match section("component") {
            Some(v) => v.try_into().map_err(|e| field_error("[component]]", e))?,
            None => Vec::new(),
        };
        if components.len() > 1 {
            return Err(format!(
                "{MANIFEST}: an application has one component; this one declares {}",
                components.len()
            ));
        }
        let config = match section("config") {
            Some(toml::Value::Table(t)) => toml_to_json(&toml::Value::Table(t)),
            Some(_) => return Err(format!("{MANIFEST}: `config` is a table")),
            None => Value::Object(Default::default()),
        };
        let ui: Vec<UiEntry> = match section("ui") {
            Some(v) => v.try_into().map_err(|e| field_error("[ui]]", e))?,
            None => Vec::new(),
        };
        let agent: Agent = match section("agent") {
            Some(v) => v.try_into().map_err(|e| field_error("agent", e))?,
            None => Agent::default(),
        };
        let methods: Vec<Method> = match section("method") {
            Some(v) => v.try_into().map_err(|e| field_error("[method]]", e))?,
            None => Vec::new(),
        };
        let topics: Vec<Topic> = match section("topic") {
            Some(v) => v.try_into().map_err(|e| field_error("[topic]]", e))?,
            None => Vec::new(),
        };
        let spaces: Vec<Space> = match section("space") {
            Some(v) => v.try_into().map_err(|e| field_error("[space]]", e))?,
            None => Vec::new(),
        };
        let inline = match section("inline") {
            Some(v) => Inline::read(&v)?,
            None => Inline::default(),
        };
        let interface: Interface = match section("interface") {
            Some(v) => v.try_into().map_err(|e| field_error("interface", e))?,
            None => Interface::default(),
        };
        let surfaces: Vec<Surface> = match section("surface") {
            Some(v) => v.try_into().map_err(|e| field_error("[surface]]", e))?,
            None => Vec::new(),
        };
        let manifest = Manifest {
            app,
            capabilities,
            component: components.into_iter().next(),
            config,
            ui,
            agent,
            methods,
            topics,
            spaces,
            inline,
            interface,
            surfaces,
        };
        manifest.check()?;
        Ok(manifest)
    }

    fn check(&self) -> Result<(), Error> {
        let app = &self.app;
        if !crate::valid_id(&app.id) || !app.id.contains('.') {
            return Err(format!(
                "{MANIFEST}: `{}` is not an application id: `<publisher>.<name>`, dot-separated \
                 segments of lowercase letters, digits and `-`, each starting with a letter",
                app.id
            ));
        }
        if app.id.split('.').next() != Some(app.publisher.as_str()) {
            return Err(format!(
                "{MANIFEST}: the publisher `{}` is not the id's first segment (`{}`)",
                app.publisher, app.id
            ));
        }
        if !valid_version(&app.version) {
            return Err(format!(
                "{MANIFEST}: `{}` is not a version: `x.y.z`, with an optional `-<pre-release>`",
                app.version
            ));
        }
        if app.harnix_api != HARNIX_API {
            return Err(format!(
                "{MANIFEST}: `harnix_api = {}`, and this Harnix implements {HARNIX_API}",
                app.harnix_api
            ));
        }
        let caps = &self.capabilities;
        for host in &caps.net {
            if host.is_empty() || host.contains(['/', ':', '@', '*', ' ']) {
                return Err(format!(
                    "{MANIFEST}: `net` lists hosts, exactly (`api.example.com`), and `{host}` is not one"
                ));
            }
        }
        for name in &caps.env {
            if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                return Err(format!(
                    "{MANIFEST}: `{name}` in `env` is not an environment variable name"
                ));
            }
        }
        for entry in &caps.hub {
            let name = entry.strip_prefix("topic:").unwrap_or(entry);
            if !valid_dotted(name) {
                return Err(format!(
                    "{MANIFEST}: `{entry}` in `hub` is not a method name, or `topic:` and a topic's"
                ));
            }
        }
        // What is written under ${data} counts toward `storage_mb`: with none, it could write nothing.
        if caps.storage_mb == 0 && caps.fs.iter().any(|p| p.starts_with("${data}")) {
            return Err(format!(
                "{MANIFEST}: `fs` grants ${{data}}, and its files count toward `storage_mb`, which is 0"
            ));
        }
        // The grants themselves are checked when the VM starts (`harnix-wasm`); the anchors here.
        for path in &caps.fs {
            if !(path.starts_with("${data}") || path.starts_with("${project}")) {
                return Err(format!(
                    "{MANIFEST}: `{path}` in `fs` must start with ${{data}} (the application's own \
                     directory) or ${{project}} (the calling session's, read-only)"
                ));
            }
        }
        if let Some(c) = &self.component {
            if !valid_name(&c.name) {
                return Err(format!(
                    "{MANIFEST}: the component's name `{}` is lowercase letters, digits and `-`",
                    c.name
                ));
            }
            if !WORLDS.contains(&c.world.as_str()) {
                return Err(format!(
                    "{MANIFEST}: the component's world is one of {}, not `{}`",
                    WORLDS.join(", "),
                    c.world
                ));
            }
            if !(1..=4096).contains(&c.memory_mb) {
                return Err(format!("{MANIFEST}: `memory_mb` is 1 to 4096"));
            }
            if !(100..=600_000).contains(&c.timeout_ms) {
                return Err(format!("{MANIFEST}: `timeout_ms` is 100 to 600000"));
            }
        }
        for entry in &self.ui {
            if entry.api_version != harnix_contract::CONTRIB_API_VERSION {
                return Err(format!(
                    "{MANIFEST}: the interface module `{}` is for contribution API {}, and this \
                     Harnix's is {}",
                    entry.module,
                    entry.api_version,
                    harnix_contract::CONTRIB_API_VERSION
                ));
            }
            if !safe_relative(&entry.module) || !entry.module.ends_with(".js") {
                return Err(format!(
                    "{MANIFEST}: the interface module `{}` is a `.js` file under ui/",
                    entry.module
                ));
            }
        }
        let backend = self.component.as_ref().is_some_and(|c| c.world == WORLDS[1]);
        if !backend && (!self.methods.is_empty() || !self.topics.is_empty()) {
            return Err(format!(
                "{MANIFEST}: `[[method]]` and `[[topic]]` are a backend's: the component's world is \
                 `{}`",
                WORLDS[1]
            ));
        }
        let mut names = std::collections::BTreeSet::new();
        for m in &self.methods {
            if !valid_dotted(&m.name) || !names.insert(format!("method {}", m.name)) {
                return Err(format!(
                    "{MANIFEST}: the method `{}` is a unique dot-separated name of letters, digits, \
                     `_` and `-`",
                    m.name
                ));
            }
            let subjects = m.subject.names();
            if subjects.is_empty() || subjects.iter().any(|s| !SUBJECTS.contains(s)) {
                return Err(format!(
                    "{MANIFEST}: the method `{}`'s `subject` is one or more of {}",
                    m.name,
                    SUBJECTS.join(", ")
                ));
            }
            if let Some(params) = &m.params {
                let schema = toml_to_json(params);
                if !schema.is_object() || jsonschema::meta::validate(&schema).is_err() {
                    return Err(format!(
                        "{MANIFEST}: the method `{}`'s `params` is not a JSON Schema",
                        m.name
                    ));
                }
            }
        }
        for t in &self.topics {
            if !valid_dotted(&t.name) || !names.insert(format!("topic {}", t.name)) {
                return Err(format!(
                    "{MANIFEST}: the topic `{}` is a unique dot-separated name of letters, digits, \
                     `_` and `-`",
                    t.name
                ));
            }
        }
        for space in &self.spaces {
            let id = &app.id;
            let ours = space.id == *id
                || space
                    .id
                    .strip_prefix(&format!("{id}."))
                    .is_some_and(valid_name);
            if !ours || !names.insert(format!("space {}", space.id)) {
                return Err(format!(
                    "{MANIFEST}: the space `{}` is `{id}` or `{id}.<name>`, once",
                    space.id
                ));
            }
        }
        crate::surface::check(&app.id, &self.interface, &self.surfaces)?;
        for target in &self.agent.install_into {
            if target.is_empty() || target.contains(['/', ' ']) {
                return Err(format!(
                    "{MANIFEST}: `{target}` in `install_into` is not an instance id"
                ));
            }
        }
        self.inline.check(self)
    }

    /// `[config]`, with `overrides` set over it key by key.
    pub fn config_with(&self, overrides: Option<&Value>) -> Value {
        let mut config = self.config.clone();
        if let (Some(Value::Object(over)), Value::Object(base)) = (overrides, &mut config) {
            for (k, v) in over {
                base.insert(k.clone(), v.clone());
            }
        }
        config
    }
}

fn field_error(section: &str, e: toml::de::Error) -> Error {
    format!("{MANIFEST}: in `[{section}]`: {}", e.message().trim())
}

pub(crate) fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .all(|c| matches!(c, 'a'..='z' | '0'..='9' | '-'))
}

/// A method or topic name: dot-separated segments of letters, digits, `_` and `-`.
pub(crate) fn valid_dotted(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && name.split('.').all(|s| {
            !s.is_empty()
                && s
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-'))
        })
}

fn valid_version(v: &str) -> bool {
    let (core, pre) = match v.split_once('-') {
        Some((core, pre)) => (core, Some(pre)),
        None => (v, None),
    };
    let numbers: Vec<&str> = core.split('.').collect();
    numbers.len() == 3
        && numbers
            .iter()
            .all(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
        && pre.is_none_or(|p| {
            !p.is_empty() && p.chars().all(|c| c.is_ascii_alphanumeric() || c == '.')
        })
        && v.len() <= 64
}

/// A relative path with no `..`, no root and no empty segment.
pub fn safe_relative(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with('/')
        && !path.contains('\\')
        && path
            .split('/')
            .all(|s| !s.is_empty() && s != "." && s != "..")
}

pub fn toml_to_json(v: &toml::Value) -> Value {
    match v {
        toml::Value::String(s) => Value::String(s.clone()),
        toml::Value::Integer(i) => Value::from(*i),
        toml::Value::Float(f) => serde_json::Number::from_f64(*f)
            .map(Value::Number)
            .unwrap_or(Value::Null),
        toml::Value::Boolean(b) => Value::Bool(*b),
        toml::Value::Datetime(d) => Value::String(d.to_string()),
        toml::Value::Array(a) => Value::Array(a.iter().map(toml_to_json).collect()),
        toml::Value::Table(t) => Value::Object(
            t.iter()
                .map(|(k, v)| (k.clone(), toml_to_json(v)))
                .collect::<serde_json::Map<_, _>>(),
        ),
    }
}

/// The overrides for one application from the profile's `[apps.config]`.
pub fn overrides_for(config: &BTreeMap<String, toml::Value>, id: &str) -> Option<Value> {
    config.get(id).map(toml_to_json)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const GOOD: &str = r#"
[app]
id = "acme.showcase"
version = "0.1.0"
harnix_api = 1
title = "Showcase"
publisher = "acme"

[capabilities]
storage_mb = 1
fs = ["${project}"]

[[component]]
name = "tools"
world = "harnix:app/tools-app@0.1.0"
timeout_ms = 5000

[config]
greeting = "hello"
nested = { a = 1 }

[[ui]]
point = "home.card"
module = "showcase.js"
api_version = 2

[agent]
install_into = ["system"]
"#;

    fn with(edit: impl Fn(&str) -> String) -> Result<Manifest, Error> {
        Manifest::parse(&edit(GOOD))
    }

    #[test]
    fn a_complete_manifest_reads() {
        let m = Manifest::parse(GOOD).unwrap();
        assert_eq!(m.app.id, "acme.showcase");
        assert_eq!(m.capabilities.storage_mb, 1);
        let c = m.component.as_ref().unwrap();
        assert_eq!(
            (c.name.as_str(), c.memory_mb, c.timeout_ms),
            ("tools", 64, 5000)
        );
        assert_eq!(m.config, json!({"greeting": "hello", "nested": {"a": 1}}));
        assert_eq!(m.ui[0].module, "showcase.js");
        assert_eq!(m.agent.install_into, ["system"]);
        assert_eq!(
            m.config_with(Some(&json!({"greeting": "hi", "extra": true}))),
            json!({"greeting": "hi", "nested": {"a": 1}, "extra": true})
        );
    }

    #[test]
    fn the_least_an_application_can_be() {
        let m = Manifest::parse(
            "[app]\nid = \"a.b\"\nversion = \"1.0.0-rc.1\"\nharnix_api = 1\npublisher = \"a\"\n",
        )
        .unwrap();
        assert!(m.component.is_none() && m.ui.is_empty());
        assert_eq!(m.capabilities, Capabilities::default());
        assert_eq!(
            m.agent.install_into,
            ["system"],
            "agent/ goes to system unless said"
        );
    }

    #[test]
    fn what_is_not_known_is_refused_by_name() {
        let cases: Vec<(String, &str)> = vec![
            (GOOD.replace("storage_mb = 1", "gpu = true"), "unknown capability `gpu`"),
            (format!("{GOOD}\n[extras]\nx = 1\n"), "unknown section `extras`"),
            (format!("{GOOD}\n[[method]]\nname = \"m\"\n"), "are a backend's"),
            (format!("{GOOD}\n[[topic]]\nname = \"t\"\n"), "are a backend's"),
            (GOOD.replace("title = \"Showcase\"", "title = \"S\"\ncolour = \"red\""), "unknown field `colour`"),
            (GOOD.replace("timeout_ms = 5000", "timeout_ms = 5000\nthreads = 4"), "unknown field `threads`"),
            (GOOD.replace("harnix_api = 1", "harnix_api = 2"), "implements 1"),
            (GOOD.replace("acme.showcase", "Acme.Showcase"), "not an application id"),
            (GOOD.replace("\"acme.showcase\"", "\"showcase\""), "not an application id"),
            (GOOD.replace("publisher = \"acme\"", "publisher = \"other\""), "first segment"),
            (GOOD.replace("0.1.0", "one"), "not a version"),
            (GOOD.replace("tools-app@0.1.0", "tools@0.1.0"), "world is one of"),
            (GOOD.replace("fs = [\"${project}\"]", "fs = [\"/etc\"]"), "must start with"),
            (
                GOOD.replace("storage_mb = 1", "storage_mb = 0")
                    .replace("fs = [\"${project}\"]", "fs = [\"${data}\"]"),
                "which is 0",
            ),
            (GOOD.replace("storage_mb = 1", "net = [\"https://x.example\"]"), "lists hosts"),
            (GOOD.replace("api_version = 2", "api_version = 1"), "contribution API 1"),
            (GOOD.replace("showcase.js", "../escape.js"), "under ui/"),
            (GOOD.replace("[app]", "[app]\n").replace("id = ", "ident = "), "in `[app]`"),
            (format!("{GOOD}\n[[component]]\nname = \"two\"\nworld = \"harnix:app/tools-app@0.1.0\"\n"), "one component"),
            ("not = [toml".to_string(), "not valid TOML"),
            ("[capabilities]\n".to_string(), "`[app]` is missing"),
        ];
        for (text, says) in cases {
            let err = Manifest::parse(&text)
                .err()
                .unwrap_or_else(|| panic!("accepted: {says}"));
            assert!(err.contains(says), "wanted `{says}`, got: {err}");
            assert!(err.starts_with(MANIFEST), "{err}");
        }
        assert!(with(|g| g.to_string()).is_ok());
    }

    const BACKEND: &str = r#"
[[method]]
name = "notes.add"
description = "Add a note"
[method.params]
type = "object"
required = ["text"]
properties.text = { type = "string", maxLength = 20 }

[[method]]
name = "notes.list"
subject = ["device", "module"]

[[topic]]
name = "notes"

[[space]]
id = "acme.showcase"
scene = "harnix"
"#;

    fn backend(extra: &str) -> String {
        format!(
            "{}\n{BACKEND}\n{extra}",
            GOOD.replace("tools-app@0.1.0", "backend-app@0.1.0")
        )
    }

    #[test]
    fn a_backend_declares_its_methods_topics_and_spaces() {
        let m = Manifest::parse(&backend("")).unwrap();
        assert_eq!(m.methods.len(), 2);
        assert_eq!(m.methods[0].subject.names(), ["device"]);
        assert_eq!(m.methods[1].subject.names(), ["device", "module"]);
        assert_eq!(
            toml_to_json(m.methods[0].params.as_ref().unwrap())["required"],
            json!(["text"])
        );
        assert_eq!(m.topics[0].name, "notes");
        assert_eq!(m.spaces[0].scene.as_deref(), Some("harnix"));
        let hub = backend("").replace(
            "storage_mb = 1",
            "storage_mb = 1\nhub = [\"session.create\", \"topic:harnix.sessions.status\"]",
        );
        assert_eq!(Manifest::parse(&hub).unwrap().capabilities.hub.len(), 2);
    }

    #[test]
    fn a_backends_declarations_are_checked() {
        let cases: Vec<(String, &str)> = vec![
            (backend("[[method]]\nname = \"notes.add\"\n"), "unique"),
            (backend("[[method]]\nname = \"bad name\"\n"), "unique"),
            (backend("[[method]]\nname = \"m\"\nsubject = \"everyone\"\n"), "`subject`"),
            (backend("[[method]]\nname = \"m\"\nsubject = []\n"), "`subject`"),
            (backend("[[method]]\nname = \"m\"\nparams = { type = 7 }\n"), "not a JSON Schema"),
            (backend("[[topic]]\nname = \"\"\n"), "the topic"),
            (backend("[[space]]\nid = \"other.space\"\n"), "the space"),
            (backend("[[space]]\nid = \"acme.showcase\"\n"), "once"),
            (backend("[[space]]\nid = \"acme.showcase.x\"\ncolour = 1\n"), "unknown field"),
            (
                backend("").replace("storage_mb = 1", "storage_mb = 1\nhub = [\"bad name\"]"),
                "in `hub`",
            ),
        ];
        for (text, says) in cases {
            let err = Manifest::parse(&text)
                .err()
                .unwrap_or_else(|| panic!("accepted: {says}"));
            assert!(err.contains(says), "wanted `{says}`, got: {err}");
        }
    }
}
