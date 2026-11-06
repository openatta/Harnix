//! The parts of a flow pack besides flows and skills (xops-04): rules, policies with their
//! permissions, gates, hooks.
//!
//! ```text
//! rules/<name>.md        prose with a name: what must hold across the flow — fed to sessions, never enforced
//! policies/<name>.yaml   a named set of permissions, referenced by flows and stations
//! gates/<name>.yaml      a named deterministic check, referenced by `gate` exits
//! hooks/<name>.yaml      tells something outside about a run's progress — no flow names it
//! ```
//!
//! Each point in time has one place that can stop work: while a task works, its **permissions**
//! (each tool call is judged `deny` > `ask` > `allow`, [`judge`]); when its station is judged,
//! its exit conditions, among them **gates**. Skills and rules are only ever read.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::def::{Assert, FlowDef, Station};

/// The kinds of part a pack holds besides flows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PartKind {
    Skill,
    Rule,
    Policy,
    Gate,
    Hook,
}

impl PartKind {
    pub const ALL: [PartKind; 5] = [Self::Skill, Self::Rule, Self::Policy, Self::Gate, Self::Hook];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Skill => "skill",
            Self::Rule => "rule",
            Self::Policy => "policy",
            Self::Gate => "gate",
            Self::Hook => "hook",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.as_str() == s)
    }

    /// The pack directory of this kind.
    pub fn dir(self) -> &'static str {
        match self {
            Self::Skill => "skills",
            Self::Rule => "rules",
            Self::Policy => "policies",
            Self::Gate => "gates",
            Self::Hook => "hooks",
        }
    }

    /// Where a part of this kind lives under its kind's directory.
    pub fn file(self, name: &str) -> String {
        match self {
            Self::Skill => format!("{name}/SKILL.md"),
            Self::Rule => format!("{name}.md"),
            Self::Policy | Self::Gate | Self::Hook => format!("{name}.yaml"),
        }
    }
}

impl std::fmt::Display for PartKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

// ── rules ────────────────────────────────────────────────────────────────────

/// A rule's title (its first `# ` heading; the name without one) and its body (the rest).
pub fn rule_parts(name: &str, text: &str) -> (String, String) {
    let t = text.trim();
    match t.strip_prefix("# ") {
        Some(rest) => {
            let (title, body) = rest.split_once('\n').unwrap_or((rest, ""));
            (title.trim().to_string(), body.trim().to_string())
        }
        None => (name.to_string(), t.to_string()),
    }
}

// ── permissions ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Effect {
    Allow,
    Ask,
    Deny,
}

impl Effect {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::Ask => "ask",
            Self::Deny => "deny",
        }
    }
}

/// One permission: which tool calls it is about, and what becomes of them.
///
/// It matches a call when the tool is one of `tools` and every scope it writes matches:
/// `paths` the call's path (`file_path`, `path` or `notebook_path`; a relative pattern is taken
/// under the run's workspace), `commands` its `command`, `args` each named top-level field of its
/// input. Patterns are globs (`*` within a path segment, `**` across them; in commands and args
/// `*` is anything) and may hold references (`${run}`, `${inputs.k}`, `${item…}`, `${workspace}`),
/// resolved when the task is dispatched.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Permission {
    pub effect: Effect,
    /// Tool names, or `ns__*` for every tool of a namespace.
    pub tools: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub paths: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub commands: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub args: BTreeMap<String, Vec<String>>,
    /// Said to the session when a call is refused, and to the person asked.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub message: String,
}

impl Permission {
    /// Whether it needs the call's input to be judged (beyond the tool's name).
    pub fn scoped(&self) -> bool {
        !self.paths.is_empty() || !self.commands.is_empty() || !self.args.is_empty()
    }

    /// Every pattern it holds, for reference checks.
    pub fn patterns(&self) -> impl Iterator<Item = &String> {
        self.paths
            .iter()
            .chain(self.commands.iter())
            .chain(self.args.values().flatten())
    }

    /// The same permission with each pattern passed through `f` (reference resolution).
    pub fn map_patterns(&self, f: impl Fn(&str) -> String) -> Self {
        Self {
            paths: self.paths.iter().map(|p| f(p)).collect(),
            commands: self.commands.iter().map(|p| f(p)).collect(),
            args: self
                .args
                .iter()
                .map(|(k, v)| (k.clone(), v.iter().map(|p| f(p)).collect()))
                .collect(),
            ..self.clone()
        }
    }
}

/// A policy: a named set of permissions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PolicyDef {
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub title: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    pub permissions: Vec<Permission>,
}

/// A permission with where it came from, as a task holds it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Granted {
    #[serde(flatten)]
    pub permission: Permission,
    /// `policy <name>`, `flow` or `station <id>`.
    pub from: String,
}

/// What a station's tasks work under, references unresolved, in order: the flow's policies, the
/// flow's own permissions, the station's policies, the station's own. A policy the parts do not
/// hold contributes nothing (validation reports it).
pub fn permissions_of(def: &FlowDef, st: &Station, policies: &BTreeMap<String, PolicyDef>) -> Vec<Granted> {
    let mut out = vec![];
    let add_policies = |names: &[String], out: &mut Vec<Granted>| {
        for n in names {
            if let Some(p) = policies.get(n) {
                out.extend(p.permissions.iter().map(|x| Granted {
                    permission: x.clone(),
                    from: format!("policy {n}"),
                }));
            }
        }
    };
    add_policies(&def.policies, &mut out);
    out.extend(def.permissions.iter().map(|x| Granted {
        permission: x.clone(),
        from: "flow".into(),
    }));
    add_policies(&st.policies, &mut out);
    out.extend(st.permissions.iter().map(|x| Granted {
        permission: x.clone(),
        from: format!("station {}", st.id),
    }));
    out
}

/// The rules a station's tasks work under: the flow's, then the station's, each once.
pub fn rules_of(def: &FlowDef, st: &Station) -> Vec<String> {
    let mut out: Vec<String> = vec![];
    for r in def.rules.iter().chain(st.rules.iter()) {
        if !out.contains(r) {
            out.push(r.clone());
        }
    }
    out
}

/// Every tool any permission of the flow names, with the guarded ones: what a flow session
/// asks about (`ask` permission rules), so each call comes to be judged.
pub fn asked_tools(def: &FlowDef, policies: &BTreeMap<String, PolicyDef>, guarded: &[String]) -> Vec<String> {
    let mut out: Vec<String> = guarded.to_vec();
    for st in &def.stations {
        for g in permissions_of(def, st, policies) {
            for t in &g.permission.tools {
                if !out.contains(t) {
                    out.push(t.clone());
                }
            }
        }
    }
    out
}

/// A tool call to judge: its name, its input when known, the paths the engine says it touches.
pub struct Call<'a> {
    pub tool: &'a str,
    pub input: Option<&'a Value>,
    pub paths: &'a [String],
    /// Where a relative path pattern is taken from: the run's workspace.
    pub workspace: Option<&'a str>,
}

/// What becomes of a call.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Verdict {
    pub effect: Effect,
    /// Why, for the session (refused) or the person (asked).
    pub reason: String,
}

/// Judge a call against a task's permissions: of those that match, `deny` > `ask` > `allow`;
/// none matching, a guarded tool is refused and any other allowed. A scoped permission whose
/// scope cannot be read from the call (no input) is taken to match when it denies or asks and
/// not to when it allows — it fails closed.
pub fn judge(granted: &[Granted], guarded: &[String], call: &Call, station: &str) -> Verdict {
    let mut best: Option<&Granted> = None;
    for g in granted {
        let p = &g.permission;
        if !tool_matches(&p.tools, call.tool) {
            continue;
        }
        let hit = match scope_matches(p, call) {
            Some(m) => m,
            None => p.effect != Effect::Allow,
        };
        if hit && best.is_none_or(|b| p.effect > b.permission.effect) {
            best = Some(g);
        }
    }
    match best {
        Some(g) => {
            let p = &g.permission;
            let reason = if !p.message.is_empty() {
                p.message.clone()
            } else {
                match p.effect {
                    Effect::Allow => String::new(),
                    Effect::Ask => format!("station {station} of the flow asks a person before {} ({})", call.tool, g.from),
                    Effect::Deny => format!("station {station} of the flow does not allow {} ({})", call.tool, g.from),
                }
            };
            Verdict { effect: p.effect, reason }
        }
        None if tool_matches(guarded, call.tool) => Verdict {
            effect: Effect::Deny,
            reason: format!(
                "station {station} of the flow does not allow {}: no permission of it allows this tool",
                call.tool
            ),
        },
        None => Verdict {
            effect: Effect::Allow,
            reason: String::new(),
        },
    }
}

/// Whether a tool name is one of `patterns` (a name, or `ns__*`).
pub fn tool_matches(patterns: &[String], tool: &str) -> bool {
    patterns.iter().any(|p| match p.strip_suffix('*') {
        Some(prefix) => tool.starts_with(prefix),
        None => p == tool,
    })
}

/// Whether the call is inside the permission's scope; `None` when the call does not say
/// (a scope that needs the input, and there is none).
fn scope_matches(p: &Permission, call: &Call) -> Option<bool> {
    if !p.scoped() {
        return Some(true);
    }
    let field = |k: &str| call.input.and_then(|i| i.get(k)).and_then(Value::as_str);
    if !p.paths.is_empty() {
        let mut paths: Vec<String> = call.paths.to_vec();
        for k in ["file_path", "path", "notebook_path"] {
            if let Some(v) = field(k) {
                paths.push(v.to_string());
            }
        }
        if paths.is_empty() {
            return None;
        }
        let pats: Vec<String> = p
            .paths
            .iter()
            .map(|pat| match (pat.starts_with('/'), call.workspace) {
                (false, Some(ws)) => format!("{}/{pat}", ws.trim_end_matches('/')),
                _ => pat.clone(),
            })
            .collect();
        if !paths.iter().all(|path| pats.iter().any(|pat| glob(pat, path, true))) {
            return Some(false);
        }
    }
    if !p.commands.is_empty() {
        call.input?;
        let Some(c) = field("command") else {
            return Some(false);
        };
        if !p.commands.iter().any(|pat| glob(pat, c.trim(), false)) {
            return Some(false);
        }
    }
    for (k, pats) in &p.args {
        call.input?;
        let v = match call.input.and_then(|i| i.get(k)) {
            Some(Value::String(s)) => s.clone(),
            Some(Value::Null) | None => return Some(false),
            Some(v) => v.to_string(),
        };
        if !pats.iter().any(|pat| glob(pat, &v, false)) {
            return Some(false);
        }
    }
    Some(true)
}

/// A glob match. `?` is one character; in path mode `*` stays within a segment and `**` crosses
/// them, otherwise `*` is anything.
pub fn glob(pattern: &str, text: &str, path: bool) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    fn go(p: &[char], t: &[char], path: bool) -> bool {
        match p.first() {
            None => t.is_empty(),
            Some('*') => {
                let double = p.get(1) == Some(&'*');
                let rest = if double { &p[2..] } else { &p[1..] };
                // `**/` also matches nothing at all.
                if double && rest.first() == Some(&'/') && go(&rest[1..], t, path) {
                    return true;
                }
                let any = double || !path;
                for i in 0..=t.len() {
                    if go(rest, &t[i..], path) {
                        return true;
                    }
                    if i < t.len() && !any && t[i] == '/' {
                        return false;
                    }
                }
                false
            }
            Some('?') => !t.is_empty() && (!path || t[0] != '/') && go(&p[1..], &t[1..], path),
            Some(c) => t.first() == Some(c) && go(&p[1..], &t[1..], path),
        }
    }
    go(&p, &t, path)
}

// ── gates ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum GateKind {
    /// Call a read-only, idempotent application tool and assert on its result; judged on the spot.
    Tool,
    /// Run a command where the run works; passes on exit status 0. Only where the application
    /// turned command gates on.
    Command,
    /// The application's own check (`Gates`), by the gate's name.
    Host,
}

/// The longest a command gate may run.
pub const MAX_GATE_SECONDS: u64 = 3600;
/// How long a command gate runs when it does not say.
pub const DEFAULT_GATE_SECONDS: u64 = 600;

/// A gate: a named deterministic check a `gate` exit refers to.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GateDef {
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub title: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    pub kind: GateKind,
    /// `tool`: the tool.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
    /// `tool`: its arguments (references resolved against the run).
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub args: Value,
    /// `tool`: what must hold of its result.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub assert: Vec<Assert>,
    /// `command`: the command line, run by `sh -c`. It holds no references: the run reaches it in
    /// the environment — `FLOW_RUN`, `FLOW_STATION`, `FLOW_WORKSPACE`, `FLOW_CONTEXT` (JSON).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    /// `command`: where it runs; the run's workspace when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    /// `command`: how long it may run (default 600, at most 3600).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_seconds: Option<u64>,
}

impl GateDef {
    pub fn timeout_seconds(&self) -> u64 {
        self.timeout_seconds
            .unwrap_or(DEFAULT_GATE_SECONDS)
            .clamp(1, MAX_GATE_SECONDS)
    }

    /// A name for people: the title, else the name.
    pub fn label(&self) -> &str {
        if self.title.is_empty() {
            &self.name
        } else {
            &self.title
        }
    }
}

// ── hooks ────────────────────────────────────────────────────────────────────

/// How a hook tells.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum HookKind {
    /// `POST` the event as JSON to the URL in `action`.
    Webhook,
}

/// A hook: when an event of a run happens, tell something outside. No flow names a hook; one
/// put in the pack applies at once, and blocks nothing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HookDef {
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub title: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    /// Event kinds, or `prefix.*`; absent: every event.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub on: Vec<String>,
    /// Only runs of these flows; absent: every flow.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub flows: Vec<String>,
    pub kind: HookKind,
    /// `webhook`: the URL.
    pub action: String,
    /// `webhook`: an environment variable holding a secret; given, each delivery is signed
    /// (`X-Harnix-Signature: sha256=<HMAC of the body>`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret_env: Option<String>,
}

impl HookDef {
    /// Whether an event of a run of `flow` is this hook's.
    pub fn wants(&self, kind: &str, flow: &str) -> bool {
        let on = self.on.is_empty()
            || self.on.iter().any(|o| match o.strip_suffix('*') {
                Some(prefix) => kind.starts_with(prefix),
                None => o == kind,
            });
        on && (self.flows.is_empty() || self.flows.iter().any(|f| f == flow))
    }
}

// ── what a run pins ──────────────────────────────────────────────────────────

/// The policies and gates a run's definition refers to, pinned with it when the run is created:
/// judging a call or a station reads only what the run holds. (Skills and rules, read only when
/// a task is sent, are pinned by digest when their station is entered.)
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Pinned {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub policies: BTreeMap<String, PolicyDef>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub gates: BTreeMap<String, GateDef>,
}

impl Pinned {
    pub fn is_empty(&self) -> bool {
        self.policies.is_empty() && self.gates.is_empty()
    }
}

/// Whether `def` names a part.
pub fn uses(def: &FlowDef, kind: PartKind, name: &str) -> bool {
    let n = |v: &[String]| v.iter().any(|x| x == name);
    match kind {
        PartKind::Skill => def.stations.iter().any(|s| {
            s.skill.as_deref() == Some(name)
                || s.tasks.iter().flatten().any(|t| t.skill.as_deref() == Some(name))
        }),
        PartKind::Rule => n(&def.rules) || def.stations.iter().any(|s| n(&s.rules)),
        PartKind::Policy => n(&def.policies) || def.stations.iter().any(|s| n(&s.policies)),
        PartKind::Gate => def.stations.iter().any(|s| {
            s.exit
                .iter()
                .any(|e| matches!(e, super::def::ExitCond::Gate { gate, .. } if gate == name))
        }),
        PartKind::Hook => false,
    }
}

/// What a run of `def` pins: every policy and gate it names that the packs have.
pub fn pin(def: &FlowDef, packs: &dyn super::host::Packs) -> Pinned {
    let mut out = Pinned::default();
    let stations = def.stations.iter();
    for n in def.policies.iter().chain(stations.clone().flat_map(|s| s.policies.iter())) {
        if let Some(p) = packs.policy(n) {
            out.policies.insert(n.clone(), p);
        }
    }
    for s in stations {
        for e in &s.exit {
            if let super::def::ExitCond::Gate { gate, .. } = e {
                if let Some(g) = packs.gate(gate) {
                    out.gates.insert(gate.clone(), g);
                }
            }
        }
    }
    out
}

/// The JSON Schema of a part file of `kind` (`None` for skills and rules, which are prose).
pub fn part_schema(kind: PartKind) -> Option<Value> {
    let v = match kind {
        PartKind::Policy => serde_json::to_value(schemars::schema_for!(PolicyDef)),
        PartKind::Gate => serde_json::to_value(schemars::schema_for!(GateDef)),
        PartKind::Hook => serde_json::to_value(schemars::schema_for!(HookDef)),
        PartKind::Skill | PartKind::Rule => return None,
    };
    v.ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn perm(yaml: &str) -> Granted {
        Granted {
            permission: serde_yaml::from_str(yaml).unwrap(),
            from: "flow".into(),
        }
    }

    fn call<'a>(tool: &'a str, input: Option<&'a Value>) -> Call<'a> {
        Call {
            tool,
            input,
            paths: &[],
            workspace: Some("/ws/r-1"),
        }
    }

    #[test]
    fn globs_keep_to_segments_in_paths() {
        assert!(glob("/a/*.md", "/a/x.md", true));
        assert!(!glob("/a/*.md", "/a/b/x.md", true));
        assert!(glob("/a/**/x.md", "/a/x.md", true));
        assert!(glob("/a/**", "/a/b/c", true));
        assert!(glob("git push*", "git push --force", false));
        assert!(glob("order_*", "order_lines", false));
        assert!(!glob("order_*", "orders", false));
    }

    #[test]
    fn deny_beats_ask_beats_allow() {
        let g = vec![
            perm("{effect: allow, tools: [Write]}"),
            perm("{effect: ask, tools: [Write], paths: [release/**], message: needs a nod}"),
            perm("{effect: deny, tools: [Bash], commands: ['git push*']}"),
        ];
        let w = json!({"file_path": "/ws/r-1/src/a.rs"});
        assert_eq!(judge(&g, &[], &call("Write", Some(&w)), "s").effect, Effect::Allow);
        let r = json!({"file_path": "/ws/r-1/release/notes.md"});
        let v = judge(&g, &[], &call("Write", Some(&r)), "s");
        assert_eq!((v.effect, v.reason.as_str()), (Effect::Ask, "needs a nod"));
        let push = json!({"command": "git push origin main"});
        assert_eq!(judge(&g, &[], &call("Bash", Some(&push)), "s").effect, Effect::Deny);
        let ls = json!({"command": "ls"});
        assert_eq!(judge(&g, &[], &call("Bash", Some(&ls)), "s").effect, Effect::Allow);
    }

    #[test]
    fn a_guarded_tool_nothing_allows_is_refused_and_others_pass() {
        let guarded = vec!["Bash".to_string(), "mcp__desk__*".to_string()];
        let g = vec![perm("{effect: allow, tools: [mcp__desk__emit_row], args: {table: [orders, 'order_*']}}")];
        let row = json!({"table": "order_lines"});
        assert_eq!(judge(&g, &guarded, &call("mcp__desk__emit_row", Some(&row)), "s").effect, Effect::Allow);
        let other = json!({"table": "users"});
        let v = judge(&g, &guarded, &call("mcp__desk__emit_row", Some(&other)), "emit");
        assert_eq!(v.effect, Effect::Deny);
        assert!(v.reason.contains("station emit"), "{}", v.reason);
        assert_eq!(judge(&g, &guarded, &call("mcp__desk__drop", None), "s").effect, Effect::Deny);
        assert_eq!(judge(&g, &guarded, &call("Read", None), "s").effect, Effect::Allow);
    }

    #[test]
    fn a_scope_the_call_does_not_show_fails_closed() {
        let allow = vec![perm("{effect: allow, tools: [Bash], commands: [ls*]}")];
        assert_eq!(judge(&allow, &["Bash".into()], &call("Bash", None), "s").effect, Effect::Deny);
        let deny = vec![
            perm("{effect: allow, tools: [Bash]}"),
            perm("{effect: deny, tools: [Bash], commands: [rm*]}"),
        ];
        assert_eq!(judge(&deny, &[], &call("Bash", None), "s").effect, Effect::Deny);
    }

    #[test]
    fn rule_title_is_its_heading() {
        assert_eq!(
            rule_parts("x", "# No secrets\n\nNever paste keys.\n"),
            ("No secrets".to_string(), "Never paste keys.".to_string())
        );
        assert_eq!(rule_parts("x", "Plain."), ("x".to_string(), "Plain.".to_string()));
    }

    #[test]
    fn hooks_filter_by_event_and_flow() {
        let h: HookDef = serde_yaml::from_str(
            "{name: h, on: [run.created, 'station.*'], flows: [a], kind: webhook, action: 'http://x'}",
        )
        .unwrap();
        assert!(h.wants("station.passed", "a"));
        assert!(!h.wants("task.ready", "a"));
        assert!(!h.wants("run.created", "b"));
    }
}
