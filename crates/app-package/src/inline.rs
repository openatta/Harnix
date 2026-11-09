//! `[inline]`: what the shell draws for an application inside its own views, from the manifest
//! alone. No code of the application's runs in the page for any of it: the shell has the
//! components, the application fills in the words and says which of its methods a button calls.
//!
//! ```toml
//! [[inline.tool_row]]           # how a call of one of its tools looks in the flow
//! tool = "echo"                 # one of the component's tools
//! icon = "terminal"             # a shell glyph
//! title = "Echo · {input.text}" # `{input.<path>}`, `{result}`
//! summary = "result"            # what follows the title: `result`, or `input.<path>`
//! collapsed = false             # the result shown unfolded (folded unless said)
//!
//! [[inline.flow_block]]         # an entry it appends (`harnix.entries.append`) with this event,
//! kind = "card"                 #   drawn as a card: `{payload.<path>}` in every text
//! icon = "file"
//! title = "{payload.title}"
//! body = "{payload.body}"       # Markdown, as the shell renders it (escaped, http(s) links only)
//! [[inline.flow_block.field]]   # key and value rows
//! label = "Notes"
//! value = "{payload.notes}"
//! [[inline.flow_block.button]]  # calls app.<id>.<method>: one of its own, callable by a device
//! label = "Count one more"
//! method = "counter.bump"
//! params = { by = "{payload.step}" }   # a string that is one placeholder keeps the value's type
//! confirm = "Count one more?"   # the shell asks first
//! done = "Now at {result.counter}"     # shown in the card after; `{result.<path>}` too
//!
//! [[inline.command]]            # a `/` command or an `@` source; app.<id>.complete gives candidates
//! trigger = "/"
//! name = "showcase"
//! description = "The showcase's commands"
//!
//! [[inline.sidebar_group]]      # how one of its spaces groups its sessions in the left column
//! space = "example.showcase"    # a `[[space]]` of its own (the application's id when absent)
//! by = "method"                 # `scene`, `field` (with `field = "<the session row's key>"`), or
//! method = "sessions.group"     #   `method`: app.<id>.<method> {space, sessions} → {groups}
//! other = "Everything else"     # the group for what no rule places
//! ```
//!
//! Every text is a string or a table of them by locale (`{ en = "…", zh-CN = "…" }`). Each point is
//! checked here against the rest of the manifest: a button's or a grouping's method is one it
//! declares for a device, a command needs its `complete`, a card needs `harnix.entries.append` in
//! its `hub` to be appended at all. Unknown points and fields are refused by name.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::manifest::{toml_to_json, Manifest, MANIFEST};

/// The points `[inline]` may declare.
pub const POINTS: &[&str] = &["tool_row", "flow_block", "command", "sidebar_group"];

/// The locales a text may be given in: the shell's.
pub const LOCALES: &[&str] = &["en", "zh-CN"];

/// What a card is drawn from: the entry an application appends, through this method.
pub const APPEND: &str = "harnix.entries.append";

/// What a command's candidates come from: `app.<id>.complete`.
pub const COMPLETE: &str = "complete";

const MAX_TEXT: usize = 200;
const MAX_BODY: usize = 4000;

#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Inline {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_row: Vec<ToolRow>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub flow_block: Vec<FlowBlock>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub command: Vec<Command>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sidebar_group: Vec<SidebarGroup>,
}

/// A string, or one per locale.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum Text {
    One(String),
    Locales(BTreeMap<String, String>),
}

impl Text {
    fn strings(&self) -> Vec<&str> {
        match self {
            Text::One(s) => vec![s.as_str()],
            Text::Locales(m) => m.values().map(String::as_str).collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ToolRow {
    pub tool: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<Text>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default = "folded")]
    pub collapsed: bool,
}

fn folded() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FlowBlock {
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    pub title: Text,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<Text>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub field: Vec<Field>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub button: Vec<Button>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Field {
    pub label: Text,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Button {
    pub label: Text,
    /// One of the application's own `[[method]]`s, without the `app.<id>.` it is called by.
    pub method: String,
    /// The params, as a table; its strings may be templates.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirm: Option<Text>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub done: Option<Text>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Command {
    pub trigger: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<Text>,
}

/// How a grouping is decided.
pub const GROUP_BY: &[&str] = &["scene", "field", "method"];

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SidebarGroup {
    /// One of its `[[space]]`s; the application's id when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub space: Option<String>,
    pub by: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub other: Option<Text>,
}

impl Inline {
    /// `[inline]` as the manifest has it: every point named, every field known.
    pub fn read(value: &toml::Value) -> Result<Self, String> {
        let table = value
            .as_table()
            .ok_or_else(|| format!("{MANIFEST}: `inline` is a table"))?;
        for key in table.keys() {
            if !POINTS.contains(&key.as_str()) {
                return Err(format!(
                    "{MANIFEST}: unknown inline point `{key}` (an application may declare: {})",
                    POINTS.join(", ")
                ));
            }
        }
        // Through JSON: a button's `params` is any table, kept as the JSON it is sent as.
        serde_json::from_value(toml_to_json(value))
            .map_err(|e| format!("{MANIFEST}: in `[inline]`: {}", e.to_string().trim()))
    }

    pub fn is_empty(&self) -> bool {
        self == &Inline::default()
    }

    /// The method `app.<id>.<name>` a device may call, by its short name.
    fn device_method(manifest: &Manifest, name: &str) -> bool {
        manifest
            .methods
            .iter()
            .any(|m| m.name == name && m.subject.names().contains(&"device"))
    }

    /// Checked against the manifest it is part of.
    pub fn check(&self, manifest: &Manifest) -> Result<(), String> {
        let id = &manifest.app.id;
        let refuse = |what: String| Err(format!("{MANIFEST}: {what}"));
        if self.tool_row.len() > 32
            || self.flow_block.len() > 16
            || self.command.len() > 8
            || self.sidebar_group.len() > 8
        {
            return refuse("`[inline]` declares more than a manifest may (32 tool rows, 16 blocks, 8 commands, 8 groupings)".into());
        }
        if !self.tool_row.is_empty() && manifest.component.is_none() {
            return refuse("a `tool_row` draws a call of one of its component's tools, and it has no component".into());
        }
        let mut seen = BTreeSet::new();
        for row in &self.tool_row {
            let at = format!("the tool row for `{}`", row.tool);
            if !tool_name(&row.tool) || !seen.insert(format!("tool {}", row.tool)) {
                return refuse(format!(
                    "{at}: `tool` is one of its tools' names (letters, digits, `_`, `-`), once"
                ));
            }
            icon(row.icon.as_deref(), &at)?;
            if let Some(title) = &row.title {
                text(title, &at, "title", MAX_TEXT, &["input", "result"])?;
            }
            if let Some(summary) = &row.summary {
                let ok = summary == "result" || summary.strip_prefix("input.").is_some_and(path);
                if !ok {
                    return refuse(format!(
                        "{at}: `summary` is `result` or `input.<path>`, not `{summary}`"
                    ));
                }
            }
        }
        if !self.flow_block.is_empty() && !manifest.capabilities.hub.iter().any(|h| h == APPEND) {
            return refuse(format!(
                "a `flow_block` is drawn from the entries the application appends, and its `hub` does not list `{APPEND}`"
            ));
        }
        for block in &self.flow_block {
            let at = format!("the block `{}`", block.kind);
            if !dotted(&block.kind) || !seen.insert(format!("block {}", block.kind)) {
                return refuse(format!(
                    "{at}: `kind` is a unique dot-separated name of letters, digits, `_` and `-`"
                ));
            }
            icon(block.icon.as_deref(), &at)?;
            text(&block.title, &at, "title", MAX_TEXT, &["payload"])?;
            if let Some(body) = &block.body {
                text(body, &at, "body", MAX_BODY, &["payload"])?;
            }
            if block.field.len() > 12 || block.button.len() > 4 {
                return refuse(format!("{at}: a card has at most 12 fields and 4 buttons"));
            }
            for field in &block.field {
                text(&field.label, &at, "a field's label", MAX_TEXT, &["payload"])?;
                template(&field.value, &at, "a field's value", MAX_TEXT, &["payload"])?;
            }
            for button in &block.button {
                text(
                    &button.label,
                    &at,
                    "a button's label",
                    MAX_TEXT,
                    &["payload"],
                )?;
                if !Self::device_method(manifest, &button.method) {
                    return refuse(format!(
                        "{at}: its button calls `{}`, which is not one of the application's `[[method]]`s a device may call",
                        button.method
                    ));
                }
                if let Some(params) = &button.params {
                    if !params.is_object() {
                        return refuse(format!("{at}: a button's `params` is a table"));
                    }
                    strings_in(params, &mut |s| {
                        template(s, &at, "a button's params", MAX_BODY, &["payload"])
                    })?;
                }
                if let Some(confirm) = &button.confirm {
                    text(confirm, &at, "a button's confirm", MAX_TEXT, &["payload"])?;
                }
                if let Some(done) = &button.done {
                    text(
                        done,
                        &at,
                        "a button's done",
                        MAX_TEXT,
                        &["payload", "result"],
                    )?;
                }
            }
        }
        for command in &self.command {
            let at = format!("the command `{}{}`", command.trigger, command.name);
            if command.trigger != "/" && command.trigger != "@" {
                return refuse(format!("{at}: `trigger` is \"/\" or \"@\""));
            }
            if !crate::manifest::valid_name(&command.name)
                || !seen.insert(format!("command {}{}", command.trigger, command.name))
            {
                return refuse(format!(
                    "{at}: `name` is lowercase letters, digits and `-`, once per trigger"
                ));
            }
            if let Some(description) = &command.description {
                text(description, &at, "description", MAX_TEXT, &[])?;
            }
            if !Self::device_method(manifest, COMPLETE) {
                return refuse(format!(
                    "{at}: its candidates come from `app.{id}.{COMPLETE}`, and the manifest declares no `[[method]]` `{COMPLETE}` a device may call"
                ));
            }
        }
        for group in &self.sidebar_group {
            let space = group.space.as_deref().unwrap_or(id);
            let at = format!("the grouping of `{space}`");
            let declared = space == id.as_str() || manifest.spaces.iter().any(|s| s.id == space);
            if !declared || !seen.insert(format!("grouping {space}")) {
                return refuse(format!("{at}: `space` is one of the application's `[[space]]`s (or its id), grouped once"));
            }
            // A space's left column is the shell's session list (which this groups) or the
            // application's own `space.sidebar` frame in its place: never both.
            if manifest
                .surfaces
                .iter()
                .any(|s| s.point == "space.sidebar" && s.space.as_deref() == Some(space))
            {
                return refuse(format!(
                    "{at}: the space `{space}` has a `space.sidebar` surface, which draws its left column in place of the session list a grouping groups; a space has one or the other"
                ));
            }
            if !GROUP_BY.contains(&group.by.as_str()) {
                return refuse(format!(
                    "{at}: `by` is one of {}, not `{}`",
                    GROUP_BY.join(", "),
                    group.by
                ));
            }
            match (group.by.as_str(), &group.field, &group.method) {
                ("scene", None, None) => {}
                ("field", Some(field), None) if path(field) => {}
                ("method", None, Some(method)) => {
                    if !Self::device_method(manifest, method) {
                        return refuse(format!(
                            "{at}: its groups come from `{method}`, which is not one of the application's `[[method]]`s a device may call"
                        ));
                    }
                }
                _ => {
                    return refuse(format!(
                        "{at}: `by = \"field\"` takes a `field` (the session row's key), `by = \"method\"` a `method`, and `scene` neither"
                    ))
                }
            }
            if let Some(other) = &group.other {
                text(other, &at, "other", MAX_TEXT, &[])?;
            }
        }
        Ok(())
    }

    /// The tool rows naming a tool the component does not list: what a start refuses.
    pub fn unknown_tools<'a>(&'a self, tools: &[String]) -> Vec<&'a str> {
        self.tool_row
            .iter()
            .filter(|row| !tools.contains(&row.tool))
            .map(|row| row.tool.as_str())
            .collect()
    }

    /// What a person installing it is told it will be told: what is typed for its commands, and its
    /// space's sessions for a grouping it computes. A line each, as `disclosure::lines` has them.
    pub fn disclosed(&self, id: &str) -> Vec<String> {
        let mut out: Vec<String> = self
            .command
            .iter()
            .map(|c| format!("command {}{}", c.trigger, c.name))
            .collect();
        out.extend(
            self.sidebar_group
                .iter()
                .filter(|g| g.by == "method")
                .map(|g| format!("sidebar.group {}", g.space.as_deref().unwrap_or(id))),
        );
        out
    }
}

fn tool_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 48
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

fn dotted(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name.split('.').all(|s| {
            !s.is_empty()
                && s.chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-'))
        })
}

/// `a.b.c`: a path into a JSON value.
fn path(p: &str) -> bool {
    dotted(p)
}

/// A shell glyph: one of its names. An application's own icons need code of its own to register,
/// which a manifest has none of.
fn icon(name: Option<&str>, at: &str) -> Result<(), String> {
    match name {
        Some(n) if n.is_empty() || n.len() > 32 || !n.chars().all(|c| matches!(c, 'a'..='z' | '0'..='9' | '-')) => Err(format!(
            "{MANIFEST}: {at}: `icon` is the name of one of the shell's glyphs (lowercase letters, digits, `-`), not `{n}`"
        )),
        _ => Ok(()),
    }
}

fn text(t: &Text, at: &str, what: &str, max: usize, roots: &[&str]) -> Result<(), String> {
    if let Text::Locales(m) = t {
        if m.is_empty() {
            return Err(format!(
                "{MANIFEST}: {at}: `{what}` is a string, or one per locale"
            ));
        }
        if let Some(locale) = m.keys().find(|k| !LOCALES.contains(&k.as_str())) {
            return Err(format!(
                "{MANIFEST}: {at}: `{what}` has the locale `{locale}`, and the shell's are {}",
                LOCALES.join(", ")
            ));
        }
    }
    for s in t.strings() {
        template(s, at, what, max, roots)?;
    }
    Ok(())
}

/// A text with `{root.path}` placeholders, each of a root this place has.
fn template(s: &str, at: &str, what: &str, max: usize, roots: &[&str]) -> Result<(), String> {
    if s.chars().count() > max {
        return Err(format!(
            "{MANIFEST}: {at}: `{what}` is at most {max} characters"
        ));
    }
    let mut rest = s;
    while let Some(open) = rest.find(['{', '}']) {
        if rest[open..].starts_with('}') {
            return Err(format!(
                "{MANIFEST}: {at}: `{what}` has a `}}` that closes nothing"
            ));
        }
        let after = &rest[open + 1..];
        let Some(close) = after.find('}') else {
            return Err(format!(
                "{MANIFEST}: {at}: `{what}` has a `{{` that is not closed"
            ));
        };
        let inner = &after[..close];
        let root = inner.split('.').next().unwrap_or_default();
        if !roots.contains(&root) || !path(inner) {
            let allowed = if roots.is_empty() {
                "no placeholders".to_string()
            } else {
                roots
                    .iter()
                    .map(|r| format!("`{{{r}.…}}`"))
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            return Err(format!(
                "{MANIFEST}: {at}: `{{{inner}}}` in `{what}` is not one it has ({allowed})"
            ));
        }
        rest = &after[close + 1..];
    }
    Ok(())
}

fn strings_in(v: &Value, f: &mut dyn FnMut(&str) -> Result<(), String>) -> Result<(), String> {
    match v {
        Value::String(s) => f(s),
        Value::Array(a) => a.iter().try_for_each(|x| strings_in(x, f)),
        Value::Object(o) => o.values().try_for_each(|x| strings_in(x, f)),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: &str = r#"
[app]
id = "acme.notes"
version = "1.0.0"
harnix_api = 1
publisher = "acme"

[capabilities]
hub = ["harnix.entries.append"]

[[component]]
name = "backend"
world = "harnix:app/backend-app@0.1.0"

[[method]]
name = "complete"

[[method]]
name = "notes.add"

[[method]]
name = "inside"
subject = "module"

[[space]]
id = "acme.notes"

[[space]]
id = "acme.notes.archive"
"#;

    const INLINE: &str = r#"
[[inline.tool_row]]
tool = "search"
icon = "search"
title = { en = "Search · {input.query}", zh-CN = "搜索 · {input.query}" }
summary = "input.query"
collapsed = false

[[inline.flow_block]]
kind = "note.card"
title = "{payload.title}"
body = "**{payload.text}**"
[[inline.flow_block.field]]
label = "By"
value = "{payload.by}"
[[inline.flow_block.button]]
label = "Pin it"
method = "notes.add"
params = { text = "{payload.text}", n = 1 }
confirm = "Pin it?"
done = "Pinned: {result.count}"

[[inline.command]]
trigger = "/"
name = "notes"

[[inline.command]]
trigger = "@"
name = "notes"

[[inline.sidebar_group]]
by = "method"
method = "notes.add"

[[inline.sidebar_group]]
space = "acme.notes.archive"
by = "field"
field = "status"
other = { en = "Other", zh-CN = "其他" }
"#;

    fn parse(text: &str) -> Result<Manifest, String> {
        Manifest::parse(text)
    }

    #[test]
    fn every_point_reads_and_is_kept_for_the_shell() {
        let m = parse(&format!("{BASE}{INLINE}")).unwrap();
        let inline = &m.inline;
        assert_eq!(inline.tool_row[0].tool, "search");
        assert!(!inline.tool_row[0].collapsed);
        assert_eq!(
            inline.flow_block[0].button[0].params.as_ref().unwrap()["n"],
            serde_json::json!(1)
        );
        assert_eq!(inline.command.len(), 2);
        assert_eq!(
            inline.sidebar_group[1].space.as_deref(),
            Some("acme.notes.archive")
        );
        let json = serde_json::to_value(inline).unwrap();
        assert_eq!(
            json["tool_row"][0]["title"]["zh-CN"],
            serde_json::json!("搜索 · {input.query}")
        );
        assert_eq!(
            json["flow_block"][0]["kind"],
            serde_json::json!("note.card")
        );
        assert_eq!(
            inline.disclosed("acme.notes"),
            [
                "command /notes",
                "command @notes",
                "sidebar.group acme.notes"
            ]
        );
        assert_eq!(inline.unknown_tools(&["other".into()]), ["search"]);
        assert!(parse(BASE).unwrap().inline.is_empty());
    }

    #[test]
    fn the_showcase_declares_one_of_each() {
        let m = parse(include_str!(
            "../../../examples/apps/showcase/harnix-app.toml"
        ))
        .unwrap();
        let inline = &m.inline;
        assert_eq!(
            (
                inline.tool_row.len(),
                inline.flow_block.len(),
                inline.command.len(),
                inline.sidebar_group.len()
            ),
            (2, 1, 2, 1)
        );
        assert_eq!(
            inline.disclosed(&m.app.id),
            [
                "command /showcase",
                "command @notes",
                "sidebar.group example.showcase.chats"
            ]
        );
    }

    #[test]
    fn what_is_not_known_or_does_not_hold_together_is_refused_by_name() {
        let cases: Vec<(String, &str)> = vec![
            (
                INLINE.replace(
                    "[[inline.command]]\ntrigger = \"/\"",
                    "[[inline.menu]]\ntrigger = \"/\"",
                ),
                "unknown inline point `menu`",
            ),
            (
                INLINE.replace("collapsed = false", "collapsed = false\ncolour = \"red\""),
                "unknown field `colour`",
            ),
            (
                INLINE.replace("label = \"By\"", "label = \"By\"\nwidth = 3"),
                "unknown field `width`",
            ),
            (
                INLINE.replace("{input.query}\", zh-CN", "{session.id}\", zh-CN"),
                "`{session.id}`",
            ),
            (
                INLINE.replace("**{payload.text}**", "{input.text}"),
                "`{input.text}`",
            ),
            (
                INLINE.replace("**{payload.text}**", "{payload.text"),
                "not closed",
            ),
            (
                INLINE.replace("zh-CN = \"搜索", "fr = \"搜索"),
                "the locale `fr`",
            ),
            (
                INLINE.replace("summary = \"input.query\"", "summary = \"output\""),
                "`summary`",
            ),
            (
                INLINE.replace("icon = \"search\"", "icon = \"acme.logo\""),
                "`icon`",
            ),
            (
                INLINE.replace(
                    "method = \"notes.add\"\nparams",
                    "method = \"notes.remove\"\nparams",
                ),
                "`notes.remove`",
            ),
            (
                INLINE.replace(
                    "method = \"notes.add\"\nparams",
                    "method = \"inside\"\nparams",
                ),
                "`inside`",
            ),
            (
                INLINE.replace("trigger = \"@\"", "trigger = \"#\""),
                "`trigger`",
            ),
            (
                INLINE.replace("trigger = \"@\"", "trigger = \"/\""),
                "once per trigger",
            ),
            (
                INLINE.replace("space = \"acme.notes.archive\"", "space = \"other.space\""),
                "`space`",
            ),
            (INLINE.replace("by = \"field\"", "by = \"tag\""), "`by`"),
            (
                INLINE.replace("field = \"status\"", "method = \"notes.add\""),
                "takes a `field`",
            ),
            (
                INLINE.replace("kind = \"note.card\"", "kind = \"note card\""),
                "`kind`",
            ),
        ];
        for (inline, says) in cases {
            let err = parse(&format!("{BASE}{inline}"))
                .err()
                .unwrap_or_else(|| panic!("accepted: {says}"));
            assert!(err.contains(says), "wanted `{says}`, got: {err}");
            assert!(err.starts_with(MANIFEST), "{err}");
        }
        // A card needs its entries appended; a command, its candidates.
        let no_append = BASE.replace("hub = [\"harnix.entries.append\"]", "hub = []");
        assert!(parse(&format!("{no_append}{INLINE}"))
            .unwrap_err()
            .contains(APPEND));
        let no_complete = BASE.replace("name = \"complete\"", "name = \"other\"");
        assert!(parse(&format!("{no_complete}{INLINE}"))
            .unwrap_err()
            .contains("`complete`"));
    }

    #[test]
    fn a_space_is_grouped_or_has_its_own_sidebar_never_both() {
        let frames = r#"
[[surface]]
point = "space"
space = "acme.notes"
page = "board.html"

[[surface]]
point = "space.sidebar"
space = "acme.notes"
page = "side.html"
"#;
        let err = parse(&format!("{BASE}{INLINE}{frames}")).unwrap_err();
        assert!(
            err.contains("the grouping of `acme.notes`") && err.contains("`space.sidebar`"),
            "{err}"
        );
        // Its own sidebar in one space, and the shell's list grouped in another: both are drawn.
        let elsewhere = INLINE.replace(
            "[[inline.sidebar_group]]\nby = \"method\"\nmethod = \"notes.add\"\n",
            "",
        );
        let m = parse(&format!("{BASE}{elsewhere}{frames}")).unwrap();
        assert_eq!(m.inline.sidebar_group.len(), 1);
    }
}
