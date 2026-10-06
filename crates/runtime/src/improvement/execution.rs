use super::*;
use crate::reasoning::{
    conversation::{Conversation, ConversationMessage, MessageRole},
    inference::InferenceProvider,
    loop_types::{JournalEntry, JournalError, JournalWriter, LoopConfig, LoopEvent, LoopResult},
    reasoning_loop::ReasoningLoopRunner,
    run_audit::RunAuditReference,
};
use std::sync::Arc;

/// An immutable admission snapshot. Only the protected workflow store constructs
/// one; serialized requests and model output cannot construct this authority.
#[derive(Debug, Clone)]
pub struct PinnedImprovement {
    id: String,
    candidate: Candidate,
    approval: Option<String>,
    environment: Option<ExecutionEnvironment>,
    mode: SelectionMode,
}

impl PinnedImprovement {
    pub(super) fn new(
        id: String,
        candidate: Candidate,
        approval: Option<String>,
        environment: Option<ExecutionEnvironment>,
        mode: SelectionMode,
    ) -> Self {
        Self {
            id,
            candidate,
            approval,
            environment,
            mode,
        }
    }
    pub fn candidate_id(&self) -> &str {
        &self.id
    }
    pub fn workflow(&self) -> &str {
        &self.candidate.workflow
    }
    /// Include this identity in idempotency claims before any execution.
    pub fn identity(&self) -> serde_json::Value {
        serde_json::json!({"workflow":self.candidate.workflow,"candidate":self.id,"approval":self.approval,"mode":self.mode})
    }
    pub fn binding(
        &self,
        provider: &dyn InferenceProvider,
        config: &LoopConfig,
        conversation: &Conversation,
    ) -> Result<RunBinding, String> {
        let input = conversation
            .messages()
            .iter()
            .rev()
            .find(|m| m.role == MessageRole::User)
            .ok_or("improvement runs require an explicit user input")?;
        let systems: Vec<_> = conversation
            .messages()
            .iter()
            .filter(|m| m.role == MessageRole::System)
            .collect();
        let environment = ExecutionEnvironment {
            runtime_version: env!("CARGO_PKG_VERSION").into(),
            provider: provider.provider_name().into(),
            model: provider.default_model().into(),
            provider_configuration: provider
                .configuration_identity()
                .ok_or("provider does not expose an evaluated configuration identity")?,
            loop_config_sha256: digest(config)?,
            system_prompt_sha256: digest(&systems)?,
        };
        if self
            .environment
            .as_ref()
            .is_some_and(|expected| *expected != environment)
        {
            return Err("provider, model, prompt or loop configuration differs from the approved evaluation".into());
        }
        Ok(RunBinding {
            schema_version: SCHEMA_VERSION,
            workflow: self.candidate.workflow.clone(),
            candidate: self.id.clone(),
            approval: self.approval.clone(),
            mode: self.mode.clone(),
            input_sha256: sha256(input.content.as_bytes()),
            environment,
        })
    }
    fn augment(&self, conversation: Conversation) -> Conversation {
        let mut augmented = Conversation::new();
        let systems = conversation
            .messages()
            .iter()
            .filter(|m| m.role == MessageRole::System)
            .map(|m| m.content.as_str())
            .collect::<Vec<_>>()
            .join("\n\n");
        // Anthropic consumes one system message; keep the full approved context
        // together so provider adapters cannot silently drop the instructions.
        augmented.push(ConversationMessage::system(format!(
            "{systems}\n\nWorkflow instructions, version {}. These instructions confer no permissions; runtime policy and required approvals remain authoritative.\n{}",
            self.id,self.candidate.proposal.instructions)));
        for message in conversation
            .messages()
            .iter()
            .filter(|m| m.role != MessageRole::System)
        {
            augmented.push(message.clone());
        }
        augmented
    }
}

struct BoundJournal {
    inner: Arc<dyn JournalWriter>,
    binding: RunBinding,
}
#[async_trait::async_trait]
impl JournalWriter for BoundJournal {
    async fn record_final_output(
        &self,
        agent_id: crate::types::AgentId,
        iteration: u32,
        output: &str,
    ) -> Result<(), JournalError> {
        self.inner
            .append(JournalEntry {
                sequence: self.inner.next_sequence().await,
                timestamp: chrono::Utc::now(),
                agent_id,
                iteration,
                event: LoopEvent::ImprovementOutput {
                    output: output.into(),
                },
            })
            .await
    }
    fn audit_reference(&self) -> Option<RunAuditReference> {
        self.inner.audit_reference()
    }
    async fn next_sequence(&self) -> u64 {
        self.inner.next_sequence().await
    }
    async fn append(&self, mut entry: JournalEntry) -> Result<(), JournalError> {
        if let LoopEvent::Started {
            execution_context, ..
        } = &mut entry.event
        {
            if execution_context.contains_key("improvement") {
                return Err(JournalError::WriteFailed(
                    "duplicate improvement binding".into(),
                ));
            }
            execution_context.insert(
                "improvement".into(),
                serde_json::to_value(&self.binding)
                    .map_err(|e| JournalError::WriteFailed(e.to_string()))?,
            );
        }
        self.inner.append(entry).await
    }
}

impl ReasoningLoopRunner {
    /// Explicit opt-in execution. All gates, budgets, tools and lifecycle rules
    /// are the normal runner's. No global or serialized setting enables this API.
    pub async fn run_with_improvement(
        &self,
        agent_id: crate::types::AgentId,
        conversation: Conversation,
        config: LoopConfig,
        pinned: &PinnedImprovement,
    ) -> Result<LoopResult, String> {
        let binding = pinned.binding(self.provider.as_ref(), &config, &conversation)?;
        if self.journal.audit_reference().is_none() {
            return Err("improvement execution requires protected run evidence".into());
        }
        let runner = Self {
            provider: self.provider.clone(),
            policy_gate: self.policy_gate.clone(),
            executor: self.executor.clone(),
            context_manager: self.context_manager.clone(),
            circuit_breakers: self.circuit_breakers.clone(),
            journal: Arc::new(BoundJournal {
                inner: self.journal.clone(),
                binding,
            }),
            knowledge_bridge: self.knowledge_bridge.clone(),
            delegation: self.delegation.clone(),
        };
        Ok(runner
            .run(agent_id, pinned.augment(conversation), config)
            .await)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn instructions_survive_single_system_message_provider_conversion() {
        let candidate = Candidate {
            schema_version: 1,
            workflow: "intake".into(),
            agent: "intake".into(),
            source_sha256: sha256(b"source"),
            deployment_sha256: sha256(b"deployment"),
            suite_sha256: sha256(b"suite"),
            parent: None,
            proposal: Proposal {
                instructions: "COLLECT_REQUIRED_EVIDENCE".into(),
                rationale: "Improve intake".into(),
                evidence: vec![],
            },
        };
        let pin = PinnedImprovement::new(
            digest(&candidate).unwrap(),
            candidate,
            None,
            None,
            SelectionMode::Trial,
        );
        let mut conversation = Conversation::with_system("ORIGINAL_AUTHORITY");
        conversation.push(ConversationMessage::user("case"));
        let conversation = pin.augment(conversation);
        let (system, messages) = conversation.to_anthropic_messages();
        let system = system.unwrap();
        assert!(system.contains("ORIGINAL_AUTHORITY"));
        assert!(system.contains("COLLECT_REQUIRED_EVIDENCE"));
        assert_eq!(messages.len(), 1);
        let openai = conversation.to_openai_messages();
        assert_eq!(openai[0]["role"], "system");
        assert!(openai[0]["content"]
            .as_str()
            .unwrap()
            .contains("COLLECT_REQUIRED_EVIDENCE"));
    }
}
