//! Action executor with parallel dispatch
//!
//! Executes approved actions concurrently using `FuturesUnordered`,
//! with per-tool timeouts, circuit breaker integration, and barrier
//! sync before returning observations.

use crate::reasoning::circuit_breaker::CircuitBreakerRegistry;
use crate::reasoning::inference::ToolDefinition;
use crate::reasoning::loop_types::{LoopConfig, Observation, ProposedAction};
use async_trait::async_trait;
use futures::stream::{FuturesUnordered, StreamExt};
use std::time::Duration;

/// Trait for executing proposed actions and producing observations.
#[async_trait]
pub trait ActionExecutor: Send + Sync {
    /// Frozen runtime-owned metadata, included in policy bindings and run audit.
    fn execution_context(&self) -> std::collections::HashMap<String, serde_json::Value> {
        std::collections::HashMap::new()
    }

    /// Validate frozen operator configuration before the reasoning provider runs.
    fn validate_configuration(&self) -> Result<(), String> {
        Ok(())
    }

    /// Execute a batch of approved actions, potentially in parallel.
    ///
    /// Returns observations from all action results. Circuit breakers
    /// are checked before each dispatch.
    async fn execute_actions(
        &self,
        actions: &[ProposedAction],
        config: &LoopConfig,
        circuit_breakers: &CircuitBreakerRegistry,
    ) -> Vec<Observation>;

    /// Stop all persistent effects owned by a run. Cancellation must be
    /// synchronous so dropping the run future can signal worker ownership.
    fn cancel_run(&self, _run: &str, _deadline: std::time::Instant) {}

    /// Await required cleanup before publishing normal loop termination.
    async fn close_run(&self, run: &str, deadline: std::time::Instant) -> Result<(), String> {
        self.cancel_run(run, deadline);
        Ok(())
    }

    /// Validate and normalize before policy evaluation. Implementations with
    /// mutable registries must attach a frozen backend snapshot here.
    fn prepare_action(
        &self,
        action: &ProposedAction,
        config: &LoopConfig,
    ) -> Result<super::prepared::PreparedAction, String> {
        prepare_registered_action(action, config, &self.tool_definitions())
    }

    /// Consume policy grants. Legacy executors are rechecked against their
    /// current registry immediately before receiving the normalized action.
    async fn execute_authorized(
        &self,
        actions: Vec<super::prepared::AuthorizedAction>,
        config: &LoopConfig,
        circuit_breakers: &CircuitBreakerRegistry,
    ) -> Vec<Observation> {
        let mut observations = Vec::new();
        for authorized in actions {
            let action = authorized.action().clone();
            let checked = authorized.check_live().and_then(|()| {
                let current = self.prepare_action(&action, config)?;
                if !current.matches_authorized_call(authorized.prepared())? {
                    return Err("tool contract changed after authorization".into());
                }
                authorized.check_live()
            });
            match checked {
                Ok(()) => {
                    let remaining = authorized
                        .deadline()
                        .saturating_duration_since(std::time::Instant::now());
                    match tokio::time::timeout(
                        remaining,
                        self.execute_actions(
                            std::slice::from_ref(&action),
                            config,
                            circuit_breakers,
                        ),
                    )
                    .await
                    {
                        Ok(results) => observations.extend(results),
                        Err(_) => {
                            if let ProposedAction::ToolCall { call_id, name, .. } = action {
                                observations.push(
                                    Observation::tool_error(
                                        name,
                                        "authorized execution timed out; completion is unconfirmed",
                                    )
                                    .with_call_id(call_id),
                                );
                            }
                        }
                    }
                }
                Err(error) => {
                    if let ProposedAction::ToolCall { call_id, name, .. } = action {
                        observations
                            .push(Observation::tool_error(name, error).with_call_id(call_id));
                    }
                }
            }
        }
        observations
    }

    /// Return tool definitions this executor can handle.
    ///
    /// The runner auto-populates `LoopConfig.tool_definitions` from this
    /// when the config's list is empty. Override in executors that discover
    /// tools dynamically.
    fn tool_definitions(&self) -> Vec<ToolDefinition> {
        Vec::new()
    }
}

/// Own persistent effects across a governed run. SDK dispatchers should keep
/// one guard for their whole run, then await `close`. Dropping an interrupted
/// run signals cancellation immediately.
pub struct ExecutionRunGuard {
    executor: std::sync::Arc<dyn ActionExecutor>,
    run: String,
    deadline: std::time::Instant,
    armed: bool,
}
impl ExecutionRunGuard {
    pub fn new(
        executor: std::sync::Arc<dyn ActionExecutor>,
        state: &super::loop_types::LoopState,
        config: &LoopConfig,
    ) -> Result<Self, String> {
        let remaining = config
            .timeout
            .saturating_sub(state.elapsed().to_std().unwrap_or(Duration::ZERO));
        let deadline = std::time::Instant::now()
            .checked_add(remaining)
            .ok_or("run deadline exceeds supported range")?;
        Ok(Self {
            executor,
            run: super::prepared::execution_run_key(state),
            deadline,
            armed: true,
        })
    }
    pub async fn close(mut self) -> Result<(), String> {
        let result = tokio::time::timeout(
            Duration::from_secs(22),
            self.executor.close_run(&self.run, self.deadline),
        )
        .await
        .unwrap_or_else(|_| Err("run cleanup acknowledgement timed out".into()));
        if result.is_ok() {
            self.armed = false;
        }
        result
    }
}
impl Drop for ExecutionRunGuard {
    fn drop(&mut self) {
        if self.armed {
            self.executor.cancel_run(&self.run, self.deadline);
        }
    }
}

pub fn prepare_registered_action(
    action: &ProposedAction,
    config: &LoopConfig,
    definitions: &[ToolDefinition],
) -> Result<super::prepared::PreparedAction, String> {
    use super::prepared::{canonical_json, digest_json, PreparedAction, ToolContract};
    let ProposedAction::ToolCall {
        call_id,
        name,
        arguments,
    } = action
    else {
        return PreparedAction::new(action.clone(), None);
    };
    super::phases::validate_tool_call_arguments(name, arguments, &config.tool_definitions)?;
    let definitions = if definitions.is_empty() {
        config.tool_definitions.as_slice()
    } else {
        definitions
    };
    let definition = definitions
        .iter()
        .find(|d| d.name == *name)
        .ok_or_else(|| format!("executor does not advertise '{name}'"))?;
    let args: serde_json::Value = serde_json::from_str(arguments).map_err(|e| e.to_string())?;
    PreparedAction::new(
        ProposedAction::ToolCall {
            call_id: call_id.clone(),
            name: name.clone(),
            arguments: canonical_json(&args)?,
        },
        Some(ToolContract {
            name: name.clone(),
            version: String::new(),
            digest: digest_json(&serde_json::to_value(definition).map_err(|e| e.to_string())?)?,
            action_type: "Action".into(),
            action_id: format!("tool_call::{name}"),
            resource_type: "Resource".into(),
            resource_id: "default".into(),
            requires_approval: false,
        }),
    )
}

/// Compatibility executor with no tool backend. Every tool call fails honestly.
pub struct DefaultActionExecutor {
    tool_timeout: Duration,
}

impl DefaultActionExecutor {
    pub fn new(tool_timeout: Duration) -> Self {
        Self { tool_timeout }
    }
}

impl Default for DefaultActionExecutor {
    fn default() -> Self {
        Self::new(Duration::from_secs(30))
    }
}

#[async_trait]
impl ActionExecutor for DefaultActionExecutor {
    async fn execute_actions(
        &self,
        actions: &[ProposedAction],
        config: &LoopConfig,
        circuit_breakers: &CircuitBreakerRegistry,
    ) -> Vec<Observation> {
        let tool_calls: Vec<&ProposedAction> = actions
            .iter()
            .filter(|a| matches!(a, ProposedAction::ToolCall { .. }))
            .collect();

        if tool_calls.is_empty() {
            return Vec::new();
        }

        let timeout = self.tool_timeout.min(config.tool_timeout);

        // Dispatch tool calls concurrently
        let mut futures = FuturesUnordered::new();

        for action in &tool_calls {
            if let ProposedAction::ToolCall {
                call_id,
                name,
                arguments,
            } = action
            {
                let name = name.clone();
                let arguments = arguments.clone();
                let call_id = call_id.clone();

                // Check circuit breaker first
                let cb_result = circuit_breakers.check(&name).await;

                futures.push(async move {
                    if let Err(cb_err) = cb_result {
                        return Observation {
                            source: name,
                            content: format!(
                                "Tool circuit is open: {}. The tool endpoint has been failing and is temporarily disabled.",
                                cb_err
                            ),
                            is_error: true,
                            call_id: Some(call_id),
                            metadata: {
                                let mut m = std::collections::HashMap::new();
                                m.insert("error_type".into(), "circuit_open".into());
                                m
                            },
                        };
                    }

                    // Execute the tool call with timeout
                    let result = tokio::time::timeout(timeout, async {
                        // No backend is installed on this compatibility executor.
                        execute_tool_call(&name, &arguments).await
                    })
                    .await;

                    match result {
                        Ok(Ok(content)) => {
                            Observation::tool_result(&name, content).with_call_id(call_id)
                        }
                        Ok(Err(err)) => {
                            Observation::tool_error(&name, err).with_call_id(call_id)
                        }
                        Err(_) => Observation {
                            source: name.clone(),
                            content: format!(
                                "Tool '{}' timed out after {:?}",
                                name, timeout
                            ),
                            is_error: true,
                            call_id: Some(call_id),
                            metadata: {
                                let mut m = std::collections::HashMap::new();
                                m.insert("error_type".into(), "timeout".into());
                                m
                            },
                        },
                    }
                });
            }
        }

        // Barrier sync: wait for all tool calls to complete
        let mut observations = Vec::with_capacity(tool_calls.len());
        while let Some(obs) = futures.next().await {
            // Record success/failure in circuit breaker
            let tool_name = obs
                .metadata
                .get("tool_name")
                .cloned()
                .unwrap_or_else(|| obs.source.clone());
            if obs.is_error {
                circuit_breakers.record_failure(&tool_name).await;
            } else {
                circuit_breakers.record_success(&tool_name).await;
            }
            observations.push(obs);
        }

        observations
    }
}

/// A default executor cannot claim an effect when no backend was installed.
async fn execute_tool_call(name: &str, _arguments: &str) -> Result<String, String> {
    Err(format!(
        "Tool '{name}' was not executed: no tool backend is configured"
    ))
}

/// An [`ActionExecutor`] for runners that have **no tool backend wired**.
///
/// It advertises no tools (so the model is not offered any) and, if a tool
/// call is nonetheless proposed, returns a clear `is_error` observation
/// instead of fabricating a success. It is the fallback for entry points like
/// `symbi run` and the DSL `reason()` builtin when no `tools/` ToolClad
/// manifests are present, so those paths never tell the model a tool
/// "executed successfully" when nothing ran.
///
/// [`DefaultActionExecutor`] also refuses unconfigured tools. Use
/// [`crate::toolclad::executor::ToolCladExecutor`] for real configured backends.
#[derive(Default)]
pub struct UnavailableToolExecutor;

#[async_trait]
impl ActionExecutor for UnavailableToolExecutor {
    async fn execute_actions(
        &self,
        actions: &[ProposedAction],
        _config: &LoopConfig,
        _circuit_breakers: &CircuitBreakerRegistry,
    ) -> Vec<Observation> {
        actions
            .iter()
            .filter_map(|action| match action {
                ProposedAction::ToolCall { call_id, name, .. } => Some(Observation {
                    source: name.clone(),
                    content: format!(
                        "Tool '{}' was not executed: this runner has no tool backend configured.",
                        name
                    ),
                    is_error: true,
                    call_id: Some(call_id.clone()),
                    metadata: Default::default(),
                }),
                _ => None,
            })
            .collect()
    }

    // tool_definitions() falls back to the trait default (empty) — advertise
    // no tools, so the model isn't offered capabilities the runner can't run.
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_default_executor_no_actions() {
        let executor = DefaultActionExecutor::default();
        let config = LoopConfig::default();
        let circuit_breakers = CircuitBreakerRegistry::default();

        let obs = executor
            .execute_actions(&[], &config, &circuit_breakers)
            .await;
        assert!(obs.is_empty());
    }

    #[tokio::test]
    async fn test_default_executor_single_tool() {
        let executor = DefaultActionExecutor::default();
        let config = LoopConfig::default();
        let circuit_breakers = CircuitBreakerRegistry::default();

        let actions = vec![ProposedAction::ToolCall {
            call_id: "c1".into(),
            name: "search".into(),
            arguments: r#"{"q": "test"}"#.into(),
        }];

        let obs = executor
            .execute_actions(&actions, &config, &circuit_breakers)
            .await;
        assert_eq!(obs.len(), 1);
        assert!(obs[0].is_error);
        assert!(obs[0].content.contains("not executed"));
        assert_eq!(obs[0].source, "search");
        assert_eq!(obs[0].call_id.as_deref(), Some("c1"));
    }

    #[tokio::test]
    async fn test_unavailable_tool_executor_reports_error_not_fake_success() {
        let executor = UnavailableToolExecutor;
        let config = LoopConfig::default();
        let circuit_breakers = CircuitBreakerRegistry::default();

        // Advertises no tools.
        assert!(executor.tool_definitions().is_empty());

        let actions = vec![ProposedAction::ToolCall {
            call_id: "c1".into(),
            name: "search".into(),
            arguments: r#"{"q": "test"}"#.into(),
        }];
        let obs = executor
            .execute_actions(&actions, &config, &circuit_breakers)
            .await;

        assert_eq!(obs.len(), 1);
        // The key property: a call is surfaced as an error, never as a
        // fabricated success.
        assert!(obs[0].is_error);
        assert_eq!(obs[0].source, "search");
        assert_eq!(obs[0].call_id.as_deref(), Some("c1"));
        assert!(obs[0].content.contains("not executed"));

        // Non-tool actions produce no observations.
        let non_tool = vec![ProposedAction::Respond {
            content: "hi".into(),
        }];
        assert!(executor
            .execute_actions(&non_tool, &config, &circuit_breakers)
            .await
            .is_empty());
    }

    #[tokio::test]
    async fn test_default_executor_parallel_dispatch() {
        let executor = DefaultActionExecutor::default();
        let config = LoopConfig::default();
        let circuit_breakers = CircuitBreakerRegistry::default();

        let actions: Vec<ProposedAction> = (0..3)
            .map(|i| ProposedAction::ToolCall {
                call_id: format!("c{}", i),
                name: format!("tool_{}", i),
                arguments: "{}".into(),
            })
            .collect();

        let obs = executor
            .execute_actions(&actions, &config, &circuit_breakers)
            .await;

        assert_eq!(obs.len(), 3);
        // No backend may fabricate success.
        assert!(obs.iter().all(|o| o.is_error));
        let ids: std::collections::HashSet<_> =
            obs.iter().map(|o| o.call_id.as_deref().unwrap()).collect();
        assert_eq!(ids, std::collections::HashSet::from(["c0", "c1", "c2"]));
    }

    #[tokio::test]
    async fn test_executor_skips_non_tool_actions() {
        let executor = DefaultActionExecutor::default();
        let config = LoopConfig::default();
        let circuit_breakers = CircuitBreakerRegistry::default();

        let actions = vec![
            ProposedAction::Respond {
                content: "done".into(),
            },
            ProposedAction::Delegate {
                call_id: "test-call".into(),
                target: "other".into(),
                message: "hi".into(),
            },
        ];

        let obs = executor
            .execute_actions(&actions, &config, &circuit_breakers)
            .await;
        assert!(obs.is_empty());
    }

    #[test]
    fn test_default_executor_has_empty_tool_definitions() {
        let executor = DefaultActionExecutor::default();
        assert!(executor.tool_definitions().is_empty());
    }

    #[tokio::test]
    async fn test_executor_circuit_breaker_integration() {
        let executor = DefaultActionExecutor::default();
        let config = LoopConfig::default();
        let circuit_breakers =
            CircuitBreakerRegistry::new(crate::reasoning::circuit_breaker::CircuitBreakerConfig {
                failure_threshold: 2,
                recovery_timeout: std::time::Duration::from_secs(30),
                half_open_max_calls: 1,
            });

        // Trip the circuit breaker for "failing_tool"
        circuit_breakers.record_failure("failing_tool").await;
        circuit_breakers.record_failure("failing_tool").await;

        let actions = vec![ProposedAction::ToolCall {
            call_id: "c1".into(),
            name: "failing_tool".into(),
            arguments: "{}".into(),
        }];

        let obs = executor
            .execute_actions(&actions, &config, &circuit_breakers)
            .await;
        assert_eq!(obs.len(), 1);
        assert!(obs[0].is_error);
        assert!(obs[0].content.contains("circuit is open"));
    }
}
