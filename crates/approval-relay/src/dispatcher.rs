use chrono::Utc;
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot};

use crate::audit::ApprovalAuditLogger;
use crate::cli::{CliPrompter, Tty};
use crate::slack_relay::SlackApprovalRelay;
use crate::types::{ApprovalDecision, ApprovalRequest, Approver, Outcome};

/// Dual-channel approval dispatcher that races CLI and Slack channels.
///
/// When a request arrives, it is sent to both channels simultaneously.
/// The first channel to respond wins, and the other is cancelled/updated.
pub struct DualChannelDispatcher<T: Tty + 'static> {
    /// CLI prompter for terminal-based approvals.
    pub cli: Arc<CliPrompter<T>>,
    /// Optional Slack relay for button-based approvals.
    pub slack: Option<Arc<SlackApprovalRelay>>,
    /// Audit logger for all approval events.
    pub audit: Arc<ApprovalAuditLogger>,
}

impl<T: Tty + 'static> DualChannelDispatcher<T> {
    /// Handle a single approval request.
    ///
    /// If Slack is configured, races CLI and Slack channels. If CLI-only,
    /// prompts on the terminal directly. The winning decision is sent
    /// back via the `respond` channel.
    pub async fn handle_one(
        &self,
        req: ApprovalRequest,
        respond: oneshot::Sender<ApprovalDecision>,
    ) {
        self.audit.log_request(&req).await;

        let request_id = req.request_id;

        match &self.slack {
            Some(slack) => {
                self.race_cli_and_slack(req, respond, slack.clone()).await;
            }
            None => {
                // CLI-only mode
                let cli = self.cli.clone();
                let audit = self.audit.clone();
                let decision = {
                    let req_clone = req.clone();
                    tokio::task::spawn_blocking(move || cli.prompt_sync(&req_clone))
                        .await
                        .unwrap_or_else(|_| ApprovalDecision {
                            request_id,
                            outcome: Outcome::Deny,
                            approver: Approver::System,
                            reason: Some("prompt task panicked".into()),
                            decided_at: Utc::now(),
                        })
                };

                audit.log_decision(&decision).await;
                let _ = respond.send(decision);
            }
        }
    }

    /// Race CLI prompt against Slack buttons. First responder wins.
    async fn race_cli_and_slack(
        &self,
        req: ApprovalRequest,
        respond: oneshot::Sender<ApprovalDecision>,
        slack: Arc<SlackApprovalRelay>,
    ) {
        let request_id = req.request_id;

        // Create a oneshot for the Slack path
        let (slack_tx, slack_rx) = oneshot::channel::<ApprovalDecision>();

        // Post to Slack (non-blocking)
        let req_for_slack = req.clone();
        let slack_post = slack.clone();
        let audit_ref = self.audit.clone();
        tokio::spawn(async move {
            if let Err(e) = slack_post.post_request(req_for_slack, slack_tx).await {
                tracing::error!("Failed to post Slack approval: {e}");
                audit_ref
                    .log_error(request_id, &format!("slack post failed: {e}"))
                    .await;
            }
        });

        // Start CLI prompt on a blocking thread
        let cli = self.cli.clone();
        let req_for_cli = req.clone();
        let cli_handle = tokio::task::spawn_blocking(move || cli.prompt_sync(&req_for_cli));

        // Race: first response wins
        let decision = tokio::select! {
            cli_result = cli_handle => {
                let decision = cli_result.unwrap_or_else(|_| ApprovalDecision {
                    request_id,
                    outcome: Outcome::Deny,
                    approver: Approver::System,
                    reason: Some("CLI prompt panicked".into()),
                    decided_at: Utc::now(),
                });

                // CLI won — update Slack message to show resolved state
                if let Some((orig_req, ts)) = slack.take_channel_ts(&request_id) {
                    let approver_label = match &decision.approver {
                        Approver::Cli { user } => user.clone(),
                        _ => "CLI".into(),
                    };
                    slack.finalize_resolution(
                        &orig_req,
                        decision.outcome,
                        &approver_label,
                        &Some(ts),
                    ).await;
                }

                decision
            }
            slack_result = slack_rx => {
                slack_result.unwrap_or_else(|_| ApprovalDecision {
                    request_id,
                    outcome: Outcome::Deny,
                    approver: Approver::System,
                    reason: Some("Slack channel closed".into()),
                    decided_at: Utc::now(),
                })
            }
        };

        self.audit.log_decision(&decision).await;
        let _ = respond.send(decision);
    }
}

/// Run the dispatcher loop, consuming requests from the channel.
///
/// Each request is handled in a spawned task for concurrency.
pub async fn run<T: Tty + 'static>(
    dispatcher: Arc<DualChannelDispatcher<T>>,
    mut rx: mpsc::Receiver<(ApprovalRequest, oneshot::Sender<ApprovalDecision>)>,
) {
    while let Some((req, respond)) = rx.recv().await {
        let d = dispatcher.clone();
        tokio::spawn(async move { d.handle_one(req, respond).await });
    }
    tracing::info!("Approval dispatcher channel closed, shutting down");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::Tty;
    use async_trait::async_trait;
    use std::io;
    use uuid::Uuid;

    struct AutoApproveTty;

    #[async_trait]
    impl Tty for AutoApproveTty {
        fn print(&self, _msg: &str) {}
        fn println(&self, _msg: &str) {}
        fn read_line(&self) -> io::Result<String> {
            Ok("y\n".into())
        }
    }

    struct AutoDenyTty;

    #[async_trait]
    impl Tty for AutoDenyTty {
        fn print(&self, _msg: &str) {}
        fn println(&self, _msg: &str) {}
        fn read_line(&self) -> io::Result<String> {
            Ok("n\n".into())
        }
    }

    fn sample_request() -> ApprovalRequest {
        ApprovalRequest {
            request_id: Uuid::new_v4(),
            context_id: "ctx-1".into(),
            agent_name: "test-agent".into(),
            tool: "test_tool".into(),
            args_redacted: serde_json::json!({}),
            target: "10.0.0.1".into(),
            risk_label: "high".into(),
            requested_at: Utc::now(),
            expires_at: Utc::now() + chrono::Duration::minutes(10),
        }
    }

    #[tokio::test]
    async fn cli_only_approve() {
        let dir = tempfile::tempdir().unwrap();
        let audit = Arc::new(ApprovalAuditLogger::new(dir.path().join("audit.jsonl")));
        let cli = Arc::new(CliPrompter::new(AutoApproveTty, "tester"));

        let dispatcher = Arc::new(DualChannelDispatcher {
            cli,
            slack: None,
            audit,
        });

        let (tx, rx_oneshot) = oneshot::channel();
        dispatcher.handle_one(sample_request(), tx).await;

        let decision = rx_oneshot.await.unwrap();
        assert!(decision.approved());
        assert!(matches!(decision.approver, Approver::Cli { ref user } if user == "tester"));
    }

    #[tokio::test]
    async fn cli_only_deny() {
        let dir = tempfile::tempdir().unwrap();
        let audit = Arc::new(ApprovalAuditLogger::new(dir.path().join("audit.jsonl")));
        let cli = Arc::new(CliPrompter::new(AutoDenyTty, "tester"));

        let dispatcher = Arc::new(DualChannelDispatcher {
            cli,
            slack: None,
            audit,
        });

        let (tx, rx_oneshot) = oneshot::channel();
        dispatcher.handle_one(sample_request(), tx).await;

        let decision = rx_oneshot.await.unwrap();
        assert!(!decision.approved());
        assert_eq!(decision.outcome, Outcome::Deny);
    }

    #[tokio::test]
    async fn run_loop_processes_multiple_requests() {
        let dir = tempfile::tempdir().unwrap();
        let audit = Arc::new(ApprovalAuditLogger::new(dir.path().join("audit.jsonl")));
        let cli = Arc::new(CliPrompter::new(AutoApproveTty, "tester"));

        let dispatcher = Arc::new(DualChannelDispatcher {
            cli,
            slack: None,
            audit,
        });

        let (tx, rx) = mpsc::channel(16);

        // Spawn the run loop
        let d = dispatcher.clone();
        let handle = tokio::spawn(async move { run(d, rx).await });

        // Send two requests
        let (resp1_tx, resp1_rx) = oneshot::channel();
        let (resp2_tx, resp2_rx) = oneshot::channel();

        tx.send((sample_request(), resp1_tx)).await.unwrap();
        tx.send((sample_request(), resp2_tx)).await.unwrap();

        let d1 = resp1_rx.await.unwrap();
        let d2 = resp2_rx.await.unwrap();

        assert!(d1.approved());
        assert!(d2.approved());

        // Drop sender to close the loop
        drop(tx);
        handle.await.unwrap();
    }

    #[tokio::test]
    async fn audit_log_written_on_cli_decision() {
        let dir = tempfile::tempdir().unwrap();
        let audit_path = dir.path().join("audit.jsonl");
        let audit = Arc::new(ApprovalAuditLogger::new(&audit_path));
        let cli = Arc::new(CliPrompter::new(AutoApproveTty, "auditor"));

        let dispatcher = Arc::new(DualChannelDispatcher {
            cli,
            slack: None,
            audit,
        });

        let (tx, rx_oneshot) = oneshot::channel();
        dispatcher.handle_one(sample_request(), tx).await;

        let _ = rx_oneshot.await.unwrap();

        // Give the file system a moment
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let content = tokio::fs::read_to_string(&audit_path).await.unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert!(lines.len() >= 2, "Expected at least 2 audit lines, got {}", lines.len());
    }
}
