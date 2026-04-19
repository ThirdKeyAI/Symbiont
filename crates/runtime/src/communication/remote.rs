//! Remote communication bus implementation.
//!
//! Proxies [`CommunicationBus`] operations over HTTP to another `symbi up`
//! instance. This enables agents running in separate processes, containers,
//! or cloud services (Cloud Run, ECS) to communicate through the same
//! trait-based API used locally.
//!
//! **Trust model:** The remote bus relies on HTTPS + bearer token auth for
//! transport security. Bus-level encryption/signing is handled by the
//! *receiving* runtime (which has its own keys). Messages created via
//! `create_internal_message` on a `RemoteCommunicationBus` are shells with
//! empty signatures — the remote runtime re-signs them on receipt.
//!
//! **What's supported:**
//! - `send_message` → POST `/api/v1/agents/:recipient/messages`
//! - `receive_messages` → GET `/api/v1/agents/:agent_id/messages`
//! - `publish` → POST with topic set
//! - `get_delivery_status` → GET `/api/v1/messages/:id/status`
//! - `check_health` → GET `/api/v1/health`
//!
//! **What's not supported (returns error or no-op):**
//! - `subscribe`/`unsubscribe` — no REST endpoints yet; local subscriptions only
//! - `request` (request-response) — would require long polling or websockets
//! - `register_agent`/`unregister_agent` — managed through `/api/v1/agents` CRUD
//! - `shutdown` — no-op (caller doesn't own the remote runtime)

use async_trait::async_trait;
use bytes::Bytes;
use reqwest::{Client, Method};
use serde_json::Value;
use std::time::{Duration, SystemTime};
use uuid::Uuid;

use super::{CommunicationBus, DeliveryStatus};
use crate::types::{
    communication::{
        EncryptedPayload, EncryptionAlgorithm, MessageSignature, MessageType, SecureMessage,
        SignatureAlgorithm,
    },
    AgentId, CommunicationError, ComponentHealth, MessageId,
};

/// JWT provider for AgentPin-authenticated cross-runtime messaging.
///
/// Called on every outbound message to obtain a fresh AgentPin JWT for
/// the sending agent. Returning `Ok(None)` means "do not attach a JWT"
/// (useful for single-tenant setups that rely on TLS + bearer only);
/// returning `Err` aborts the send.
///
/// Kept async so implementations can mint short-lived JWTs on demand or
/// pull them from a per-agent cache with refresh semantics.
pub type JwtProvider = std::sync::Arc<
    dyn Fn(
            AgentId,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<Option<String>, String>> + Send + Sync>,
        > + Send
        + Sync,
>;

/// HTTP-backed implementation of [`CommunicationBus`] that proxies to a
/// remote `symbi up` runtime.
#[derive(Clone)]
pub struct RemoteCommunicationBus {
    client: Client,
    base_url: String,
    token: Option<String>,
    /// Local agent identity used as the default sender when forwarding
    /// messages whose sender was not explicitly set.
    local_agent_id: AgentId,
    /// Optional AgentPin JWT provider. When present, every outbound
    /// message is accompanied by a JWT minted for the message's sender,
    /// which the receiving runtime can verify via its AgentPin
    /// configuration to authenticate the cross-runtime origin.
    jwt_provider: Option<JwtProvider>,
}

/// Decide whether a base URL is safe to use for the remote bus.
///
/// Returns `Err(reason)` when the URL is an HTTP URL pointing at a
/// non-loopback host AND `SYMBIONT_REMOTE_BUS_REQUIRE_TLS=1`. Otherwise returns
/// `Ok(())`, but logs a warning for any non-TLS, non-loopback URL —
/// especially important when a bearer token is present, since it would be
/// sent in plaintext.
/// Parse a single wire envelope JSON value into a [`SecureMessage`].
///
/// Exposed (`pub`) so fuzz targets can drive this code path directly
/// without needing an HTTP server. Strict parsing: every field the
/// envelope format requires must be present and well-typed, otherwise
/// an `InvalidFormat` error is returned.
///
/// The emitted message has empty encryption/signature metadata because
/// the remote bus is a transport shim — the receiving runtime is
/// responsible for bus-level crypto on its side.
pub fn parse_envelope(m: &Value) -> Result<SecureMessage, CommunicationError> {
    // Strict parsing: required fields must be present and well-typed.
    // Silently defaulting missing fields masks upstream bugs and lets a
    // malicious/broken peer smuggle placeholder message IDs or senders
    // of Uuid::nil, which collide across messages.
    let message_id_str = m
        .get("message_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| CommunicationError::InvalidFormat("Missing message_id".to_string()))?;
    let id = Uuid::parse_str(message_id_str)
        .map_err(|e| CommunicationError::InvalidFormat(e.to_string()))?;

    let sender: AgentId = m
        .get("sender")
        .ok_or_else(|| CommunicationError::InvalidFormat("Missing sender".to_string()))
        .and_then(|v| {
            serde_json::from_value(v.clone())
                .map_err(|e| CommunicationError::InvalidFormat(e.to_string()))
        })?;

    let recipient: Option<AgentId> = match m.get("recipient") {
        None | Some(Value::Null) => None,
        Some(v) => Some(
            serde_json::from_value(v.clone())
                .map_err(|e| CommunicationError::InvalidFormat(e.to_string()))?,
        ),
    };

    let topic = m
        .get("topic")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let payload_str = m
        .get("payload")
        .and_then(|v| v.as_str())
        .ok_or_else(|| CommunicationError::InvalidFormat("Missing payload".to_string()))?;

    let message_type_str = m
        .get("message_type")
        .and_then(|v| v.as_str())
        .ok_or_else(|| CommunicationError::InvalidFormat("Missing message_type".to_string()))?;

    let message_type = match message_type_str {
        "direct" => MessageType::Direct(recipient.unwrap_or_default()),
        "publish" => MessageType::Publish(topic.clone().unwrap_or_default()),
        "subscribe" => MessageType::Subscribe(topic.clone().unwrap_or_default()),
        "broadcast" => MessageType::Broadcast,
        // Request/Response carry RequestId; we don't get those from the wire,
        // so synthesize a new one for client-side tracking.
        "request" => MessageType::Request(crate::types::RequestId::new()),
        "response" => MessageType::Response(crate::types::RequestId::new()),
        other => {
            return Err(CommunicationError::InvalidFormat(format!(
                "Unknown message_type: {}",
                other
            )))
        }
    };

    let ttl_seconds = m
        .get("ttl_seconds")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| CommunicationError::InvalidFormat("Missing ttl_seconds".to_string()))?;

    let timestamp_secs = m
        .get("timestamp_secs")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| CommunicationError::InvalidFormat("Missing timestamp_secs".to_string()))?;

    let timestamp = SystemTime::UNIX_EPOCH + Duration::from_secs(timestamp_secs);

    Ok(SecureMessage {
        id: MessageId(id),
        sender,
        recipient,
        topic,
        message_type,
        payload: EncryptedPayload {
            data: Bytes::from(payload_str.as_bytes().to_vec()),
            nonce: Vec::new(),
            encryption_algorithm: EncryptionAlgorithm::None,
        },
        signature: MessageSignature {
            signature: Vec::new(),
            algorithm: SignatureAlgorithm::None,
            public_key: Vec::new(),
        },
        ttl: Duration::from_secs(ttl_seconds),
        timestamp,
    })
}

/// Truncate a string to at most `limit` bytes on a UTF-8 char boundary,
/// appending an ellipsis if shortened.
fn truncate_for_error(s: &str, limit: usize) -> String {
    if s.len() <= limit {
        return s.to_string();
    }
    // Walk back from the limit to the nearest char boundary.
    let mut end = limit;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &s[..end])
}

fn check_base_url_security(base_url: &str, has_token: bool) -> Result<(), String> {
    let lowered = base_url.to_ascii_lowercase();
    let strict = std::env::var("SYMBIONT_REMOTE_BUS_REQUIRE_TLS")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);

    if lowered.starts_with("https://") {
        return Ok(());
    }

    if !lowered.starts_with("http://") {
        // Let reqwest surface the error at send time; this helper only
        // gates on the http/https distinction.
        return Ok(());
    }

    // http:// — inspect the host. Loopback is fine for dev.
    let host_and_rest = &base_url[7..];
    let host = host_and_rest
        .split_once('/')
        .map(|(h, _)| h)
        .unwrap_or(host_and_rest);
    let host = host.split_once('@').map(|(_, h)| h).unwrap_or(host);
    let host_only = host.rsplit_once(':').map(|(h, _)| h).unwrap_or(host);

    let is_loopback = matches!(host_only, "localhost" | "127.0.0.1" | "::1" | "[::1]")
        || host_only.starts_with("127.")
        || host_only.ends_with(".localhost");

    if is_loopback {
        return Ok(());
    }

    if strict {
        return Err(format!(
            "refusing non-TLS, non-loopback base_url {:?}; unset SYMBIONT_REMOTE_BUS_REQUIRE_TLS to allow",
            base_url
        ));
    }

    if has_token {
        tracing::error!(
            "RemoteCommunicationBus using plaintext HTTP with bearer token ({}). Traffic is observable and tokens can be stolen — switch to HTTPS.",
            base_url
        );
    } else {
        tracing::warn!(
            "RemoteCommunicationBus using plaintext HTTP ({}). Traffic is observable; prefer HTTPS.",
            base_url
        );
    }
    Ok(())
}

impl std::fmt::Debug for RemoteCommunicationBus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RemoteCommunicationBus")
            .field("base_url", &self.base_url)
            .field("has_token", &self.token.is_some())
            .field("local_agent_id", &self.local_agent_id)
            .field("has_jwt_provider", &self.jwt_provider.is_some())
            .finish()
    }
}

impl RemoteCommunicationBus {
    /// Construct a new remote bus.
    ///
    /// `base_url` should point to a reachable `symbi up` instance
    /// (e.g. `https://my-svc.run.app` or `http://localhost:8080` for
    /// local development). The trailing slash is trimmed.
    ///
    /// For plaintext `http://` URLs that are not loopback, a warning is
    /// logged because the bus's trust model relies on TLS to protect the
    /// bearer token and message bodies in transit. If
    /// `SYMBIONT_REMOTE_BUS_REQUIRE_TLS=1` is set in the environment, such
    /// URLs are rejected at construction time instead of warned.
    pub fn new(base_url: &str, token: Option<String>, local_agent_id: AgentId) -> Self {
        Self::try_new(base_url, token, local_agent_id).unwrap_or_else(|msg| {
            // Fall back to attempting construction anyway for backward
            // compatibility. Callers that want strict behavior should use
            // `try_new` directly. Log loudly so operators see it.
            tracing::error!(
                "RemoteCommunicationBus rejected URL, falling back to unsafe construction: {}",
                msg
            );
            Self::new_unchecked(base_url, None, local_agent_id)
        })
    }

    /// Fallible constructor that enforces TLS when
    /// `SYMBIONT_REMOTE_BUS_REQUIRE_TLS=1`. Always warns on non-loopback
    /// plaintext HTTP.
    pub fn try_new(
        base_url: &str,
        token: Option<String>,
        local_agent_id: AgentId,
    ) -> Result<Self, String> {
        let trimmed = base_url.trim_end_matches('/');
        check_base_url_security(trimmed, token.is_some())?;
        Ok(Self::new_unchecked(trimmed, token, local_agent_id))
    }

    /// Construct without running URL safety checks. Internal helper.
    fn new_unchecked(base_url: &str, token: Option<String>, local_agent_id: AgentId) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .unwrap_or_default();
        Self {
            client,
            base_url: base_url.trim_end_matches('/').to_string(),
            token,
            local_agent_id,
            jwt_provider: None,
        }
    }

    /// Install a JWT provider so every outbound message carries a fresh
    /// AgentPin credential for its sender. See [`JwtProvider`] for
    /// semantics. Returning the bus builder-style keeps construction
    /// chainable.
    pub fn with_jwt_provider(mut self, provider: JwtProvider) -> Self {
        self.jwt_provider = Some(provider);
        self
    }

    /// Fetch a JWT for `sender` via the configured provider, if any.
    /// Returns Ok(None) when no provider is configured.
    async fn fetch_jwt_for(&self, sender: AgentId) -> Result<Option<String>, CommunicationError> {
        let Some(ref provider) = self.jwt_provider else {
            return Ok(None);
        };
        provider(sender)
            .await
            .map_err(|reason| CommunicationError::DeliveryFailed {
                message_id: None,
                reason: format!("JWT provider failed for {}: {}", sender, reason).into_boxed_str(),
            })
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url, path)
    }

    async fn request_json(
        &self,
        method: Method,
        path: &str,
        body: Option<Value>,
    ) -> Result<Value, CommunicationError> {
        let mut req = self.client.request(method.clone(), self.url(path));
        if let Some(ref token) = self.token {
            req = req.bearer_auth(token);
        }
        if let Some(body) = body {
            req = req.json(&body);
        }

        let resp = req.send().await.map_err(|e| {
            CommunicationError::ConnectionFailed(format!("{} {}: {}", method, path, e))
        })?;

        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();

        if !status.is_success() {
            // Upstream error bodies can be large (HTML pages, traces); truncate
            // before echoing into our error chain so we don't log-bomb on a
            // single failed request. Leave `message_id` unset — we haven't
            // been assigned one by the remote bus.
            let snippet = truncate_for_error(&text, 512);
            return Err(CommunicationError::DeliveryFailed {
                message_id: None,
                reason: format!("HTTP {}: {}", status.as_u16(), snippet).into_boxed_str(),
            });
        }

        if text.is_empty() {
            return Ok(Value::Null);
        }

        serde_json::from_str(&text).map_err(|e| {
            let snippet = truncate_for_error(&text, 512);
            CommunicationError::InvalidFormat(format!("Parse error: {} (body: {})", e, snippet))
        })
    }
}

#[async_trait]
impl CommunicationBus for RemoteCommunicationBus {
    async fn send_message(&self, message: SecureMessage) -> Result<MessageId, CommunicationError> {
        let recipient = message.recipient.ok_or_else(|| {
            CommunicationError::InvalidFormat(
                "RemoteCommunicationBus requires an explicit recipient".to_string(),
            )
        })?;

        // Extract plaintext payload (remote bus doesn't use local encryption).
        let payload = String::from_utf8_lossy(&message.payload.data).to_string();

        // Topic is extracted from MessageType::Publish if present
        let topic = match &message.message_type {
            MessageType::Publish(t) => Some(t.clone()),
            _ => None,
        };

        // Mint an AgentPin JWT for this sender if a provider is configured.
        // The receiving runtime's AgentPin verifier (if enabled) will fail
        // the request when the JWT is missing or doesn't cover `sender`.
        let agentpin_jwt = self.fetch_jwt_for(message.sender).await?;

        let body = serde_json::json!({
            "sender": message.sender,
            "payload": payload,
            "ttl_seconds": message.ttl.as_secs(),
            "topic": topic,
            "agentpin_jwt": agentpin_jwt,
        });

        let path = format!("/api/v1/agents/{}/messages", recipient.0);
        let response = self.request_json(Method::POST, &path, Some(body)).await?;

        let message_id_str = response
            .get("message_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                CommunicationError::InvalidFormat("Missing message_id in response".to_string())
            })?;

        let uuid = Uuid::parse_str(message_id_str).map_err(|e| {
            CommunicationError::InvalidFormat(format!("Invalid message_id UUID: {}", e))
        })?;

        Ok(MessageId(uuid))
    }

    async fn receive_messages(
        &self,
        agent_id: AgentId,
    ) -> Result<Vec<SecureMessage>, CommunicationError> {
        let path = format!("/api/v1/agents/{}/messages", agent_id.0);
        let response = self.request_json(Method::GET, &path, None).await?;

        let messages_json = response
            .get("messages")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        let mut result = Vec::with_capacity(messages_json.len());
        for m in messages_json {
            result.push(parse_envelope(&m)?);
        }

        Ok(result)
    }

    async fn subscribe(
        &self,
        _agent_id: AgentId,
        _topic: String,
    ) -> Result<(), CommunicationError> {
        Err(CommunicationError::InvalidFormat(
            "subscribe is not supported on RemoteCommunicationBus (no REST endpoint yet)"
                .to_string(),
        ))
    }

    async fn unsubscribe(
        &self,
        _agent_id: AgentId,
        _topic: String,
    ) -> Result<(), CommunicationError> {
        Err(CommunicationError::InvalidFormat(
            "unsubscribe is not supported on RemoteCommunicationBus".to_string(),
        ))
    }

    async fn publish(
        &self,
        topic: String,
        message: SecureMessage,
    ) -> Result<(), CommunicationError> {
        // Publish is send_message with a topic — the remote runtime handles fan-out
        // to subscribed agents. We still need a recipient placeholder for the URL,
        // so use the sender as a pivot. The body's `topic` field overrides routing.
        let payload = String::from_utf8_lossy(&message.payload.data).to_string();
        let agentpin_jwt = self.fetch_jwt_for(message.sender).await?;
        let body = serde_json::json!({
            "sender": message.sender,
            "payload": payload,
            "ttl_seconds": message.ttl.as_secs(),
            "topic": topic,
            "agentpin_jwt": agentpin_jwt,
        });
        // For publish, route via the local agent's inbox — the remote runtime
        // uses the topic to fan out, ignoring the recipient in the path.
        let path = format!("/api/v1/agents/{}/messages", self.local_agent_id.0);
        let _ = self.request_json(Method::POST, &path, Some(body)).await?;
        Ok(())
    }

    async fn get_delivery_status(
        &self,
        message_id: MessageId,
    ) -> Result<DeliveryStatus, CommunicationError> {
        let path = format!("/api/v1/messages/{}/status", message_id.0);
        let response = self.request_json(Method::GET, &path, None).await?;

        let status_str = response
            .get("status")
            .and_then(|v| v.as_str())
            .unwrap_or("pending");

        Ok(match status_str {
            "delivered" => DeliveryStatus::Delivered,
            "failed" => DeliveryStatus::Failed,
            "expired" => DeliveryStatus::Expired,
            _ => DeliveryStatus::Pending,
        })
    }

    async fn register_agent(&self, _agent_id: AgentId) -> Result<(), CommunicationError> {
        // Remote agent registration happens via /api/v1/agents CRUD, not the bus.
        // The bus is just a message transport.
        Ok(())
    }

    async fn unregister_agent(&self, _agent_id: AgentId) -> Result<(), CommunicationError> {
        Ok(())
    }

    async fn request(
        &self,
        _target_agent: AgentId,
        _request_payload: Bytes,
        _timeout_duration: Duration,
    ) -> Result<Bytes, CommunicationError> {
        Err(CommunicationError::InvalidFormat(
            "request/response is not supported on RemoteCommunicationBus (use send_message + poll)"
                .to_string(),
        ))
    }

    async fn shutdown(&self) -> Result<(), CommunicationError> {
        // Caller doesn't own the remote runtime — no-op.
        Ok(())
    }

    async fn check_health(&self) -> Result<ComponentHealth, CommunicationError> {
        let mut req = self.client.get(self.url("/api/v1/health"));
        if let Some(ref token) = self.token {
            req = req.bearer_auth(token);
        }
        let resp = req
            .send()
            .await
            .map_err(|e| CommunicationError::ConnectionFailed(format!("health check: {}", e)))?;
        if resp.status().is_success() {
            Ok(ComponentHealth::healthy(Some(format!(
                "Remote bus connected to {}",
                self.base_url
            ))))
        } else {
            Err(CommunicationError::ConnectionFailed(format!(
                "Health check returned HTTP {}",
                resp.status().as_u16()
            )))
        }
    }

    fn create_internal_message(
        &self,
        sender: AgentId,
        recipient: AgentId,
        payload_data: Bytes,
        message_type: MessageType,
        ttl: Duration,
    ) -> SecureMessage {
        // Remote bus doesn't sign locally — the receiving runtime re-signs
        // on receipt. We produce a "transport-only" message.
        SecureMessage {
            id: MessageId::new(),
            sender,
            recipient: Some(recipient),
            topic: None,
            message_type,
            payload: EncryptedPayload {
                data: payload_data,
                nonce: Vec::new(),
                encryption_algorithm: EncryptionAlgorithm::None,
            },
            signature: MessageSignature {
                signature: Vec::new(),
                algorithm: SignatureAlgorithm::None,
                public_key: Vec::new(),
            },
            ttl,
            timestamp: SystemTime::now(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_trim_trailing_slash() {
        let bus =
            RemoteCommunicationBus::new("http://example.com:8080/", None, AgentId(Uuid::nil()));
        assert_eq!(bus.base_url, "http://example.com:8080");
    }

    #[test]
    fn test_url_construction() {
        let bus = RemoteCommunicationBus::new("http://localhost:8080", None, AgentId(Uuid::nil()));
        assert_eq!(
            bus.url("/api/v1/health"),
            "http://localhost:8080/api/v1/health"
        );
    }

    #[test]
    fn test_debug_hides_token() {
        let bus = RemoteCommunicationBus::new(
            "http://example.com",
            Some("super-secret".to_string()),
            AgentId(Uuid::nil()),
        );
        let debug_str = format!("{:?}", bus);
        assert!(!debug_str.contains("super-secret"));
        assert!(debug_str.contains("has_token: true"));
    }

    #[test]
    fn test_create_internal_message_has_empty_signature() {
        let bus = RemoteCommunicationBus::new("http://example.com", None, AgentId(Uuid::nil()));
        let sender = AgentId(Uuid::new_v4());
        let recipient = AgentId(Uuid::new_v4());
        let msg = bus.create_internal_message(
            sender,
            recipient,
            Bytes::from("hello"),
            MessageType::Direct(recipient),
            Duration::from_secs(60),
        );
        assert_eq!(msg.sender, sender);
        assert_eq!(msg.recipient, Some(recipient));
        assert!(msg.signature.signature.is_empty());
        assert!(matches!(msg.signature.algorithm, SignatureAlgorithm::None));
        assert!(matches!(
            msg.payload.encryption_algorithm,
            EncryptionAlgorithm::None
        ));
    }

    #[test]
    fn test_url_security_accepts_https() {
        assert!(check_base_url_security("https://example.com", true).is_ok());
    }

    #[test]
    fn test_url_security_accepts_loopback_http() {
        assert!(check_base_url_security("http://localhost:8080", false).is_ok());
        assert!(check_base_url_security("http://127.0.0.1", false).is_ok());
        assert!(check_base_url_security("http://[::1]:8080", false).is_ok());
    }

    #[test]
    #[serial_test::serial(remote_bus_tls_env)]
    fn test_url_security_strict_rejects_plaintext_public() {
        std::env::set_var("SYMBIONT_REMOTE_BUS_REQUIRE_TLS", "1");
        let r = check_base_url_security("http://public.example.com", true);
        std::env::remove_var("SYMBIONT_REMOTE_BUS_REQUIRE_TLS");
        assert!(r.is_err(), "public http with strict flag must be rejected");
    }

    #[test]
    #[serial_test::serial(remote_bus_tls_env)]
    fn test_url_security_lax_allows_plaintext_public() {
        // Default (strict flag unset) permits plaintext HTTP but logs loudly.
        std::env::remove_var("SYMBIONT_REMOTE_BUS_REQUIRE_TLS");
        let r = check_base_url_security("http://public.example.com", false);
        assert!(r.is_ok());
    }

    #[tokio::test]
    async fn test_unsupported_subscribe_returns_error() {
        let bus = RemoteCommunicationBus::new("http://example.com", None, AgentId(Uuid::nil()));
        let result = bus
            .subscribe(AgentId(Uuid::nil()), "topic".to_string())
            .await;
        assert!(result.is_err());
    }
}
