//! Governed execution of one scheduled invocation.

use super::{
    task_manager::{TaskCompletion, TaskStatus},
    ScheduledTask,
};
use crate::{
    reasoning::{
        circuit_breaker::CircuitBreakerRegistry,
        context_manager::DefaultContextManager,
        conversation::{Conversation, ConversationMessage},
        inference::InferenceProvider,
        loop_types::{LoopConfig, TerminationReason},
        policy_bridge::ReasoningPolicyGate,
        ReasoningLoopRunner,
    },
    sandbox::command::{CommandBoundary, CommandTier},
    types::SecurityTier,
};
use async_trait::async_trait;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;

/// Implementations return only after their effects have stopped or cleanup has
/// explicitly failed. A queue admission is never a completed execution.
#[async_trait]
pub trait ScheduledAgentExecutor: Send + Sync {
    /// Trusted storage root for durable admission. Custom services opt in explicitly.
    #[cfg(unix)]
    fn invocation_project(&self) -> Result<&Path, String> {
        Err("execution service does not support persistent admission".into())
    }

    async fn execute(
        &self,
        task: &ScheduledTask,
        budget: Duration,
        cancellation: CancellationToken,
    ) -> TaskCompletion;
}

/// The project and injected services are trusted operator configuration.
pub struct GovernedAgentExecutor {
    project: Result<PathBuf, String>,
    provider: Option<Arc<dyn InferenceProvider>>,
    gate: Option<Arc<dyn ReasoningPolicyGate>>,
}

impl Default for GovernedAgentExecutor {
    fn default() -> Self {
        Self {
            project: std::env::current_dir()
                .and_then(|p| p.canonicalize())
                .map_err(|e| e.to_string()),
            provider: None,
            gate: None,
        }
    }
}

impl GovernedAgentExecutor {
    pub fn new(project: &Path) -> Result<Self, String> {
        Ok(Self {
            project: Ok(project.canonicalize().map_err(|e| e.to_string())?),
            provider: None,
            gate: None,
        })
    }
    pub fn with_provider(mut self, provider: Arc<dyn InferenceProvider>) -> Self {
        self.provider = Some(provider);
        self
    }
    pub fn with_policy_gate(mut self, gate: Arc<dyn ReasoningPolicyGate>) -> Self {
        self.gate = Some(gate);
        self
    }

    #[cfg(unix)]
    async fn execute_inner(
        &self,
        task: &ScheduledTask,
        budget: Duration,
        cancellation: CancellationToken,
    ) -> Result<TaskCompletion, String> {
        if cancellation.is_cancelled() || budget.is_zero() {
            return Ok(TaskCompletion::new(
                task,
                if budget.is_zero() {
                    TaskStatus::TimedOut
                } else {
                    TaskStatus::Terminated
                },
                Some("scheduled invocation cancelled or expired before execution".into()),
            ));
        }
        if matches!(
            task.config.execution_mode,
            crate::types::ExecutionMode::External { .. }
        ) {
            return Err("external agents are not executed by the scheduler".into());
        }
        if task.route_decision.is_some() {
            return Err(
                "the scheduled executor cannot honor a routed provider or sandbox selection".into(),
            );
        }
        let started = Instant::now();
        let project = self.project.as_ref().map_err(Clone::clone)?.clone();
        let source_policy =
            dsl::ExecutionPolicy::parse(&task.config.dsl_source, &task.config.name)?;
        let settings = source_policy.settings().clone();
        let tree = dsl::parse_dsl(&task.config.dsl_source).map_err(|e| e.to_string())?;
        let metadata = dsl::extract_metadata(&tree, &task.config.dsl_source);
        if metadata
            .get("executor")
            .is_some_and(|kind| kind.trim_matches('"') != "orga")
        {
            return Err("the scheduled executor requires an ORGA agent; managed CLI scheduling is unavailable".into());
        }
        let limits = task.config.resource_limits.clone();
        let tier = task.config.security_tier.clone();
        let frozen_settings = settings.clone();
        let executor = tokio::task::spawn_blocking(move || {
            let mut boundary = CommandBoundary::load_for_agent(&project, &frozen_settings)?;
            let selected = match boundary.tier {
                CommandTier::DevelopmentHost => SecurityTier::None,
                CommandTier::Docker => SecurityTier::Tier1,
                CommandTier::GVisor => SecurityTier::Tier2,
                CommandTier::Firecracker => SecurityTier::Tier3,
                CommandTier::E2B => SecurityTier::Hosted,
                // The landlock tier has no registered SecurityTier equivalent,
                // so a scheduled agent cannot declare it. Selecting it for a
                // registered run is a configuration error, not a silent
                // downgrade to a neighbouring tier.
                CommandTier::Landlock => {
                    return Err(
                        "the landlock tier cannot be selected for a registered agent; \
                         declare it in [sandbox] for direct runs"
                            .to_string(),
                    )
                }
            };
            if selected != tier {
                return Err(
                    "registered security tier conflicts with the selected agent sandbox"
                        .to_string(),
                );
            }
            boundary.tighten_resources(&limits)?;
            let tools = crate::reasoning::tool_executor_builder::build_tool_executor_with_boundary(
                &project.join("tools"),
                boundary,
            );
            Ok(Arc::new(
                crate::reasoning::source_policy::SourcePolicyExecutor::new(tools, source_policy),
            ))
        })
        .await
        .map_err(|e| e.to_string())??;
        let provider = self
            .provider
            .clone()
            .or_else(provider_from_env)
            .ok_or("no inference provider configured for scheduled execution")?;
        let project = self.project.as_ref().map_err(Clone::clone)?;
        let gate = match &self.gate {
            Some(gate) => gate.clone(),
            None => {
                crate::reasoning::governed_gate(crate::reasoning::GateOptions {
                    policies_dir: project.join("policies"),
                    surface: Some("scheduler".into()),
                    insecure_allow_all: false,
                    escalation: None,
                })
                .await
            }
        };
        let (journal, audit) = if let Some(claimed) = &task.claimed_journal {
            (
                claimed.writer.clone(),
                super::task_manager::TaskAudit {
                    path: claimed.audit.path.clone(),
                    public_key: claimed.audit.public_key.clone(),
                },
            )
        } else {
            let journal = Arc::new(
                crate::reasoning::protected_journal::ProtectedJournal::create_run(
                    &project.join(".symbiont/governed"),
                    task.agent_id,
                    task.handle.run_id(),
                )
                .map_err(|e| e.to_string())?,
            );
            let audit = super::task_manager::TaskAudit {
                path: journal.path().to_path_buf(),
                public_key: hex::encode(journal.public_key()),
            };
            (
                journal as Arc<dyn crate::reasoning::loop_types::JournalWriter>,
                audit,
            )
        };
        let mut conversation = Conversation::with_system(format!(
            "You are agent {:?}. Execute the requested task using the available governed tools.\n--- Agent DSL ---\n{}\n--- End DSL ---",
            settings.agent_name, settings.agent_source,
        ));
        let input = if task.input.is_null() {
            "Execute this scheduled agent's configured task.".to_string()
        } else {
            serde_json::to_string(&task.input).map_err(|e| e.to_string())?
        };
        if input.len() > 1024 * 1024 {
            return Err("scheduled input exceeds 1 MiB".into());
        }
        conversation.push(ConversationMessage::user(input));
        let timeout = budget
            .saturating_sub(started.elapsed())
            .min(task.config.resource_limits.execution_timeout)
            .min(
                settings
                    .timeout_seconds
                    .map(Duration::from_secs)
                    .unwrap_or(budget),
            );
        let config = LoopConfig {
            timeout,
            ..Default::default()
        };
        let runner = ReasoningLoopRunner {
            provider,
            policy_gate: gate,
            executor,
            context_manager: Arc::new(DefaultContextManager::default()),
            circuit_breakers: Arc::new(CircuitBreakerRegistry::default()),
            journal,
            knowledge_bridge: None,
            delegation: None,
        };
        let result = runner
            .run_cancellable(task.agent_id, conversation, config, cancellation.clone())
            .await;
        let (status, error) = match result.termination_reason {
            TerminationReason::Completed if !cancellation.is_cancelled() => {
                (TaskStatus::Completed, None)
            }
            TerminationReason::Timeout => (
                TaskStatus::TimedOut,
                Some("scheduled invocation timed out".into()),
            ),
            TerminationReason::Error { message } if message == "agent execution cancelled" => {
                (TaskStatus::Terminated, Some(message))
            }
            reason => (
                TaskStatus::Failed,
                Some(format!("scheduled invocation did not complete: {reason:?}")),
            ),
        };
        let mut completion = TaskCompletion::new(task, status, error);
        completion.audit = Some(audit);
        completion.total_usage = Some(result.total_usage);
        completion.budget = result.budget;
        if completion.status == TaskStatus::Completed {
            if result.output.len() > 1024 * 1024 {
                completion.status = TaskStatus::Failed;
                completion.error = Some("scheduled output exceeds 1 MiB".into());
            } else {
                completion.output = Some(result.output);
            }
        }
        Ok(completion)
    }
}

#[async_trait]
impl ScheduledAgentExecutor for GovernedAgentExecutor {
    #[cfg(unix)]
    fn invocation_project(&self) -> Result<&Path, String> {
        self.project.as_deref().map_err(Clone::clone)
    }

    async fn execute(
        &self,
        task: &ScheduledTask,
        budget: Duration,
        cancellation: CancellationToken,
    ) -> TaskCompletion {
        #[cfg(unix)]
        {
            self.execute_inner(task, budget, cancellation)
                .await
                .unwrap_or_else(|error| TaskCompletion::new(task, TaskStatus::Failed, Some(error)))
        }
        #[cfg(not(unix))]
        {
            let _ = (budget, cancellation);
            TaskCompletion::new(
                task,
                TaskStatus::Failed,
                Some("protected scheduled execution requires Unix".into()),
            )
        }
    }
}

fn provider_from_env() -> Option<Arc<dyn InferenceProvider>> {
    #[cfg(feature = "cloud-llm")]
    {
        crate::reasoning::providers::cloud::CloudInferenceProvider::from_env()
            .map(|p| Arc::new(p) as Arc<dyn InferenceProvider>)
    }
    #[cfg(not(feature = "cloud-llm"))]
    {
        None
    }
}
