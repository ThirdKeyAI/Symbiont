use chrono::Utc;
use dashmap::DashMap;
use std::sync::Arc;
use tokio::sync::oneshot;
use uuid::Uuid;

use crate::audit::ApprovalAuditLogger;
use crate::blocks;
use crate::config::ResolvedSlackConfig;
use crate::types::{ApprovalDecision, ApprovalRequest, Approver, Outcome};

/// Pending approval state stored while waiting for a Slack callback.
struct PendingApproval {
    request: ApprovalRequest,
    respond: oneshot::Sender<ApprovalDecision>,
    channel_ts: Option<String>,
}

/// Slack approval relay that posts Block Kit messages and handles callbacks.
pub struct SlackApprovalRelay {
    config: ResolvedSlackConfig,
    client: reqwest::Client,
    pending: DashMap<Uuid, PendingApproval>,
    audit: Arc<ApprovalAuditLogger>,
}

impl SlackApprovalRelay {
    /// Create a new Slack approval relay.
    pub fn new(config: ResolvedSlackConfig, audit: Arc<ApprovalAuditLogger>) -> Self {
        Self {
            config,
            client: reqwest::Client::new(),
            pending: DashMap::new(),
            audit,
        }
    }

    /// Post an approval request to Slack and register it as pending.
    ///
    /// The `respond` sender will be used to deliver the decision when a
    /// Slack button callback arrives.
    pub async fn post_request(
        &self,
        req: ApprovalRequest,
        respond: oneshot::Sender<ApprovalDecision>,
    ) -> Result<(), SlackError> {
        let blocks_json = blocks::approval_blocks(&req);
        let request_id = req.request_id;

        let body = serde_json::json!({
            "channel": self.config.channel_id,
            "text": format!("Approval required: {} [{}]", req.tool, req.risk_label),
            "blocks": blocks_json,
        });

        let resp = self
            .client
            .post("https://slack.com/api/chat.postMessage")
            .header("Authorization", format!("Bearer {}", self.config.bot_token))
            .header("Content-Type", "application/json; charset=utf-8")
            .json(&body)
            .send()
            .await
            .map_err(|e| SlackError::Http(e.to_string()))?;

        let resp_json: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| SlackError::Http(e.to_string()))?;

        if resp_json["ok"].as_bool() != Some(true) {
            let err = resp_json["error"].as_str().unwrap_or("unknown").to_string();
            return Err(SlackError::Api(err));
        }

        let ts = resp_json["ts"].as_str().unwrap_or_default().to_string();

        self.audit
            .log_slack_posted(request_id, &self.config.channel_id, &ts)
            .await;

        self.pending.insert(
            request_id,
            PendingApproval {
                request: req,
                respond,
                channel_ts: Some(ts),
            },
        );

        Ok(())
    }

    /// Handle a Slack interaction callback (button press).
    ///
    /// Returns `true` if the callback was for a known pending request.
    pub async fn handle_callback(
        &self,
        request_id: Uuid,
        action_id: &str,
        user_id: &str,
        message_ts: &str,
    ) -> bool {
        let entry = match self.pending.remove(&request_id) {
            Some((_, entry)) => entry,
            None => {
                tracing::warn!("Callback for unknown request {request_id}");
                return false;
            }
        };

        let now = Utc::now();

        // Refuse decisions that arrive after the request's TTL. Without this
        // check a late button-press could retroactively approve a lapsed
        // action. The pending entry has already been removed so the decision
        // cannot be applied twice.
        if now > entry.request.expires_at {
            let expired_decision = ApprovalDecision {
                request_id,
                outcome: Outcome::Expired,
                approver: Approver::Slack {
                    user_id: user_id.to_string(),
                    message_ts: message_ts.to_string(),
                },
                reason: Some("decision arrived after expires_at".to_string()),
                decided_at: now,
            };
            self.audit.log_decision(&expired_decision).await;
            self.finalize_resolution(&entry.request, Outcome::Expired, user_id, &entry.channel_ts)
                .await;
            let _ = entry.respond.send(expired_decision);
            tracing::warn!(
                %request_id,
                expired_at = %entry.request.expires_at,
                arrived_at = %now,
                "Slack approval callback arrived after expiry; refused"
            );
            return true;
        }

        let outcome = match action_id {
            "approve" => Outcome::Approve,
            _ => Outcome::Deny,
        };

        let decision = ApprovalDecision {
            request_id,
            outcome,
            approver: Approver::Slack {
                user_id: user_id.to_string(),
                message_ts: message_ts.to_string(),
            },
            reason: None,
            decided_at: now,
        };

        self.audit.log_decision(&decision).await;

        // Update the Slack message to show the resolved state.
        self.finalize_resolution(&entry.request, outcome, user_id, &entry.channel_ts)
            .await;

        let _ = entry.respond.send(decision);
        true
    }

    /// Take the channel_ts for a pending request (used when CLI wins the race).
    pub fn take_channel_ts(&self, request_id: &Uuid) -> Option<(ApprovalRequest, String)> {
        self.pending
            .remove(request_id)
            .and_then(|(_, entry)| entry.channel_ts.map(|ts| (entry.request, ts)))
    }

    /// Update a Slack message to show the final resolved state.
    pub async fn finalize_resolution(
        &self,
        req: &ApprovalRequest,
        outcome: Outcome,
        approver_label: &str,
        channel_ts: &Option<String>,
    ) {
        let ts = match channel_ts {
            Some(ts) => ts,
            None => return,
        };

        let resolved = blocks::resolved_blocks(req, outcome, approver_label);

        let body = serde_json::json!({
            "channel": self.config.channel_id,
            "ts": ts,
            "blocks": resolved,
            "text": format!("{:?}: {} [{}]", outcome, req.tool, req.risk_label),
        });

        let result = self
            .client
            .post("https://slack.com/api/chat.update")
            .header("Authorization", format!("Bearer {}", self.config.bot_token))
            .header("Content-Type", "application/json; charset=utf-8")
            .json(&body)
            .send()
            .await;

        match result {
            Ok(_) => {
                self.audit
                    .log_slack_finalized(req.request_id, outcome)
                    .await;
            }
            Err(e) => {
                tracing::error!("Failed to finalize Slack message: {e}");
                self.audit
                    .log_error(req.request_id, &format!("finalize failed: {e}"))
                    .await;
            }
        }
    }

    /// Return the signing secret for HMAC verification.
    pub fn signing_secret(&self) -> &str {
        &self.config.signing_secret
    }

    /// Check if there is a pending approval for the given request ID.
    pub fn has_pending(&self, request_id: &Uuid) -> bool {
        self.pending.contains_key(request_id)
    }
}

/// Errors from Slack API operations.
#[derive(Debug, thiserror::Error)]
pub enum SlackError {
    #[error("Slack HTTP error: {0}")]
    Http(String),
    #[error("Slack API error: {0}")]
    Api(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::ApprovalAuditLogger;

    fn make_config() -> ResolvedSlackConfig {
        ResolvedSlackConfig {
            bot_token: "xoxb-test".into(),
            signing_secret: "test-secret".into(),
            channel_id: "C123".into(),
            callback_port: 3456,
        }
    }

    #[test]
    fn has_pending_false_initially() {
        let dir = tempfile::tempdir().unwrap();
        let audit = Arc::new(ApprovalAuditLogger::new(dir.path().join("audit.jsonl")));
        let relay = SlackApprovalRelay::new(make_config(), audit);
        assert!(!relay.has_pending(&Uuid::new_v4()));
    }

    #[test]
    fn signing_secret_accessor() {
        let dir = tempfile::tempdir().unwrap();
        let audit = Arc::new(ApprovalAuditLogger::new(dir.path().join("audit.jsonl")));
        let relay = SlackApprovalRelay::new(make_config(), audit);
        assert_eq!(relay.signing_secret(), "test-secret");
    }
}
