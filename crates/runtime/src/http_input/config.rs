//! HTTP Input configuration types
//!
//! This module defines configuration structures for HTTP input sources that invoke Symbiont agents.

#[cfg(feature = "http-input")]
use serde::{Deserialize, Serialize};

#[cfg(feature = "http-input")]
use crate::types::AgentId;

/// Configuration for HTTP input source that invokes Symbiont agents
#[cfg(feature = "http-input")]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpInputConfig {
    /// Address to bind the HTTP server (default: "127.0.0.1"; use "0.0.0.0" to listen on all interfaces)
    pub bind_address: String,

    /// Port number to listen on (e.g., 8081)
    pub port: u16,

    /// HTTP path to listen on (e.g., "/webhook", "/ingest")
    pub path: String,

    /// Default agent to invoke for all requests (can be overridden by routing rules)
    pub agent: AgentId,

    /// Optional static token for bearer authorization
    pub auth_header: Option<String>,

    /// Optional JWT public key path for JWT-based auth
    pub jwt_public_key_path: Option<String>,

    /// Maximum size of incoming request body in bytes
    pub max_body_bytes: usize,

    /// Max number of concurrent request-handling tasks
    pub concurrency: usize,

    /// Optional routing rules to determine agent by path or request content
    pub routing_rules: Option<Vec<AgentRoutingRule>>,

    /// Optional custom response formatters (agent → HTTP status + body)
    pub response_control: Option<ResponseControlConfig>,

    /// Optional headers to inject into agent input context
    pub forward_headers: Vec<String>,

    /// Optional CORS origin allow-list (empty = CORS disabled, `["*"]` = permissive)
    pub cors_origins: Vec<String>,

    /// Enable structured audit logging of all received events
    pub audit_enabled: bool,

    /// Webhook signature verification configuration.
    pub webhook_verify: Option<WebhookVerifyConfig>,
}

#[cfg(feature = "http-input")]
impl Default for HttpInputConfig {
    fn default() -> Self {
        Self {
            bind_address: "127.0.0.1".to_string(),
            port: 8081,
            path: "/webhook".to_string(),
            agent: AgentId::new(),
            auth_header: None,
            jwt_public_key_path: None,
            max_body_bytes: 65536, // 64 KB
            concurrency: 10,
            routing_rules: None,
            response_control: None,
            forward_headers: vec![],
            cors_origins: vec![],
            audit_enabled: true,
            webhook_verify: None,
        }
    }
}

/// Rule to route an HTTP request to a specific agent based on path, header, or body field
#[cfg(feature = "http-input")]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentRoutingRule {
    pub condition: RouteMatch,
    pub agent: AgentId,
}

/// Conditions for routing HTTP requests to specific agents
#[cfg(feature = "http-input")]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RouteMatch {
    /// Match by URL path prefix (e.g., "/api/github")
    PathPrefix(String),
    /// Match by header name and value (e.g., ("X-Source", "slack"))
    HeaderEquals(String, String),
    /// Match by JSON field name and value (e.g., ("source", "twilio"))
    JsonFieldEquals(String, String),
}

/// Control what the HTTP response should return after agent execution
#[cfg(feature = "http-input")]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponseControlConfig {
    /// Default HTTP status code for successful responses
    pub default_status: u16,
    /// Whether to include agent output as JSON in response body
    pub agent_output_to_json: bool,
    /// HTTP status code for error responses
    pub error_status: u16,
    /// Whether to echo the input request body on error
    pub echo_input_on_error: bool,
}

/// Configuration for webhook signature verification.
#[cfg(feature = "http-input")]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookVerifyConfig {
    /// Provider preset (github, stripe, slack, custom).
    pub provider: String,
    /// Secret for signature verification (can be a secret:// reference).
    pub secret: String,
}

#[cfg(feature = "http-input")]
impl Default for ResponseControlConfig {
    fn default() -> Self {
        Self {
            default_status: 200,
            agent_output_to_json: true,
            error_status: 500,
            echo_input_on_error: false,
        }
    }
}
