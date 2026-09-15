//! Chat HITL: resolve held actions from `/symbi gate approve|deny <id>` (allowlisted)
//! and post approval prompts to a configured channel.
use crate::escalation::{
    Approver, Decision, EscalationNotifier, EscalationQueue, HeldAction, ResolveError, Surface,
};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use symbi_channel_adapter::traits::InboundCommandInterceptor;
use symbi_channel_adapter::types::{ChatPlatform, InboundMessage, OutboundMessage};
use symbi_channel_adapter::ChannelAdapterManager;

/// Authorization is scoped per approval channel: a sender may only resolve held
/// actions from a `(platform, channel_id)` that is explicitly configured AND
/// lists them as an approver. This prevents (a) resolving from any channel the
/// bot happens to read, and (b) an approver authorized for one channel acting in
/// another.
pub type ChannelApprovers = HashMap<(ChatPlatform, String), HashSet<String>>;

/// Intercepts `/symbi gate approve|deny <id>` slash commands from allowlisted senders,
/// resolving held actions in the escalation queue.
pub struct EscalationCommandInterceptor {
    queue: Arc<EscalationQueue>,
    channel_approvers: ChannelApprovers,
}

impl EscalationCommandInterceptor {
    pub fn new(queue: Arc<EscalationQueue>, channel_approvers: ChannelApprovers) -> Self {
        Self {
            queue,
            channel_approvers,
        }
    }

    /// Is `sender` allowed to resolve held actions in this message's exact
    /// `(platform, channel_id)`? Fail-closed: unknown channel or unknown sender
    /// both deny.
    fn is_authorized(&self, msg: &InboundMessage) -> bool {
        self.channel_approvers
            .get(&(msg.platform, msg.channel_id.clone()))
            .map(|approvers| approvers.contains(&msg.sender_id))
            .unwrap_or(false)
    }

    fn parse(msg: &InboundMessage) -> Option<Vec<String>> {
        if let Some(cmd) = &msg.command {
            return (cmd.name == "symbi" && cmd.subcommand.as_deref() == Some("gate"))
                .then(|| cmd.args.clone());
        }
        let parts: Vec<&str> = msg.content.split_whitespace().collect();
        if parts.len() >= 2 && parts[0] == "/symbi" && parts[1] == "gate" {
            Some(parts[2..].iter().map(|part| (*part).to_owned()).collect())
        } else {
            None
        }
    }
}

#[async_trait::async_trait]
impl InboundCommandInterceptor for EscalationCommandInterceptor {
    async fn try_handle(&self, msg: &InboundMessage) -> Option<String> {
        let parts = Self::parse(msg)?;
        if !self.is_authorized(msg) {
            return Some(
                "You are not authorized to review or resolve held actions in this channel.".into(),
            );
        }
        let valid = match parts.first().map(String::as_str) {
            Some("show") => parts.len() == 2,
            Some("approve") => parts.len() == 3,
            Some("deny") => parts.len() == 2,
            _ => false,
        };
        if !valid {
            return Some(
                "Usage: /symbi gate show <id>, approve <id> <review-digest>, or deny <id>.".into(),
            );
        }
        let sub = &parts[0];
        let id = &parts[1];
        if id.len() != 16 || !id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Some("Invalid held action ID.".into());
        }
        if sub != "deny" {
            let action = self
                .queue
                .list_pending_async()
                .await
                .into_iter()
                .find(|action| action.id == *id);
            let Some(action) = action else {
                return Some(format!("Held action {id} is no longer pending."));
            };
            let review = match chat_review(&action) {
                Ok(review) => review,
                Err(_) => return Some(unavailable_review(id)),
            };
            if sub == "show" {
                return Some(review.content);
            }
            if parts[2] != review.digest {
                return Some(format!("Review does not match. Use /symbi gate show {id} and copy its complete approval command."));
            }
        }
        let approver = Approver {
            surface: Surface::Chat,
            id: format!(
                "{}:{}:{}:{}",
                msg.platform, msg.workspace_id, msg.channel_id, msg.sender_id
            ),
            display: msg.sender_name.clone(),
        };
        let decision = if sub == "approve" {
            Decision::Approve {
                reason: Some(format!("chat review {}", parts[2])),
            }
        } else {
            Decision::Deny { reason: None }
        };
        match self.queue.resolve_async(id, decision, approver).await {
            Ok(()) => Some(format!(
                "Held action {id} {}.",
                if sub == "approve" {
                    "approved"
                } else {
                    "denied"
                }
            )),
            Err(ResolveError::NotFound) => Some(format!("Unknown held action {id}.")),
            Err(ResolveError::AlreadyResolved) => {
                Some(format!("Held action {id} was already resolved."))
            }
            Err(ResolveError::Expired) => Some(format!("Held action {id} expired.")),
        }
    }
}

// A local transport budget, including all JSON and both decision commands.
// Larger requests remain reviewable through authenticated REST or the Gate panel.
const MAX_CHAT_REVIEW_BYTES: usize = 8 * 1024;

struct ChatReview {
    content: String,
    digest: String,
}

fn chat_review(action: &HeldAction) -> Result<ChatReview, String> {
    let json = super::render_approval_request(action)?;
    let mut escaped = String::with_capacity(json.len());
    for ch in json.chars() {
        // These characters occur only inside JSON strings. Escaping them leaves
        // a complete parseable document and prevents fences, mentions and links.
        if matches!(
            ch,
            '`' | '<' | '>' | '&' | '*' | '_' | '~' | '#' | '!' | '/'
        ) {
            use std::fmt::Write;
            write!(escaped, "\\u{:04x}", ch as u32).map_err(|error| error.to_string())?;
        } else {
            escaped.push(ch);
        }
        if escaped.len() > MAX_CHAT_REVIEW_BYTES {
            return Err("complete chat review exceeds limit".into());
        }
    }
    let digest = hex::encode(Sha256::digest(escaped.as_bytes()));
    let content = format!(
        "Held action {}. Review the complete request and expiry.\n```json\n{}\n```\nApprove: /symbi gate approve {} {}\nDeny: /symbi gate deny {}",
        action.id, escaped, action.id, digest, action.id,
    );
    if content.len() > MAX_CHAT_REVIEW_BYTES {
        return Err("complete chat review exceeds limit".into());
    }
    Ok(ChatReview { content, digest })
}

fn unavailable_review(id: &str) -> String {
    format!("Held action {id} cannot be completely reviewed in chat. Use the terminal, Gate panel or authenticated approval API. Chat approval is disabled; /symbi gate deny {id} remains available.")
}

/// Posts approval-prompt messages to a chat channel whenever a new action is held.
pub struct ChatEscalationNotifier {
    manager: Arc<ChannelAdapterManager>,
    platform: ChatPlatform,
    channel_id: String,
}

impl ChatEscalationNotifier {
    pub fn new(
        manager: Arc<ChannelAdapterManager>,
        platform: ChatPlatform,
        channel_id: String,
    ) -> Self {
        Self {
            manager,
            platform,
            channel_id,
        }
    }
}

#[async_trait::async_trait]
impl EscalationNotifier for ChatEscalationNotifier {
    async fn notify(&self, action: &HeldAction) {
        // Invalid IDs cannot be included even in a fallback notification.
        if action.id.len() != 16 || !action.id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return;
        }
        let content = chat_review(action)
            .map(|review| review.content)
            .unwrap_or_else(|_| unavailable_review(&action.id));
        let msg = OutboundMessage {
            channel_id: self.channel_id.clone(),
            thread_id: None,
            content,
            blocks: None,
            ephemeral: false,
            user_id: None,
            metadata: None,
        };
        let _ = self.manager.send_to(self.platform, msg).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::escalation::{EscalationQueue, EscalationRequest, HeldActionKind};
    use std::collections::{HashMap, HashSet};
    use std::sync::Arc;
    use std::time::Duration;
    use symbi_channel_adapter::traits::InboundCommandInterceptor;
    use symbi_channel_adapter::types::{ChatPlatform, InboundMessage, SlashCommand};

    /// Allowlist: Slack `C0APPROVERS` → {U0ALICE}.
    fn approvers() -> ChannelApprovers {
        let mut m: ChannelApprovers = HashMap::new();
        m.insert(
            (ChatPlatform::Slack, "C0APPROVERS".to_string()),
            HashSet::from(["U0ALICE".to_string()]),
        );
        m
    }

    fn inbound_in(channel: &str, sender: &str, sub: &str, id: &str) -> InboundMessage {
        InboundMessage {
            id: "m".into(),
            platform: ChatPlatform::Slack,
            workspace_id: "w".into(),
            channel_id: channel.into(),
            thread_id: None,
            sender_id: sender.into(),
            sender_name: sender.into(),
            content: format!("/symbi gate {sub} {id}"),
            command: Some(SlashCommand {
                name: "symbi".into(),
                subcommand: Some("gate".into()),
                args: vec![sub.into(), id.into()],
                agent_name: None,
            }),
            timestamp: chrono::Utc::now(),
            raw_payload: None,
        }
    }

    fn inbound(sender: &str, sub: &str, id: &str) -> InboundMessage {
        inbound_in("C0APPROVERS", sender, sub, id)
    }

    #[tokio::test]
    async fn allowlisted_sender_can_approve() {
        let q = Arc::new(EscalationQueue::new());
        let icpt = EscalationCommandInterceptor::new(q.clone(), approvers());
        let q2 = q.clone();
        let h = tokio::spawn(async move {
            q2.enqueue(
                EscalationRequest {
                    agent_id: "a".into(),
                    kind: HeldActionKind::ToolCall,
                    summary: "s".into(),
                    reason: "r".into(),
                    context_snapshot: None,
                },
                Duration::from_secs(5),
            )
            .await
        });
        let id = loop {
            if let Some(x) = q.list_pending_async().await.first() {
                break x.id.clone();
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        };
        let review = icpt
            .try_handle(&inbound("U0ALICE", "show", &id))
            .await
            .unwrap();
        let digest = review
            .lines()
            .find(|line| line.starts_with("Approve: "))
            .unwrap()
            .split_whitespace()
            .last()
            .unwrap();
        let mut approval = inbound("U0ALICE", "approve", &id);
        approval
            .command
            .as_mut()
            .unwrap()
            .args
            .push(digest.to_owned());
        let reply = icpt.try_handle(&approval).await;
        assert!(reply.unwrap().to_lowercase().contains("approved"));
        assert!(matches!(
            h.await.unwrap(),
            crate::escalation::Decision::Approve { .. }
        ));
    }

    #[tokio::test]
    async fn non_allowlisted_sender_is_rejected() {
        let q = Arc::new(EscalationQueue::new());
        let icpt = EscalationCommandInterceptor::new(q.clone(), approvers());
        let reply = icpt
            .try_handle(&inbound("U0MALLORY", "approve", "0000"))
            .await;
        assert!(reply.unwrap().to_lowercase().contains("not authorized"));
    }

    #[tokio::test]
    async fn approver_rejected_from_unconfigured_channel() {
        // U0ALICE is an approver for C0APPROVERS, but NOT for some other channel
        // the bot also reads. A resolve attempt from that channel must be denied.
        let q = Arc::new(EscalationQueue::new());
        let icpt = EscalationCommandInterceptor::new(q.clone(), approvers());
        let reply = icpt
            .try_handle(&inbound_in("C0RANDOM", "U0ALICE", "approve", "0000"))
            .await;
        assert!(reply.unwrap().to_lowercase().contains("not authorized"));
        // And nothing was resolved (no held action existed; the point is the
        // authorization gate fired before any resolve attempt).
    }

    #[tokio::test]
    async fn non_gate_message_passes_through() {
        let q = Arc::new(EscalationQueue::new());
        let icpt = EscalationCommandInterceptor::new(q.clone(), approvers());
        let mut m = inbound("U0ALICE", "approve", "0000");
        m.command = None;
        m.content = "hello".into();
        assert!(icpt.try_handle(&m).await.is_none());
    }
    fn action() -> HeldAction {
        HeldAction {
            id: "0123456789abcdef".into(),
            agent_id: "fixture".into(),
            kind: HeldActionKind::ToolCall,
            summary: "tool_call fixture".into(),
            reason: "Review fixture".into(),
            context_snapshot: Some(
                serde_json::json!({"invocation":{"arguments":{"text":"<!here> ``` [link](https://example.invalid) *bold* _value_ \u{202e}\u{1b}\n\\u002f"}}}),
            ),
            created_at: chrono::Utc::now(),
            expires_at: chrono::Utc::now() + chrono::Duration::minutes(1),
            status: crate::escalation::HeldStatus::Pending,
        }
    }

    #[test]
    fn chat_review_preserves_every_argument_without_active_markup() {
        let action = action();
        let review = chat_review(&action).unwrap();
        let json = review
            .content
            .split_once("```json\n")
            .unwrap()
            .1
            .split_once("\n```")
            .unwrap()
            .0;
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(json).unwrap(),
            serde_json::to_value(&action).unwrap()
        );
        assert_eq!(review.digest, hex::encode(Sha256::digest(json.as_bytes())));
        assert!(json.is_ascii());
        for unsafe_text in ["<!here>", "```", "https://", "\u{202e}", "\u{1b}"] {
            assert!(!json.contains(unsafe_text));
        }
        assert!(review.content.len() <= MAX_CHAT_REVIEW_BYTES);
    }

    #[test]
    fn oversized_or_expired_review_has_no_chat_approval_command() {
        let mut action = action();
        action.summary = "x".repeat(MAX_CHAT_REVIEW_BYTES);
        assert!(chat_review(&action).is_err());
        assert!(!unavailable_review(&action.id).contains("/symbi gate approve"));
        action.summary.clear();
        action.expires_at = chrono::Utc::now() - chrono::Duration::seconds(1);
        assert!(chat_review(&action).is_err());
    }

    #[test]
    fn review_digest_changes_with_request_identity_arguments_and_expiry() {
        let action = action();
        let original = chat_review(&action).unwrap().digest;
        for index in 0..3 {
            let mut changed = action.clone();
            match index {
                0 => changed.id = "fedcba9876543210".into(),
                1 => changed.context_snapshot = Some(serde_json::json!({"different":"arguments"})),
                _ => changed.expires_at += chrono::Duration::seconds(1),
            }
            assert_ne!(original, chat_review(&changed).unwrap().digest);
        }
    }

    #[tokio::test]
    async fn chat_approvals_require_the_exact_review_and_reject_extra_arguments() {
        let queue = Arc::new(EscalationQueue::new());
        let interceptor = EscalationCommandInterceptor::new(queue.clone(), approvers());
        let active = queue.clone();
        let task = tokio::spawn(async move {
            active
                .enqueue(
                    EscalationRequest {
                        agent_id: "fixture".into(),
                        kind: HeldActionKind::ToolCall,
                        summary: "s".into(),
                        reason: "r".into(),
                        context_snapshot: None,
                    },
                    Duration::from_secs(3),
                )
                .await
        });
        let held = tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if let Some(action) = queue.list_pending_async().await.first() {
                    break action.clone();
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let valid = chat_review(&held).unwrap().digest;
        for suffix in [
            vec![],
            vec!["incorrect".to_owned()],
            vec![valid.clone(), "extra".into()],
        ] {
            let mut message = inbound("U0ALICE", "approve", &held.id);
            message.command.as_mut().unwrap().args.extend(suffix);
            let reply = interceptor.try_handle(&message).await.unwrap();
            assert!(!reply.contains("approved."), "{reply}");
            assert_eq!(queue.list_pending_async().await.len(), 1);
        }
        let mut authorized = inbound("U0ALICE", "approve", &held.id);
        authorized.command.as_mut().unwrap().args.push(valid);
        assert!(interceptor
            .try_handle(&authorized)
            .await
            .unwrap()
            .contains("approved."));
        assert!(matches!(task.await.unwrap(), Decision::Approve { .. }));
        assert!(interceptor
            .try_handle(&authorized)
            .await
            .unwrap()
            .contains("no longer pending"));
    }

    #[tokio::test]
    async fn command_prefix_and_authorized_review_are_required() {
        let interceptor =
            EscalationCommandInterceptor::new(Arc::new(EscalationQueue::new()), approvers());
        let mut message = inbound("U0ALICE", "show", "0123456789abcdef");
        message.command = None;
        for text in [
            "anything gate show 0123456789abcdef",
            "/other gate show 0123456789abcdef",
            "hello /symbi gate show 0123456789abcdef",
        ] {
            message.content = text.into();
            assert!(interceptor.try_handle(&message).await.is_none());
        }
        message.content = "/symbi gate show 0123456789abcdef".into();
        message.sender_id = "untrusted".into();
        assert!(interceptor
            .try_handle(&message)
            .await
            .unwrap()
            .contains("not authorized"));
        message.sender_id = "U0ALICE".into();
        message.channel_id = "different-channel".into();
        assert!(interceptor
            .try_handle(&message)
            .await
            .unwrap()
            .contains("not authorized"));
    }
    #[tokio::test]
    async fn oversized_pending_request_cannot_be_approved_in_chat() {
        let queue = Arc::new(EscalationQueue::new());
        let interceptor = EscalationCommandInterceptor::new(queue.clone(), approvers());
        let active = queue.clone();
        let task = tokio::spawn(async move {
            active
                .enqueue(
                    EscalationRequest {
                        agent_id: "fixture".into(),
                        kind: HeldActionKind::ToolCall,
                        summary: "x".repeat(MAX_CHAT_REVIEW_BYTES),
                        reason: "fixture".into(),
                        context_snapshot: None,
                    },
                    Duration::from_secs(3),
                )
                .await
        });
        let held = tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if let Some(held) = queue.list_pending_async().await.first() {
                    break held.clone();
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let mut message = inbound("U0ALICE", "approve", &held.id);
        message.command.as_mut().unwrap().args.push("0".repeat(64));
        assert!(interceptor
            .try_handle(&message)
            .await
            .unwrap()
            .contains("Chat approval is disabled"));
        assert_eq!(queue.list_pending_async().await.len(), 1);
        assert!(interceptor
            .try_handle(&inbound("U0ALICE", "deny", &held.id))
            .await
            .unwrap()
            .contains("denied."));
        assert!(matches!(task.await.unwrap(), Decision::Deny { .. }));
    }
}
