//! SSE transport for Composio MCP endpoints
//!
//! Implements a lightweight Server-Sent Events client over `reqwest` streaming,
//! exchanging JSON-RPC 2.0 messages with Composio's MCP servers.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use reqwest::header::{HeaderMap, HeaderValue};
use serde::{Deserialize, Serialize};

use super::error::ComposioError;

/// SSE transport that connects to a Composio MCP endpoint
pub struct SseTransport {
    client: reqwest::Client,
    endpoint_url: String,
    api_key: String,
    request_timeout: Duration,
    next_id: AtomicU64,
}

impl SseTransport {
    /// Create a new SSE transport for the given endpoint and API key.
    pub fn new(endpoint_url: String, api_key: String) -> Self {
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-api-key",
            HeaderValue::from_str(&api_key).unwrap_or_else(|_| HeaderValue::from_static("")),
        );

        let client = reqwest::Client::builder()
            .default_headers(headers)
            .build()
            .expect("failed to build reqwest client");

        Self {
            client,
            endpoint_url,
            api_key,
            request_timeout: Duration::from_secs(30),
            next_id: AtomicU64::new(1),
        }
    }

    /// Create a transport with a custom request timeout.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.request_timeout = timeout;
        self
    }

    /// Open an SSE connection and return a stream reader.
    pub async fn connect(&self) -> Result<SseConnection, ComposioError> {
        let response = self
            .client
            .get(&self.endpoint_url)
            .header("Accept", "text/event-stream")
            .send()
            .await?;

        let status = response.status();
        if !status.is_success() {
            return Err(ComposioError::TransportError {
                reason: format!("SSE connect returned HTTP {}", status),
            });
        }

        Ok(SseConnection {
            buffer: String::new(),
            body: response,
        })
    }

    /// Send a JSON-RPC request via HTTP POST and return the result.
    ///
    /// Composio's MCP endpoint returns SSE-formatted responses even for POST requests,
    /// so we parse `data:` lines from the response body to extract the JSON-RPC result.
    pub async fn request(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, ComposioError> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);

        let request = JsonRpcRequest {
            jsonrpc: "2.0",
            id,
            method: method.to_string(),
            params,
        };

        let response = self
            .client
            .post(&self.endpoint_url)
            .header("Accept", "application/json, text/event-stream")
            .json(&request)
            .timeout(self.request_timeout)
            .send()
            .await?;

        let status = response.status();
        if !status.is_success() {
            return Err(ComposioError::TransportError {
                reason: format!("JSON-RPC POST returned HTTP {}", status),
            });
        }

        // Composio returns SSE-formatted responses; extract JSON from data: lines
        let body = response.text().await?;
        let rpc_response = self.parse_sse_response(&body)?;

        if let Some(err) = rpc_response.error {
            return Err(ComposioError::JsonRpcError {
                code: err.code,
                message: err.message,
            });
        }

        rpc_response
            .result
            .ok_or_else(|| ComposioError::TransportError {
                reason: "JSON-RPC response has neither result nor error".to_string(),
            })
    }

    /// Parse a response body that may be plain JSON or SSE-formatted.
    ///
    /// SSE responses have lines like `event: message\ndata: {...}\n\n`.
    /// Plain JSON responses are just the JSON object directly.
    ///
    /// Composio may return either a full JSON-RPC response or just the
    /// result object (e.g. `{"result":{"tools":[...]}}`).
    fn parse_sse_response(&self, body: &str) -> Result<JsonRpcResponse, ComposioError> {
        // Try plain JSON-RPC response first
        if let Ok(resp) = serde_json::from_str::<JsonRpcResponse>(body) {
            return Ok(resp);
        }

        // Parse as SSE: collect all data: lines
        let mut data_parts: Vec<&str> = Vec::new();
        for line in body.lines() {
            if let Some(value) = line.strip_prefix("data:") {
                data_parts.push(value.trim_start());
            }
        }

        if data_parts.is_empty() {
            return Err(ComposioError::TransportError {
                reason: "response contains no JSON-RPC data".to_string(),
            });
        }

        let data = data_parts.join("\n");

        // Try parsing as full JSON-RPC response
        if let Ok(resp) = serde_json::from_str::<JsonRpcResponse>(&data) {
            return Ok(resp);
        }

        // Composio may return just the result object (no jsonrpc/id wrapper)
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(&data) {
            // Check for error field
            if let Some(err_obj) = value.get("error") {
                let code = err_obj.get("code").and_then(|v| v.as_i64()).unwrap_or(0);
                let message = err_obj
                    .get("message")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown error")
                    .to_string();
                return Ok(JsonRpcResponse {
                    jsonrpc: "2.0".to_string(),
                    id: 0,
                    result: None,
                    error: Some(JsonRpcErrorData { code, message }),
                });
            }

            // Treat the whole value or its "result" field as the result
            let result = if value.get("result").is_some() {
                value.get("result").cloned()
            } else {
                Some(value)
            };

            return Ok(JsonRpcResponse {
                jsonrpc: "2.0".to_string(),
                id: 0,
                result,
                error: None,
            });
        }

        Err(ComposioError::TransportError {
            reason: "failed to parse response from SSE data".to_string(),
        })
    }

    /// Returns the configured endpoint URL.
    pub fn endpoint_url(&self) -> &str {
        &self.endpoint_url
    }

    /// Returns the configured API key.
    pub fn api_key(&self) -> &str {
        &self.api_key
    }
}

/// A live SSE connection that yields events from the stream.
pub struct SseConnection {
    buffer: String,
    body: reqwest::Response,
}

impl SseConnection {
    /// Read the next SSE event from the stream.
    ///
    /// Returns `Ok(None)` when the stream ends.
    pub async fn next_event(&mut self) -> Result<Option<SseEvent>, ComposioError> {
        loop {
            // Try to extract a complete event from the buffer
            if let Some(event) = self.try_parse_event() {
                return Ok(Some(event));
            }

            // Read more data from the body
            let chunk = self.body.chunk().await?;
            match chunk {
                Some(bytes) => {
                    let text = String::from_utf8_lossy(&bytes);
                    self.buffer.push_str(&text);
                }
                None => {
                    // Stream ended — try to parse any remaining buffered data
                    if let Some(event) = self.try_parse_event() {
                        return Ok(Some(event));
                    }
                    return Ok(None);
                }
            }
        }
    }

    /// Try to parse a complete SSE event from the internal buffer.
    ///
    /// SSE events are delimited by blank lines (`\n\n`).
    fn try_parse_event(&mut self) -> Option<SseEvent> {
        let delimiter = "\n\n";
        let pos = self.buffer.find(delimiter)?;
        let raw_event = self.buffer[..pos].to_string();
        self.buffer = self.buffer[pos + delimiter.len()..].to_string();

        if raw_event.trim().is_empty() {
            return None;
        }

        Some(SseEvent::parse(&raw_event))
    }
}

/// A parsed Server-Sent Event
#[derive(Debug, Clone, PartialEq)]
pub struct SseEvent {
    /// The `event:` field (None if not specified)
    pub event_type: Option<String>,
    /// The `data:` field content (concatenated if multiple data lines)
    pub data: String,
    /// The `id:` field (None if not specified)
    pub id: Option<String>,
}

impl SseEvent {
    /// Parse an SSE event from raw text lines.
    ///
    /// Handles the standard SSE fields: `data:`, `event:`, `id:`.
    /// Multiple `data:` lines are joined with newlines.
    pub fn parse(raw: &str) -> Self {
        let mut event_type = None;
        let mut data_parts: Vec<&str> = Vec::new();
        let mut id = None;

        for line in raw.lines() {
            if let Some(value) = line.strip_prefix("data:") {
                data_parts.push(value.trim_start());
            } else if let Some(value) = line.strip_prefix("event:") {
                event_type = Some(value.trim_start().to_string());
            } else if let Some(value) = line.strip_prefix("id:") {
                id = Some(value.trim_start().to_string());
            }
            // Lines starting with `:` are comments, skip them
        }

        SseEvent {
            event_type,
            data: data_parts.join("\n"),
            id,
        }
    }
}

// --- JSON-RPC types ---

#[derive(Serialize)]
struct JsonRpcRequest {
    jsonrpc: &'static str,
    id: u64,
    method: String,
    params: serde_json::Value,
}

#[derive(Deserialize)]
struct JsonRpcResponse {
    #[allow(dead_code)]
    jsonrpc: String,
    #[allow(dead_code)]
    id: u64,
    result: Option<serde_json::Value>,
    error: Option<JsonRpcErrorData>,
}

#[derive(Deserialize)]
struct JsonRpcErrorData {
    code: i64,
    message: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_url_construction() {
        let transport = SseTransport::new(
            "https://backend.composio.dev/v3/mcp/srv_123?user_id=usr_456".to_string(),
            "test-key".to_string(),
        );
        assert_eq!(
            transport.endpoint_url(),
            "https://backend.composio.dev/v3/mcp/srv_123?user_id=usr_456"
        );
    }

    #[test]
    fn test_api_key_stored() {
        let transport =
            SseTransport::new("https://example.com".to_string(), "my-api-key".to_string());
        assert_eq!(transport.api_key(), "my-api-key");
    }

    #[test]
    fn test_sse_event_parse_data_only() {
        let raw = "data: {\"id\":1}";
        let event = SseEvent::parse(raw);
        assert_eq!(event.data, "{\"id\":1}");
        assert!(event.event_type.is_none());
        assert!(event.id.is_none());
    }

    #[test]
    fn test_sse_event_parse_full() {
        let raw = "event: message\nid: 42\ndata: hello world";
        let event = SseEvent::parse(raw);
        assert_eq!(event.event_type.as_deref(), Some("message"));
        assert_eq!(event.id.as_deref(), Some("42"));
        assert_eq!(event.data, "hello world");
    }

    #[test]
    fn test_sse_event_parse_multi_data() {
        let raw = "data: line1\ndata: line2\ndata: line3";
        let event = SseEvent::parse(raw);
        assert_eq!(event.data, "line1\nline2\nline3");
    }

    #[test]
    fn test_sse_event_parse_with_comment() {
        let raw = ": this is a comment\ndata: actual data";
        let event = SseEvent::parse(raw);
        assert_eq!(event.data, "actual data");
    }

    #[test]
    fn test_jsonrpc_request_serialization() {
        let req = JsonRpcRequest {
            jsonrpc: "2.0",
            id: 1,
            method: "tools/list".to_string(),
            params: serde_json::json!({}),
        };
        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["jsonrpc"], "2.0");
        assert_eq!(json["id"], 1);
        assert_eq!(json["method"], "tools/list");
    }

    #[test]
    fn test_jsonrpc_response_deserialization_success() {
        let json = r#"{"jsonrpc":"2.0","id":1,"result":{"tools":[]}}"#;
        let resp: JsonRpcResponse = serde_json::from_str(json).unwrap();
        assert!(resp.result.is_some());
        assert!(resp.error.is_none());
    }

    #[test]
    fn test_jsonrpc_response_deserialization_error() {
        let json =
            r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32600,"message":"Invalid Request"}}"#;
        let resp: JsonRpcResponse = serde_json::from_str(json).unwrap();
        assert!(resp.result.is_none());
        let err = resp.error.unwrap();
        assert_eq!(err.code, -32600);
        assert_eq!(err.message, "Invalid Request");
    }

    #[test]
    fn test_timeout_configuration() {
        let transport = SseTransport::new("https://example.com".to_string(), "key".to_string())
            .with_timeout(Duration::from_secs(60));
        assert_eq!(transport.request_timeout, Duration::from_secs(60));
    }

    #[test]
    fn test_parse_sse_response_plain_json() {
        let transport = SseTransport::new("https://example.com".to_string(), "key".to_string());
        let body = r#"{"jsonrpc":"2.0","id":1,"result":{"tools":[]}}"#;
        let resp = transport.parse_sse_response(body).unwrap();
        assert!(resp.result.is_some());
        assert!(resp.error.is_none());
    }

    #[test]
    fn test_parse_sse_response_composio_format() {
        let transport = SseTransport::new("https://example.com".to_string(), "key".to_string());
        let body = "event: message\ndata: {\"result\":{\"tools\":[{\"name\":\"TEST_TOOL\"}]}}";
        let resp = transport.parse_sse_response(body).unwrap();
        let result = resp.result.unwrap();
        let tools = result.get("tools").unwrap().as_array().unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0]["name"], "TEST_TOOL");
    }

    #[test]
    fn test_parse_sse_response_error() {
        let transport = SseTransport::new("https://example.com".to_string(), "key".to_string());
        let body = "data: {\"error\":{\"code\":-32000,\"message\":\"Not Acceptable\"}}";
        let resp = transport.parse_sse_response(body).unwrap();
        assert!(resp.error.is_some());
        let err = resp.error.unwrap();
        assert_eq!(err.code, -32000);
    }

    #[test]
    fn test_parse_sse_response_empty_body() {
        let transport = SseTransport::new("https://example.com".to_string(), "key".to_string());
        let result = transport.parse_sse_response("event: ping\n\n");
        assert!(result.is_err());
    }
}
