use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tokio::io::AsyncWriteExt;
use uuid::Uuid;

use crate::types::{ApprovalDecision, ApprovalRequest, Outcome};

/// An event recorded in the audit log.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEvent {
    /// Timestamp of the event.
    pub timestamp: DateTime<Utc>,
    /// Type of event.
    pub event_type: AuditEventType,
    /// The approval request ID.
    pub request_id: Uuid,
    /// Additional structured data.
    pub data: serde_json::Value,
}

/// Types of audit events.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditEventType {
    /// An approval request was created.
    RequestCreated,
    /// A decision was made (approve, deny, or expired).
    DecisionMade,
    /// A Slack message was posted.
    SlackPosted,
    /// A Slack message was updated (finalized).
    SlackFinalized,
    /// An error occurred during processing.
    Error,
}

/// Append-only JSONL audit logger for approval events.
pub struct ApprovalAuditLogger {
    path: PathBuf,
}

impl ApprovalAuditLogger {
    /// Create a new audit logger that writes to the given file path.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// Return the path to the audit log file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Log an approval request creation.
    pub async fn log_request(&self, req: &ApprovalRequest) {
        let event = AuditEvent {
            timestamp: Utc::now(),
            event_type: AuditEventType::RequestCreated,
            request_id: req.request_id,
            data: serde_json::json!({
                "context_id": req.context_id,
                "agent": req.agent_name,
                "tool": req.tool,
                "target": req.target,
                "risk_label": req.risk_label,
                "expires_at": req.expires_at,
            }),
        };
        self.append(&event).await;
    }

    /// Log an approval decision.
    pub async fn log_decision(&self, decision: &ApprovalDecision) {
        let event = AuditEvent {
            timestamp: Utc::now(),
            event_type: AuditEventType::DecisionMade,
            request_id: decision.request_id,
            data: serde_json::json!({
                "outcome": decision.outcome,
                "approver": decision.approver,
                "reason": decision.reason,
            }),
        };
        self.append(&event).await;
    }

    /// Log a Slack message posting.
    pub async fn log_slack_posted(&self, request_id: Uuid, channel: &str, ts: &str) {
        let event = AuditEvent {
            timestamp: Utc::now(),
            event_type: AuditEventType::SlackPosted,
            request_id,
            data: serde_json::json!({
                "channel": channel,
                "message_ts": ts,
            }),
        };
        self.append(&event).await;
    }

    /// Log a Slack message finalization.
    pub async fn log_slack_finalized(&self, request_id: Uuid, outcome: Outcome) {
        let event = AuditEvent {
            timestamp: Utc::now(),
            event_type: AuditEventType::SlackFinalized,
            request_id,
            data: serde_json::json!({
                "outcome": outcome,
            }),
        };
        self.append(&event).await;
    }

    /// Log an error.
    pub async fn log_error(&self, request_id: Uuid, error: &str) {
        let event = AuditEvent {
            timestamp: Utc::now(),
            event_type: AuditEventType::Error,
            request_id,
            data: serde_json::json!({
                "error": error,
            }),
        };
        self.append(&event).await;
    }

    async fn append(&self, event: &AuditEvent) {
        let mut line = match serde_json::to_string(event) {
            Ok(l) => l,
            Err(e) => {
                tracing::error!("Failed to serialize audit event: {e}");
                return;
            }
        };
        line.push('\n');

        let result = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .await;

        match result {
            Ok(mut file) => {
                if let Err(e) = file.write_all(line.as_bytes()).await {
                    tracing::error!("Failed to write audit event: {e}");
                }
            }
            Err(e) => {
                tracing::error!("Failed to open audit log {}: {e}", self.path.display());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn audit_log_writes_jsonl() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audit.jsonl");
        let logger = ApprovalAuditLogger::new(&path);

        let req = crate::types::ApprovalRequest {
            request_id: Uuid::new_v4(),
            context_id: "ctx-1".into(),
            agent_name: "test-agent".into(),
            tool: "nmap_scan".into(),
            args_redacted: serde_json::json!({}),
            target: "10.0.0.1".into(),
            risk_label: "high".into(),
            requested_at: Utc::now(),
            expires_at: Utc::now() + chrono::Duration::minutes(10),
        };

        logger.log_request(&req).await;

        let decision = crate::types::ApprovalDecision {
            request_id: req.request_id,
            outcome: Outcome::Approve,
            approver: crate::types::Approver::Cli {
                user: "alice".into(),
            },
            reason: Some("looks good".into()),
            decided_at: Utc::now(),
        };
        logger.log_decision(&decision).await;

        let content = tokio::fs::read_to_string(&path).await.unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 2);

        let event0: AuditEvent = serde_json::from_str(lines[0]).unwrap();
        assert!(matches!(event0.event_type, AuditEventType::RequestCreated));
        assert_eq!(event0.request_id, req.request_id);

        let event1: AuditEvent = serde_json::from_str(lines[1]).unwrap();
        assert!(matches!(event1.event_type, AuditEventType::DecisionMade));
    }

    #[test]
    fn audit_event_round_trip() {
        let event = AuditEvent {
            timestamp: Utc::now(),
            event_type: AuditEventType::SlackPosted,
            request_id: Uuid::new_v4(),
            data: serde_json::json!({"channel": "C123"}),
        };
        let json = serde_json::to_string(&event).unwrap();
        let restored: AuditEvent = serde_json::from_str(&json).unwrap();
        assert!(matches!(restored.event_type, AuditEventType::SlackPosted));
    }
}
