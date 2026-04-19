//! Dual-channel (CLI + Slack) human approval relay.
//!
//! Provides a `DualChannelDispatcher` that races a terminal CLI prompt against
//! Slack inline-button approval. First responder wins. HMAC-SHA256 signature
//! verification on the Slack webhook endpoint ensures only authorized callbacks
//! are accepted.
//!
//! This crate is transport-layer only — it knows nothing about the agent runtime.
//! Consumers map their own approval types to `ApprovalRequest` and consume
//! `ApprovalDecision` from the dispatcher.

pub mod audit;
pub mod blocks;
pub mod cli;
pub mod config;
pub mod dispatcher;
pub mod http;
pub mod slack_relay;
pub mod types;

pub use config::SlackApprovalConfig;
pub use dispatcher::DualChannelDispatcher;
pub use types::{ApprovalDecision, ApprovalRequest, Approver, Outcome};

use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot};

use audit::ApprovalAuditLogger;
use cli::{CliPrompter, StdinStdoutTty};
use config::ResolvedSlackConfig;
use slack_relay::SlackApprovalRelay;

/// Start the approval relay with the default TTY (stdin/stdout).
///
/// Returns a sender for feeding approval requests and a join handle for
/// the dispatcher loop. The caller sends `(ApprovalRequest, oneshot::Sender<ApprovalDecision>)`
/// tuples and awaits the decision on the oneshot receiver.
///
/// If `slack_config` is `Some` and enabled, a Slack relay and HTTP callback
/// server are also started.
pub async fn start(
    slack_config: Option<SlackApprovalConfig>,
    audit_path: impl Into<PathBuf>,
    user_label: impl Into<String>,
) -> Result<
    (
        mpsc::Sender<(ApprovalRequest, oneshot::Sender<ApprovalDecision>)>,
        tokio::task::JoinHandle<()>,
    ),
    Box<dyn std::error::Error>,
> {
    start_with_tty(StdinStdoutTty, slack_config, audit_path, user_label).await
}

/// Start the approval relay with a custom TTY implementation.
///
/// This is useful for testing or for embedding the relay in a non-terminal
/// environment.
pub async fn start_with_tty<T: cli::Tty + 'static>(
    tty: T,
    slack_config: Option<SlackApprovalConfig>,
    audit_path: impl Into<PathBuf>,
    user_label: impl Into<String>,
) -> Result<
    (
        mpsc::Sender<(ApprovalRequest, oneshot::Sender<ApprovalDecision>)>,
        tokio::task::JoinHandle<()>,
    ),
    Box<dyn std::error::Error>,
> {
    let audit = Arc::new(ApprovalAuditLogger::new(audit_path.into()));
    let cli = Arc::new(CliPrompter::new(tty, user_label));

    // Resolve Slack config
    let resolved: Option<ResolvedSlackConfig> = match slack_config {
        Some(cfg) => cfg.resolve()?,
        None => None,
    };

    let slack = match resolved {
        Some(ref cfg) => {
            let relay = Arc::new(SlackApprovalRelay::new(cfg.clone(), audit.clone()));

            // Start the HTTP callback server
            let relay_for_http = relay.clone();
            let port = cfg.callback_port;
            tokio::spawn(async move {
                if let Err(e) = http::serve_slack_callbacks(relay_for_http, port).await {
                    tracing::error!("Slack callback server error: {e}");
                }
            });

            Some(relay)
        }
        None => None,
    };

    let dispatcher = Arc::new(DualChannelDispatcher { cli, slack, audit });

    let (tx, rx) = mpsc::channel(64);
    let handle = tokio::spawn(dispatcher::run(dispatcher, rx));

    Ok((tx, handle))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::Tty;
    use async_trait::async_trait;
    use chrono::Utc;
    use uuid::Uuid;

    struct TestTty;

    #[async_trait]
    impl Tty for TestTty {
        fn print(&self, _msg: &str) {}
        fn println(&self, _msg: &str) {}
        fn read_line(&self) -> std::io::Result<String> {
            Ok("y\n".into())
        }
    }

    #[tokio::test]
    async fn start_with_tty_cli_only() {
        let dir = tempfile::tempdir().unwrap();
        let audit_path = dir.path().join("audit.jsonl");

        let (tx, handle) = start_with_tty(TestTty, None, &audit_path, "tester")
            .await
            .unwrap();

        let req = ApprovalRequest {
            request_id: Uuid::new_v4(),
            context_id: "test-ctx".into(),
            agent_name: "agent".into(),
            tool: "tool".into(),
            args_redacted: serde_json::json!({}),
            target: "localhost".into(),
            risk_label: "low".into(),
            requested_at: Utc::now(),
            expires_at: Utc::now() + chrono::Duration::minutes(5),
        };

        let (resp_tx, resp_rx) = oneshot::channel();
        tx.send((req, resp_tx)).await.unwrap();

        let decision = resp_rx.await.unwrap();
        assert!(decision.approved());

        drop(tx);
        handle.await.unwrap();
    }

    #[tokio::test]
    async fn start_with_disabled_slack() {
        let dir = tempfile::tempdir().unwrap();
        let audit_path = dir.path().join("audit.jsonl");

        let slack_cfg = SlackApprovalConfig {
            enabled: false,
            ..Default::default()
        };

        let (tx, handle) = start_with_tty(TestTty, Some(slack_cfg), &audit_path, "tester")
            .await
            .unwrap();

        let req = ApprovalRequest {
            request_id: Uuid::new_v4(),
            context_id: "test-ctx".into(),
            agent_name: "agent".into(),
            tool: "tool".into(),
            args_redacted: serde_json::json!({}),
            target: "localhost".into(),
            risk_label: "low".into(),
            requested_at: Utc::now(),
            expires_at: Utc::now() + chrono::Duration::minutes(5),
        };

        let (resp_tx, resp_rx) = oneshot::channel();
        tx.send((req, resp_tx)).await.unwrap();

        let decision = resp_rx.await.unwrap();
        assert!(decision.approved());

        drop(tx);
        handle.await.unwrap();
    }
}
