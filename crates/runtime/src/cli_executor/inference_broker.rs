//! Credential-isolated, fixed-destination inference for a contained CLI.

use crate::reasoning::{governed_session::GovernedToolSession, loop_types::LoopEvent};
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::UnixStream,
    sync::Mutex,
};
use zeroize::Zeroizing;

const REQUEST_LIMIT: usize = 1024 * 1024;
const RESPONSE_LIMIT: usize = 4 * 1024 * 1024;

/// Trusted project configuration; no field can be changed by the worker.
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct InferenceBrokerConfig {
    pub base_url: String,
    pub model: String,
    /// Name of a host-side variable, read only by the trusted launcher.
    pub api_key_env: String,
    #[serde(default = "default_requests")]
    pub max_requests: u32,
    #[serde(default = "default_output")]
    pub max_output_tokens_per_request: u64,
    #[serde(default = "default_timeout")]
    pub request_timeout_seconds: u64,
    #[serde(default)]
    pub beta_headers: Vec<String>,
}
fn default_requests() -> u32 {
    32
}
fn default_output() -> u64 {
    4096
}
fn default_timeout() -> u64 {
    60
}

impl InferenceBrokerConfig {
    pub fn from_project(project: &std::path::Path) -> Result<Self, String> {
        let path = project.join("symbiont.toml");
        if std::fs::metadata(&path)
            .map_err(|_| "managed CLI requires symbiont.toml")?
            .len()
            > REQUEST_LIMIT as u64
        {
            return Err("managed CLI configuration exceeds its size bound".into());
        }
        let text =
            std::fs::read_to_string(path).map_err(|_| "cannot read managed CLI configuration")?;
        let root: toml::Value =
            toml::from_str(&text).map_err(|_| "invalid managed CLI configuration")?;
        let value = root.get("managed_cli").and_then(|v| v.get("inference"))
            .ok_or("managed CLI requires [managed_cli.inference] with an explicit endpoint, model and api_key_env")?;
        let config: Self = value
            .clone()
            .try_into()
            .map_err(|e| format!("invalid managed inference configuration: {e}"))?;
        config.validate()?;
        Ok(config)
    }

    /// Validate before serializing configuration into policy or audit records.
    pub fn validate(&self) -> Result<(), String> {
        if self.api_key_env.is_empty()
            || self.api_key_env.len() > 128
            || !self
                .api_key_env
                .bytes()
                .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
        {
            return Err("api_key_env must name an explicit uppercase environment variable".into());
        }
        let base = url::Url::parse(&self.base_url).map_err(|_| "invalid inference base URL")?;
        if !matches!(base.scheme(), "http" | "https")
            || base.host_str().is_none()
            || !base.username().is_empty()
            || base.password().is_some()
            || base.query().is_some()
            || base.fragment().is_some()
            || self.model.is_empty()
            || self.model.len() > 128
            || self.model.bytes().any(|b| b.is_ascii_control())
            || self.max_requests == 0
            || self.max_requests > 128
            || self.max_output_tokens_per_request == 0
            || self.max_output_tokens_per_request > 32768
            || self.request_timeout_seconds == 0
            || self.request_timeout_seconds > 120
            || self.beta_headers.len() > 8
            || self.beta_headers.iter().any(|v| {
                v.is_empty()
                    || v.len() > 128
                    || !v.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
            })
        {
            return Err("invalid protected inference configuration".into());
        }
        Ok(())
    }
}

struct Budget {
    requests: u32,
    remaining_output: u64,
}

/// The credential never enters a child environment, mount, journal or URL.
/// Output allowance is reserved before a request and is not refunded after
/// interruption or errors. Request/response bytes, count and time are bounded.
pub struct ProtectedInference {
    config: InferenceBrokerConfig,
    client: reqwest::Client,
    credential: Zeroizing<String>,
    messages_url: url::Url,
    count_url: url::Url,
    deadline: Instant,
    budget: Mutex<Budget>,
    session: Arc<GovernedToolSession>,
}

impl ProtectedInference {
    pub fn config(&self) -> &InferenceBrokerConfig {
        &self.config
    }
    pub fn new(
        config: InferenceBrokerConfig,
        credential: String,
        output_budget: u64,
        deadline: Instant,
        session: Arc<GovernedToolSession>,
    ) -> Result<Self, String> {
        config.validate()?;
        if output_budget == 0
            || output_budget > 1_000_000
            || credential.is_empty()
            || credential.len() > 4096
            || credential.bytes().any(|b| b.is_ascii_control())
            || deadline <= Instant::now()
        {
            return Err("invalid protected inference session configuration".into());
        }
        let messages_url = url::Url::parse(&format!(
            "{}/v1/messages",
            config.base_url.trim_end_matches('/')
        ))
        .map_err(|_| "invalid inference URL")?;
        let count_url = url::Url::parse(&format!("{}/count_tokens", messages_url))
            .map_err(|_| "invalid token count URL")?;
        // This endpoint is explicit trusted operator configuration, including
        // local inference. Redirects and ambient proxy discovery stay disabled.
        let client = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(10))
            .build()
            .map_err(|_| "cannot build inference client")?;
        Ok(Self {
            config,
            client,
            credential: Zeroizing::new(credential),
            messages_url,
            count_url,
            deadline,
            session,
            budget: Mutex::new(Budget {
                requests: 0,
                remaining_output: output_budget,
            }),
        })
    }

    async fn forward(
        &self,
        path: &str,
        body: &[u8],
    ) -> Result<(u16, &'static str, Vec<u8>), String> {
        let counting = match path {
            "/v1/messages" | "/v1/messages?beta=true" => false,
            "/v1/messages/count_tokens" | "/v1/messages/count_tokens?beta=true" => true,
            _ => return Err("inference route is unavailable".into()),
        };
        let value: Value = serde_json::from_slice(body).map_err(|_| "invalid inference JSON")?;
        let fields = value
            .as_object()
            .ok_or("inference body must be an object")?;
        // Do not enable provider-hosted tools, remote MCP, containers, advisor
        // services or future fields with their own effects or credentials.
        let allowed = [
            "model",
            "messages",
            "system",
            "tools",
            "tool_choice",
            "max_tokens",
            "stream",
            "temperature",
            "top_p",
            "top_k",
            "stop_sequences",
            "metadata",
            "thinking",
            "output_config",
            "cache_control",
            "context_management",
        ];
        if let Some(field) = fields.keys().find(|key| !allowed.contains(&key.as_str())) {
            return Err(format!(
                "inference field is unavailable: {:?}",
                crate::text_util::truncate_utf8(field, 128)
            ));
        }
        if value["model"].as_str() != Some(self.config.model.as_str())
            || !value["messages"].is_array()
        {
            return Err("inference request differs from the configured contract".into());
        }
        if let Some(tools) = fields.get("tools") {
            let tools = tools.as_array().ok_or("tools must be an array")?;
            if tools.len() > 256
                || tools.iter().any(|tool| {
                    !tool.is_object()
                        || tool.get("type").is_some_and(|kind| kind != "custom")
                        || tool.get("name").and_then(Value::as_str).is_none()
                        || !tool.get("input_schema").is_some_and(Value::is_object)
                })
            {
                return Err("only client-executed custom tools are available".into());
            }
        }
        if let Some(context) = fields.get("context_management") {
            validate_context_management(context)?;
        }
        reject_remote_sources(&value)?;
        let output = if counting {
            0
        } else {
            let output = value["max_tokens"]
                .as_u64()
                .ok_or("max_tokens is required")?;
            if output == 0 || output > self.config.max_output_tokens_per_request {
                return Err("inference output exceeds its configured bound".into());
            }
            output
        };
        // Serialize the reservation and upstream request, including queued work
        // in the outer connection deadline. Uncertain calls consume allowance.
        let mut budget = self.budget.lock().await;
        if budget.requests >= self.config.max_requests || output > budget.remaining_output {
            return Err("inference session budget exhausted".into());
        }
        budget.requests += 1;
        budget.remaining_output -= output;
        let endpoint = if counting {
            &self.count_url
        } else {
            &self.messages_url
        };
        let canonical_body =
            serde_json::to_vec(&value).map_err(|_| "cannot encode inference request")?;
        let mut in_flight = self.session.external_guard();
        let request_id = uuid::Uuid::new_v4().to_string();
        self.session
            .record_external(LoopEvent::InferenceRequested {
                request_id: request_id.clone(),
                endpoint: endpoint.to_string(),
                model: self.config.model.clone(),
                request_hash: hex::encode(Sha256::digest(&canonical_body)),
                reserved_output_tokens: output,
            })
            .await?;
        let mut request = self
            .client
            .post(endpoint.clone())
            .header("x-api-key", self.credential.as_str())
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .body(canonical_body);
        if !self.config.beta_headers.is_empty() {
            request = request.header("anthropic-beta", self.config.beta_headers.join(","));
        }
        let response = request
            .send()
            .await
            .map_err(|_| "inference upstream request failed")?;
        let status = response.status().as_u16();
        if !(200..300).contains(&status) {
            // Error bodies and headers may echo credentials or contain redirects.
            let body = serde_json::to_vec(&json!({"type":"error","error":{"type":"api_error","message":"configured inference upstream rejected the request"}})).unwrap();
            self.record_completed(request_id, status, &body).await?;
            in_flight.complete();
            return Ok((status, "application/json", body));
        }
        let content_type = if response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.starts_with("text/event-stream"))
        {
            "text/event-stream"
        } else {
            "application/json"
        };
        let mut stream = response.bytes_stream();
        let mut output = Vec::new();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|_| "inference response stream failed")?;
            if output.len().saturating_add(chunk.len()) > RESPONSE_LIMIT {
                return Err("inference response exceeds its byte bound".into());
            }
            output.extend_from_slice(&chunk);
        }
        if output
            .windows(self.credential.len())
            .any(|part| part == self.credential.as_bytes())
        {
            self.session
                .cancel_with_reason("inference response contains protected credentials");
            return Err("inference response contains protected credentials".into());
        }
        self.record_completed(request_id, status, &output).await?;
        in_flight.complete();
        Ok((status, content_type, output))
    }

    async fn record_completed(
        &self,
        request_id: String,
        status: u16,
        body: &[u8],
    ) -> Result<(), String> {
        self.session
            .record_external(LoopEvent::InferenceCompleted {
                request_id,
                status,
                response_hash: hex::encode(Sha256::digest(body)),
                response_bytes: body.len() as u64,
            })
            .await
    }
}

// Permit the CLI's thinking-history retention hint only. Compaction may
// initiate additional inference and future context strategies are not implicit.
fn validate_context_management(context: &Value) -> Result<(), String> {
    let object = context.as_object().ok_or("invalid context management")?;
    let edits = object
        .get("edits")
        .and_then(Value::as_array)
        .ok_or("invalid context management edits")?;
    if object.len() != 1
        || edits.len() > 1
        || edits.iter().any(|edit| {
            let Some(fields) = edit.as_object() else {
                return true;
            };
            fields
                .keys()
                .any(|key| !matches!(key.as_str(), "type" | "keep"))
                || edit["type"] != "clear_thinking_20251015"
                || fields.get("keep").is_some_and(|keep| keep != "all")
        })
    {
        return Err("context management strategy is unavailable".into());
    }
    Ok(())
}

fn reject_remote_sources(value: &Value) -> Result<(), String> {
    match value {
        Value::Object(map) => {
            if map
                .get("source")
                .and_then(|v| v.get("type"))
                .is_some_and(|kind| kind == "url")
            {
                return Err("remote document and image sources are unavailable".into());
            }
            for value in map.values() {
                reject_remote_sources(value)?;
            }
        }
        Value::Array(values) => {
            for value in values {
                reject_remote_sources(value)?;
            }
        }
        _ => {}
    }
    Ok(())
}

pub(crate) async fn connection(
    stream: UnixStream,
    inference: Arc<ProtectedInference>,
) -> Result<(), String> {
    let timeout = Duration::from_secs(inference.config.request_timeout_seconds)
        .min(inference.deadline.saturating_duration_since(Instant::now()));
    tokio::time::timeout(timeout, async {
        let mut reader = BufReader::new(stream);
        let mut headers = Vec::new();
        loop {
            let start = headers.len();
            let read = (&mut reader).take(16385u64.saturating_sub(headers.len() as u64)).read_until(b'\n', &mut headers).await.map_err(|_| "inference header read failed")?;
            if read == 0 || headers.len() > 16384 { return Err("invalid or oversized inference headers".into()); }
            if &headers[start..] == b"\r\n" { break; }
        }
        let text = std::str::from_utf8(&headers).map_err(|_| "invalid inference headers")?;
        let mut lines = text.split("\r\n");
        let request: Vec<_> = lines.next().unwrap_or_default().split(' ').collect();
        if request.len() != 3 || request[0] != "POST" || request[2] != "HTTP/1.1" { return Err("inference requires a POST request".into()); }
        let mut length = None;
        for line in lines.filter(|line| !line.is_empty()) {
            let (name, value) = line.split_once(':').ok_or("invalid inference header")?;
            if name.eq_ignore_ascii_case("transfer-encoding") || name.eq_ignore_ascii_case("expect") { return Err("unsupported inference framing".into()); }
            if name.eq_ignore_ascii_case("content-length") {
                if length.is_some() { return Err("duplicate inference content length".into()); }
                length = Some(value.trim().parse::<usize>().map_err(|_| "invalid inference content length")?);
            }
        }
        let length = length.filter(|length| *length > 0 && *length <= REQUEST_LIMIT).ok_or("inference request exceeds its byte bound")?;
        let mut body = vec![0; length];
        reader.read_exact(&mut body).await.map_err(|_| "incomplete inference request")?;
        let (status, content_type, body) = match inference.forward(request[1], &body).await {
            Ok(response) => response,
            Err(error) => (400, "application/json", serde_json::to_vec(&json!({"type":"error","error":{"type":"invalid_request_error","message":error}})).unwrap()),
        };
        let headers = format!("HTTP/1.1 {status} Broker response\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
        reader.get_mut().write_all(headers.as_bytes()).await.map_err(|_| "inference response write failed")?;
        reader.get_mut().write_all(&body).await.map_err(|_| "inference response write failed")?;
        Ok(())
    }).await.map_err(|_| "inference request deadline exceeded")?
}
