//! Opt-in approvals from the runtime operator's controlling terminal.

#[cfg(any(unix, test))]
use super::Decision;
use super::EscalationQueue;
use super::{HeldAction, HeldStatus};
use std::sync::Arc;

/// Open the operator's terminal before starting a run. Worker stdin and stdout
/// are never used as approval authority.
pub async fn terminal_approval_queue() -> Result<Arc<EscalationQueue>, String> {
    #[cfg(unix)]
    {
        let queue = Arc::new(EscalationQueue::new());
        let notifier = unix::TerminalNotifier::open(Arc::downgrade(&queue))
            .map_err(|error| format!("terminal approval unavailable: {error}"))?;
        queue.subscribe(Arc::new(notifier)).await;
        Ok(queue)
    }
    #[cfg(not(unix))]
    {
        Err("terminal approval requires a Unix controlling terminal".into())
    }
}

const MAX_DISPLAY_BYTES: usize = 64 * 1024;
#[cfg(unix)]
const MAX_ANSWER_BYTES: usize = 128;

/// Keep JSON whitespace readable while escaping all non-ASCII text, including
/// bidirectional controls. The resulting complete document remains valid JSON.
pub fn render_approval_request(action: &HeldAction) -> Result<String, String> {
    if action.id.len() != 16
        || !action.id.bytes().all(|byte| byte.is_ascii_hexdigit())
        || action.status != HeldStatus::Pending
        || chrono::Utc::now() >= action.expires_at
    {
        return Err("invalid or expired terminal approval request".into());
    }
    let json = serde_json::to_string_pretty(action).map_err(|error| error.to_string())?;
    if json.len() > MAX_DISPLAY_BYTES {
        return Err("complete approval request exceeds terminal display limit".into());
    }
    let mut escaped = String::new();
    for ch in json.chars() {
        if ch == '\n' || ch.is_ascii_graphic() || ch == ' ' {
            escaped.push(ch);
        } else {
            for unit in ch.encode_utf16(&mut [0; 2]) {
                use std::fmt::Write;
                write!(escaped, "\\u{unit:04x}").map_err(|error| error.to_string())?;
            }
        }
        if escaped.len() > MAX_DISPLAY_BYTES {
            return Err("complete approval request exceeds terminal display limit".into());
        }
    }
    Ok(escaped)
}

#[cfg(any(unix, test))]
fn terminal_decision(id: &str, answer: &[u8]) -> Decision {
    if answer == format!("approve {id}").as_bytes() {
        Decision::Approve { reason: None }
    } else {
        Decision::Deny {
            reason: Some("terminal approval denied".into()),
        }
    }
}

#[cfg(unix)]
mod unix {
    use super::*;
    use crate::escalation::{Approver, EscalationNotifier, Surface};
    use std::{
        fs::{File, OpenOptions},
        io::{self, Read, Write},
        os::{fd::AsRawFd, unix::fs::OpenOptionsExt},
        sync::Weak,
    };
    use tokio::{io::unix::AsyncFd, sync::Mutex};

    pub(super) struct TerminalNotifier {
        queue: Weak<EscalationQueue>,
        tty: Mutex<AsyncFd<File>>,
        approver: Approver,
    }

    fn foreground(tty: &File) -> io::Result<()> {
        let fd = tty.as_raw_fd();
        // The descriptor remains owned by `tty` for the duration of each call.
        if unsafe { libc::isatty(fd) } != 1 {
            return Err(io::Error::other("approval descriptor is not a terminal"));
        }
        let owner = unsafe { libc::tcgetpgrp(fd) };
        if owner < 0 {
            return Err(io::Error::last_os_error());
        }
        if owner != unsafe { libc::getpgrp() } {
            return Err(io::Error::other(
                "approval requires the terminal foreground process group",
            ));
        }
        Ok(())
    }

    fn discard_input(tty: &File) -> io::Result<()> {
        foreground(tty)?;
        if unsafe { libc::tcflush(tty.as_raw_fd(), libc::TCIFLUSH) } != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    struct InputLease<'a>(&'a File);
    impl Drop for InputLease<'_> {
        fn drop(&mut self) {
            // Do not flush input belonging to a different foreground job.
            let _ = discard_input(self.0);
        }
    }

    impl TerminalNotifier {
        pub(super) fn open(queue: Weak<EscalationQueue>) -> io::Result<Self> {
            let tty = OpenOptions::new()
                .read(true)
                .write(true)
                .custom_flags(libc::O_NOCTTY | libc::O_NONBLOCK | libc::O_CLOEXEC)
                .open("/dev/tty")?;
            foreground(&tty)?;
            let uid = unsafe { libc::geteuid() };
            Ok(Self {
                queue,
                tty: Mutex::new(AsyncFd::new(tty)?),
                approver: Approver {
                    surface: Surface::Terminal,
                    id: format!("uid:{uid}"),
                    display: format!("Local terminal (uid {uid})"),
                },
            })
        }

        async fn prompt(&self, action: &HeldAction) -> Result<Decision, String> {
            let rendered = render_approval_request(action)?;
            let tty = self.tty.lock().await;
            foreground(tty.get_ref()).map_err(|error| error.to_string())?;
            let _lease = InputLease(tty.get_ref());
            discard_input(tty.get_ref()).map_err(|error| error.to_string())?;
            write_all(
                &tty,
                format!("\nApproval request (complete JSON with escaped text):\n{rendered}\n")
                    .as_bytes(),
            )
            .await
            .map_err(|error| error.to_string())?;
            // Input supplied before the complete request is shown is not an answer.
            discard_input(tty.get_ref()).map_err(|error| error.to_string())?;
            write_all(
                &tty,
                format!(
                    "Type approve {} to approve; any other answer denies:\n> ",
                    action.id
                )
                .as_bytes(),
            )
            .await
            .map_err(|error| error.to_string())?;
            let answer = read_answer(&tty).await.map_err(|error| error.to_string())?;
            Ok(terminal_decision(&action.id, &answer))
        }
    }

    #[async_trait::async_trait]
    impl EscalationNotifier for TerminalNotifier {
        async fn notify(&self, action: &HeldAction) {
            let Some(queue) = self.queue.upgrade() else {
                return;
            };
            let decision = self
                .prompt(action)
                .await
                .unwrap_or_else(|error| Decision::Deny {
                    reason: Some(format!("terminal approval unavailable: {error}")),
                });
            let _ = queue
                .resolve_async(&action.id, decision, self.approver.clone())
                .await;
        }
    }

    async fn write_all(tty: &AsyncFd<File>, mut bytes: &[u8]) -> io::Result<()> {
        while !bytes.is_empty() {
            foreground(tty.get_ref())?;
            let mut ready = tty.writable().await?;
            foreground(tty.get_ref())?;
            match ready.try_io(|inner| {
                let mut file = inner.get_ref();
                file.write(bytes)
            }) {
                Ok(Ok(0)) => {
                    return Err(io::Error::new(
                        io::ErrorKind::WriteZero,
                        "approval terminal write failed",
                    ))
                }
                Ok(Ok(count)) => bytes = &bytes[count..],
                Ok(Err(error)) if error.kind() == io::ErrorKind::Interrupted => continue,
                Ok(Err(error)) => return Err(error),
                Err(_) => continue,
            }
        }
        Ok(())
    }

    async fn read_answer(tty: &AsyncFd<File>) -> io::Result<Vec<u8>> {
        let mut answer = Vec::new();
        loop {
            foreground(tty.get_ref())?;
            let mut ready = tty.readable().await?;
            foreground(tty.get_ref())?;
            let mut byte = [0];
            match ready.try_io(|inner| {
                let mut file = inner.get_ref();
                file.read(&mut byte)
            }) {
                Ok(Ok(0)) => {
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "approval terminal closed",
                    ))
                }
                Ok(Ok(_)) if matches!(byte[0], b'\n' | b'\r') => return Ok(answer),
                Ok(Ok(_)) => {
                    if answer.len() == MAX_ANSWER_BYTES {
                        return Err(io::Error::other("approval answer exceeds input limit"));
                    }
                    answer.push(byte[0]);
                }
                Ok(Err(error)) if error.kind() == io::ErrorKind::Interrupted => continue,
                Ok(Err(error)) => return Err(error),
                Err(_) => continue,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::escalation::HeldActionKind;

    fn request() -> HeldAction {
        HeldAction {
            id: "0123456789abcdef".into(),
            agent_id: "fixture".into(),
            kind: HeldActionKind::ToolCall,
            summary: "tool_call fixture".into(),
            reason: "exact-call human approval required".into(),
            context_snapshot: Some(
                serde_json::json!({"invocation":{"arguments":{"note":"\u{1b}[2J\rFAKE\n\u{202e}text\u{2066}\u{7f}😀"}}}),
            ),
            created_at: chrono::Utc::now(),
            expires_at: chrono::Utc::now() + chrono::Duration::seconds(120),
            status: HeldStatus::Pending,
        }
    }

    #[test]
    fn terminal_controls_and_unicode_are_visible_without_changing_the_request() {
        let request = request();
        let rendered = render_approval_request(&request).unwrap();
        assert!(rendered
            .bytes()
            .all(|b| b == b'\n' || (0x20..=0x7e).contains(&b)));
        assert!(rendered.contains("\\u202e"));
        assert!(rendered.contains("\\ud83d\\ude00"));
        let decoded: HeldAction = serde_json::from_str(&rendered).unwrap();
        assert_eq!(decoded.context_snapshot, request.context_snapshot);
        assert_eq!(decoded.id, request.id);
    }

    #[test]
    fn oversized_requests_are_refused_instead_of_displaying_a_prefix() {
        let mut request = request();
        for text in [
            "a".repeat(MAX_DISPLAY_BYTES),
            "\u{202e}".repeat(MAX_DISPLAY_BYTES / 4),
        ] {
            request.reason = text;
            assert!(render_approval_request(&request)
                .unwrap_err()
                .contains("display limit"));
        }
    }

    #[test]
    fn invalid_and_expired_request_ids_cannot_be_prompted() {
        let mut request = request();
        request.id = "id\nType approve".into();
        assert!(render_approval_request(&request).is_err());
        request.id = "0123456789abcdef".into();
        request.expires_at = chrono::Utc::now();
        assert!(render_approval_request(&request).is_err());
        request.expires_at = chrono::Utc::now() + chrono::Duration::seconds(120);
        request.status = HeldStatus::Approved;
        assert!(render_approval_request(&request).is_err());
    }

    #[test]
    fn an_answer_must_name_this_exact_request() {
        let id = "0123456789abcdef";
        assert!(matches!(
            terminal_decision(id, b"approve 0123456789abcdef"),
            Decision::Approve { .. }
        ));
        for answer in [
            b"yes".as_slice(),
            b"y",
            b"approve",
            b"approve fedcba9876543210",
            b"approve 0123456789abcdef extra",
            b" approve 0123456789abcdef",
            b"APPROVE 0123456789abcdef",
            b"",
        ] {
            assert!(matches!(
                terminal_decision(id, answer),
                Decision::Deny { .. }
            ));
        }
    }
}
