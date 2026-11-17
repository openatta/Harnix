//! A run's output directory, as a template: where its deliverables go, relative to the place the
//! run works (its project; a run of no project, its own directory).
//!
//! A template is a relative path that may say `${flow}`, `${date}` (`YYYYMMDD`), `${seq}` (two
//! digits), `${title}` and `${inputs.<k>}`; [`DEFAULT_OUTPUT_DIR`](super::def::DEFAULT_OUTPUT_DIR)
//! is `flows/${flow}-${date}${seq}`. A value put in never adds a segment to the path: its `/` and
//! other characters a name cannot hold become `-`. What `${seq}` is — one more than the highest
//! already there — is the host's to find on disk; this only renders.

use std::collections::BTreeMap;

use serde_json::Value;

use super::def::InputDef;
use super::refs::refs_in;

/// The variables a template may say, besides `inputs.<k>`.
pub const OUTPUT_VARS: &[&str] = &["flow", "date", "seq", "title"];

/// What a template is rendered with.
#[derive(Debug, Clone, Copy)]
pub struct OutputVars<'a> {
    pub flow: &'a str,
    /// `YYYYMMDD`, the local date the run is started on.
    pub date: &'a str,
    pub title: &'a str,
    /// The run's inputs, defaults applied.
    pub inputs: &'a Value,
}

/// Whether a template says `${seq}`.
pub fn has_seq(template: &str) -> bool {
    refs_in(template).contains(&"seq")
}

/// What is wrong with a template, on its own: a variable it may not say, an input the flow does
/// not declare, a path that is absolute or climbs out with `..`.
pub fn template_problems(template: &str, inputs: &BTreeMap<String, InputDef>) -> Vec<String> {
    let mut out = vec![];
    if template.trim().is_empty() {
        out.push("is empty".to_string());
        return out;
    }
    for r in refs_in(template) {
        match r.strip_prefix("inputs.") {
            Some(k) if inputs.contains_key(k) => {}
            Some(k) => out.push(format!("${{{r}}}: no input {k}")),
            None if OUTPUT_VARS.contains(&r) => {}
            None => out.push(format!(
                "${{{r}}}: a template says only ${{flow}}, ${{date}}, ${{seq}}, ${{title}} and ${{inputs.<k>}}"
            )),
        }
    }
    if let Err(m) = shape(template) {
        out.push(m);
    }
    out
}

/// A rendered path's segments, if it is relative and stays where it starts.
fn shape(path: &str) -> Result<Vec<&str>, String> {
    if path.starts_with('/') || path.starts_with('\\') || path.get(1..3) == Some(":\\") {
        return Err(format!("{path:?} is absolute: an output directory is relative to the project"));
    }
    let segs: Vec<&str> = path.split(['/', '\\']).filter(|s| !s.is_empty() && *s != ".").collect();
    if segs.contains(&"..") {
        return Err(format!("{path:?} climbs out with `..`: an output directory stays inside the project"));
    }
    if segs.is_empty() {
        return Err(format!("{path:?} names no directory: it would be the project itself"));
    }
    Ok(segs)
}

/// A value as one piece of a name: nothing that separates segments or that a file name cannot
/// hold, and never `.` or `..` alone.
pub fn path_safe(value: &str) -> String {
    let s: String = value
        .trim()
        .chars()
        .map(|c| if c == '/' || c == '\\' || c == ':' || c.is_control() { '-' } else { c })
        .collect();
    if s.chars().all(|c| c == '.') && !s.is_empty() {
        "-".repeat(s.len())
    } else {
        s
    }
}

/// The template with its variables put in (`seq`: what `${seq}` is; `None` when it says none),
/// as a relative path of plain segments joined by `/`.
pub fn render(template: &str, vars: &OutputVars, seq: Option<u32>) -> Result<String, String> {
    let mut out = String::new();
    let mut rest = template;
    while let Some(i) = rest.find("${") {
        out.push_str(&rest[..i]);
        let after = &rest[i + 2..];
        let Some(j) = after.find('}') else {
            return Err(format!("{template:?}: a `${{` is not closed"));
        };
        let name = after[..j].trim();
        let value = match name {
            "flow" => vars.flow.to_string(),
            "date" => vars.date.to_string(),
            "title" => vars.title.to_string(),
            "seq" => match seq {
                Some(n) => format!("{n:02}"),
                None => return Err(format!("{template:?}: no number for ${{seq}}")),
            },
            other => match other.strip_prefix("inputs.") {
                Some(k) => match vars.inputs.get(k) {
                    Some(Value::String(s)) => s.clone(),
                    Some(Value::Null) | None => String::new(),
                    Some(v) => v.to_string(),
                },
                None => return Err(format!("{template:?}: ${{{other}}} is not a variable of an output directory")),
            },
        };
        out.push_str(&path_safe(&value));
        rest = &after[j + 1..];
    }
    out.push_str(rest);
    Ok(shape(&out)?.join("/"))
}

/// The UTC date of an instant, `YYYYMMDD` — for a host that cannot tell the local one.
pub fn date_utc(epoch_ms: u64) -> String {
    let days = (epoch_ms / 86_400_000) as i64;
    // Howard Hinnant's civil_from_days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}{m:02}{d:02}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn vars<'a>(inputs: &'a Value, title: &'a str) -> OutputVars<'a> {
        OutputVars { flow: "review", date: "20261116", title, inputs }
    }

    #[test]
    fn the_default_is_the_flow_the_date_and_a_number() {
        let none = json!({});
        let t = super::super::def::DEFAULT_OUTPUT_DIR;
        assert!(has_seq(t));
        assert_eq!(render(t, &vars(&none, ""), Some(1)).unwrap(), "flows/review-2026111601");
        assert_eq!(render(t, &vars(&none, ""), Some(12)).unwrap(), "flows/review-2026111612");
        assert!(render(t, &vars(&none, ""), None).is_err());
        assert!(!has_seq("out/${date}"));
    }

    #[test]
    fn a_value_never_climbs_out_or_adds_a_segment() {
        let inputs = json!({"module": "../../etc", "n": 3, "dots": ".."});
        let v = vars(&inputs, "a/b: c");
        assert_eq!(render("out/${inputs.module}", &v, None).unwrap(), "out/..-..-etc");
        assert_eq!(render("out/${inputs.dots}", &v, None).unwrap(), "out/--");
        assert_eq!(render("out/${title}", &v, None).unwrap(), "out/a-b- c");
        assert_eq!(render("out/${inputs.n}/./x//", &v, None).unwrap(), "out/3/x");
        assert_eq!(render("out/${inputs.missing}x", &v, None).unwrap(), "out/x");
        // What the template itself says is not let through.
        for bad in ["../out", "out/../../x", "/abs/out", "", "./", "${title}"] {
            assert!(render(bad, &vars(&json!({}), ""), None).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn a_template_says_only_its_variables_and_declared_inputs() {
        let inputs: BTreeMap<String, InputDef> =
            [("mode".to_string(), serde_yaml::from_str("{schema: {type: string}}").unwrap())].into();
        assert!(template_problems("flows/${flow}-${inputs.mode}-${date}${seq}", &inputs).is_empty());
        let p = template_problems("/x/${run}/${inputs.other}/../y", &inputs);
        assert_eq!(p.len(), 3, "{p:?}");
        assert!(p.iter().any(|m| m.contains("${run}")) && p.iter().any(|m| m.contains("no input other")), "{p:?}");
        assert!(p.iter().any(|m| m.contains("absolute")), "{p:?}");
        assert!(template_problems("a/../b", &inputs).iter().any(|m| m.contains("..")));
    }

    #[test]
    fn a_utc_date() {
        assert_eq!(date_utc(0), "19700101");
        assert_eq!(date_utc(1_794_787_200_000), "20261116");
        assert_eq!(date_utc(951_782_400_000), "20000229");
    }
}
