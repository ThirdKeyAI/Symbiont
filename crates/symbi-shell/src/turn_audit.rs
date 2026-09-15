//! Durable per-turn journals with retained run and cancellation ownership.

use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{Arc, Mutex},
};
use symbi_runtime::{
    reasoning::{
        conversation::Conversation,
        executor::ActionExecutor,
        inference::InferenceProvider,
        loop_types::{
            BufferedJournal, JournalEntry, JournalError, JournalWriter, LoopConfig, LoopResult,
        },
        policy_bridge::ReasoningPolicyGate,
        reasoning_loop::ReasoningLoopRunner,
        run_audit::{open_run_journal, RunAuditReference},
    },
    types::AgentId,
};
use tokio_util::sync::CancellationToken;

#[derive(Default, Clone, serde::Serialize)]
pub struct AuditReferences {
    pub entries: VecDeque<RunAuditReference>,
    pub omitted: u64,
}

pub struct TurnAudit {
    pub display: Arc<BufferedJournal>,
    owners: tokio_util::task::TaskTracker,
    display_lock: tokio::sync::Mutex<()>,
    references: Mutex<AuditReferences>,
}

impl Default for TurnAudit {
    fn default() -> Self {
        Self {
            display: Arc::new(BufferedJournal::new(1000)),
            owners: tokio_util::task::TaskTracker::new(),
            display_lock: tokio::sync::Mutex::new(()),
            references: Mutex::new(AuditReferences::default()),
        }
    }
}

impl TurnAudit {
    /// Graceful shell exit waits for retained owners, including cancelled calls.
    pub async fn close(&self) -> Result<(), String> {
        self.owners.close();
        tokio::time::timeout(std::time::Duration::from_secs(25), self.owners.wait())
            .await
            .map_err(|_| {
                "shell audit owners did not finish before shutdown; evidence may be incomplete"
                    .into()
            })
    }

    pub fn references(&self) -> Result<AuditReferences, String> {
        self.references
            .lock()
            .map(|history| history.clone())
            .map_err(|_| "shell audit reference display is unavailable".into())
    }

    fn publish(&self, reference: RunAuditReference) -> Result<(), String> {
        let mut history = self
            .references
            .lock()
            .map_err(|_| "shell audit reference display is unavailable")?;
        if history.entries.len() == 256 {
            history.entries.pop_front();
            history.omitted = history.omitted.saturating_add(1);
        }
        history.entries.push_back(reference);
        Ok(())
    }
}

struct Journal {
    durable: Arc<dyn JournalWriter>,
    audit: Arc<TurnAudit>,
}

#[async_trait::async_trait]
impl JournalWriter for Journal {
    async fn append(&self, entry: JournalEntry) -> Result<(), JournalError> {
        self.durable.append(entry.clone()).await?;
        // The live display has a session-wide sequence, independent of each
        // signed journal's per-run sequence. It cannot authorize an effect.
        let _display = self.audit.display_lock.lock().await;
        let mut entry = entry;
        entry.sequence = self.audit.display.next_sequence().await;
        self.audit.display.append(entry).await
    }

    async fn next_sequence(&self) -> u64 {
        self.durable.next_sequence().await
    }
}

#[derive(Clone)]
pub struct TurnRuntime {
    pub provider: Arc<dyn InferenceProvider>,
    pub executor: Arc<dyn ActionExecutor>,
    pub gate: Arc<dyn ReasoningPolicyGate>,
    pub project: Result<PathBuf, String>,
}

struct CancelOnDrop(CancellationToken);
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

impl TurnRuntime {
    pub async fn run(
        &self,
        agent: AgentId,
        conversation: Conversation,
        config: LoopConfig,
        audit: Arc<TurnAudit>,
    ) -> Result<(LoopResult, RunAuditReference), String> {
        let owner_token = audit.owners.token();
        if audit.owners.is_closed() {
            return Err("shell turn admission is closed".into());
        }
        let runtime = self.clone();
        let cancel = CancellationToken::new();
        let _cancellation = CancelOnDrop(cancel.clone());
        // Ownership begins before journal initialization, including its blocking
        // filesystem work. Dropping the caller requests graceful termination.
        let owner = tokio::spawn(async move {
            let _owner_token = owner_token;
            let project = runtime.project?;
            let (durable, reference) = open_run_journal(&project, agent)
                .await
                .map_err(|error| format!("Required shell audit initialization failed: {error}"))?;
            audit.publish(reference.clone())?;
            let journal = Arc::new(Journal { durable, audit });
            let runner = ReasoningLoopRunner::builder()
                .provider(runtime.provider)
                .executor(runtime.executor)
                .policy_gate(runtime.gate)
                .journal(journal)
                .build();
            let result = runner
                .run_cancellable(agent, conversation, config, cancel)
                .await;
            Ok((result, reference))
        });
        owner
            .await
            .map_err(|error| format!("Shell turn owner failed: {error}"))?
    }
}
