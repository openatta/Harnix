//! Values made from a JSON Schema: what a dry run hands in for a task nobody mocked.

use serde_json::{json, Map, Value};

/// A small value meant to satisfy `schema`: its first `examples` entry, else its `default`, its
/// `const`, its first `enum` value; else one built from its type — an object with every
/// required property, an array of `minItems` items, the empty string (padded to `minLength`),
/// the lowest number allowed, `false`, `null`. `oneOf` / `anyOf` take their first branch, `allOf`
/// merges its branches' objects. A schema that says nothing gives `{}`.
///
/// Best effort, not a solver: a `pattern` or a `format` other than `date-time`, `date`,
/// `email` and `uri` is not honoured, and a value the schema refuses is possible.
pub fn minimal(schema: &Value) -> Value {
    minimal_at(schema, 0)
}

/// How deep a schema is followed; deeper parts are `null`.
const MAX_DEPTH: usize = 16;

fn minimal_at(schema: &Value, depth: usize) -> Value {
    if depth > MAX_DEPTH {
        return Value::Null;
    }
    let Some(o) = schema.as_object() else {
        // `true` (anything) or `false` (nothing): an empty object either way.
        return json!({});
    };
    if let Some(first) = o
        .get("examples")
        .and_then(Value::as_array)
        .and_then(|a| a.first())
    {
        return first.clone();
    }
    if let Some(d) = o.get("default") {
        return d.clone();
    }
    if let Some(c) = o.get("const") {
        return c.clone();
    }
    if let Some(first) = o
        .get("enum")
        .and_then(Value::as_array)
        .and_then(|a| a.first())
    {
        return first.clone();
    }
    for key in ["oneOf", "anyOf"] {
        if let Some(first) = o.get(key).and_then(Value::as_array).and_then(|a| a.first()) {
            return minimal_at(first, depth + 1);
        }
    }
    if let Some(all) = o.get("allOf").and_then(Value::as_array) {
        let mut merged = Map::new();
        let mut other = None;
        for branch in all {
            match minimal_at(branch, depth + 1) {
                Value::Object(m) => merged.extend(m),
                v => other = Some(v),
            }
        }
        // The schema's own keywords (beside `allOf`) count too.
        let mut rest = o.clone();
        rest.remove("allOf");
        if let Value::Object(m) = minimal_at(&Value::Object(rest), depth + 1) {
            merged.extend(m);
        }
        return if merged.is_empty() {
            other.unwrap_or(json!({}))
        } else {
            Value::Object(merged)
        };
    }
    match type_of(o) {
        Some("object") => object(o, depth),
        Some("array") => array(o, depth),
        Some("string") => string(o),
        Some("integer") => number(o, true),
        Some("number") => number(o, false),
        Some("boolean") => json!(false),
        Some("null") => Value::Null,
        _ => json!({}),
    }
}

/// The schema's type, or the one its keywords imply. Of a list, the first that is not `null`.
fn type_of(o: &Map<String, Value>) -> Option<&str> {
    match o.get("type") {
        Some(Value::String(t)) => Some(t.as_str()),
        Some(Value::Array(ts)) => ts
            .iter()
            .filter_map(Value::as_str)
            .find(|t| *t != "null")
            .or_else(|| ts.first().and_then(Value::as_str)),
        _ if o.contains_key("properties") || o.contains_key("required") => Some("object"),
        _ if o.contains_key("items") || o.contains_key("prefixItems") => Some("array"),
        _ => None,
    }
}

fn object(o: &Map<String, Value>, depth: usize) -> Value {
    let props = o.get("properties").and_then(Value::as_object);
    let mut out = Map::new();
    for k in o
        .get("required")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
    {
        let sub = props.and_then(|p| p.get(k)).cloned().unwrap_or(json!({}));
        out.insert(k.to_string(), minimal_at(&sub, depth + 1));
    }
    Value::Object(out)
}

fn array(o: &Map<String, Value>, depth: usize) -> Value {
    let min = o.get("minItems").and_then(Value::as_u64).unwrap_or(0) as usize;
    let prefix: Vec<Value> = o
        .get("prefixItems")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let items = o.get("items").cloned().unwrap_or(json!({}));
    let mut out = vec![];
    for i in 0..min.min(1000) {
        let sub = prefix.get(i).unwrap_or(&items);
        out.push(minimal_at(sub, depth + 1));
    }
    Value::Array(out)
}

fn string(o: &Map<String, Value>) -> Value {
    let base = match o.get("format").and_then(Value::as_str) {
        Some("date-time") => "1970-01-01T00:00:00Z",
        Some("date") => "1970-01-01",
        Some("email") => "a@example.com",
        Some("uri") => "https://example.com",
        _ => "",
    };
    let min = o.get("minLength").and_then(Value::as_u64).unwrap_or(0) as usize;
    let mut s = base.to_string();
    while s.chars().count() < min.min(10_000) {
        s.push('x');
    }
    Value::String(s)
}

fn number(o: &Map<String, Value>, integer: bool) -> Value {
    let f = |k: &str| o.get(k).and_then(Value::as_f64);
    let mut v = match (f("minimum"), f("exclusiveMinimum")) {
        (Some(m), _) => m,
        (None, Some(m)) => m + 1.0,
        (None, None) => 0.0,
    };
    if let Some(max) = f("maximum") {
        v = v.min(max);
    }
    if let Some(max) = f("exclusiveMaximum") {
        if v >= max {
            v = max - 1.0;
        }
    }
    if integer || v.fract() == 0.0 {
        json!(v.ceil() as i64)
    } else {
        json!(v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid(schema: &Value) {
        let v = minimal(schema);
        let validator = jsonschema::validator_for(schema).unwrap();
        assert!(validator.is_valid(&v), "{v} does not satisfy {schema}");
    }

    #[test]
    fn examples_default_const_and_enum_come_first() {
        assert_eq!(
            minimal(&json!({"type": "object", "examples": [{"a": 1}], "default": {"b": 2}})),
            json!({"a": 1})
        );
        assert_eq!(
            minimal(&json!({"type": "object", "default": {"b": 2}})),
            json!({"b": 2})
        );
        assert_eq!(minimal(&json!({"const": 7})), json!(7));
        assert_eq!(
            minimal(&json!({"enum": ["manual", "auto"]})),
            json!("manual")
        );
    }

    #[test]
    fn an_object_gets_its_required_properties_by_type() {
        let s = json!({
            "type": "object",
            "required": ["findings", "n", "ok", "name", "step", "kind"],
            "properties": {
                "findings": {"type": "array", "minItems": 1, "items": {"type": "object", "required": ["severity"], "properties": {"severity": {"enum": ["high", "low"]}}}},
                "n": {"type": "integer", "minimum": 3},
                "ok": {"type": "boolean"},
                "name": {"type": "string", "minLength": 2},
                "step": {"const": 2},
                "kind": {"type": ["null", "string"]},
                "optional": {"type": "string"}
            }
        });
        assert_eq!(
            minimal(&s),
            json!({"findings": [{"severity": "high"}], "n": 3, "ok": false, "name": "xx", "step": 2, "kind": ""})
        );
        valid(&s);
    }

    #[test]
    fn what_a_schema_does_not_say_is_an_empty_object() {
        assert_eq!(minimal(&json!({})), json!({}));
        assert_eq!(minimal(&json!({"type": "object"})), json!({}));
        assert_eq!(minimal(&json!(true)), json!({}));
        valid(&json!({"type": "object"}));
    }

    #[test]
    fn branches_and_bounds() {
        valid(&json!({"oneOf": [{"type": "string"}, {"type": "number"}]}));
        valid(&json!({"allOf": [{"type": "object", "required": ["a"]}, {"required": ["b"]}]}));
        valid(&json!({"type": "number", "exclusiveMinimum": 0.5}));
        valid(&json!({"type": "integer", "maximum": -4}));
        valid(
            &json!({"type": "array", "prefixItems": [{"type": "string"}, {"type": "integer"}], "minItems": 2}),
        );
        valid(&json!({"type": "string", "format": "date-time"}));
    }
}
