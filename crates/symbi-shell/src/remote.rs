//! Remote attach: HTTP client for connecting to a running `symbi up` instance.
//!
//! When attached, the shell proxies /cron, /channels, /secrets, /agents, and
//! other runtime commands through the runtime's REST API instead of running
//! them in-process. This lets the shell manage both local daemons and
//! deployed containers (local Docker, Cloud Run, AWS).

#![allow(dead_code)] // API methods for future command wiring

use anyhow::{anyhow, Result};
use reqwest::{Client, Method};
use serde_json::Value;
use std::time::Duration;

/// A connection to a remote `symbi up` instance.
#[derive(Clone)]
pub struct RemoteConnection {
    client: Client,
    base_url: String,
    token: Option<String>,
}

impl std::fmt::Debug for RemoteConnection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RemoteConnection")
            .field("base_url", &self.base_url)
            .field("has_token", &self.token.is_some())
            .finish()
    }
}

impl RemoteConnection {
    /// Create a new remote connection.
    ///
    /// `base_url` should be like `http://localhost:8080` (no trailing slash).
    /// `token` is an optional bearer token for authentication.
    pub fn new(base_url: &str, token: Option<String>) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .unwrap_or_default();

        let base_url = base_url.trim_end_matches('/').to_string();

        Self {
            client,
            base_url,
            token,
        }
    }

    /// Get the base URL (for display).
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Check if the remote is reachable by calling /health.
    pub async fn ping(&self) -> Result<()> {
        let url = format!("{}/api/v1/health", self.base_url);
        let mut req = self.client.request(Method::GET, &url);
        if let Some(ref token) = self.token {
            req = req.bearer_auth(token);
        }
        let resp = req
            .send()
            .await
            .map_err(|e| anyhow!("Failed to reach {}: {}", self.base_url, e))?;

        if !resp.status().is_success() {
            return Err(anyhow!(
                "Health check failed: HTTP {}",
                resp.status().as_u16()
            ));
        }
        Ok(())
    }

    /// GET a JSON endpoint.
    pub async fn get(&self, path: &str) -> Result<Value> {
        self.request(Method::GET, path, None).await
    }

    /// POST a JSON endpoint with optional body.
    pub async fn post(&self, path: &str, body: Option<Value>) -> Result<Value> {
        self.request(Method::POST, path, body).await
    }

    /// PUT a JSON endpoint with optional body.
    pub async fn put(&self, path: &str, body: Option<Value>) -> Result<Value> {
        self.request(Method::PUT, path, body).await
    }

    /// DELETE a JSON endpoint.
    pub async fn delete(&self, path: &str) -> Result<Value> {
        self.request(Method::DELETE, path, None).await
    }

    async fn request(&self, method: Method, path: &str, body: Option<Value>) -> Result<Value> {
        let url = format!("{}{}", self.base_url, path);
        let mut req = self.client.request(method.clone(), &url);

        if let Some(ref token) = self.token {
            req = req.bearer_auth(token);
        }

        if let Some(body) = body {
            req = req.json(&body);
        }

        let resp = req
            .send()
            .await
            .map_err(|e| anyhow!("Request failed: {} {}: {}", method, url, e))?;

        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();

        if !status.is_success() {
            return Err(anyhow!("HTTP {}: {}", status.as_u16(), text));
        }

        // Empty body is OK — return null
        if text.is_empty() {
            return Ok(Value::Null);
        }

        serde_json::from_str(&text)
            .map_err(|e| anyhow!("Failed to parse response JSON: {} (body: {})", e, text))
    }

    // ─── Schedule management ───

    pub async fn list_schedules(&self) -> Result<Value> {
        self.get("/api/v1/schedules").await
    }

    pub async fn create_schedule(&self, schedule: Value) -> Result<Value> {
        self.post("/api/v1/schedules", Some(schedule)).await
    }

    pub async fn get_schedule(&self, id: &str) -> Result<Value> {
        self.get(&format!("/api/v1/schedules/{}", id)).await
    }

    pub async fn pause_schedule(&self, id: &str) -> Result<Value> {
        self.post(&format!("/api/v1/schedules/{}/pause", id), None)
            .await
    }

    pub async fn resume_schedule(&self, id: &str) -> Result<Value> {
        self.post(&format!("/api/v1/schedules/{}/resume", id), None)
            .await
    }

    pub async fn trigger_schedule(&self, id: &str) -> Result<Value> {
        self.post(&format!("/api/v1/schedules/{}/trigger", id), None)
            .await
    }

    pub async fn schedule_history(&self, id: &str) -> Result<Value> {
        self.get(&format!("/api/v1/schedules/{}/history", id)).await
    }

    // ─── Agent management ───

    pub async fn list_agents_remote(&self) -> Result<Value> {
        self.get("/api/v1/agents").await
    }

    pub async fn get_agent_status(&self, id: &str) -> Result<Value> {
        self.get(&format!("/api/v1/agents/{}/status", id)).await
    }

    // ─── Channel management ───

    pub async fn list_channels(&self) -> Result<Value> {
        self.get("/api/v1/channels").await
    }

    // ─── Metrics ───

    pub async fn metrics(&self) -> Result<Value> {
        self.get("/api/v1/metrics").await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_base_url_trim() {
        let conn = RemoteConnection::new("http://localhost:8080/", None);
        assert_eq!(conn.base_url(), "http://localhost:8080");
    }

    #[test]
    fn test_base_url_no_trim() {
        let conn = RemoteConnection::new("http://localhost:8080", None);
        assert_eq!(conn.base_url(), "http://localhost:8080");
    }

    #[test]
    fn test_debug_hides_token() {
        let conn = RemoteConnection::new("http://localhost:8080", Some("secret".to_string()));
        let debug_str = format!("{:?}", conn);
        assert!(!debug_str.contains("secret"));
        assert!(debug_str.contains("has_token: true"));
    }
}
