//! What a session sees beyond its conversation: the skills it may use and the hooks its turns run,
//! each with where it comes from — what a session's context tab shows.
//!
//! Read from the engine as it says them (the session's command list, its listed skill directories,
//! its project's trust, the settings tiers' `hooks_config`) and from what this process registered
//! on its connection; the hub adds which package a directory is. A hook is never shown by what it
//! runs: a command, a URL, a header or a prompt can carry a credential, so a row says only its
//! event, kind and matcher.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use harnix_contract::sync::read;
use harnix_contract::wire::{self, HookRow, HookSource, SessionHooks, SessionSkills, ShadowedSkill, SkillDirRow, SkillRow, SkillTier};
use harnix_contract::{RpcError, SkillSource};
use harnix_engine_link::HostHook;
use serde_json::{json, Value};

use crate::Hub;

/// The most characters of a matcher a row carries.
const MATCHER_CHARS: usize = 80;

/// A project's own skills, under its root.
const PROJECT_SKILLS: &str = ".agents/skills";

/// The settings hook types the engine runs; any other is `other`.
const HOOK_KINDS: &[&str] = &["command", "prompt", "http", "agent"];

impl Hub {
    /// `harnix.session.skills {instance?, session_id}`.
    pub(crate) async fn session_skills(&self, params: Value) -> Result<Value, RpcError> {
        let key = self.key_of(&params)?;
        let linked = self.engine(&key.0)?;
        let ops = linked.ops();
        let info = ops.get(&key.1).await?;
        let session = info.value();
        let project = session.get("project_root").and_then(Value::as_str);
        let scene = session.get("scene").and_then(Value::as_str);
        let status = match project {
            Some(root) => ops.passthrough("engine.project.status", json!({"project_root": root, "scene": scene})).await.ok(),
            None => None,
        };
        // The engine lists a session's own commands only while it is in memory.
        let commands = match info.loaded() {
            true => ops.passthrough("engine.commands.list", json!({"session_id": key.1})).await.ok(),
            false => None,
        };
        let plugins = ops.passthrough("engine.plugin.list", json!({})).await.ok();
        let packages: Vec<SkillSource> = read(&self.skills).values().flatten().cloned().collect();
        let mut out = skills_of(session, commands.as_ref(), status.as_ref(), plugins.as_ref(), &packages);
        out.instance = key.0;
        out.session_id = key.1;
        Ok(wire::params(&out))
    }

    /// `harnix.session.hooks {instance?, session_id}`.
    pub(crate) async fn session_hooks(&self, params: Value) -> Result<Value, RpcError> {
        let key = self.key_of(&params)?;
        let linked = self.engine(&key.0)?;
        let ops = linked.ops();
        let info = ops.get(&key.1).await?;
        let session = info.value();
        let project = session.get("project_root").and_then(Value::as_str);
        let scene = session.get("scene").and_then(Value::as_str);
        let trust = match project {
            Some(root) => ops
                .passthrough("engine.project.status", json!({"project_root": root, "scene": scene}))
                .await
                .ok()
                .and_then(|s| s.get("trust").and_then(Value::as_str).map(str::to_string)),
            None => None,
        };
        let mut tiers = Vec::new();
        for (source, tier) in [(HookSource::Global, "global"), (HookSource::Scene, "scene"), (HookSource::Project, "project")] {
            if (source == HookSource::Scene && scene.is_none()) || (source == HookSource::Project && project.is_none()) {
                continue;
            }
            let ask = json!({"scene": scene, "project_root": project, "tier": tier});
            let settings = ops.passthrough("engine.config.get", ask).await.ok().map(|a| a.get("settings").cloned().unwrap_or(Value::Null));
            tiers.push((source, settings));
        }
        let (hooks, unread) = hooks_of(&tiers, trust.as_deref(), &linked.link.host_hooks());
        Ok(wire::params(&SessionHooks { instance: key.0, session_id: key.1, trust, hooks, unread }))
    }
}

/// The skills a session sees, from what the engine said of it: `session` (`engine.session.get`), its
/// commands (`engine.commands.list`, absent when it is not in memory), its project's status and the
/// engine's plugin packages; `packages` are the directories the host components published.
fn skills_of(session: &Value, commands: Option<&Value>, status: Option<&Value>, plugins: Option<&Value>, packages: &[SkillSource]) -> SessionSkills {
    let package_of = |path: &str| packages.iter().find(|s| s.dir == Path::new(path)).map(|s| s.ext.clone());
    let dirs: Vec<SkillDirRow> = session
        .get("skill_dirs")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|d| {
            let path = d.get("path")?.as_str()?.to_string();
            Some(SkillDirRow {
                package: package_of(&path),
                state: d.get("state").and_then(Value::as_str).map(str::to_string),
                skills: strings(d.get("skills")),
                shadowed: d
                    .get("shadowed")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|s| Some(ShadowedSkill { skill: s.get("skill")?.as_str()?.to_string(), by: s.get("by")?.as_str()?.to_string() }))
                    .collect(),
                path,
            })
        })
        .collect();
    let trust = status.and_then(|s| s.get("trust")).and_then(Value::as_str).map(str::to_string);
    let held: BTreeSet<String> = match trust.as_deref() {
        Some("trusted") => BTreeSet::new(),
        _ => strings(status.and_then(|s| s.get("requires_trust")))
            .into_iter()
            .filter_map(|item| item.strip_prefix("skills/").map(str::to_string))
            .collect(),
    };
    let project = session.get("project_root").and_then(Value::as_str);
    let plugins: Vec<&Value> = plugins
        .and_then(|p| p.get("plugins"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|p| p.get("enabled").and_then(Value::as_bool).unwrap_or(false))
        .collect();
    let mut skills = Vec::new();
    for command in commands.and_then(|c| c.get("commands")).and_then(Value::as_array).into_iter().flatten() {
        // Skills are the commands that expand into a prompt; an MCP server's prompts and the
        // engine's local commands are not.
        if command.get("kind").and_then(Value::as_str) != Some("prompt") {
            continue;
        }
        let Some(name) = command.get("name").and_then(Value::as_str) else { continue };
        let tier = match command.get("source").and_then(Value::as_str) {
            Some("host") => SkillTier::Space,
            Some("project") => SkillTier::Project,
            Some("plugin") => SkillTier::Plugin,
            Some("user") => SkillTier::User,
            Some("builtin") => SkillTier::Builtin,
            _ => continue,
        };
        let mut row = SkillRow {
            name: name.to_string(),
            description: command.get("description").and_then(Value::as_str).unwrap_or_default().to_string(),
            tier,
            dir: None,
            package: None,
            plugin: None,
            withheld: false,
        };
        match tier {
            SkillTier::Space => {
                if let Some(dir) = dirs.iter().find(|d| d.skills.iter().any(|s| s == name)) {
                    row.dir = Some(dir.path.clone());
                    row.package = dir.package.clone();
                }
            }
            SkillTier::Project => {
                row.dir = project.map(|root| Path::new(root).join(PROJECT_SKILLS).display().to_string());
                row.withheld = held.contains(name);
            }
            SkillTier::Plugin => {
                if let Some(plugin) = plugins.iter().find(|p| strings(p.get("skills")).iter().any(|s| s == name)) {
                    row.plugin = plugin.get("name").and_then(Value::as_str).map(str::to_string);
                    row.dir = plugin.get("root").and_then(Value::as_str).map(str::to_string);
                }
            }
            SkillTier::User | SkillTier::Builtin => {}
        }
        skills.push(row);
    }
    SessionSkills { instance: String::new(), session_id: String::new(), listed: commands.is_some(), trust, skills, dirs }
}

/// The hooks a session's turns run: each settings tier's (`tiers`, lowest first; `None` for one
/// that could not be read), then what this process registered. Of the settings tiers, the highest
/// that configures an event replaces the lower ones' list for it; a project's counts only when
/// the engine says the project is trusted.
fn hooks_of(tiers: &[(HookSource, Option<Value>)], trust: Option<&str>, host: &[HostHook]) -> (Vec<HookRow>, Vec<HookSource>) {
    let project_held = trust != Some("trusted");
    let mut rows = Vec::new();
    let mut unread = Vec::new();
    // The tier whose list each event takes.
    let mut winner: BTreeMap<String, HookSource> = BTreeMap::new();
    for (source, settings) in tiers {
        let Some(settings) = settings else {
            unread.push(*source);
            continue;
        };
        let withheld = *source == HookSource::Project && project_held;
        let Some(config) = settings.get("hooks_config").and_then(Value::as_object) else { continue };
        for (event, entries) in config {
            if !withheld {
                winner.insert(event.clone(), *source);
            }
            let entries: Vec<&Value> = match entries {
                Value::Array(list) => list.iter().collect(),
                one @ Value::Object(_) => vec![one],
                _ => Vec::new(),
            };
            for entry in entries {
                rows.push(settings_hook(event, *source, entry, withheld));
            }
        }
    }
    for row in &mut rows {
        if let Some(&by) = winner.get(&row.event).filter(|&&by| by != row.source && !row.withheld) {
            row.replaced_by = Some(by);
            row.in_force = false;
        }
    }
    for hook in host {
        let matcher = (!hook.tools.is_empty()).then(|| cut(&hook.tools.join(", ")));
        rows.push(HookRow {
            event: hook.event.clone(),
            source: HookSource::Host,
            kind: "host".into(),
            label: label("host", matcher.as_deref()),
            matcher,
            on_error: Some(hook.on_error.clone()),
            in_force: true,
            withheld: false,
            replaced_by: None,
            providers: hook.providers.clone(),
        });
    }
    (rows, unread)
}

/// One settings hook as a row: its type, its `if` and its `on_error`, and nothing it runs.
fn settings_hook(event: &str, source: HookSource, entry: &Value, withheld: bool) -> HookRow {
    let kind = match entry.get("type").and_then(Value::as_str) {
        Some(t) if HOOK_KINDS.contains(&t) => t,
        _ => "other",
    };
    let matcher = entry.get("if").and_then(Value::as_str).map(str::trim).filter(|m| !m.is_empty()).map(cut);
    let on_error = entry.get("on_error").and_then(Value::as_str).filter(|e| matches!(*e, "allow" | "block")).map(str::to_string);
    HookRow {
        event: event.to_string(),
        source,
        kind: kind.to_string(),
        label: label(kind, matcher.as_deref()),
        matcher,
        on_error,
        in_force: !withheld,
        withheld,
        replaced_by: None,
        providers: Vec::new(),
    }
}

fn label(kind: &str, matcher: Option<&str>) -> String {
    match matcher {
        Some(m) => format!("{kind} · {m}"),
        None => kind.to_string(),
    }
}

/// At most [`MATCHER_CHARS`] characters, an ellipsis where it was cut.
fn cut(text: &str) -> String {
    match text.char_indices().nth(MATCHER_CHARS) {
        Some((at, _)) => format!("{}…", &text[..at]),
        None => text.to_string(),
    }
}

fn strings(value: Option<&Value>) -> Vec<String> {
    value.and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(ext: &str) -> SkillSource {
        SkillSource::new(ext, format!("/state/ext/{ext}/skills"), Vec::new())
    }

    #[test]
    fn each_skill_says_its_tier_and_where_it_was_read() {
        let session = json!({
            "project_root": "/p",
            "skill_dirs": [{"path": "/state/ext/acme.notes/skills", "state": "ok", "skills": ["tour"], "shadowed": [{"skill": "x", "by": "user", "path": "/u"}]}],
        });
        let commands = json!({"commands": [
            {"name": "tour", "description": "a tour", "kind": "prompt", "source": "host"},
            {"name": "deploy", "description": "ship it", "kind": "prompt", "source": "project"},
            {"name": "lint", "description": "", "kind": "prompt", "source": "project"},
            {"name": "board", "description": "", "kind": "prompt", "source": "plugin"},
            {"name": "mine", "description": "", "kind": "prompt", "source": "user"},
            {"name": "clear", "description": "", "kind": "local", "source": "builtin"},
            {"name": "srv:ask", "description": "", "kind": "mcp-prompt", "source": "mcp"},
        ]});
        let status = json!({"trust": "untrusted", "requires_trust": ["skills/deploy", "hooks_config"]});
        let plugins = json!({"plugins": [
            {"name": "off", "enabled": false, "root": "/off", "skills": ["board"]},
            {"name": "boards", "enabled": true, "root": "/plug/boards", "skills": ["board"]},
        ]});
        let out = skills_of(&session, Some(&commands), Some(&status), Some(&plugins), &[source("acme.notes")]);
        assert!(out.listed);
        assert_eq!(out.trust.as_deref(), Some("untrusted"));
        let names: Vec<(&str, SkillTier)> = out.skills.iter().map(|s| (s.name.as_str(), s.tier)).collect();
        assert_eq!(
            names,
            [("tour", SkillTier::Space), ("deploy", SkillTier::Project), ("lint", SkillTier::Project), ("board", SkillTier::Plugin), ("mine", SkillTier::User)]
        );
        let tour = &out.skills[0];
        assert_eq!((tour.dir.as_deref(), tour.package.as_deref()), (Some("/state/ext/acme.notes/skills"), Some("acme.notes")));
        assert_eq!(out.skills[1].dir.as_deref(), Some("/p/.agents/skills"));
        assert!(out.skills[1].withheld, "a project skill the engine holds back until trust");
        assert!(!out.skills[2].withheld);
        assert_eq!((out.skills[3].plugin.as_deref(), out.skills[3].dir.as_deref()), (Some("boards"), Some("/plug/boards")));
        assert_eq!(out.dirs[0].package.as_deref(), Some("acme.notes"));
        assert_eq!(out.dirs[0].shadowed, vec![ShadowedSkill { skill: "x".into(), by: "user".into() }]);
    }

    #[test]
    fn a_trusted_project_holds_nothing_back_and_a_session_on_disk_lists_nothing() {
        let session = json!({"project_root": "/p", "skill_dirs": [{"path": "/elsewhere"}]});
        let commands = json!({"commands": [{"name": "deploy", "kind": "prompt", "source": "project"}]});
        let status = json!({"trust": "trusted", "requires_trust": ["skills/deploy"]});
        let out = skills_of(&session, Some(&commands), Some(&status), None, &[]);
        assert!(!out.skills[0].withheld);
        let on_disk = skills_of(&session, None, None, None, &[]);
        assert!(!on_disk.listed && on_disk.skills.is_empty());
        assert_eq!((on_disk.dirs[0].package.clone(), on_disk.dirs[0].state.clone()), (None, None));
    }

    fn tier(source: HookSource, hooks: Value) -> (HookSource, Option<Value>) {
        (source, Some(json!({"hooks_config": hooks})))
    }

    #[test]
    fn a_hook_is_shown_by_its_kind_and_matcher_and_never_by_what_it_runs() {
        let global = tier(
            HookSource::Global,
            json!({"PreToolUse": [
                {"type": "command", "command": "secret-cmd --token=abc", "if": "Bash(git push:*)", "on_error": "block"},
                {"type": "http", "url": "https://user:pw@hooks.example.com/x", "headers": {"authorization": "Bearer xyz"}},
                {"type": "prompt", "prompt": "a private prompt"},
                {"type": "agent", "if": "  "},
                {"type": "teleport"},
            ]}),
        );
        let (rows, unread) = hooks_of(&[global], None, &[]);
        assert!(unread.is_empty());
        let kinds: Vec<&str> = rows.iter().map(|r| r.kind.as_str()).collect();
        assert_eq!(kinds, ["command", "http", "prompt", "agent", "other"]);
        assert_eq!(rows[0].label, "command · Bash(git push:*)");
        assert_eq!(rows[0].on_error.as_deref(), Some("block"));
        assert_eq!((rows[1].label.as_str(), rows[3].matcher.clone()), ("http", None));
        let text = serde_json::to_string(&rows).unwrap();
        for secret in ["secret-cmd", "abc", "user:pw", "hooks.example.com", "Bearer", "xyz", "private prompt"] {
            assert!(!text.contains(secret), "{secret} leaked: {text}");
        }
    }

    #[test]
    fn a_long_matcher_is_cut() {
        let long = "x".repeat(200);
        let (rows, _) = hooks_of(&[tier(HookSource::Global, json!({"Stop": [{"type": "command", "if": long}]}))], None, &[]);
        let matcher = rows[0].matcher.clone().unwrap();
        assert_eq!(matcher.chars().count(), MATCHER_CHARS + 1);
        assert!(matcher.ends_with('…'));
    }

    #[test]
    fn the_highest_tier_with_an_event_replaces_the_lower_ones_and_an_untrusted_project_counts_for_nothing() {
        let tiers = [
            tier(HookSource::Global, json!({"PreToolUse": [{"type": "command"}], "Stop": [{"type": "command"}]})),
            tier(HookSource::Scene, json!({"PreToolUse": [{"type": "http"}]})),
            tier(HookSource::Project, json!({"Stop": [{"type": "prompt"}], "SessionEnd": [{"type": "command"}]})),
        ];
        let find = |rows: &[HookRow], event: &str, source: HookSource| rows.iter().find(|r| r.event == event && r.source == source).cloned().unwrap();

        let (rows, _) = hooks_of(&tiers, Some("untrusted"), &[]);
        let pre = find(&rows, "PreToolUse", HookSource::Global);
        assert_eq!((pre.in_force, pre.replaced_by), (false, Some(HookSource::Scene)));
        assert!(find(&rows, "PreToolUse", HookSource::Scene).in_force);
        // The project's `Stop` does not replace the global one while it is held back.
        let stop = find(&rows, "Stop", HookSource::Global);
        assert_eq!((stop.in_force, stop.replaced_by), (true, None));
        let held = find(&rows, "Stop", HookSource::Project);
        assert_eq!((held.in_force, held.withheld, held.replaced_by), (false, true, None));

        let (rows, _) = hooks_of(&tiers, Some("trusted"), &[]);
        let stop = find(&rows, "Stop", HookSource::Global);
        assert_eq!((stop.in_force, stop.replaced_by), (false, Some(HookSource::Project)));
        assert!(find(&rows, "Stop", HookSource::Project).in_force);
        assert!(find(&rows, "SessionEnd", HookSource::Project).in_force);
    }

    #[test]
    fn a_tier_not_read_is_named_and_the_hosts_hooks_follow_the_settings() {
        let host = [
            HostHook::new("PreToolUse", vec!["*".into()], "allow").providers(vec!["harnix-packages".into()]),
            HostHook::new("UserPromptSubmit", Vec::new(), "block"),
        ];
        let (rows, unread) = hooks_of(&[(HookSource::Global, None), tier(HookSource::Scene, json!({}))], None, &host);
        assert_eq!(unread, [HookSource::Global]);
        assert_eq!(rows.len(), 2);
        assert_eq!((rows[0].kind.as_str(), rows[0].label.as_str(), rows[0].providers.clone()), ("host", "host · *", vec!["harnix-packages".to_string()]));
        assert_eq!((rows[1].event.as_str(), rows[1].matcher.clone(), rows[1].on_error.as_deref()), ("UserPromptSubmit", None, Some("block")));
        assert!(rows.iter().all(|r| r.in_force && r.source == HookSource::Host));
    }
}
