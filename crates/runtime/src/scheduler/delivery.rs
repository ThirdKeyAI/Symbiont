//! Delivery routing for scheduled job output.
//!
//! After a cron-triggered agent finishes, the `DeliveryRouter` dispatches its
//! output to one or more configured channels (webhook, Slack, log file, etc.).

use async_trait::async_trait;
use chrono::Utc;
use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use super::cron_types::{DeliveryChannel, DeliveryConfig, DeliveryReceipt};

/// Result of delivering to all configured channels.
#[derive(Debug, Clone)]
pub struct DeliveryResult {
    pub receipts: Vec<DeliveryReceipt>,
    pub all_succeeded: bool,
}

/// Trait for routing job output to delivery channels.
#[async_trait]
pub trait DeliveryRouter: Send + Sync {
    /// Deliver `payload` to all channels in `config`.
    async fn deliver(&self, payload: &serde_json::Value, config: &DeliveryConfig)
        -> DeliveryResult;
}

/// Handler for a single custom delivery channel.
#[async_trait]
pub trait CustomDeliveryHandler: Send + Sync {
    async fn deliver(
        &self,
        payload: &serde_json::Value,
        config: &HashMap<String, String>,
    ) -> Result<(), String>;
}

/// Default implementation that dispatches to built-in channel handlers.
pub struct DefaultDeliveryRouter {
    custom_handlers: HashMap<String, Arc<dyn CustomDeliveryHandler>>,
    /// Allowlisted base directory for `LogFile` delivery. When `None`, log-file
    /// delivery is refused (fail-closed). Initialized from `SYMBIONT_LOG_DIR`.
    log_base_dir: Option<PathBuf>,
}

impl DefaultDeliveryRouter {
    pub fn new() -> Self {
        Self {
            custom_handlers: HashMap::new(),
            log_base_dir: std::env::var_os("SYMBIONT_LOG_DIR").map(PathBuf::from),
        }
    }

    /// Confine `LogFile` delivery to `dir`. Any configured path that resolves
    /// outside this directory (via `..`, absolute paths, or symlinks) is refused.
    pub fn with_log_base_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.log_base_dir = Some(dir.into());
        self
    }

    /// Register a custom delivery handler.
    pub fn register_custom_handler(
        &mut self,
        name: String,
        handler: Arc<dyn CustomDeliveryHandler>,
    ) {
        self.custom_handlers.insert(name, handler);
    }

    /// Resolve a configured `LogFile` path to a concrete, safe target inside the
    /// allowlisted base directory, or reject it.
    ///
    /// Defends against the scheduler-delivery path-traversal vector
    /// (codered F-pattern-scout-0001): the destination string is data-driven
    /// (cron job JSON) and must never be allowed to append to arbitrary files
    /// such as `/etc/cron.d/*` or `~/.ssh/authorized_keys`.
    fn resolve_log_path(&self, path: &str) -> Result<PathBuf, String> {
        if path.is_empty() || path.contains('\0') {
            return Err("invalid log file path".to_string());
        }
        let base = self.log_base_dir.as_ref().ok_or_else(|| {
            "log_file delivery is disabled: set SYMBIONT_LOG_DIR to an allowlisted directory"
                .to_string()
        })?;
        // The base directory must exist and be resolvable; canonicalize collapses
        // any symlinks so the containment check below is sound.
        let base_canon = base
            .canonicalize()
            .map_err(|e| format!("log base dir {} is not accessible: {}", base.display(), e))?;

        let requested = Path::new(path);
        let candidate = if requested.is_absolute() {
            requested.to_path_buf()
        } else {
            base_canon.join(requested)
        };

        // Reject any `..` traversal lexically before touching the filesystem.
        if candidate
            .components()
            .any(|c| matches!(c, Component::ParentDir))
        {
            return Err("log file path must not contain '..'".to_string());
        }

        // Canonicalize the parent (the file itself may not exist yet) so symlink
        // escapes are resolved, then enforce containment within the base dir.
        let parent = candidate
            .parent()
            .ok_or_else(|| "log file path has no parent directory".to_string())?;
        let parent_canon = parent.canonicalize().map_err(|e| {
            format!(
                "log file directory {} is not accessible: {}",
                parent.display(),
                e
            )
        })?;
        if !parent_canon.starts_with(&base_canon) {
            return Err("log file path escapes the allowlisted log directory".to_string());
        }
        let file_name = candidate
            .file_name()
            .ok_or_else(|| "log file path has no file name".to_string())?;
        Ok(parent_canon.join(file_name))
    }

    async fn deliver_to_channel(
        &self,
        payload: &serde_json::Value,
        channel: &DeliveryChannel,
    ) -> DeliveryReceipt {
        match channel {
            DeliveryChannel::Stdout => self.deliver_stdout(payload),
            DeliveryChannel::LogFile { path } => self.deliver_log_file(payload, path).await,
            DeliveryChannel::Webhook {
                url,
                method,
                headers,
                retry_count,
                timeout_secs,
            } => {
                self.deliver_webhook(payload, url, method, headers, *retry_count, *timeout_secs)
                    .await
            }
            DeliveryChannel::Slack {
                webhook_url,
                channel,
            } => {
                self.deliver_slack(payload, webhook_url, channel.as_deref())
                    .await
            }
            DeliveryChannel::Email {
                smtp_host,
                smtp_port,
                to,
                from,
                subject_template,
            } => {
                self.deliver_email(
                    payload,
                    smtp_host,
                    *smtp_port,
                    to,
                    from,
                    subject_template.as_deref(),
                )
                .await
            }
            DeliveryChannel::Custom {
                handler_name,
                config,
            } => self.deliver_custom(payload, handler_name, config).await,
            DeliveryChannel::ChannelAdapter {
                adapter_name,
                channel_id,
                thread_id,
            } => {
                self.deliver_channel_adapter(
                    payload,
                    adapter_name,
                    channel_id,
                    thread_id.as_deref(),
                )
                .await
            }
        }
    }

    fn deliver_stdout(&self, payload: &serde_json::Value) -> DeliveryReceipt {
        let formatted =
            serde_json::to_string_pretty(payload).unwrap_or_else(|_| payload.to_string());
        println!("{}", formatted);
        DeliveryReceipt {
            channel_description: "stdout".to_string(),
            delivered_at: Utc::now(),
            success: true,
            status_code: None,
            error: None,
        }
    }

    async fn deliver_log_file(&self, payload: &serde_json::Value, path: &str) -> DeliveryReceipt {
        let resolved = match self.resolve_log_path(path) {
            Ok(p) => p,
            Err(reason) => {
                return DeliveryReceipt {
                    channel_description: format!("log_file:{}", path),
                    delivered_at: Utc::now(),
                    success: false,
                    status_code: None,
                    error: Some(format!("rejected: {}", reason)),
                };
            }
        };
        let line = format!(
            "[{}] {}\n",
            Utc::now().to_rfc3339(),
            serde_json::to_string(payload).unwrap_or_else(|_| payload.to_string())
        );
        match tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&resolved)
            .await
        {
            Ok(mut file) => {
                use tokio::io::AsyncWriteExt;
                match file.write_all(line.as_bytes()).await {
                    Ok(_) => {
                        let _ = file.flush().await;
                        DeliveryReceipt {
                            channel_description: format!("log_file:{}", path),
                            delivered_at: Utc::now(),
                            success: true,
                            status_code: None,
                            error: None,
                        }
                    }
                    Err(e) => DeliveryReceipt {
                        channel_description: format!("log_file:{}", path),
                        delivered_at: Utc::now(),
                        success: false,
                        status_code: None,
                        error: Some(format!("write failed: {}", e)),
                    },
                }
            }
            Err(e) => DeliveryReceipt {
                channel_description: format!("log_file:{}", path),
                delivered_at: Utc::now(),
                success: false,
                status_code: None,
                error: Some(format!("open failed: {}", e)),
            },
        }
    }

    async fn deliver_webhook(
        &self,
        payload: &serde_json::Value,
        url: &str,
        method: &str,
        headers: &HashMap<String, String>,
        retry_count: u32,
        timeout_secs: u64,
    ) -> DeliveryReceipt {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(timeout_secs))
            .build();

        let client = match client {
            Ok(c) => c,
            Err(e) => {
                return DeliveryReceipt {
                    channel_description: format!("webhook:{}", url),
                    delivered_at: Utc::now(),
                    success: false,
                    status_code: None,
                    error: Some(format!("client build failed: {}", e)),
                };
            }
        };

        let mut last_error = String::new();
        for attempt in 0..=retry_count {
            let mut request = match method.to_uppercase().as_str() {
                "PUT" => client.put(url),
                "PATCH" => client.patch(url),
                _ => client.post(url),
            };

            for (k, v) in headers {
                request = request.header(k.as_str(), v.as_str());
            }

            request = request.json(payload);

            match request.send().await {
                Ok(resp) => {
                    let status = resp.status().as_u16();
                    if resp.status().is_success() {
                        return DeliveryReceipt {
                            channel_description: format!("webhook:{}", url),
                            delivered_at: Utc::now(),
                            success: true,
                            status_code: Some(status),
                            error: None,
                        };
                    }
                    last_error = format!("HTTP {}", status);
                }
                Err(e) => {
                    last_error = e.to_string();
                }
            }

            if attempt < retry_count {
                // Exponential backoff: 1s, 2s, 4s, ...
                let delay = std::time::Duration::from_secs(1 << attempt);
                tokio::time::sleep(delay).await;
            }
        }

        DeliveryReceipt {
            channel_description: format!("webhook:{}", url),
            delivered_at: Utc::now(),
            success: false,
            status_code: None,
            error: Some(format!(
                "failed after {} retries: {}",
                retry_count, last_error
            )),
        }
    }

    async fn deliver_slack(
        &self,
        payload: &serde_json::Value,
        webhook_url: &str,
        channel: Option<&str>,
    ) -> DeliveryReceipt {
        let text = serde_json::to_string_pretty(payload).unwrap_or_else(|_| payload.to_string());

        let mut slack_payload = serde_json::json!({ "text": text });
        if let Some(ch) = channel {
            slack_payload["channel"] = serde_json::Value::String(ch.to_string());
        }

        let client = match reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .build()
        {
            Ok(c) => c,
            Err(e) => {
                return DeliveryReceipt {
                    channel_description: "slack".to_string(),
                    delivered_at: Utc::now(),
                    success: false,
                    status_code: None,
                    error: Some(e.to_string()),
                };
            }
        };

        match client.post(webhook_url).json(&slack_payload).send().await {
            Ok(resp) => {
                let status = resp.status().as_u16();
                DeliveryReceipt {
                    channel_description: "slack".to_string(),
                    delivered_at: Utc::now(),
                    success: resp.status().is_success(),
                    status_code: Some(status),
                    error: if resp.status().is_success() {
                        None
                    } else {
                        Some(format!("HTTP {}", status))
                    },
                }
            }
            Err(e) => DeliveryReceipt {
                channel_description: "slack".to_string(),
                delivered_at: Utc::now(),
                success: false,
                status_code: None,
                error: Some(e.to_string()),
            },
        }
    }

    async fn deliver_email(
        &self,
        _payload: &serde_json::Value,
        smtp_host: &str,
        smtp_port: u16,
        to: &[String],
        from: &str,
        subject_template: Option<&str>,
    ) -> DeliveryReceipt {
        // Email delivery requires a full SMTP client (lettre crate).
        // For now, log the intent and return a placeholder receipt.
        tracing::info!(
            "Email delivery requested: from={}, to={:?}, host={}:{}, subject={:?}",
            from,
            to,
            smtp_host,
            smtp_port,
            subject_template,
        );
        DeliveryReceipt {
            channel_description: format!("email:{}:{}", smtp_host, smtp_port),
            delivered_at: Utc::now(),
            success: false,
            status_code: None,
            error: Some("SMTP delivery not yet implemented; add lettre dependency".to_string()),
        }
    }

    async fn deliver_channel_adapter(
        &self,
        _payload: &serde_json::Value,
        adapter_name: &str,
        channel_id: &str,
        thread_id: Option<&str>,
    ) -> DeliveryReceipt {
        // Channel adapter delivery is handled by the ChannelAdapterManager,
        // which is registered as a custom handler. This method provides a
        // fallback when no adapter manager is wired up.
        tracing::info!(
            adapter = %adapter_name,
            channel = %channel_id,
            thread = ?thread_id,
            "Channel adapter delivery requested"
        );
        DeliveryReceipt {
            channel_description: format!("channel_adapter:{}:{}", adapter_name, channel_id),
            delivered_at: Utc::now(),
            success: false,
            status_code: None,
            error: Some(format!(
                "no channel adapter '{}' registered; start one with `symbi chat connect`",
                adapter_name
            )),
        }
    }

    async fn deliver_custom(
        &self,
        payload: &serde_json::Value,
        handler_name: &str,
        config: &HashMap<String, String>,
    ) -> DeliveryReceipt {
        match self.custom_handlers.get(handler_name) {
            Some(handler) => match handler.deliver(payload, config).await {
                Ok(()) => DeliveryReceipt {
                    channel_description: format!("custom:{}", handler_name),
                    delivered_at: Utc::now(),
                    success: true,
                    status_code: None,
                    error: None,
                },
                Err(e) => DeliveryReceipt {
                    channel_description: format!("custom:{}", handler_name),
                    delivered_at: Utc::now(),
                    success: false,
                    status_code: None,
                    error: Some(e),
                },
            },
            None => DeliveryReceipt {
                channel_description: format!("custom:{}", handler_name),
                delivered_at: Utc::now(),
                success: false,
                status_code: None,
                error: Some(format!("no handler registered for '{}'", handler_name)),
            },
        }
    }
}

impl Default for DefaultDeliveryRouter {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl DeliveryRouter for DefaultDeliveryRouter {
    async fn deliver(
        &self,
        payload: &serde_json::Value,
        config: &DeliveryConfig,
    ) -> DeliveryResult {
        let mut receipts = Vec::with_capacity(config.channels.len());
        let mut all_succeeded = true;

        for channel in &config.channels {
            let receipt = self.deliver_to_channel(payload, channel).await;
            if !receipt.success {
                all_succeeded = false;
                if config.fail_fast {
                    receipts.push(receipt);
                    break;
                }
            }
            receipts.push(receipt);
        }

        DeliveryResult {
            receipts,
            all_succeeded,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn stdout_delivery_succeeds() {
        let router = DefaultDeliveryRouter::new();
        let payload = serde_json::json!({"status": "ok"});
        let config = DeliveryConfig {
            channels: vec![DeliveryChannel::Stdout],
            fail_fast: false,
        };
        let result = router.deliver(&payload, &config).await;
        assert!(result.all_succeeded);
        assert_eq!(result.receipts.len(), 1);
        assert!(result.receipts[0].success);
    }

    #[tokio::test]
    async fn log_file_delivery_creates_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.log");

        let router = DefaultDeliveryRouter::new().with_log_base_dir(dir.path());
        let payload = serde_json::json!({"result": "pass"});
        let config = DeliveryConfig {
            channels: vec![DeliveryChannel::LogFile {
                path: "test.log".to_string(),
            }],
            fail_fast: false,
        };
        let result = router.deliver(&payload, &config).await;
        assert!(result.all_succeeded);

        let content = tokio::fs::read_to_string(&path).await.unwrap();
        assert!(content.contains("pass"));
    }

    #[tokio::test]
    async fn log_file_rejects_parent_dir_traversal() {
        let dir = tempfile::tempdir().unwrap();
        let router = DefaultDeliveryRouter::new().with_log_base_dir(dir.path());
        let payload = serde_json::json!({"x": 1});
        let config = DeliveryConfig {
            channels: vec![DeliveryChannel::LogFile {
                path: "../escape.log".to_string(),
            }],
            fail_fast: false,
        };
        let result = router.deliver(&payload, &config).await;
        assert!(!result.all_succeeded);
        assert!(result.receipts[0]
            .error
            .as_ref()
            .unwrap()
            .contains("rejected"));
        // Nothing must be written outside the base dir.
        assert!(!dir.path().parent().unwrap().join("escape.log").exists());
    }

    #[tokio::test]
    async fn log_file_rejects_absolute_escape() {
        let base = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let evil = outside.path().join("evil.log");
        let router = DefaultDeliveryRouter::new().with_log_base_dir(base.path());
        let payload = serde_json::json!({"x": 1});
        let config = DeliveryConfig {
            channels: vec![DeliveryChannel::LogFile {
                path: evil.to_str().unwrap().to_string(),
            }],
            fail_fast: false,
        };
        let result = router.deliver(&payload, &config).await;
        assert!(!result.all_succeeded);
        assert!(!evil.exists());
    }

    #[tokio::test]
    async fn log_file_disabled_without_base_dir() {
        // No base dir configured => fail-closed.
        let router = DefaultDeliveryRouter {
            custom_handlers: HashMap::new(),
            log_base_dir: None,
        };
        let payload = serde_json::json!({"x": 1});
        let config = DeliveryConfig {
            channels: vec![DeliveryChannel::LogFile {
                path: "anything.log".to_string(),
            }],
            fail_fast: false,
        };
        let result = router.deliver(&payload, &config).await;
        assert!(!result.all_succeeded);
        assert!(result.receipts[0]
            .error
            .as_ref()
            .unwrap()
            .contains("disabled"));
    }

    #[tokio::test]
    async fn custom_handler_not_found() {
        let router = DefaultDeliveryRouter::new();
        let payload = serde_json::json!({"x": 1});
        let config = DeliveryConfig {
            channels: vec![DeliveryChannel::Custom {
                handler_name: "nonexistent".to_string(),
                config: HashMap::new(),
            }],
            fail_fast: false,
        };
        let result = router.deliver(&payload, &config).await;
        assert!(!result.all_succeeded);
        assert!(result.receipts[0]
            .error
            .as_ref()
            .unwrap()
            .contains("no handler"));
    }

    #[tokio::test]
    async fn fail_fast_stops_after_first_failure() {
        let dir = tempfile::tempdir().unwrap();
        let good_path = dir.path().join("good.log");

        let router = DefaultDeliveryRouter::new();
        let payload = serde_json::json!({"x": 1});
        let config = DeliveryConfig {
            channels: vec![
                // This will fail (no handler)
                DeliveryChannel::Custom {
                    handler_name: "missing".to_string(),
                    config: HashMap::new(),
                },
                // This should NOT be attempted due to fail_fast
                DeliveryChannel::LogFile {
                    path: good_path.to_str().unwrap().to_string(),
                },
            ],
            fail_fast: true,
        };
        let result = router.deliver(&payload, &config).await;
        assert!(!result.all_succeeded);
        // Only one receipt (stopped after failure)
        assert_eq!(result.receipts.len(), 1);
    }

    #[tokio::test]
    async fn multiple_channels_all_succeed() {
        let dir = tempfile::tempdir().unwrap();

        let router = DefaultDeliveryRouter::new().with_log_base_dir(dir.path());
        let payload = serde_json::json!({"multi": true});
        let config = DeliveryConfig {
            channels: vec![
                DeliveryChannel::Stdout,
                DeliveryChannel::LogFile {
                    path: "a.log".to_string(),
                },
                DeliveryChannel::LogFile {
                    path: "b.log".to_string(),
                },
            ],
            fail_fast: false,
        };
        let result = router.deliver(&payload, &config).await;
        assert!(result.all_succeeded);
        assert_eq!(result.receipts.len(), 3);
    }

    #[tokio::test]
    async fn delivery_config_serialization() {
        let config = DeliveryConfig {
            channels: vec![
                DeliveryChannel::Webhook {
                    url: "https://example.com/hook".to_string(),
                    method: "POST".to_string(),
                    headers: {
                        let mut h = HashMap::new();
                        h.insert("X-Token".to_string(), "abc".to_string());
                        h
                    },
                    retry_count: 2,
                    timeout_secs: 10,
                },
                DeliveryChannel::Slack {
                    webhook_url: "https://hooks.slack.com/xxx".to_string(),
                    channel: Some("#alerts".to_string()),
                },
            ],
            fail_fast: true,
        };
        let json = serde_json::to_string(&config).unwrap();
        let parsed: DeliveryConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.channels.len(), 2);
        assert!(parsed.fail_fast);
    }
}
