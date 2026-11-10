//! References (`${...}`) and assertions.
//!
//! Both read the same thing: a JSON context built from the run —
//! `{run, subject, inputs, stations: {<id>: {status, round, output}}, item, task, workspace}`.
//! A reference names a path into it (`${stations.collect.output.modules}`); an
//! assertion in `when` does too (`path: inputs.mode`). A `tool` exit's
//! assertions select from the tool's result instead.
//!
//! Paths are `.`-separated; a segment may carry `[n]` (index) or `[field=value]`
//! (the first array element whose `field` is that string).

use serde_json::{json, Map, Value};

use super::def::{Assert, FlowDef, Work};
use super::run::{RunRecord, TaskStatus};

/// The roots a reference may start with.
pub const ROOTS: &[&str] = &["run", "subject", "inputs", "stations", "item", "task", "workspace"];

/// The directory a run works in under the application's workspaces (`<root>/<run>`, the run's id
/// made safe as a name): `${workspace}`.
pub fn workspace_of(root: &std::path::Path, run: &str) -> std::path::PathBuf {
    let safe: String = run
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '-' })
        .collect();
    root.join(safe)
}

/// A station's output as downstream sees it: a single task's output; `{task id: output}`
/// for static tasks; an array in item order for `foreach`; `null` for a checkpoint.
pub fn station_output(def: &FlowDef, run: &RunRecord, station: &str) -> Value {
    let (Some(st), Some(rec)) = (def.station(station), run.stations.get(station)) else {
        return Value::Null;
    };
    let out = |t: &super::run::TaskRec| {
        if t.status == TaskStatus::Submitted {
            t.output.clone().unwrap_or(Value::Null)
        } else {
            Value::Null
        }
    };
    match st.work() {
        Work::None => Value::Null,
        Work::One(_) => rec.tasks.first().map(out).unwrap_or(Value::Null),
        Work::Tasks(_) => Value::Object(rec.tasks.iter().map(|t| (t.id.clone(), out(t))).collect()),
        Work::Foreach { .. } => Value::Array(rec.tasks.iter().map(out).collect()),
    }
}

/// The context references and `when` assertions are read against.
pub fn context(run: &RunRecord, item: Option<&Value>, task: Option<&str>) -> Value {
    context_in(run, item, task, None)
}

/// [`context`] with the run's workspace (`null` when the application keeps none).
pub fn context_in(run: &RunRecord, item: Option<&Value>, task: Option<&str>, workspace: Option<&str>) -> Value {
    let stations: Map<String, Value> = run
        .stations
        .iter()
        .map(|(id, r)| {
            (
                id.clone(),
                json!({
                    "status": r.status,
                    "round": r.round,
                    "output": station_output(&run.def, run, id),
                }),
            )
        })
        .collect();
    json!({
        "run": run.id,
        "subject": run.subject,
        "inputs": run.inputs,
        "stations": stations,
        "item": item.cloned().unwrap_or(Value::Null),
        "task": task.map_or(Value::Null, |t| Value::String(t.into())),
        "workspace": workspace.map_or(Value::Null, |w| Value::String(w.into())),
    })
}

/// Select a path from a value; `None` if nothing is there.
pub fn select<'v>(v: &'v Value, path: &str) -> Option<&'v Value> {
    let mut cur = v;
    for seg in path.split('.').filter(|s| !s.is_empty()) {
        let (name, mut rest) = match seg.find('[') {
            Some(i) => (&seg[..i], &seg[i..]),
            None => (seg, ""),
        };
        if !name.is_empty() {
            cur = cur.get(name)?;
        }
        while let Some(r) = rest.strip_prefix('[') {
            let end = r.find(']')?;
            let sel = &r[..end];
            rest = &r[end + 1..];
            cur = match sel.split_once('=') {
                Some((k, want)) => cur.as_array()?.iter().find(|e| {
                    e.get(k.trim()).is_some_and(|x| match x {
                        Value::String(s) => s == want.trim(),
                        other => {
                            *other
                                == serde_json::from_str::<Value>(want.trim()).unwrap_or(Value::Null)
                        }
                    })
                })?,
                None => cur.get(sel.trim().parse::<usize>().ok()?)?,
            };
        }
    }
    Some(cur)
}

/// The `${...}` references in a string, in order (their inner paths).
pub fn refs_in(s: &str) -> Vec<&str> {
    let mut out = vec![];
    let mut rest = s;
    while let Some(i) = rest.find("${") {
        let after = &rest[i + 2..];
        match after.find('}') {
            Some(j) => {
                out.push(after[..j].trim());
                rest = &after[j + 1..];
            }
            None => break,
        }
    }
    out
}

/// Every reference inside a JSON value (strings at any depth).
pub fn refs_in_value(v: &Value) -> Vec<String> {
    match v {
        Value::String(s) => refs_in(s).into_iter().map(String::from).collect(),
        Value::Array(a) => a.iter().flat_map(refs_in_value).collect(),
        Value::Object(o) => o.values().flat_map(refs_in_value).collect(),
        _ => vec![],
    }
}

/// Resolve a string. A string that is exactly one reference keeps the referenced JSON
/// type; otherwise references are spliced in as text (strings bare, the rest as JSON).
pub fn resolve_str(s: &str, ctx: &Value) -> Value {
    let t = s.trim();
    if let Some(inner) = t.strip_prefix("${").and_then(|r| r.strip_suffix('}')) {
        if !inner.contains("${") && !inner.contains('}') {
            return select(ctx, inner.trim()).cloned().unwrap_or(Value::Null);
        }
    }
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find("${") {
        out.push_str(&rest[..i]);
        let after = &rest[i + 2..];
        let Some(j) = after.find('}') else {
            out.push_str(&rest[i..]);
            rest = "";
            break;
        };
        match select(ctx, after[..j].trim()) {
            Some(Value::String(v)) => out.push_str(v),
            Some(Value::Null) | None => {}
            Some(v) => out.push_str(&v.to_string()),
        }
        rest = &after[j + 1..];
    }
    out.push_str(rest);
    Value::String(out)
}

/// Resolve every string inside a JSON value.
pub fn resolve(v: &Value, ctx: &Value) -> Value {
    match v {
        Value::String(s) => resolve_str(s, ctx),
        Value::Array(a) => Value::Array(a.iter().map(|x| resolve(x, ctx)).collect()),
        Value::Object(o) => Value::Object(
            o.iter()
                .map(|(k, x)| (k.clone(), resolve(x, ctx)))
                .collect(),
        ),
        other => other.clone(),
    }
}

fn count(v: &Value) -> usize {
    match v {
        Value::Array(a) => a.len(),
        Value::Object(o) => o.len(),
        Value::Null => 0,
        Value::String(s) if s.is_empty() => 0,
        _ => 1,
    }
}

/// `None` when the assertion holds; otherwise why not.
pub fn assert_fails(v: &Value, a: &Assert) -> Option<String> {
    let got = select(v, &a.path).unwrap_or(&Value::Null);
    let name = a.label.as_deref().unwrap_or(&a.path);
    if let Some(e) = &a.eq {
        return (got != e).then(|| format!("{name}: expected {e}, got {got}"));
    }
    if let Some(e) = &a.ne {
        return (got == e).then(|| format!("{name}: expected anything but {e}"));
    }
    if let Some(n) = a.len {
        let c = count(got);
        return (c != n).then(|| format!("{name}: expected {n} elements, got {c}"));
    }
    if let Some(n) = a.min_len {
        let c = count(got);
        return (c < n).then(|| format!("{name}: expected at least {n} elements, got {c}"));
    }
    if let Some(want) = a.exists {
        let has = !got.is_null();
        return (has != want).then(|| {
            if want {
                format!("{name}: missing")
            } else {
                format!("{name}: expected absent, got {got}")
            }
        });
    }
    Some(format!("{name}: the assertion has no operator"))
}

/// All failures of a list of assertions, joined; `None` when all hold.
pub fn asserts_fail(v: &Value, asserts: &[Assert]) -> Option<String> {
    let bad: Vec<String> = asserts.iter().filter_map(|a| assert_fails(v, a)).collect();
    (!bad.is_empty()).then(|| bad.join("; "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn select_paths() {
        let v = json!({"a": {"b": [1, {"c": 2}], "l": [{"k": "x", "v": 1}, {"k": "y", "v": 2}]}});
        assert_eq!(select(&v, "a.b[1].c"), Some(&json!(2)));
        assert_eq!(select(&v, "a.l[k=y].v"), Some(&json!(2)));
        assert_eq!(select(&v, "a.zz"), None);
        assert_eq!(select(&v, ""), Some(&v));
    }

    #[test]
    fn whole_reference_keeps_type_and_splices_otherwise() {
        let ctx = json!({"run": "r-1", "inputs": {"n": 3, "l": [1, 2]}});
        assert_eq!(resolve_str("${inputs.l}", &ctx), json!([1, 2]));
        assert_eq!(resolve_str("${inputs.n}", &ctx), json!(3));
        assert_eq!(
            resolve_str("run ${run} n=${inputs.n}", &ctx),
            json!("run r-1 n=3")
        );
        assert_eq!(
            resolve(&json!({"x": "${run}", "y": ["${inputs.n}"]}), &ctx),
            json!({"x": "r-1", "y": [3]})
        );
        assert_eq!(refs_in("a ${x.y} b ${ z }"), vec!["x.y", "z"]);
    }

    #[test]
    fn assertion_operators() {
        let v = json!({"total": 0, "list": [1, 2], "s": "x"});
        let a = |f: fn(&mut Assert)| {
            let mut a = Assert {
                path: "total".into(),
                ..Default::default()
            };
            f(&mut a);
            a
        };
        assert!(assert_fails(&v, &a(|a| a.eq = Some(json!(0)))).is_none());
        assert!(assert_fails(&v, &a(|a| a.ne = Some(json!(0)))).is_some());
        assert!(assert_fails(
            &v,
            &a(|a| {
                a.path = "list".into();
                a.len = Some(2)
            })
        )
        .is_none());
        assert!(assert_fails(
            &v,
            &a(|a| {
                a.path = "list".into();
                a.min_len = Some(3)
            })
        )
        .is_some());
        assert!(assert_fails(
            &v,
            &a(|a| {
                a.path = "nope".into();
                a.exists = Some(false)
            })
        )
        .is_none());
        assert!(assert_fails(&v, &a(|a| a.exists = Some(true))).is_none());
    }
}
