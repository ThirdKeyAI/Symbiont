use serde_json::json;

use crate::types::{ApprovalRequest, Outcome};

/// Build Slack Block Kit blocks for an approval request message.
pub fn approval_blocks(req: &ApprovalRequest) -> serde_json::Value {
    let header_text = format!(
        ":lock: Approval required \u{2014} {} [{}]",
        req.tool, req.risk_label
    );

    json!([
        {
            "type": "header",
            "text": {
                "type": "plain_text",
                "text": header_text,
                "emoji": true
            }
        },
        {
            "type": "section",
            "fields": [
                {
                    "type": "mrkdwn",
                    "text": format!("*Agent:*\n{}", req.agent_name)
                },
                {
                    "type": "mrkdwn",
                    "text": format!("*Target:*\n{}", req.target)
                },
                {
                    "type": "mrkdwn",
                    "text": format!("*Context:*\n{}", req.context_id)
                },
                {
                    "type": "mrkdwn",
                    "text": format!("*Risk:*\n{}", req.risk_label)
                }
            ]
        },
        {
            "type": "section",
            "text": {
                "type": "mrkdwn",
                "text": format!("```\n{}\n```", serde_json::to_string_pretty(&req.args_redacted).unwrap_or_default())
            }
        },
        {
            "type": "actions",
            "block_id": format!("approval_{}", req.request_id),
            "elements": [
                {
                    "type": "button",
                    "text": {
                        "type": "plain_text",
                        "text": ":white_check_mark: Approve",
                        "emoji": true
                    },
                    "style": "primary",
                    "action_id": "approve",
                    "value": req.request_id.to_string()
                },
                {
                    "type": "button",
                    "text": {
                        "type": "plain_text",
                        "text": ":x: Deny",
                        "emoji": true
                    },
                    "style": "danger",
                    "action_id": "deny",
                    "value": req.request_id.to_string()
                }
            ]
        },
        {
            "type": "context",
            "elements": [
                {
                    "type": "mrkdwn",
                    "text": format!("Request `{}` \u{2014} expires <<!date^{}^{{date_short_pretty}} {{time}}|{}>>",
                        req.request_id,
                        req.expires_at.timestamp(),
                        req.expires_at
                    )
                }
            ]
        }
    ])
}

/// Build Slack Block Kit blocks for a finalized (resolved) approval message.
pub fn resolved_blocks(
    req: &ApprovalRequest,
    outcome: Outcome,
    approver_label: &str,
) -> serde_json::Value {
    let emoji = match outcome {
        Outcome::Approve => ":white_check_mark:",
        Outcome::Deny => ":no_entry_sign:",
        Outcome::Expired => ":hourglass:",
    };
    let status = match outcome {
        Outcome::Approve => "Approved",
        Outcome::Deny => "Denied",
        Outcome::Expired => "Expired",
    };

    let header_text = format!(
        "{} {} \u{2014} {} [{}]",
        emoji, status, req.tool, req.risk_label
    );

    json!([
        {
            "type": "header",
            "text": {
                "type": "plain_text",
                "text": header_text,
                "emoji": true
            }
        },
        {
            "type": "section",
            "fields": [
                {
                    "type": "mrkdwn",
                    "text": format!("*Agent:*\n{}", req.agent_name)
                },
                {
                    "type": "mrkdwn",
                    "text": format!("*Target:*\n{}", req.target)
                },
                {
                    "type": "mrkdwn",
                    "text": format!("*Decided by:*\n{}", approver_label)
                },
                {
                    "type": "mrkdwn",
                    "text": format!("*Risk:*\n{}", req.risk_label)
                }
            ]
        }
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use uuid::Uuid;

    fn sample_request() -> ApprovalRequest {
        ApprovalRequest {
            request_id: Uuid::new_v4(),
            context_id: "deploy-42".into(),
            agent_name: "exploit-agent".into(),
            tool: "metasploit_run".into(),
            args_redacted: serde_json::json!({"module": "exploit/multi/handler"}),
            target: "10.0.0.5".into(),
            risk_label: "critical".into(),
            requested_at: Utc::now(),
            expires_at: Utc::now() + chrono::Duration::minutes(15),
        }
    }

    #[test]
    fn approval_blocks_contain_tool_and_risk() {
        let req = sample_request();
        let blocks = approval_blocks(&req);
        let text = serde_json::to_string(&blocks).unwrap();
        assert!(text.contains("metasploit_run"));
        assert!(text.contains("critical"));
        assert!(text.contains("exploit-agent"));
        assert!(text.contains("10.0.0.5"));
    }

    #[test]
    fn approval_blocks_have_action_buttons() {
        let req = sample_request();
        let blocks = approval_blocks(&req);
        let arr = blocks.as_array().unwrap();
        let actions = arr.iter().find(|b| b["type"] == "actions").unwrap();
        let elements = actions["elements"].as_array().unwrap();
        assert_eq!(elements.len(), 2);
        assert_eq!(elements[0]["action_id"], "approve");
        assert_eq!(elements[1]["action_id"], "deny");
    }

    #[test]
    fn resolved_blocks_show_outcome() {
        let req = sample_request();

        let approved = resolved_blocks(&req, Outcome::Approve, "alice");
        let text = serde_json::to_string(&approved).unwrap();
        assert!(text.contains("Approved"));
        assert!(text.contains("alice"));

        let denied = resolved_blocks(&req, Outcome::Deny, "bob");
        let text = serde_json::to_string(&denied).unwrap();
        assert!(text.contains("Denied"));

        let expired = resolved_blocks(&req, Outcome::Expired, "system");
        let text = serde_json::to_string(&expired).unwrap();
        assert!(text.contains("Expired"));
    }

    #[test]
    fn blocks_use_context_id_not_engagement_id() {
        let req = sample_request();
        let blocks = approval_blocks(&req);
        let text = serde_json::to_string(&blocks).unwrap();
        assert!(text.contains("deploy-42"));
    }
}
