//! The protocol layer: the text the module puts before a skill's body in a session task.
//!
//! It is generated from the station's definition — what to hand in, how, what
//! may be written, how the station is judged — so a skill only has to say how
//! to do the work well. Where the two disagree, the protocol wins.

use serde_json::Value;

use super::def::{ExitCond, FlowDef, Station};

/// A SKILL.md body without its front matter.
pub fn skill_body(text: &str) -> String {
    let t = text.trim_start();
    let body = t
        .strip_prefix("---\n")
        .and_then(|r| r.split_once("\n---").map(|(_, b)| b))
        .unwrap_or(t);
    body.trim().to_string()
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
    match &st.output {
        Some(schema) => s.push_str(&format!(
            "- Hand in one JSON object satisfying this contract:\n\n```json\n{}\n```\n",
            pretty(schema)
        )),
        None => s.push_str(
            "- There is no output contract; hand in a JSON object summarising what you did.\n",
        ),
    }
    s.push_str(&format!(
        "- Submit with `flow_submit {{output}}` once the work is done. You may submit again until \
         the station is passed; the last submission counts. Check what is still missing with \
         `flow_state`, not by your own judgement.\n- The station passes only when all of these hold: {}.\n",
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
    if st.writes.is_empty() {
        s.push_str("- This station is read-only for you: any tool that writes will be refused.\n");
    } else {
        s.push_str(&format!(
            "- The only write tools you may call here are: {}. Other writes will be refused.\n",
            st.writes.join(", ")
        ));
    }
    s.push_str(
        "- Rework and cancellation are not yours to decide: propose them with \
         `flow_propose {kind: rework|cancel, to?, reason}` and a person decides. You cannot confirm \
         or force a station.\n",
    );
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
        });
        for want in [
            "demo-1",
            "findings",
            "app.x.findings_*",
            "flow_submit",
            "flow_propose",
            "`count`",
            "\"m1\"",
        ] {
            assert!(t.contains(want), "missing {want}: {t}");
        }
    }
}
