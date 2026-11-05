//! The protocol layer: the text the module puts before a skill's body in a session task.
//!
//! What a session is opened with — its scene (so how it hands in), its output contract when it
//! hands in by structured output, the guarded tools it is refused, its model and place — it
//! keeps for good. Tasks that may share a session (`per_run`, or the same `{key}` expression)
//! must therefore agree on all of them; validation says so, and the module refuses to put a
//! task into a shared session opened otherwise. Guarded patterns are names or `ns__*` groups,
//! and a group is opened whole or not at all ([`refused`], [`partial_writes`]).
//!
//! It is generated from the station's definition — what to hand in, how, what
//! may be written, how the station is judged — so a skill only has to say how
//! to do the work well. Where the two disagree, the protocol wins.

use serde_json::Value;

use super::def::{Executor, ExitCond, FlowDef, Station};
use super::host::Delivery;

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

/// A station's write that opens only part of a guarded group: `(write, group)`. A `deny` rule
/// cannot carry an exception, so such a write cannot be granted — validation reports it, and
/// [`refused`] keeps the whole group refused.
pub fn partial_writes(guarded: &[String], writes: &[String]) -> Vec<(String, String)> {
    let mut out = vec![];
    for g in guarded {
        let Some(prefix) = g.strip_suffix('*') else {
            continue;
        };
        for w in writes {
            let inside = match w.strip_suffix('*') {
                // A wider (or the same) group covers the whole guarded group.
                Some(wp) => wp.starts_with(prefix) && wp != prefix,
                None => w.starts_with(prefix),
            };
            if inside {
                out.push((w.clone(), g.clone()));
            }
        }
    }
    out
}

/// The guarded tools a station's session is refused, as `(pattern, rule tool name)`: what the
/// protocol layer names and what the module writes as the session's `deny` permission rules.
///
/// Fails closed. A guarded name is refused unless the station's `writes` allow it; a guarded
/// group (`ns__*`, the rule names the namespace `ns`) is refused unless a write covers the
/// whole group — a write of part of it does not open it ([`partial_writes`], a validation
/// problem). A pattern [`guard_problem`] refuses (the module does not start with one) is
/// refused as widely as a rule can say: up to its last `__`, or as written.
pub fn refused(guarded: &[String], writes: &[String]) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = vec![];
    for g in guarded {
        let rule = match g.strip_suffix('*') {
            None => {
                if matches_any(writes, g) {
                    continue;
                }
                g.clone()
            }
            Some(prefix) => {
                let covered = writes
                    .iter()
                    .any(|w| w.strip_suffix('*').is_some_and(|wp| prefix.starts_with(wp)));
                if covered {
                    continue;
                }
                match prefix.strip_suffix("__") {
                    Some(ns) if !ns.is_empty() => ns.to_string(),
                    _ => match prefix.rfind("__") {
                        Some(i) if i > 0 => prefix[..i].to_string(),
                        _ => prefix.to_string(),
                    },
                }
            }
        };
        if !out.iter().any(|(_, r)| *r == rule) {
            out.push((g.clone(), rule));
        }
    }
    out
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

/// The whole first message a session task's session is sent: the protocol layer, then the
/// skill's body under its name (`skill`: name and SKILL.md text).
pub fn task_message(protocol: &str, skill: Option<(&str, &str)>) -> String {
    let mut text = protocol.to_string();
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
    /// The tools the host refuses as writes unless the station lists them (`Engine::with_guarded`).
    pub guarded: &'a [String],
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
             with {} and a prompt that gives the sub-agent the task — the item, the contract and \
             the skill below. When it answers, check its result against the contract and hand it \
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
        if let ExitCond::Tool { tool, args, .. } = e {
            s.push_str(&format!(
                "- Exit check `{tool}` is called with {}; you may call it yourself before submitting.\n",
                args
            ));
        }
    }
    let refused: Vec<String> = refused(t.guarded, &st.writes)
        .into_iter()
        .map(|(p, _)| p)
        .collect();
    let guarded = if refused.is_empty() {
        "the host refuses no tool as a write".to_string()
    } else {
        format!("the host refuses these as writes: {}", refused.join(", "))
    };
    if st.writes.is_empty() {
        s.push_str(&format!(
            "- This station allows no writes ({guarded}). Do not change anything outside the flow.\n"
        ));
    } else {
        s.push_str(&format!(
            "- This station allows these writes: {}; beyond them {guarded}.\n",
            st.writes.join(", ")
        ));
    }
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
    fn refused_writes_are_the_guarded_tools_a_station_does_not_open() {
        let v = |xs: &[&str]| xs.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        let g = v(&["Bash", "Write", "mcp__db__*", "Note*"]);
        let rules = |w: &[&str]| -> Vec<String> {
            refused(&g, &v(w)).into_iter().map(|(_, r)| r).collect()
        };
        assert_eq!(
            rules(&[]),
            ["Bash", "Write", "mcp__db", "Note"],
            "a group by its namespace; a pattern no rule can say is refused as written"
        );
        assert_eq!(rules(&["Write"]), ["Bash", "mcp__db", "Note"]);
        assert_eq!(
            rules(&["mcp__db__insert"]),
            ["Bash", "Write", "mcp__db", "Note"],
            "a group partly opened stays refused: it fails closed"
        );
        assert_eq!(rules(&["mcp__*"]), ["Bash", "Write", "Note"]);
        assert_eq!(
            rules(&["Bash", "Write", "mcp__db__*", "Note*"]),
            Vec::<String>::new()
        );
    }

    #[test]
    fn guarded_patterns_are_names_or_namespace_groups() {
        assert_eq!(guard_problem("Bash"), None);
        assert_eq!(guard_problem("mcp__db__*"), None);
        for bad in ["Note*", "mcp__db_*", "__*", "a*b", "", " Bash", "mcp__*__*"] {
            assert!(guard_problem(bad).is_some(), "{bad:?}");
        }
        let v = |xs: &[&str]| xs.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        assert_eq!(
            partial_writes(
                &v(&["Bash", "mcp__db__*"]),
                &v(&[
                    "mcp__db__insert",
                    "Bash",
                    "mcp__db__x*",
                    "mcp__*",
                    "mcp__db__*"
                ])
            ),
            [
                ("mcp__db__insert".to_string(), "mcp__db__*".to_string()),
                ("mcp__db__x*".to_string(), "mcp__db__*".to_string()),
            ]
        );
    }

    #[test]
    fn structured_output_is_the_default_way_in() {
        let def = FlowDef::parse(
            "name: d\nstations: [{id: a, executor: {kind: session, scene: chat}, skill: s, output: {type: object}, exit: [{kind: output}]}]",
        )
        .unwrap();
        let t = render(&TaskContext {
            def: &def,
            station: &def.stations[0],
            run: "d-1",
            subject: "x",
            round: 1,
            task: "a",
            item: None,
            guarded: &[],
            delivery: Delivery::StructuredOutput,
            executor: None,
        });
        assert!(t.contains("`StructuredOutput`"), "{t}");
        assert!(
            !t.contains("flow_submit") && !t.contains("flow_propose"),
            "{t}"
        );
        assert!(t.contains("refuses no tool"), "{t}");
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
    writes: [app.x.findings_*]
    exit: [{kind: output}, {kind: tool, tool: count, args: {run: "${run}"}, assert: [{path: total, eq: 0}]}]
"#,
        )
        .unwrap();
        let t = render(&TaskContext {
            def: &def,
            station: &def.stations[0],
            run: "demo-1",
            subject: "repo",
            round: 1,
            task: "analyze",
            item: Some(&serde_json::json!("m1")),
            guarded: &["Bash".to_string()],
            delivery: Delivery::FlowSubmit,
            executor: None,
        });
        for want in [
            "demo-1",
            "findings",
            "app.x.findings_*",
            "flow_submit",
            "flow_propose",
            "`count`",
            "\"m1\"",
            "refuses these as writes: Bash",
        ] {
            assert!(t.contains(want), "missing {want}: {t}");
        }
    }
}
