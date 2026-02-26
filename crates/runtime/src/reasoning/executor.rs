//! Action executor with parallel dispatch
//!
//! Executes approved actions concurrently using `FuturesUnordered`,
//! with per-tool timeouts, circuit breaker integration, and barrier
//! sync before returning observations.

use crate::reasoning::circuit_breaker::CircuitBreakerRegistry;
use crate::reasoning::loop_types::{LoopConfig, Observation, ProposedAction};
use async_trait::async_trait;
use futures::stream::{FuturesUnordered, StreamExt};
use std::time::Duration;

/// Trait for executing proposed actions and producing observations.
#[async_trait]
pub trait ActionExecutor: Send + Sync {
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
}

/// Default executor that dispatches tool calls in parallel.
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
                            source: call_id,
                            content: format!(
                                "Tool '{}' circuit is open: {}. The tool endpoint has been failing and is temporarily disabled.",
                                name, cb_err
                            ),
                            is_error: true,
                            metadata: {
                                let mut m = std::collections::HashMap::new();
                                m.insert("tool_name".into(), name);
                                m.insert("error_type".into(), "circuit_open".into());
                                m
                            },
                        };
                    }

                    // Execute the tool call with timeout
                    let result = tokio::time::timeout(timeout, async {
                        // In production, this would call the ToolInvocationEnforcer.
                        // For now, produce an observation indicating the tool was called.
                        execute_tool_call(&name, &arguments).await
                    })
                    .await;

                    match result {
                        Ok(Ok(content)) => Observation::tool_result(call_id, content),
                        Ok(Err(err)) => Observation::tool_error(call_id, err),
                        Err(_) => Observation {
                            source: call_id,
                            content: format!(
                                "Tool '{}' timed out after {:?}",
                                name, timeout
                            ),
                            is_error: true,
                            metadata: {
                                let mut m = std::collections::HashMap::new();
                                m.insert("tool_name".into(), name);
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

/// Execute a single tool call. In production, this delegates to the
/// ToolInvocationEnforcer → MCP client pipeline. This default implementation
/// returns the arguments as the "result" for testing purposes.
async fn execute_tool_call(name: &str, arguments: &str) -> Result<String, String> {
    tracing::debug!("Executing tool '{}' with arguments: {}", name, arguments);
    // Production implementation would call through ToolInvocationEnforcer here.
    // For the reasoning loop infrastructure, we return a placeholder.
    Ok(format!(
        "Tool '{}' executed successfully with arguments: {}",
        name, arguments
    ))
}

/// An executor that delegates to a real ToolInvocationEnforcer.
pub struct EnforcedActionExecutor {
    enforcer: std::sync::Arc<dyn crate::integrations::tool_invocation::ToolInvocationEnforcer>,
}

impl EnforcedActionExecutor {
    pub fn new(
        enforcer: std::sync::Arc<dyn crate::integrations::tool_invocation::ToolInvocationEnforcer>,
    ) -> Self {
        Self { enforcer }
    }
}

#[async_trait]
impl ActionExecutor for EnforcedActionExecutor {
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

        let timeout = config.tool_timeout;
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
                let enforcer = self.enforcer.clone();

                let cb_result = circuit_breakers.check(&name).await;

                futures.push(async move {
                    if let Err(cb_err) = cb_result {
                        return Observation {
                            source: call_id,
                            content: format!("Tool '{}' circuit is open: {}", name, cb_err),
                            is_error: true,
                            metadata: {
                                let mut m = std::collections::HashMap::new();
                                m.insert("tool_name".into(), name);
                                m.insert("error_type".into(), "circuit_open".into());
                                m
                            },
                        };
                    }

                    let tool = crate::integrations::mcp::McpTool {
                        name: name.clone(),
                        description: String::new(),
                        schema: serde_json::json!({}),
                        provider: crate::integrations::mcp::ToolProvider {
                            identifier: "reasoning_loop".into(),
                            name: "Reasoning Loop".into(),
                            public_key_url: String::new(),
                            version: None,
                        },
                        verification_status:
                            crate::integrations::mcp::VerificationStatus::Skipped {
                                reason: "Invoked via reasoning loop".into(),
                            },
                        metadata: None,
                        sensitive_params: vec![],
                    };

                    let args: serde_json::Value =
                        serde_json::from_str(&arguments).unwrap_or(serde_json::json!({}));

                    let context = crate::integrations::tool_invocation::InvocationContext {
                        agent_id: crate::types::AgentId::new(),
                        tool_name: name.clone(),
                        arguments: args,
                        timestamp: chrono::Utc::now(),
                        metadata: std::collections::HashMap::new(),
                        agent_credential: None,
                    };

                    match tokio::time::timeout(
                        timeout,
                        enforcer.execute_tool_with_enforcement(&tool, context),
                    )
                    .await
                    {
                        Ok(Ok(result)) => {
                            Observation::tool_result(call_id, result.result.to_string())
                        }
                        Ok(Err(err)) => Observation::tool_error(call_id, err.to_string()),
                        Err(_) => Observation {
                            source: call_id,
                            content: format!("Tool '{}' timed out", name),
                            is_error: true,
                            metadata: {
                                let mut m = std::collections::HashMap::new();
                                m.insert("tool_name".into(), name);
                                m.insert("error_type".into(), "timeout".into());
                                m
                            },
                        },
                    }
                });
            }
        }

        let mut observations = Vec::with_capacity(tool_calls.len());
        while let Some(obs) = futures.next().await {
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
        assert!(!obs[0].is_error);
        assert_eq!(obs[0].source, "c1");
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

        let start = std::time::Instant::now();
        let obs = executor
            .execute_actions(&actions, &config, &circuit_breakers)
            .await;
        let elapsed = start.elapsed();

        assert_eq!(obs.len(), 3);
        // All should succeed
        assert!(obs.iter().all(|o| !o.is_error));
        // Parallel dispatch means wall-clock ≈ max(individual), not sum
        // Individual calls are near-instant in the default executor,
        // so elapsed should be well under 100ms
        assert!(
            elapsed.as_millis() < 100,
            "Parallel dispatch took {}ms, expected <100ms",
            elapsed.as_millis()
        );
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
                target: "other".into(),
                message: "hi".into(),
            },
        ];

        let obs = executor
            .execute_actions(&actions, &config, &circuit_breakers)
            .await;
        assert!(obs.is_empty());
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
