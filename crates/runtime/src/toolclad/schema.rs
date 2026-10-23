//! A small JSON-Schema subset validator for `[output.schema]` (ported from
//! the ToolClad reference runtime). Validation is fail-closed.
//!
//! Bundled, with no new dependency, so parsed output is checked against the
//! declared schema on every call. Supported keywords:
//!
//! * `type` — a name or an array of names among `object array string number
//!   integer boolean null`;
//! * `properties`, `required`, `additionalProperties` (boolean only);
//! * `items` — a single sub-schema applied to every element;
//! * `enum` — deep equality with one member;
//! * `minimum` / `maximum` (inclusive), `minLength` / `maxLength` (code
//!   points), `pattern` (anchored as `^(?:pat)$`, like every other pattern in
//!   ToolClad).
//!
//! Unknown keywords are ignored. `integer` accepts a number with no fractional
//! part (`30.0`). An error is `"<pointer>: <message>"`, for example
//! `/hosts/0/port: expected integer, got string`; the document root renders
//! as `/`. The first failure in document order is reported.

use super::validator::anchor_pattern;
use regex::Regex;
use serde_json::Value;

/// Validate `value` against `schema`, returning `"<pointer>: <message>"` for
/// the first failure.
pub fn validate(schema: &Value, value: &Value) -> Result<(), String> {
    let mut path = Vec::new();
    check(value, schema, &mut path)
}

fn type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(n) if n.is_i64() || n.is_u64() => "integer",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn matches_type(value: &Value, name: &str) -> bool {
    match name {
        "null" => value.is_null(),
        "boolean" => value.is_boolean(),
        "number" => value.is_number(),
        "integer" => match value {
            Value::Number(n) => {
                n.is_i64() || n.is_u64() || n.as_f64().is_some_and(|f| f.fract() == 0.0)
            }
            _ => false,
        },
        "string" => value.is_string(),
        "array" => value.is_array(),
        "object" => value.is_object(),
        _ => false,
    }
}

/// JSON equality: `true` is not `1`, and `1` equals `1.0`.
fn deep_equal(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => match (x.as_i64(), y.as_i64()) {
            (Some(x), Some(y)) => x == y,
            _ => x.as_f64() == y.as_f64(),
        },
        (Value::Array(x), Value::Array(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(a, b)| deep_equal(a, b))
        }
        (Value::Object(x), Value::Object(y)) => {
            x.len() == y.len()
                && x.iter()
                    .all(|(k, v)| y.get(k).is_some_and(|w| deep_equal(v, w)))
        }
        _ => a == b,
    }
}

fn escape(token: &str) -> String {
    token.replace('~', "~0").replace('/', "~1")
}

fn pointer(path: &[String]) -> String {
    if path.is_empty() {
        "/".to_string()
    } else {
        path.iter().map(|p| format!("/{p}")).collect()
    }
}

fn fmt_num(v: &Value) -> String {
    v.to_string()
}

fn check(value: &Value, schema: &Value, path: &mut Vec<String>) -> Result<(), String> {
    let Some(schema) = schema.as_object() else {
        return Ok(());
    };
    let fail = |path: &[String], msg: String| Err(format!("{}: {msg}", pointer(path)));

    if let Some(wanted) = schema.get("type") {
        let names: Vec<&Value> = match wanted {
            Value::Array(a) => a.iter().collect(),
            other => vec![other],
        };
        let ok = names
            .iter()
            .any(|n| n.as_str().is_some_and(|n| matches_type(value, n)));
        if !ok {
            let expected: Vec<String> = names
                .iter()
                .map(|n| n.as_str().map_or_else(|| n.to_string(), str::to_string))
                .collect();
            return fail(
                path,
                format!(
                    "expected {}, got {}",
                    expected.join(" or "),
                    type_name(value)
                ),
            );
        }
    }

    if let Some(Value::Array(members)) = schema.get("enum") {
        if !members.iter().any(|m| deep_equal(value, m)) {
            return fail(path, "value is not one of the allowed enum values".into());
        }
    }

    if let Some(n) = value.as_f64() {
        if let Some(min) = schema.get("minimum").filter(|m| m.is_number()) {
            if n < min.as_f64().unwrap_or(f64::NAN) {
                return fail(
                    path,
                    format!("{value} is less than the minimum {}", fmt_num(min)),
                );
            }
        }
        if let Some(max) = schema.get("maximum").filter(|m| m.is_number()) {
            if n > max.as_f64().unwrap_or(f64::NAN) {
                return fail(
                    path,
                    format!("{value} is greater than the maximum {}", fmt_num(max)),
                );
            }
        }
    }

    if let Value::String(s) = value {
        let len = s.chars().count() as f64;
        if let Some(min) = schema.get("minLength").and_then(Value::as_f64) {
            if len < min {
                return fail(
                    path,
                    format!(
                        "string is shorter than minLength {}",
                        fmt_num(&schema["minLength"])
                    ),
                );
            }
        }
        if let Some(max) = schema.get("maxLength").and_then(Value::as_f64) {
            if len > max {
                return fail(
                    path,
                    format!(
                        "string is longer than maxLength {}",
                        fmt_num(&schema["maxLength"])
                    ),
                );
            }
        }
        if let Some(Value::String(pat)) = schema.get("pattern") {
            match Regex::new(&anchor_pattern(pat)) {
                Ok(re) if re.is_match(s) => {}
                Ok(_) => return fail(path, format!("string does not match pattern '{pat}'")),
                Err(e) => return fail(path, format!("invalid pattern '{pat}': {e}")),
            }
        }
    }

    if let Value::Object(map) = value {
        if let Some(Value::Array(required)) = schema.get("required") {
            for name in required.iter().filter_map(Value::as_str) {
                if !map.contains_key(name) {
                    return fail(path, format!("missing required property '{name}'"));
                }
            }
        }
        let props = schema.get("properties").and_then(Value::as_object);
        let closed = schema.get("additionalProperties") == Some(&Value::Bool(false));
        for (key, item) in map {
            match props.and_then(|p| p.get(key)) {
                Some(sub) => {
                    path.push(escape(key));
                    check(item, sub, path)?;
                    path.pop();
                }
                None if closed => {
                    path.push(escape(key));
                    let r = fail(path, "additional property is not allowed".into());
                    path.pop();
                    return r;
                }
                None => {}
            }
        }
    }

    if let (Value::Array(items), Some(sub)) = (value, schema.get("items")) {
        if sub.is_object() {
            for (i, item) in items.iter().enumerate() {
                path.push(i.to_string());
                check(item, sub, path)?;
                path.pop();
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn err(schema: Value, value: Value) -> String {
        validate(&schema, &value).unwrap_err()
    }

    #[test]
    fn type_keyword() {
        assert!(validate(&json!({"type": "object"}), &json!({})).is_ok());
        assert_eq!(
            err(json!({"type": "object"}), json!([1])),
            "/: expected object, got array"
        );
        assert!(validate(&json!({"type": ["string", "null"]}), &json!(null)).is_ok());
        assert_eq!(
            err(json!({"type": ["string", "null"]}), json!(1)),
            "/: expected string or null, got integer"
        );
        assert!(validate(&json!({"type": "number"}), &json!(1)).is_ok());
        assert!(validate(&json!({"type": "boolean"}), &json!(1)).is_err());
    }

    #[test]
    fn integer_accepts_whole_float() {
        assert!(validate(&json!({"type": "integer"}), &json!(30.0)).is_ok());
        assert_eq!(
            err(json!({"type": "integer"}), json!(30.5)),
            "/: expected integer, got number"
        );
    }

    #[test]
    fn properties_required_and_pointer_path() {
        let schema = json!({
            "type": "object",
            "required": ["hosts"],
            "properties": {
                "hosts": {"type": "array", "items": {
                    "type": "object",
                    "properties": {"port": {"type": "integer"}}
                }}
            }
        });
        assert_eq!(
            err(
                schema.clone(),
                json!({"hosts": [{"port": 1}, {"port": "x"}]})
            ),
            "/hosts/1/port: expected integer, got string"
        );
        assert_eq!(
            err(schema, json!({})),
            "/: missing required property 'hosts'"
        );
        assert_eq!(
            err(
                json!({"properties": {"a/b": {"type": "string"}}}),
                json!({"a/b": 1})
            ),
            "/a~1b: expected string, got integer"
        );
    }

    #[test]
    fn additional_properties_false() {
        let schema = json!({"properties": {"a": {}}, "additionalProperties": false});
        assert!(validate(&schema, &json!({"a": 1})).is_ok());
        assert_eq!(
            err(schema, json!({"a": 1, "b": 2})),
            "/b: additional property is not allowed"
        );
        // Without the keyword, extra keys are fine.
        assert!(validate(&json!({"properties": {"a": {}}}), &json!({"b": 1})).is_ok());
    }

    #[test]
    fn nested_items() {
        let schema = json!({"items": {"items": {"type": "string"}}});
        assert!(validate(&schema, &json!([["a"], ["b", "c"]])).is_ok());
        assert_eq!(
            err(schema, json!([["a"], ["b", 3]])),
            "/1/1: expected string, got integer"
        );
    }

    #[test]
    fn enum_keyword() {
        let schema = json!({"enum": ["open", 1, {"a": [true]}]});
        assert!(validate(&schema, &json!("open")).is_ok());
        assert!(validate(&schema, &json!(1.0)).is_ok());
        assert!(validate(&schema, &json!({"a": [true]})).is_ok());
        assert!(validate(&schema, &json!(true)).is_err());
        assert!(validate(&schema, &json!("closed")).is_err());
    }

    #[test]
    fn numeric_bounds_inclusive() {
        let schema = json!({"minimum": 1, "maximum": 10});
        assert!(validate(&schema, &json!(1)).is_ok());
        assert!(validate(&schema, &json!(10)).is_ok());
        assert_eq!(
            err(schema.clone(), json!(0)),
            "/: 0 is less than the minimum 1"
        );
        assert_eq!(
            err(schema, json!(10.5)),
            "/: 10.5 is greater than the maximum 10"
        );
    }

    #[test]
    fn string_lengths_count_code_points() {
        let schema = json!({"minLength": 2, "maxLength": 3});
        assert!(validate(&schema, &json!("\u{e9}\u{e9}\u{e9}")).is_ok());
        assert!(validate(&schema, &json!("a")).is_err());
        assert!(validate(&schema, &json!("abcd")).is_err());
    }

    #[test]
    fn pattern_is_anchored() {
        let schema = json!({"pattern": "[a-z]+"});
        assert!(validate(&schema, &json!("abc")).is_ok());
        assert_eq!(
            err(schema.clone(), json!("abc1")),
            "/: string does not match pattern '[a-z]+'"
        );
        assert!(validate(&schema, &json!("1abc")).is_err());
        // Alternation stays inside the anchors.
        assert!(validate(&json!({"pattern": "a|b"}), &json!("ab")).is_err());
    }

    #[test]
    fn unknown_keywords_ignored_and_first_failure_reported() {
        assert!(validate(&json!({"format": "email", "x-foo": 1}), &json!("nope")).is_ok());
        let schema = json!({"properties": {"a": {"type": "string"}, "b": {"type": "string"}}});
        assert_eq!(
            err(schema, json!({"a": 1, "b": 2})),
            "/a: expected string, got integer"
        );
    }
}
