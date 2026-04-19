use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A request for human approval before executing a tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalRequest {
    /// Unique identifier for this approval request.
    pub request_id: Uuid,
    /// Opaque context identifier (engagement, deployment, ticket, etc.).
    pub context_id: String,
    /// Name of the agent requesting approval.
    pub agent_name: String,
    /// Tool that the agent wants to execute.
    pub tool: String,
    /// Redacted arguments (safe to display in Slack or logs).
    pub args_redacted: serde_json::Value,
    /// Target of the action (host, URL, resource, etc.).
    pub target: String,
    /// Free-form risk label (e.g. "critical", "high", "medium").
    pub risk_label: String,
    /// When the request was created.
    pub requested_at: DateTime<Utc>,
    /// When the request expires (auto-denied after this).
    pub expires_at: DateTime<Utc>,
}

/// The outcome of an approval decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    /// The action was approved.
    Approve,
    /// The action was denied.
    Deny,
    /// The request expired without a decision.
    Expired,
}

/// Identity of the approver.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "channel", rename_all = "snake_case")]
pub enum Approver {
    /// Approved via CLI terminal prompt.
    Cli { user: String },
    /// Approved via Slack inline buttons.
    Slack { user_id: String, message_ts: String },
    /// System-generated decision (e.g. expiration).
    System,
}

/// A completed approval decision.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalDecision {
    /// The request this decision responds to.
    pub request_id: Uuid,
    /// Whether approved, denied, or expired.
    pub outcome: Outcome,
    /// Who made the decision.
    pub approver: Approver,
    /// Optional reason for the decision.
    pub reason: Option<String>,
    /// When the decision was made.
    pub decided_at: DateTime<Utc>,
}

impl ApprovalDecision {
    /// Returns `true` if the outcome is `Approve`.
    pub fn approved(&self) -> bool {
        matches!(self.outcome, Outcome::Approve)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn approver_serializes_with_channel_tag() {
        let cli = Approver::Cli {
            user: "alice".into(),
        };
        let json = serde_json::to_string(&cli).unwrap();
        assert!(json.contains(r#""channel":"cli""#), "got: {json}");

        let slack = Approver::Slack {
            user_id: "U123".into(),
            message_ts: "1234567890.123456".into(),
        };
        let json = serde_json::to_string(&slack).unwrap();
        assert!(json.contains(r#""channel":"slack""#), "got: {json}");

        let system = Approver::System;
        let json = serde_json::to_string(&system).unwrap();
        assert!(json.contains(r#""channel":"system""#), "got: {json}");

        // Round-trip
        let restored: Approver = serde_json::from_str(&json).unwrap();
        assert!(matches!(restored, Approver::System));
    }

    #[test]
    fn decision_approved_flag() {
        let base = ApprovalDecision {
            request_id: Uuid::new_v4(),
            outcome: Outcome::Approve,
            approver: Approver::System,
            reason: None,
            decided_at: Utc::now(),
        };
        assert!(base.approved());

        let denied = ApprovalDecision {
            outcome: Outcome::Deny,
            ..base.clone()
        };
        assert!(!denied.approved());

        let expired = ApprovalDecision {
            outcome: Outcome::Expired,
            ..base
        };
        assert!(!expired.approved());
    }

    #[test]
    fn outcome_serde_snake_case() {
        let json = serde_json::to_string(&Outcome::Approve).unwrap();
        assert_eq!(json, r#""approve""#);

        let json = serde_json::to_string(&Outcome::Deny).unwrap();
        assert_eq!(json, r#""deny""#);

        let json = serde_json::to_string(&Outcome::Expired).unwrap();
        assert_eq!(json, r#""expired""#);

        let restored: Outcome = serde_json::from_str(r#""approve""#).unwrap();
        assert_eq!(restored, Outcome::Approve);
    }

    #[test]
    fn approval_request_round_trip() {
        let req = ApprovalRequest {
            request_id: Uuid::new_v4(),
            context_id: "engagement-42".into(),
            agent_name: "exploit-agent".into(),
            tool: "metasploit_run".into(),
            args_redacted: serde_json::json!({"module": "exploit/multi/handler"}),
            target: "10.0.0.5".into(),
            risk_label: "critical".into(),
            requested_at: Utc::now(),
            expires_at: Utc::now() + chrono::Duration::minutes(15),
        };

        let json = serde_json::to_string(&req).unwrap();
        let restored: ApprovalRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.request_id, req.request_id);
        assert_eq!(restored.context_id, "engagement-42");
        assert_eq!(restored.risk_label, "critical");
    }
}
