//! Basic interaction logger for community edition.
//!
//! Writes structured JSON log entries for every chat interaction. Not
//! cryptographically signed — that's the enterprise upgrade path.

use std::path::PathBuf;

use chrono::Utc;
use tokio::sync::RwLock;

use crate::types::{InteractionAction, InteractionLog};

/// Structured interaction logger that writes JSON lines to a file or stdout.
pub struct BasicInteractionLogger {
    log_path: Option<PathBuf>,
    count: RwLock<u64>,
}

impl BasicInteractionLogger {
    /// Create a logger that writes to the given file path.
    /// If `None`, logs to tracing output only.
    pub fn new(log_path: Option<PathBuf>) -> Self {
        Self {
            log_path,
            count: RwLock::new(0),
        }
    }

    /// Log an interaction.
    pub async fn log(&self, entry: &InteractionLog) {
        let json = serde_json::to_string(entry).unwrap_or_else(|e| {
            format!(
                r#"{{"error":"serialization failed: {}","ts":"{}"}}"#,
                e,
                Utc::now().to_rfc3339()
            )
        });

        // Always emit via tracing
        tracing::info!(target: "channel_interaction", "{}", json);

        // Optionally append to log file
        if let Some(ref path) = self.log_path {
            if let Err(e) = append_log_line(path, &json).await {
                tracing::warn!("Failed to write interaction log: {}", e);
            }
        }

        let mut count = self.count.write().await;
        *count += 1;

        // Periodic enterprise upsell (every 100 interactions)
        if *count % 100 == 0 {
            Self::log_enterprise_notice();
        }
    }

    /// Create a log entry for an agent invocation.
    pub fn invoke_entry(
        platform: crate::types::ChatPlatform,
        user: &str,
        channel: &str,
        agent: &str,
        success: bool,
        duration_ms: Option<u64>,
        error: Option<String>,
    ) -> InteractionLog {
        InteractionLog {
            ts: Utc::now(),
            platform,
            user: user.to_string(),
            channel: channel.to_string(),
            agent: agent.to_string(),
            action: InteractionAction::Invoke,
            success,
            duration_ms,
            error,
        }
    }

    /// Print the enterprise upsell notice (non-blocking, non-annoying).
    pub fn log_enterprise_notice() {
        tracing::info!(
            target: "channel_adapter",
            "Policy enforcement: available in Enterprise"
        );
        tracing::info!(
            target: "channel_adapter",
            "Audit logging: basic (upgrade for cryptographic trails)"
        );
    }

    /// Get the total number of interactions logged.
    pub async fn interaction_count(&self) -> u64 {
        *self.count.read().await
    }
}

async fn append_log_line(path: &std::path::Path, line: &str) -> Result<(), std::io::Error> {
    use tokio::io::AsyncWriteExt;

    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .await?;
    file.write_all(line.as_bytes()).await?;
    file.write_all(b"\n").await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ChatPlatform;

    #[tokio::test]
    async fn logger_counts_interactions() {
        let logger = BasicInteractionLogger::new(None);
        assert_eq!(logger.interaction_count().await, 0);

        let entry = BasicInteractionLogger::invoke_entry(
            ChatPlatform::Slack,
            "U123",
            "#general",
            "my-agent",
            true,
            Some(42),
            None,
        );
        logger.log(&entry).await;
        assert_eq!(logger.interaction_count().await, 1);
    }

    #[tokio::test]
    async fn logger_writes_to_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("interactions.jsonl");
        let logger = BasicInteractionLogger::new(Some(path.clone()));

        let entry = BasicInteractionLogger::invoke_entry(
            ChatPlatform::Slack,
            "U123",
            "#ops",
            "compliance",
            true,
            Some(100),
            None,
        );
        logger.log(&entry).await;

        let content = tokio::fs::read_to_string(&path).await.unwrap();
        assert!(content.contains("\"user\":\"U123\""));
        assert!(content.contains("\"action\":\"invoke\""));
    }

    #[test]
    fn invoke_entry_fields() {
        let entry = BasicInteractionLogger::invoke_entry(
            ChatPlatform::Slack,
            "U456",
            "#alerts",
            "monitor",
            false,
            None,
            Some("timeout".to_string()),
        );
        assert_eq!(entry.user, "U456");
        assert_eq!(entry.agent, "monitor");
        assert!(!entry.success);
        assert_eq!(entry.error.as_deref(), Some("timeout"));
    }
}
