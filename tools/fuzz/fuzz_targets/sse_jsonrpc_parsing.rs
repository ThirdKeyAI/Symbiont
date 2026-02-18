#![no_main]

//! Fuzz target for SSE event parsing and JSON-RPC response deserialization.
//!
//! Exercises `SseEvent::parse` with arbitrary inputs and attempts
//! `serde_json::from_str` for JSON-RPC-shaped structures to ensure
//! neither path panics on adversarial data.

use libfuzzer_sys::{arbitrary::Arbitrary, fuzz_target};
use symbi_runtime::integrations::composio::SseEvent;

// ── Clamp helper (reused pattern from other fuzz targets) ─────────────────

/// Clamp a string to `max` bytes, respecting char boundaries.
/// Returns `fallback` when the string is empty.
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

// ── Structured SSE fragment ───────────────────────────────────────────────

#[derive(Arbitrary, Debug)]
struct SseFragment {
    /// Which SSE field to emit
    field: SseField,
    /// The value portion after the colon
    value: String,
}

#[derive(Arbitrary, Debug)]
enum SseField {
    Data,
    Event,
    Id,
    Comment,
    /// Arbitrary field name (e.g. "retry:", "bogus:", or something that looks
    /// like another field embedded inside data).
    Custom(String),
}

// ── Structured JSON-RPC fields ────────────────────────────────────────────

#[derive(Arbitrary, Debug)]
struct JsonRpcFields {
    jsonrpc: Option<String>,
    id: Option<u64>,
    method: Option<String>,
    has_result: bool,
    result_json: Option<String>,
    has_error: bool,
    error_code: Option<i64>,
    error_message: Option<String>,
    extra_fields: Vec<(String, String)>,
}

// ── Top-level fuzz input ──────────────────────────────────────────────────

#[derive(Arbitrary, Debug)]
struct Input {
    mode: FuzzMode,
}

#[derive(Arbitrary, Debug)]
enum FuzzMode {
    /// Feed completely raw bytes to SseEvent::parse
    SseRaw(String),
    /// Build SSE text from structured fragments, then parse
    SseStructured { fragments: Vec<SseFragment> },
    /// Feed a raw string to serde_json for JSON-RPC deserialization
    JsonRpcRaw(String),
    /// Build a JSON-RPC-like object from structured fields, then deserialize
    JsonRpcStructured(JsonRpcFields),
    /// Combine: wrap structured JSON-RPC inside SSE data: lines, then parse both
    Combined {
        sse_fragments: Vec<SseFragment>,
        jsonrpc_fields: JsonRpcFields,
    },
}

// ── JSON-RPC response type (mirrors the private type in transport.rs) ─────

#[derive(serde::Deserialize, Debug)]
#[allow(dead_code)]
struct JsonRpcResponse {
    jsonrpc: String,
    id: u64,
    result: Option<serde_json::Value>,
    error: Option<JsonRpcErrorData>,
}

#[derive(serde::Deserialize, Debug)]
#[allow(dead_code)]
struct JsonRpcErrorData {
    code: i64,
    message: String,
}

// ── Helpers ───────────────────────────────────────────────────────────────

/// Render a single SSE fragment as a text line.
fn render_fragment(frag: &SseFragment, out: &mut String) {
    let value = clamp(frag.value.clone(), 4096, "");
    match &frag.field {
        SseField::Data => {
            out.push_str("data: ");
            out.push_str(&value);
        }
        SseField::Event => {
            out.push_str("event: ");
            out.push_str(&value);
        }
        SseField::Id => {
            out.push_str("id: ");
            out.push_str(&value);
        }
        SseField::Comment => {
            out.push(':');
            out.push_str(&value);
        }
        SseField::Custom(name) => {
            let name = clamp(name.clone(), 64, "x-custom");
            out.push_str(&name);
            out.push_str(": ");
            out.push_str(&value);
        }
    }
    out.push('\n');
}

/// Build SSE text from a list of structured fragments.
fn build_sse_text(fragments: &[SseFragment]) -> String {
    // Limit number of fragments to avoid excessive memory use
    let limit = fragments.len().min(1024);
    let mut text = String::new();
    for frag in &fragments[..limit] {
        render_fragment(frag, &mut text);
    }
    text
}

/// Build a JSON string from structured JSON-RPC fields.
fn build_jsonrpc_json(fields: &JsonRpcFields) -> String {
    let mut obj = serde_json::Map::new();

    if let Some(ref v) = fields.jsonrpc {
        let v = clamp(v.clone(), 32, "2.0");
        obj.insert("jsonrpc".to_string(), serde_json::Value::String(v));
    }

    if let Some(id) = fields.id {
        obj.insert("id".to_string(), serde_json::Value::Number(id.into()));
    }

    if let Some(ref m) = fields.method {
        let m = clamp(m.clone(), 256, "tools/list");
        obj.insert("method".to_string(), serde_json::Value::String(m));
    }

    if fields.has_result {
        let result_val = fields
            .result_json
            .as_ref()
            .and_then(|s| {
                let s = clamp(s.clone(), 4096, "{}");
                serde_json::from_str::<serde_json::Value>(&s).ok()
            })
            .unwrap_or(serde_json::Value::Object(serde_json::Map::new()));
        obj.insert("result".to_string(), result_val);
    }

    if fields.has_error {
        let mut err_obj = serde_json::Map::new();
        if let Some(code) = fields.error_code {
            err_obj.insert("code".to_string(), serde_json::Value::Number(code.into()));
        }
        if let Some(ref msg) = fields.error_message {
            let msg = clamp(msg.clone(), 256, "error");
            err_obj.insert("message".to_string(), serde_json::Value::String(msg));
        }
        obj.insert(
            "error".to_string(),
            serde_json::Value::Object(err_obj),
        );
    }

    // Inject extra fields (tests that unknown keys don't cause panics)
    for (k, v) in fields.extra_fields.iter().take(16) {
        let k = clamp(k.clone(), 64, "extra");
        let v = clamp(v.clone(), 256, "value");
        obj.insert(k, serde_json::Value::String(v));
    }

    serde_json::to_string(&serde_json::Value::Object(obj)).unwrap_or_default()
}

// ── Fuzz target ───────────────────────────────────────────────────────────

fuzz_target!(|input: Input| {
    match input.mode {
        // ── 1. Raw SSE parsing ────────────────────────────────────────
        FuzzMode::SseRaw(raw) => {
            let raw = clamp(raw, 65536, "data: hello");
            // Must never panic, regardless of input.
            let event = SseEvent::parse(&raw);
            // Sanity: returned struct is well-formed (fields are just strings).
            let _ = event.event_type;
            let _ = event.data;
            let _ = event.id;
        }

        // ── 2. Structured SSE parsing ─────────────────────────────────
        FuzzMode::SseStructured { fragments } => {
            let text = build_sse_text(&fragments);
            let event = SseEvent::parse(&text);
            let _ = event.event_type;
            let _ = event.data;
            let _ = event.id;
        }

        // ── 3. Raw JSON-RPC deserialization ───────────────────────────
        FuzzMode::JsonRpcRaw(raw) => {
            let raw = clamp(raw, 65536, "{}");
            // Attempt deserialization of the full JSON-RPC response type.
            // Either succeeds or returns Err — must never panic.
            let _ = serde_json::from_str::<JsonRpcResponse>(&raw);
            // Also try deserializing as a generic Value (mirrors what
            // parse_sse_response does as a fallback).
            let _ = serde_json::from_str::<serde_json::Value>(&raw);
        }

        // ── 4. Structured JSON-RPC deserialization ────────────────────
        FuzzMode::JsonRpcStructured(fields) => {
            let json = build_jsonrpc_json(&fields);
            let _ = serde_json::from_str::<JsonRpcResponse>(&json);
            let _ = serde_json::from_str::<serde_json::Value>(&json);
        }

        // ── 5. Combined: JSON-RPC wrapped in SSE data: lines ──────────
        FuzzMode::Combined {
            sse_fragments,
            jsonrpc_fields,
        } => {
            // First: parse SSE text from fragments
            let sse_text = build_sse_text(&sse_fragments);
            let event = SseEvent::parse(&sse_text);
            let _ = event.event_type;
            let _ = event.id;

            // Second: build JSON-RPC, embed as data: lines in SSE
            let json = build_jsonrpc_json(&jsonrpc_fields);
            let sse_with_json = format!("event: message\ndata: {}\n", json);
            let event2 = SseEvent::parse(&sse_with_json);
            // The data field should contain the JSON; try to deserialize it.
            let _ = serde_json::from_str::<JsonRpcResponse>(&event2.data);
            let _ = serde_json::from_str::<serde_json::Value>(&event2.data);

            // Third: parse the raw event data from step 1 as JSON too
            // (it is likely not valid JSON, but must not panic).
            let _ = serde_json::from_str::<JsonRpcResponse>(&event.data);
            let _ = serde_json::from_str::<serde_json::Value>(&event.data);
        }
    }
});
