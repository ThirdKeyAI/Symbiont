//! Saga orchestrator for multi-step tool sequences
//!
//! Provides forward execution with backward compensation on failure.
//! Tool actions are classified as ReadOnly, Compensatable, or Final,
//! enabling automatic rollback when a sequence fails partway through.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

/// Classification of a saga step's side effects.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum StepClassification {
    /// No side effects. Safe to skip during compensation.
    ReadOnly,
    /// Has side effects but can be reversed via a compensation action.
    Compensatable,
    /// Irreversible. Only permitted after all preceding steps succeed.
    Final,
}

/// A single step in a saga.
#[derive(Debug, Clone)]
pub struct SagaStep {
    /// Step name for logging and audit.
    pub name: String,
    /// Side-effect classification.
    pub classification: StepClassification,
    /// The forward action to execute.
    pub action: SagaAction,
    /// The compensation action (only for Compensatable steps).
    pub compensation: Option<SagaAction>,
}

/// An action in a saga (either forward or compensation).
#[derive(Debug, Clone)]
pub struct SagaAction {
    /// Tool name to invoke.
    pub tool_name: String,
    /// Arguments for the tool.
    pub arguments: serde_json::Value,
}

/// Result of a single saga step execution.
#[derive(Debug, Clone)]
pub struct StepResult {
    /// Step name.
    pub name: String,
    /// Whether the step succeeded.
    pub success: bool,
    /// Output from the step.
    pub output: String,
    /// Error message if failed.
    pub error: Option<String>,
}

/// Overall saga execution result.
#[derive(Debug, Clone)]
pub struct SagaResult {
    /// Whether the entire saga succeeded.
    pub success: bool,
    /// Results from each step (forward execution).
    pub step_results: Vec<StepResult>,
    /// Results from compensation steps (if saga failed).
    pub compensation_results: Vec<StepResult>,
    /// Summary of what happened.
    pub summary: String,
}

/// Unique key for idempotent execution of saga steps.
///
/// Enables safe retries of Final steps: if a step has already been executed
/// with a given key, re-execution is skipped.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct IdempotencyKey(pub String);

impl IdempotencyKey {
    /// Generate a new unique idempotency key.
    pub fn generate() -> Self {
        Self(uuid::Uuid::new_v4().to_string())
    }
}

/// Persistence hook called before Final steps execute.
///
/// Records intent before execution and outcome after, enabling recovery
/// of Final steps that were started but not confirmed (e.g., process crash).
#[async_trait::async_trait]
pub trait SagaCheckpoint: Send + Sync {
    /// Record that we intend to execute this Final step.
    async fn record_intent(&self, step: &SagaStep, key: &IdempotencyKey) -> Result<(), SagaError>;

    /// Record the outcome of a Final step execution.
    async fn record_outcome(&self, key: &IdempotencyKey, success: bool) -> Result<(), SagaError>;

    /// Return all intents that were recorded but have no corresponding outcome.
    async fn pending_intents(&self) -> Result<Vec<(SagaStep, IdempotencyKey)>, SagaError>;
}

/// In-memory checkpoint implementation.
///
/// Suitable for testing and single-process use. Pairs with the
/// `MemoryJournalStorage` philosophy — no external dependencies required.
pub struct InMemoryCheckpoint {
    intents: Mutex<Vec<(SagaStep, IdempotencyKey)>>,
    outcomes: Mutex<HashMap<String, bool>>,
}

impl Default for InMemoryCheckpoint {
    fn default() -> Self {
        Self::new()
    }
}

impl InMemoryCheckpoint {
    pub fn new() -> Self {
        Self {
            intents: Mutex::new(Vec::new()),
            outcomes: Mutex::new(HashMap::new()),
        }
    }
}

#[async_trait::async_trait]
impl SagaCheckpoint for InMemoryCheckpoint {
    async fn record_intent(&self, step: &SagaStep, key: &IdempotencyKey) -> Result<(), SagaError> {
        self.intents.lock().await.push((step.clone(), key.clone()));
        Ok(())
    }

    async fn record_outcome(&self, key: &IdempotencyKey, success: bool) -> Result<(), SagaError> {
        self.outcomes.lock().await.insert(key.0.clone(), success);
        Ok(())
    }

    async fn pending_intents(&self) -> Result<Vec<(SagaStep, IdempotencyKey)>, SagaError> {
        let intents = self.intents.lock().await;
        let outcomes = self.outcomes.lock().await;
        Ok(intents
            .iter()
            .filter(|(_, key)| !outcomes.contains_key(&key.0))
            .cloned()
            .collect())
    }
}

/// Orchestrates saga execution with compensation.
pub struct SagaOrchestrator {
    steps: Vec<SagaStep>,
}

impl SagaOrchestrator {
    /// Create a new saga with the given steps.
    ///
    /// Validates that Final steps only appear after all Compensatable steps.
    pub fn new(steps: Vec<SagaStep>) -> Result<Self, SagaError> {
        // Validate: no Compensatable or ReadOnly steps after a Final step
        let mut seen_final = false;
        for step in &steps {
            if seen_final && step.classification != StepClassification::Final {
                return Err(SagaError::InvalidStepOrder {
                    step: step.name.clone(),
                    reason: "Non-final steps cannot appear after a Final step".into(),
                });
            }
            if step.classification == StepClassification::Final {
                seen_final = true;
            }
        }

        // Validate: Compensatable steps must have compensation actions
        for step in &steps {
            if step.classification == StepClassification::Compensatable
                && step.compensation.is_none()
            {
                return Err(SagaError::MissingCompensation {
                    step: step.name.clone(),
                });
            }
        }

        Ok(Self { steps })
    }

    /// Execute the saga using the provided executor function.
    ///
    /// The executor takes (tool_name, arguments) and returns (success, output).
    pub async fn execute<F, Fut>(&self, executor: F) -> SagaResult
    where
        F: Fn(String, serde_json::Value) -> Fut,
        Fut: std::future::Future<Output = Result<String, String>>,
    {
        self.execute_inner(&executor, None).await
    }

    /// Execute the saga with a checkpoint for Final step durability.
    ///
    /// Before each Final step, records intent via the checkpoint. After
    /// execution, records the outcome. On crash, `recover()` can identify
    /// Final steps that were intended but not confirmed.
    pub async fn execute_with_checkpoint<F, Fut>(
        &self,
        executor: F,
        checkpoint: Arc<dyn SagaCheckpoint>,
    ) -> SagaResult
    where
        F: Fn(String, serde_json::Value) -> Fut,
        Fut: std::future::Future<Output = Result<String, String>>,
    {
        self.execute_inner(&executor, Some(checkpoint)).await
    }

    async fn execute_inner<F, Fut>(
        &self,
        executor: &F,
        checkpoint: Option<Arc<dyn SagaCheckpoint>>,
    ) -> SagaResult
    where
        F: Fn(String, serde_json::Value) -> Fut,
        Fut: std::future::Future<Output = Result<String, String>>,
    {
        let mut step_results = Vec::new();
        let mut completed_compensatable: Vec<&SagaStep> = Vec::new();

        for step in &self.steps {
            // For Final steps with a checkpoint, record intent before execution
            let idempotency_key = if step.classification == StepClassification::Final {
                if let Some(cp) = &checkpoint {
                    let key = IdempotencyKey::generate();
                    if let Err(e) = cp.record_intent(step, &key).await {
                        return SagaResult {
                            success: false,
                            step_results,
                            compensation_results: self
                                .compensate(&completed_compensatable, executor)
                                .await,
                            summary: format!(
                                "Failed to record intent for Final step '{}': {}",
                                step.name, e
                            ),
                        };
                    }
                    Some(key)
                } else {
                    None
                }
            } else {
                None
            };

            let result =
                executor(step.action.tool_name.clone(), step.action.arguments.clone()).await;

            match result {
                Ok(output) => {
                    // Record successful outcome for Final steps
                    if let Some(key) = &idempotency_key {
                        if let Some(cp) = &checkpoint {
                            let _ = cp.record_outcome(key, true).await;
                        }
                    }

                    step_results.push(StepResult {
                        name: step.name.clone(),
                        success: true,
                        output,
                        error: None,
                    });

                    if step.classification == StepClassification::Compensatable {
                        completed_compensatable.push(step);
                    }
                }
                Err(error) => {
                    // Record failed outcome for Final steps
                    if let Some(key) = &idempotency_key {
                        if let Some(cp) = &checkpoint {
                            let _ = cp.record_outcome(key, false).await;
                        }
                    }

                    step_results.push(StepResult {
                        name: step.name.clone(),
                        success: false,
                        output: String::new(),
                        error: Some(error.clone()),
                    });

                    // Compensate in reverse order
                    let compensation_results =
                        self.compensate(&completed_compensatable, executor).await;

                    return SagaResult {
                        success: false,
                        step_results,
                        compensation_results,
                        summary: format!("Saga failed at step '{}': {}", step.name, error),
                    };
                }
            }
        }

        SagaResult {
            success: true,
            step_results,
            compensation_results: Vec::new(),
            summary: "Saga completed successfully".into(),
        }
    }

    /// Recover pending Final steps from a checkpoint.
    ///
    /// Returns the list of Final steps that were intended but never got an
    /// outcome recorded (e.g., due to process crash). The caller can then
    /// decide to re-execute or report them.
    pub async fn recover<F, Fut>(
        checkpoint: &dyn SagaCheckpoint,
        executor: F,
    ) -> Result<Vec<StepResult>, SagaError>
    where
        F: Fn(String, serde_json::Value) -> Fut,
        Fut: std::future::Future<Output = Result<String, String>>,
    {
        let pending = checkpoint.pending_intents().await?;
        let mut results = Vec::new();

        for (step, key) in &pending {
            let result =
                executor(step.action.tool_name.clone(), step.action.arguments.clone()).await;

            let success = result.is_ok();
            let _ = checkpoint.record_outcome(key, success).await;

            results.push(StepResult {
                name: format!("recover:{}", step.name),
                success,
                output: result.as_ref().cloned().unwrap_or_default(),
                error: result.err(),
            });
        }

        Ok(results)
    }

    async fn compensate<F, Fut>(&self, completed: &[&SagaStep], executor: &F) -> Vec<StepResult>
    where
        F: Fn(String, serde_json::Value) -> Fut,
        Fut: std::future::Future<Output = Result<String, String>>,
    {
        let mut results = Vec::new();

        // Compensate in reverse order
        for step in completed.iter().rev() {
            if let Some(compensation) = &step.compensation {
                let result = executor(
                    compensation.tool_name.clone(),
                    compensation.arguments.clone(),
                )
                .await;

                results.push(StepResult {
                    name: format!("compensate:{}", step.name),
                    success: result.is_ok(),
                    output: result.as_ref().cloned().unwrap_or_default(),
                    error: result.err(),
                });
            }
        }

        results
    }

    /// Get the step count.
    pub fn step_count(&self) -> usize {
        self.steps.len()
    }

    /// Get steps by classification.
    pub fn steps_by_classification(&self) -> HashMap<String, usize> {
        let mut counts = HashMap::new();
        for step in &self.steps {
            let key = format!("{:?}", step.classification);
            *counts.entry(key).or_insert(0) += 1;
        }
        counts
    }
}

/// Errors from the saga orchestrator.
#[derive(Debug, thiserror::Error)]
pub enum SagaError {
    #[error("Invalid step order for '{step}': {reason}")]
    InvalidStepOrder { step: String, reason: String },

    #[error("Compensatable step '{step}' is missing a compensation action")]
    MissingCompensation { step: String },

    #[error("Checkpoint operation failed: {0}")]
    CheckpointFailed(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read_step(name: &str) -> SagaStep {
        SagaStep {
            name: name.into(),
            classification: StepClassification::ReadOnly,
            action: SagaAction {
                tool_name: "read".into(),
                arguments: serde_json::json!({"path": name}),
            },
            compensation: None,
        }
    }

    fn write_step(name: &str) -> SagaStep {
        SagaStep {
            name: name.into(),
            classification: StepClassification::Compensatable,
            action: SagaAction {
                tool_name: "write".into(),
                arguments: serde_json::json!({"path": name, "data": "content"}),
            },
            compensation: Some(SagaAction {
                tool_name: "delete".into(),
                arguments: serde_json::json!({"path": name}),
            }),
        }
    }

    fn final_step(name: &str) -> SagaStep {
        SagaStep {
            name: name.into(),
            classification: StepClassification::Final,
            action: SagaAction {
                tool_name: "publish".into(),
                arguments: serde_json::json!({"target": name}),
            },
            compensation: None,
        }
    }

    #[test]
    fn test_valid_saga_creation() {
        let saga = SagaOrchestrator::new(vec![
            read_step("check"),
            write_step("create"),
            write_step("update"),
            final_step("publish"),
        ]);
        assert!(saga.is_ok());
        assert_eq!(saga.unwrap().step_count(), 4);
    }

    #[test]
    fn test_invalid_order_non_final_after_final() {
        let saga = SagaOrchestrator::new(vec![
            write_step("create"),
            final_step("publish"),
            read_step("check"), // Invalid: ReadOnly after Final
        ]);
        assert!(saga.is_err());
    }

    #[test]
    fn test_missing_compensation() {
        let step = SagaStep {
            name: "bad".into(),
            classification: StepClassification::Compensatable,
            action: SagaAction {
                tool_name: "write".into(),
                arguments: serde_json::json!({}),
            },
            compensation: None, // Missing!
        };
        let saga = SagaOrchestrator::new(vec![step]);
        assert!(saga.is_err());
    }

    #[tokio::test]
    async fn test_saga_all_succeed() {
        let saga = SagaOrchestrator::new(vec![
            read_step("check"),
            write_step("create"),
            write_step("update"),
        ])
        .unwrap();

        let result = saga
            .execute(|_tool, _args| async { Ok("success".to_string()) })
            .await;

        assert!(result.success);
        assert_eq!(result.step_results.len(), 3);
        assert!(result.compensation_results.is_empty());
    }

    #[tokio::test]
    async fn test_saga_fail_at_step_3_compensates_2_and_1() {
        let saga = SagaOrchestrator::new(vec![
            read_step("check"),
            write_step("create"),
            write_step("update"),
            write_step("finalize"),
            final_step("publish"),
        ])
        .unwrap();

        let call_count = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
        let cc = call_count.clone();

        let result = saga
            .execute(move |_tool, _args| {
                let count = cc.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                async move {
                    if count == 3 {
                        // Fail at step 4 (finalize)
                        Err("Connection refused".to_string())
                    } else {
                        Ok("ok".to_string())
                    }
                }
            })
            .await;

        assert!(!result.success);
        assert_eq!(result.step_results.len(), 4); // 3 succeeded + 1 failed
        assert!(!result.step_results[3].success);

        // Compensation should run for create and update (in reverse order)
        assert_eq!(result.compensation_results.len(), 2);
        assert_eq!(result.compensation_results[0].name, "compensate:update");
        assert_eq!(result.compensation_results[1].name, "compensate:create");
    }

    #[tokio::test]
    async fn test_saga_fail_at_readonly_no_compensation() {
        let saga = SagaOrchestrator::new(vec![read_step("check")]).unwrap();

        let result = saga
            .execute(|_tool, _args| async { Err("fail".to_string()) })
            .await;

        assert!(!result.success);
        assert!(result.compensation_results.is_empty());
    }

    #[test]
    fn test_steps_by_classification() {
        let saga = SagaOrchestrator::new(vec![
            read_step("r1"),
            write_step("w1"),
            write_step("w2"),
            final_step("f1"),
        ])
        .unwrap();

        let counts = saga.steps_by_classification();
        assert_eq!(counts.get("ReadOnly"), Some(&1));
        assert_eq!(counts.get("Compensatable"), Some(&2));
        assert_eq!(counts.get("Final"), Some(&1));
    }

    #[tokio::test]
    async fn test_final_step_with_checkpoint_records_intent_and_outcome() {
        let checkpoint = Arc::new(InMemoryCheckpoint::new());
        let saga =
            SagaOrchestrator::new(vec![write_step("prepare"), final_step("publish")]).unwrap();

        let result = saga
            .execute_with_checkpoint(
                |_tool, _args| async { Ok("done".to_string()) },
                checkpoint.clone(),
            )
            .await;

        assert!(result.success);

        // Intent was recorded for the Final step
        let intents = checkpoint.intents.lock().await;
        assert_eq!(intents.len(), 1);
        assert_eq!(intents[0].0.name, "publish");

        // Outcome was recorded
        let outcomes = checkpoint.outcomes.lock().await;
        assert_eq!(outcomes.len(), 1);
        assert!(outcomes.values().next().unwrap());

        // No pending intents (outcome recorded)
        drop(intents);
        drop(outcomes);
        let pending = checkpoint.pending_intents().await.unwrap();
        assert!(pending.is_empty());
    }

    #[tokio::test]
    async fn test_simulated_crash_leaves_pending_intent() {
        let checkpoint = Arc::new(InMemoryCheckpoint::new());

        // Manually record an intent with no outcome (simulates crash)
        let step = final_step("deploy");
        let key = IdempotencyKey::generate();
        checkpoint.record_intent(&step, &key).await.unwrap();

        // No outcome recorded → pending_intents should return it
        let pending = checkpoint.pending_intents().await.unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].0.name, "deploy");
        assert_eq!(pending[0].1, key);
    }

    #[tokio::test]
    async fn test_recovery_re_executes_pending_final_step() {
        let checkpoint = Arc::new(InMemoryCheckpoint::new());

        // Simulate a crash: intent recorded, no outcome
        let step = final_step("deploy");
        let key = IdempotencyKey::generate();
        checkpoint.record_intent(&step, &key).await.unwrap();

        // Recover
        let results = SagaOrchestrator::recover(checkpoint.as_ref(), |_tool, _args| async {
            Ok("recovered".to_string())
        })
        .await
        .unwrap();

        assert_eq!(results.len(), 1);
        assert!(results[0].success);
        assert_eq!(results[0].name, "recover:deploy");
        assert_eq!(results[0].output, "recovered");

        // After recovery, no more pending intents
        let pending = checkpoint.pending_intents().await.unwrap();
        assert!(pending.is_empty());
    }

    #[tokio::test]
    async fn test_idempotency_key_prevents_double_execution() {
        let checkpoint = Arc::new(InMemoryCheckpoint::new());

        // Record intent + outcome (completed step)
        let step = final_step("deploy");
        let key = IdempotencyKey::generate();
        checkpoint.record_intent(&step, &key).await.unwrap();
        checkpoint.record_outcome(&key, true).await.unwrap();

        // Recovery should find nothing pending
        let results = SagaOrchestrator::recover(checkpoint.as_ref(), |_tool, _args| async {
            Ok("should not run".to_string())
        })
        .await
        .unwrap();

        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn test_execute_without_checkpoint_backward_compat() {
        let saga =
            SagaOrchestrator::new(vec![write_step("create"), final_step("publish")]).unwrap();

        // Original execute() still works without any checkpoint
        let result = saga
            .execute(|_tool, _args| async { Ok("ok".to_string()) })
            .await;

        assert!(result.success);
        assert_eq!(result.step_results.len(), 2);
    }
}
