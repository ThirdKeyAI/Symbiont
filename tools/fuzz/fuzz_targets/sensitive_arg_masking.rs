#![no_main]

use libfuzzer_sys::{arbitrary::Arbitrary, fuzz_target};
use symbi_runtime::integrations::mask_sensitive_arguments;

#[derive(Arbitrary, Debug)]
struct Input {
    /// Raw bytes that will be attempted as JSON first, fallback to constructed JSON.
    json_bytes: Vec<u8>,
    /// Parameter names to treat as sensitive.
    sensitive_params: Vec<String>,
    /// How deep to nest constructed JSON (clamped to 0..=5).
    nest_depth: u8,
}

/// Clamp a string to `max` bytes on a valid char boundary.
fn clamp(mut s: String, max: usize, fallback: &str) -> String {
    if s.is_empty() {
        return fallback.to_string();
    }
    if s.len() > max {
        let mut end = max;
        while !s.is_char_boundary(end) {
            end -= 1;
        }
        s.truncate(end);
    }
    s
}

const MAX_DEPTH: usize = 64;

/// Recursively walk `result` and assert that every sensitive key has been
/// replaced with exactly `[REDACTED:key_name]`.
fn assert_no_leak(result: &serde_json::Value, sensitive_params: &[String], depth: usize) {
    if depth >= MAX_DEPTH {
        return;
    }
    match result {
        serde_json::Value::Object(map) => {
            for (key, value) in map {
                if sensitive_params.iter().any(|p| p == key) {
                    // This key is sensitive -- its value MUST be exactly "[REDACTED:key]".
                    let expected = format!("[REDACTED:{}]", key);
                    assert_eq!(
                        value,
                        &serde_json::Value::String(expected.clone()),
                        "Sensitive key '{}' was not properly redacted: got {:?}, expected {:?}",
                        key,
                        value,
                        expected
                    );
                } else {
                    // Non-sensitive key -- recurse to check nested structures.
                    assert_no_leak(value, sensitive_params, depth + 1);
                }
            }
        }
        serde_json::Value::Array(arr) => {
            for item in arr {
                assert_no_leak(item, sensitive_params, depth + 1);
            }
        }
        _ => {}
    }
}

/// Build a nested JSON object from the sensitive param list.
fn build_nested_json(sensitive_params: &[String], depth: u8) -> serde_json::Value {
    let mut obj = serde_json::Map::new();

    for param in sensitive_params {
        obj.insert(
            param.clone(),
            serde_json::Value::String("secret_value".into()),
        );
    }

    obj.insert(
        "safe_key".to_string(),
        serde_json::Value::String("public_value".into()),
    );

    if depth > 0 {
        let nested = build_nested_json(sensitive_params, depth - 1);
        obj.insert("nested".to_string(), nested);

        let nested_arr = build_nested_json(sensitive_params, depth - 1);
        obj.insert(
            "items".to_string(),
            serde_json::Value::Array(vec![nested_arr]),
        );
    }

    serde_json::Value::Object(obj)
}

fuzz_target!(|input: Input| {
    // Clamp sensitive_params: max 8 params, each max 32 chars.
    let sensitive_params: Vec<String> = input
        .sensitive_params
        .into_iter()
        .take(8)
        .map(|s| clamp(s, 32, "secret"))
        .collect();

    // Clamp nest depth to 0..=5.
    let nest_depth = input.nest_depth.min(5);

    // Try to parse json_bytes as valid JSON; fall back to constructed JSON.
    let json_value =
        if let Ok(parsed) = serde_json::from_slice::<serde_json::Value>(&input.json_bytes) {
            parsed
        } else {
            build_nested_json(&sensitive_params, nest_depth)
        };

    // Call the function under test -- it must never panic.
    let result = mask_sensitive_arguments(&json_value, &sensitive_params);

    // Verify no sensitive value leaked through.
    assert_no_leak(&result, &sensitive_params, 0);
});
