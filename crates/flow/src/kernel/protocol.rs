//! The protocol layer: the text the module puts before a skill's body in a session task.
//!
//! What a session is opened with — its scene (so how it hands in), its output contract when it
//! hands in by structured output, its model and place — it keeps for good. Tasks that may share
//! a session (`per_run`, or the same `{key}` expression) must therefore agree on all of them;
//! validation says so, and the module refuses to put a task into a shared session opened
//! otherwise. What a task may do is not kept by the session: the module judges each call against
//! the permissions of the task the turn works on (xops-04 §3). Tool patterns are names or `ns__*`
//! groups ([`guard_problem`], [`rule_tool`]).
//!
//! It is generated from the station's definition — what to hand in, how, what
//! may be done, how the station is judged — so a skill only has to say how
//! to do the work well. Where the two disagree, the protocol wins.

use serde_json::Value;

use std::collections::BTreeMap;

use super::def::{Executor, ExitCond, FlowDef, Station};
use super::host::Delivery;
use super::parts::{Effect, GateDef, GateKind, Granted};

/// The name of the engine's tool a session hands a structured output in with.
pub const STRUCTURED_OUTPUT_TOOL: &str = "StructuredOutput";

/// Whether `name` matches any of `patterns` (`prefix*` matches every name with that prefix).
pub fn matches_any(patterns: &[String], name: &str) -> bool {
    patterns.iter().any(|p| match p.strip_suffix('*') {
        Some(prefix) => name.starts_with(prefix),
        None => p == name,
    })
}

/// What is wrong with a guarded-tool pattern, if anything. A pattern is a tool's exact name or
/// a namespace group `ns__*` (every tool under `ns`, which the engine's permission rules match
/// at `__` boundaries); nothing else can be written as a `deny` rule, so nothing else is
/// accepted — a pattern that would be dropped would silently weaken the guard.
pub fn guard_problem(pattern: &str) -> Option<String> {
    let bad = |why: &str| Some(format!("guarded tool pattern {pattern:?}: {why}"));
    if pattern.trim().is_empty() || pattern.trim() != pattern {
        return bad("empty or padded");
    }
    match pattern.strip_suffix('*') {
        None if pattern.contains('*') => bad("`*` only goes at the end, after `__`"),
        None => None,
        Some(prefix) => match prefix.strip_suffix("__") {
            Some(ns) if !ns.is_empty() && !ns.contains('*') => None,
            _ => bad(
                "a group must be a namespace, `ns__*` — the engine's permission rules cannot \
                 express another prefix; list the tools one by one",
            ),
        },
    }
}

/// Every problem of a guarded-tool list ([`guard_problem`]).
pub fn guard_problems(guarded: &[String]) -> Vec<String> {
    guarded.iter().filter_map(|g| guard_problem(g)).collect()
}

/// The tool name an engine permission rule gives for a pattern: the name, or for a group
/// `ns__*` its namespace `ns` (the engine matches rules at `__` boundaries). A pattern
/// [`guard_problem`] refuses is taken as widely as a rule can say: up to its last `__`, or as
/// written.
pub fn rule_tool(pattern: &str) -> String {
    match pattern.strip_suffix('*') {
        None => pattern.to_string(),
        Some(prefix) => match prefix.strip_suffix("__") {
            Some(ns) if !ns.is_empty() => ns.to_string(),
            _ => match prefix.rfind("__") {
                Some(i) if i > 0 => prefix[..i].to_string(),
                _ => prefix.to_string(),
            },
        },
    }
}

/// A session's `ask` permission rules for tool patterns, each rule once: every call of those
/// tools comes to be judged.
pub fn ask_rules(patterns: &[String]) -> Value {
    let mut tools: Vec<String> = vec![];
    for p in patterns {
        let t = rule_tool(p);
        if !tools.contains(&t) {
            tools.push(t);
        }
    }
    Value::Array(
        tools
            .into_iter()
            .map(|t| serde_json::json!({"source": "session", "behavior": "ask", "tool_name": t}))
            .collect(),
    )
}

/// A SKILL.md body without its front matter.
pub fn skill_body(text: &str) -> String {
    let t = text.trim_start();
    let body = t
        .strip_prefix("---\n")
        .and_then(|r| r.split_once("\n---").map(|(_, b)| b))
        .unwrap_or(t);
    body.trim().to_string()
}

/// The whole first message a session task's session is sent: the protocol layer, the rules it
/// works under (`rules`: name and text), then the skill's body under its name (`skill`: name and
/// SKILL.md text).
pub fn task_message(protocol: &str, rules: &[(String, String)], skill: Option<(&str, &str)>) -> String {
    let mut text = protocol.to_string();
    if !rules.is_empty() {
        text.push_str(
            "\n\n## Rules\n\nThese hold for the whole flow. They are not checked here by \
             reading your words: keep to them.\n",
        );
        for (name, body) in rules {
            let (title, body) = super::parts::rule_parts(name, body);
            text.push_str(&format!("\n### {title} (`{name}`)\n\n{body}\n"));
        }
    }
    if let Some((name, body)) = skill {
        text.push_str(&format!("\n\n## Skill `{name}`\n\n{}\n", skill_body(body)));
    }
    text
}

/// What one session task is told before its skill.
pub struct TaskContext<'a> {
    pub def: &'a FlowDef,
    pub station: &'a Station,
    pub run: &'a str,
    pub subject: &'a str,
    pub round: u32,
    pub task: &'a str,
    pub item: Option<&'a Value>,
    /// The tools the host refuses unless a permission allows them (`Engine::with_guarded`).
    pub guarded: &'a [String],
    /// What the task works under, references resolved.
    pub permissions: &'a [Granted],
    /// The run's pinned gates, for the station's `gate` exits.
    pub gates: &'a BTreeMap<String, GateDef>,
    /// How the output is handed in.
    pub delivery: Delivery,
    /// The task's executor: a sub-agent task (`kind: agent`) and a task in the main session
    /// are told so.
    pub executor: Option<&'a Executor>,
}

/// The agent type a sub-agent task asks for when its station names none: the module provides
/// it where the main session works.
pub const DEFAULT_AGENT_TYPE: &str = "flow-agent";

/// What a sub-agent task's sub-agent is named by (the `Agent` tool's `description`), so the
/// module can tell it apart among the main session's tasks.
pub fn agent_tag(run: &str, station: &str, task: &str) -> String {
    format!("flow:{run}/{station}/{task}")
}

pub fn render(t: &TaskContext) -> String {
    let st = t.station;
    let title = if st.title.is_empty() {
        &st.id
    } else {
        &st.title
    };
    let mut s = String::from(
        "## Flow protocol\n\n\
         Generated from the flow definition. It cannot be changed, and where the skill below \
         disagrees with it, this protocol wins.\n\n",
    );
    s.push_str(&format!(
        "- Run `{}` of flow `{}`, subject `{}`.\n- Station `{}` ({title}), round {}; your task is `{}`.\n",
        t.run, t.def.name, t.subject, st.id, t.round, t.task
    ));
    if let Some(item) = t.item {
        s.push_str(&format!("- Your item:\n\n```json\n{}\n```\n", pretty(item)));
    }
    if t.executor.is_some_and(Executor::in_main) {
        s.push_str(
            "- You are this run's main agent: this message is one task of the run, and the other \
             messages of this session belong to other tasks of it.\n",
        );
    }
    if let Some(Executor::Agent {
        agent_type, model, ..
    }) = t.executor
    {
        let mut how = vec![format!(
            "`description: \"{}\"`",
            agent_tag(t.run, &st.id, t.task)
        )];
        how.push(format!(
            "`subagent_type: \"{}\"`",
            agent_type.as_deref().unwrap_or(DEFAULT_AGENT_TYPE)
        ));
        if let Some(m) = model {
            how.push(format!("`model: \"{m}\"`"));
        }
        s.push_str(&format!(
            "- **This task is a sub-agent's.** Do not do it yourself: call the `Agent` tool once, \
             with {} and a prompt that gives the sub-agent the task — the item, the contract, \
             the rules and the skill below. When it answers, check its result against the contract and hand it \
             in with `flow_submit`; if it falls short, call it again with what is missing.\n",
            how.join(", ")
        ));
    }
    match &st.output {
        Some(schema) => s.push_str(&format!(
            "- Hand in one JSON object satisfying this contract:\n\n```json\n{}\n```\n",
            pretty(schema)
        )),
        None => s.push_str(
            "- There is no output contract; hand in a JSON object summarising what you did.\n",
        ),
    }
    match t.delivery {
        Delivery::FlowSubmit => s.push_str(
            "- Submit with `flow_submit {output}` once the work is done. You may submit again until \
             the station is passed; the last submission counts. Check what is still missing with \
             `flow_state`, not by your own judgement.\n",
        ),
        Delivery::StructuredOutput => s.push_str(&format!(
            "- Hand the output in by calling the `{STRUCTURED_OUTPUT_TOOL}` tool with the output as \
             its input, once the work is done; it is checked against the contract on the spot, and \
             you may call it again until it is accepted. When your turn ends, the flow takes the \
             last output you handed in; if the station still lacks something, the next message \
             says what, and you hand in again.\n"
        )),
    }
    s.push_str(&format!(
        "- The station passes only when all of these hold: {}.\n",
        exits(st)
    ));
    for e in &st.exit {
        let ExitCond::Gate { gate, .. } = e else { continue };
        let Some(g) = t.gates.get(gate) else { continue };
        let what = if g.description.is_empty() { g.label().to_string() } else { g.description.clone() };
        match g.kind {
            GateKind::Tool => s.push_str(&format!(
                "- Gate `{gate}`: {what} It calls `{}` with {}; you may call it yourself before handing in.\n",
                g.tool.as_deref().unwrap_or_default(),
                g.args
            )),
            GateKind::Command => s.push_str(&format!(
                "- Gate `{gate}`: {what} It runs `{}` once your output is in.\n",
                g.command.as_deref().unwrap_or_default()
            )),
            GateKind::Host => s.push_str(&format!("- Gate `{gate}`: {what}\n")),
        }
    }
    s.push_str(&permissions_text(t.permissions, t.guarded));
    match t.delivery {
        Delivery::FlowSubmit => s.push_str(
            "- Rework and cancellation are not yours to decide: propose them with \
             `flow_propose {kind: rework|cancel, to?, reason}` and a person decides. You cannot confirm \
             or force a station.\n",
        ),
        Delivery::StructuredOutput => s.push_str(
            "- Rework and cancellation are not yours to decide: if you cannot do the task, or it needs \
             an earlier station done again, say so in your reply and a person decides. You cannot \
             confirm or force a station.\n",
        ),
    }
    s
}

/// What the task may do, as the session is told: each permission, then the tools refused unless
/// one allows them.
fn permissions_text(granted: &[Granted], guarded: &[String]) -> String {
    let mut s = String::from(
        "- The flow judges each tool call against these permissions (deny over ask over allow); a \
         call refused says why — do not try it another way. A call a person is asked about waits for \
         their answer.\n",
    );
    if granted.is_empty() {
        s.push_str("  - (no permissions)\n");
    }
    for g in granted {
        let p = &g.permission;
        let mut scope = vec![];
        if !p.paths.is_empty() {
            scope.push(format!("paths {}", p.paths.join(", ")));
        }
        if !p.commands.is_empty() {
            scope.push(format!("commands {}", p.commands.join(", ")));
        }
        for (k, v) in &p.args {
            scope.push(format!("{k} {}", v.join(", ")));
        }
        let scope = if scope.is_empty() { String::new() } else { format!(" — {}", scope.join("; ")) };
        let verb = match p.effect {
            Effect::Allow => "allowed",
            Effect::Ask => "a person is asked",
            Effect::Deny => "refused",
        };
        let msg = if p.message.is_empty() { String::new() } else { format!(": {}", p.message) };
        s.push_str(&format!("  - {} {verb}{scope} ({}){msg}\n", p.tools.join(", "), g.from));
    }
    if guarded.is_empty() {
        s.push_str("- No tool is refused unless a permission says so.\n");
    } else {
        s.push_str(&format!(
            "- Refused unless a permission allows them: {}. Do not change anything outside the flow otherwise.\n",
            guarded.join(", ")
        ));
    }
    s
}

fn exits(st: &Station) -> String {
    let labels: Vec<String> = st.exit.iter().map(ExitCond::label).collect();
    if labels.is_empty() {
        "(none)".into()
    } else {
        labels.join("; ")
    }
}

fn pretty(v: &Value) -> String {
    serde_json::to_string_pretty(v).unwrap_or_else(|_| v.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn body_drops_front_matter() {
        assert_eq!(skill_body("---\nname: x\n---\n\nBody\n"), "Body");
        assert_eq!(skill_body("Just text"), "Just text");
    }

    #[test]
    fn a_group_is_asked_by_its_namespace() {
        let v = |xs: &[&str]| xs.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        let rules = ask_rules(&v(&["Bash", "mcp__db__*", "mcp__db__insert", "Note*", "Bash"]));
        let names: Vec<&str> = rules
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["tool_name"].as_str().unwrap())
            .collect();
        assert_eq!(names, ["Bash", "mcp__db", "mcp__db__insert", "Note"]);
        assert!(rules.as_array().unwrap().iter().all(|r| r["behavior"] == "ask"));
    }

    #[test]
    fn guarded_patterns_are_names_or_namespace_groups() {
        assert_eq!(guard_problem("Bash"), None);
        assert_eq!(guard_problem("mcp__db__*"), None);
        for bad in ["Note*", "mcp__db_*", "__*", "a*b", "", " Bash", "mcp__*__*"] {
            assert!(guard_problem(bad).is_some(), "{bad:?}");
        }
    }

    static M1: std::sync::LazyLock<Value> = std::sync::LazyLock::new(|| serde_json::json!("m1"));

    fn ctx<'a>(def: &'a FlowDef, granted: &'a [Granted], gates: &'a BTreeMap<String, GateDef>, guarded: &'a [String], delivery: Delivery) -> TaskContext<'a> {
        TaskContext {
            def,
            station: &def.stations[0],
            run: "demo-1",
            subject: "repo",
            round: 1,
            task: &def.stations[0].id,
            item: Some(&M1),
            guarded,
            permissions: granted,
            gates,
            delivery,
            executor: None,
        }
    }

    #[test]
    fn structured_output_is_the_default_way_in() {
        let def = FlowDef::parse(
            "name: d\nstations: [{id: a, executor: {kind: session, scene: chat}, skill: s, output: {type: object}, exit: [{kind: output}]}]",
        )
        .unwrap();
        let t = render(&ctx(&def, &[], &BTreeMap::new(), &[], Delivery::StructuredOutput));
        assert!(t.contains("`StructuredOutput`"), "{t}");
        assert!(!t.contains("flow_submit") && !t.contains("flow_propose"), "{t}");
        assert!(t.contains("No tool is refused"), "{t}");
    }

    #[test]
    fn layer_comes_from_the_station() {
        let def = FlowDef::parse(
            r#"
name: demo
stations:
  - id: analyze
    executor: {kind: session, scene: chat}
    skill: analyze
    output: {type: object, required: [findings]}
    exit: [{kind: output}, {kind: gate, gate: counted}]
"#,
        )
        .unwrap();
        let granted = vec![Granted {
            permission: serde_yaml::from_str("{effect: allow, tools: [app.x.findings_add], message: only findings}").unwrap(),
            from: "station analyze".into(),
        }];
        let gates: BTreeMap<String, GateDef> = [(
            "counted".to_string(),
            serde_yaml::from_str("{name: counted, kind: tool, tool: count, args: {run: '${run}'}, assert: [{path: total, eq: 0}]}").unwrap(),
        )]
        .into();
        let t = render(&ctx(&def, &granted, &gates, &["Bash".to_string()], Delivery::FlowSubmit));
        for want in [
            "demo-1",
            "findings",
            "app.x.findings_add allowed (station analyze): only findings",
            "flow_submit",
            "flow_propose",
            "`count`",
            "\"m1\"",
            "Refused unless a permission allows them: Bash",
        ] {
            assert!(t.contains(want), "missing {want}: {t}");
        }
    }

    #[test]
    fn rules_come_before_the_skill() {
        let m = task_message("P", &[("no-secrets".into(), "# No secrets\n\nNever paste keys.".into())], Some(("s", "Do it.")));
        let (r, k) = (m.find("### No secrets (`no-secrets`)").unwrap(), m.find("## Skill `s`").unwrap());
        assert!(r < k, "{m}");
        assert!(m.contains("Never paste keys."));
    }
}
