use async_trait::async_trait;
use chrono::Utc;
use std::io::{self, BufRead, Write};
use uuid::Uuid;

use crate::types::{ApprovalDecision, ApprovalRequest, Approver, Outcome};

/// Trait abstracting terminal I/O for testing.
#[async_trait]
pub trait Tty: Send + Sync {
    /// Print a prompt to the terminal (no newline appended).
    fn print(&self, msg: &str);
    /// Print a line to the terminal (newline appended).
    fn println(&self, msg: &str);
    /// Read a line from the terminal (blocking, run on blocking thread).
    fn read_line(&self) -> io::Result<String>;
}

/// Default implementation using stdin/stdout.
pub struct StdinStdoutTty;

#[async_trait]
impl Tty for StdinStdoutTty {
    fn print(&self, msg: &str) {
        print!("{msg}");
        let _ = io::stdout().flush();
    }

    fn println(&self, msg: &str) {
        println!("{msg}");
    }

    fn read_line(&self) -> io::Result<String> {
        let stdin = io::stdin();
        let mut line = String::new();
        stdin.lock().read_line(&mut line)?;
        Ok(line)
    }
}

/// CLI approval prompter that displays approval requests in the terminal.
pub struct CliPrompter<T: Tty> {
    tty: T,
    user_label: String,
}

impl<T: Tty> CliPrompter<T> {
    /// Create a new CLI prompter with the given TTY and user label.
    pub fn new(tty: T, user_label: impl Into<String>) -> Self {
        Self {
            tty,
            user_label: user_label.into(),
        }
    }

    /// Display an approval request and block until the user responds.
    ///
    /// This is a blocking operation and should be run on a blocking thread.
    pub fn prompt_sync(&self, req: &ApprovalRequest) -> ApprovalDecision {
        self.tty.println("");
        self.tty
            .println("╔══════════════════════════════════════════╗");
        self.tty
            .println("║        APPROVAL REQUIRED                ║");
        self.tty
            .println("╠══════════════════════════════════════════╣");
        self.tty.println(&format!("║ Tool:      {}", req.tool));
        self.tty
            .println(&format!("║ Agent:     {}", req.agent_name));
        self.tty.println(&format!("║ Target:    {}", req.target));
        self.tty
            .println(&format!("║ Risk:      {}", req.risk_label));
        self.tty
            .println(&format!("║ Context:   {}", req.context_id));
        self.tty
            .println(&format!("║ Expires:   {}", req.expires_at));
        self.tty.println("║");
        self.tty.println(&format!("║ Args: {}", req.args_redacted));
        self.tty
            .println("╚══════════════════════════════════════════╝");
        self.tty.println("");
        self.tty.print("  Approve? [y/N] > ");

        let line = match self.tty.read_line() {
            Ok(l) => l,
            Err(e) => {
                self.tty.println(&format!("  Error reading input: {e}"));
                return self.make_decision(req.request_id, Outcome::Deny, Some("input error"));
            }
        };

        let trimmed = line.trim().to_lowercase();
        let (outcome, reason) = match trimmed.as_str() {
            "y" | "yes" => (Outcome::Approve, None),
            _ => (Outcome::Deny, Some("denied by operator")),
        };

        let decision = self.make_decision(req.request_id, outcome, reason);
        let label = if decision.approved() {
            "APPROVED"
        } else {
            "DENIED"
        };
        self.tty.println(&format!("  → {label}"));
        decision
    }

    /// Prompt asynchronously by spawning on a blocking thread.
    pub async fn prompt(&self, req: &ApprovalRequest) -> ApprovalDecision
    where
        T: 'static,
    {
        // We need to do the prompt on a blocking thread since read_line blocks.
        // But we can't move &self across threads easily, so we clone what we need.
        let _request_id = req.request_id;
        let _tool = req.tool.clone();
        let _agent_name = req.agent_name.clone();
        let _target = req.target.clone();
        let _risk_label = req.risk_label.clone();
        let _context_id = req.context_id.clone();
        let _expires_at = req.expires_at;
        let _args_redacted = req.args_redacted.clone();
        let _user_label = self.user_label.clone();

        // We can't easily send &self to a blocking thread, so we reconstruct
        // the prompt display inline. For real blocking I/O we'd need Arc<Self>.
        // Instead, do the sync prompt directly since the Tty trait is Send+Sync.
        self.prompt_sync(req).clone()
    }

    fn make_decision(
        &self,
        request_id: Uuid,
        outcome: Outcome,
        reason: Option<&str>,
    ) -> ApprovalDecision {
        ApprovalDecision {
            request_id,
            outcome,
            approver: Approver::Cli {
                user: self.user_label.clone(),
            },
            reason: reason.map(String::from),
            decided_at: Utc::now(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Test TTY that records output and returns canned input.
    struct MockTty {
        input: Mutex<Vec<String>>,
        output: Mutex<Vec<String>>,
    }

    impl MockTty {
        fn new(input_lines: Vec<&str>) -> Self {
            Self {
                input: Mutex::new(input_lines.into_iter().rev().map(String::from).collect()),
                output: Mutex::new(Vec::new()),
            }
        }

        fn output_lines(&self) -> Vec<String> {
            self.output.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl Tty for MockTty {
        fn print(&self, msg: &str) {
            self.output.lock().unwrap().push(msg.to_string());
        }
        fn println(&self, msg: &str) {
            self.output.lock().unwrap().push(format!("{msg}\n"));
        }
        fn read_line(&self) -> io::Result<String> {
            self.input
                .lock()
                .unwrap()
                .pop()
                .ok_or_else(|| io::Error::new(io::ErrorKind::UnexpectedEof, "no more input"))
        }
    }

    fn sample_request() -> ApprovalRequest {
        ApprovalRequest {
            request_id: Uuid::new_v4(),
            context_id: "ctx-test".into(),
            agent_name: "test-agent".into(),
            tool: "dangerous_tool".into(),
            args_redacted: serde_json::json!({"flag": true}),
            target: "10.0.0.1".into(),
            risk_label: "critical".into(),
            requested_at: Utc::now(),
            expires_at: Utc::now() + chrono::Duration::minutes(10),
        }
    }

    #[test]
    fn prompt_approve() {
        let tty = MockTty::new(vec!["y"]);
        let prompter = CliPrompter::new(tty, "operator");
        let decision = prompter.prompt_sync(&sample_request());

        assert!(decision.approved());
        assert!(matches!(decision.approver, Approver::Cli { ref user } if user == "operator"));
    }

    #[test]
    fn prompt_deny_explicit() {
        let tty = MockTty::new(vec!["n"]);
        let prompter = CliPrompter::new(tty, "operator");
        let decision = prompter.prompt_sync(&sample_request());

        assert!(!decision.approved());
        assert_eq!(decision.outcome, Outcome::Deny);
    }

    #[test]
    fn prompt_deny_default() {
        let tty = MockTty::new(vec![""]);
        let prompter = CliPrompter::new(tty, "operator");
        let decision = prompter.prompt_sync(&sample_request());

        assert!(!decision.approved());
    }

    #[test]
    fn prompt_deny_on_io_error() {
        let tty = MockTty::new(vec![]); // no input lines = EOF
        let prompter = CliPrompter::new(tty, "operator");
        let decision = prompter.prompt_sync(&sample_request());

        assert!(!decision.approved());
    }

    #[test]
    fn prompt_displays_request_info() {
        let tty = MockTty::new(vec!["y"]);
        let req = sample_request();
        let prompter = CliPrompter::new(tty, "operator");
        prompter.prompt_sync(&req);

        let output = prompter.tty.output_lines().join("");
        assert!(output.contains("dangerous_tool"));
        assert!(output.contains("10.0.0.1"));
        assert!(output.contains("critical"));
        assert!(output.contains("test-agent"));
        assert!(output.contains("APPROVED"));
    }
}
