//! Chat HITL: resolve held actions from `/symbi gate approve|deny <id>` (allowlisted)
//! and post approval prompts to a configured channel.
use crate::escalation::{
    Approver, Decision, EscalationNotifier, EscalationQueue, HeldAction, ResolveError, Surface,
};
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

    fn parse(msg: &InboundMessage) -> Option<(String, String)> {
        if let Some(cmd) = &msg.command {
            if cmd.subcommand.as_deref() == Some("gate") && cmd.args.len() >= 2 {
                return Some((cmd.args[0].to_lowercase(), cmd.args[1].clone()));
            }
        }
        let parts: Vec<&str> = msg.content.split_whitespace().collect();
        if parts.len() >= 4 && parts[1] == "gate" {
            return Some((parts[2].to_lowercase(), parts[3].to_string()));
        }
        None
    }
}

#[async_trait::async_trait]
impl InboundCommandInterceptor for EscalationCommandInterceptor {
    async fn try_handle(&self, msg: &InboundMessage) -> Option<String> {
        let (sub, id) = Self::parse(msg)?;
        if sub != "approve" && sub != "deny" {
            return Some("Usage: /symbi gate approve|deny <id> [reason]".to_string());
        }
        if !self.is_authorized(msg) {
            return Some(format!(
                "\u{26d4} {} is not authorized to resolve held actions in this channel.",
                msg.sender_name
            ));
        }
        let approver = Approver {
            surface: Surface::Chat,
            id: msg.sender_id.clone(),
            display: msg.sender_name.clone(),
        };
        let decision = if sub == "approve" {
            Decision::Approve { reason: None }
        } else {
            Decision::Deny { reason: None }
        };
        match self.queue.resolve_async(&id, decision, approver).await {
            Ok(()) => Some(format!(
                "\u{2705} {} {}d held action {}.",
                msg.sender_name, sub, id
            )),
            Err(ResolveError::NotFound) => Some(format!("Unknown held action {id}.")),
            Err(ResolveError::AlreadyResolved) => {
                Some(format!("Held action {id} was already resolved."))
            }
        }
    }
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
        let content = format!(
            "\u{26a0} Held for approval \u{2014} agent `{}` wants to {} ({}).\n\
             Reply: `/symbi gate approve {}` or `/symbi gate deny {} [reason]`",
            action.agent_id, action.summary, action.reason, action.id, action.id
        );
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
        let reply = icpt.try_handle(&inbound("U0ALICE", "approve", &id)).await;
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
}
