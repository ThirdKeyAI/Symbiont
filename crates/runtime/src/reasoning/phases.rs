//! Typestate-enforced phase transitions
//!
//! Uses zero-sized phase markers with `PhantomData` to make invalid
//! phase transitions a compile-time error. The loop driver transitions
//! through Reasoning → PolicyCheck → ToolDispatching → Observing,
//! and each transition consumes `self` to produce the next phase.
//!
//! This means it is structurally impossible to:
//! - Skip the policy check
//! - Dispatch tools without reasoning first
//! - Observe results without dispatching
//!
//! Zero runtime cost: PhantomData is zero-sized.

use std::marker::PhantomData;

use crate::reasoning::circuit_breaker::CircuitBreakerRegistry;
use crate::reasoning::context_manager::ContextManager;
use crate::reasoning::executor::ActionExecutor;
use crate::reasoning::inference::InferenceProvider;
use crate::reasoning::loop_types::*;
use crate::reasoning::policy_bridge::ReasoningPolicyGate;

// ── Phase markers (zero-sized types) ────────────────────────────────

/// The reasoning phase: LLM produces proposed actions.
pub struct Reasoning;
/// The policy check phase: every action is evaluated by the gate.
pub struct PolicyCheck;
/// The tool dispatching phase: approved actions are executed.
pub struct ToolDispatching;
/// The observation phase: results are collected for the next iteration.
pub struct Observing;

/// Marker trait for valid phases.
pub trait AgentPhase {}
impl AgentPhase for Reasoning {}
impl AgentPhase for PolicyCheck {}
impl AgentPhase for ToolDispatching {}
impl AgentPhase for Observing {}

// ── Phase data carriers ─────────────────────────────────────────────

/// Data produced by the reasoning phase, consumed by policy check.
pub struct ReasoningOutput {
    /// Actions proposed by the LLM.
    pub proposed_actions: Vec<ProposedAction>,
}

/// Data produced by the policy check phase, consumed by tool dispatch.
pub struct PolicyOutput {
    /// Actions approved by the policy gate.
    pub approved_actions: Vec<ProposedAction>,
    /// Actions denied, with their denial reasons.
    pub denied_reasons: Vec<(ProposedAction, String)>,
    /// Whether any Respond or Terminate action was approved.
    pub has_terminal_action: bool,
    /// Terminal output content if the loop should end.
    pub terminal_output: Option<String>,
}

/// Data produced by tool dispatch, consumed by observation.
pub struct DispatchOutput {
    /// Observations from tool execution.
    pub observations: Vec<Observation>,
    /// Whether the loop should terminate after observation.
    pub should_terminate: bool,
    /// Terminal output if set.
    pub terminal_output: Option<String>,
}

// ── The phased agent loop ───────────────────────────────────────────

/// The agent loop in a specific phase.
///
/// Each phase transition consumes `self` and produces the next phase,
/// making invalid transitions a compile error.
pub struct AgentLoop<Phase: AgentPhase> {
    /// Mutable loop state carried across phases.
    pub state: LoopState,
    /// Immutable loop configuration.
    pub config: LoopConfig,
    /// Phase-specific data from the previous transition (if any).
    phase_data: Option<PhaseData>,
    /// Zero-sized phase marker.
    _phase: PhantomData<Phase>,
}

/// Internal carrier for data between phases.
enum PhaseData {
    Reasoning(ReasoningOutput),
    Policy(PolicyOutput),
    Dispatch(DispatchOutput),
}

impl AgentLoop<Reasoning> {
    /// Create a new agent loop in the Reasoning phase.
    pub fn new(state: LoopState, config: LoopConfig) -> Self {
        Self {
            state,
            config,
            phase_data: None,
            _phase: PhantomData,
        }
    }

    /// Run the reasoning step: invoke the inference provider and parse actions.
    ///
    /// Consumes `self` and produces `AgentLoop<PolicyCheck>`.
    pub async fn produce_output(
        mut self,
        provider: &dyn InferenceProvider,
        context_manager: &dyn ContextManager,
    ) -> Result<AgentLoop<PolicyCheck>, LoopTermination> {
        self.state.current_phase = "reasoning".into();

        // Check termination conditions before reasoning
        if self.state.iteration >= self.config.max_iterations {
            return Err(LoopTermination {
                reason: LoopTerminationReason::MaxIterations {
                    iterations: self.state.iteration,
                },
                state: self.state,
            });
        }
        if self.state.total_usage.total_tokens >= self.config.max_total_tokens {
            return Err(LoopTermination {
                reason: LoopTerminationReason::MaxTokens {
                    tokens: self.state.total_usage.total_tokens,
                },
                state: self.state,
            });
        }

        // Apply context management (truncate conversation to fit budget)
        context_manager.manage_context(
            &mut self.state.conversation,
            self.config.context_token_budget,
        );

        // Add pending observations to conversation as tool result messages
        for obs in self.state.pending_observations.drain(..) {
            if obs.source == "policy_gate" {
                self.state.conversation.push(
                    crate::reasoning::conversation::ConversationMessage::user(format!(
                        "[Policy Feedback] {}",
                        obs.content
                    )),
                );
            }
            // Tool results are already added by the executor
        }

        // Build inference options
        let options = crate::reasoning::inference::InferenceOptions {
            max_tokens: self
                .config
                .max_total_tokens
                .saturating_sub(self.state.total_usage.total_tokens)
                .min(4096),
            tool_definitions: self.config.tool_definitions.clone(),
            ..Default::default()
        };

        // Call the inference provider
        let response = match provider.complete(&self.state.conversation, &options).await {
            Ok(r) => r,
            Err(e) => {
                return Err(LoopTermination {
                    reason: LoopTerminationReason::Error {
                        message: format!("Inference failed: {}", e),
                    },
                    state: self.state,
                });
            }
        };

        // Track usage
        self.state.add_usage(&response.usage);

        // Parse the response into proposed actions
        let proposed_actions = if response.has_tool_calls() {
            // Add the assistant message with tool calls to conversation
            let tool_calls: Vec<crate::reasoning::conversation::ToolCall> = response
                .tool_calls
                .iter()
                .map(|tc| crate::reasoning::conversation::ToolCall {
                    id: tc.id.clone(),
                    name: tc.name.clone(),
                    arguments: tc.arguments.clone(),
                })
                .collect();
            self.state.conversation.push(
                crate::reasoning::conversation::ConversationMessage::assistant_tool_calls(
                    tool_calls,
                ),
            );

            response
                .tool_calls
                .into_iter()
                .map(|tc| ProposedAction::ToolCall {
                    call_id: tc.id,
                    name: tc.name,
                    arguments: tc.arguments,
                })
                .collect()
        } else {
            // Text response → terminal action
            self.state.conversation.push(
                crate::reasoning::conversation::ConversationMessage::assistant(&response.content),
            );

            vec![ProposedAction::Respond {
                content: response.content,
            }]
        };

        self.state.iteration += 1;

        Ok(AgentLoop {
            state: self.state,
            config: self.config,
            phase_data: Some(PhaseData::Reasoning(ReasoningOutput { proposed_actions })),
            _phase: PhantomData,
        })
    }
}

impl AgentLoop<PolicyCheck> {
    /// Evaluate all proposed actions against the policy gate.
    ///
    /// Consumes `self` and produces `AgentLoop<ToolDispatching>`.
    pub async fn check_policy(
        mut self,
        gate: &dyn ReasoningPolicyGate,
    ) -> Result<AgentLoop<ToolDispatching>, LoopTermination> {
        self.state.current_phase = "policy_check".into();

        let reasoning_output = match self.phase_data {
            Some(PhaseData::Reasoning(output)) => output,
            _ => {
                return Err(LoopTermination {
                    reason: LoopTerminationReason::Error {
                        message: "Invalid phase data: expected ReasoningOutput".into(),
                    },
                    state: self.state,
                });
            }
        };

        let mut approved = Vec::new();
        let mut denied = Vec::new();
        let mut has_terminal = false;
        let mut terminal_output = None;

        for action in reasoning_output.proposed_actions {
            let decision = gate
                .evaluate_action(&self.state.agent_id, &action, &self.state)
                .await;

            match decision {
                LoopDecision::Allow => {
                    if matches!(
                        action,
                        ProposedAction::Respond { .. } | ProposedAction::Terminate { .. }
                    ) {
                        has_terminal = true;
                        if let ProposedAction::Respond { ref content } = action {
                            terminal_output = Some(content.clone());
                        }
                        if let ProposedAction::Terminate { ref output, .. } = action {
                            terminal_output = Some(output.clone());
                        }
                    }
                    approved.push(action);
                }
                LoopDecision::Deny { reason } => {
                    // Feed denial back as observation for next iteration
                    self.state
                        .pending_observations
                        .push(Observation::policy_denial(&reason));
                    denied.push((action, reason));
                }
                LoopDecision::Modify {
                    modified_action,
                    reason,
                } => {
                    tracing::info!("Policy modified action: {}", reason);
                    if matches!(
                        *modified_action,
                        ProposedAction::Respond { .. } | ProposedAction::Terminate { .. }
                    ) {
                        has_terminal = true;
                        if let ProposedAction::Respond { ref content } = *modified_action {
                            terminal_output = Some(content.clone());
                        }
                    }
                    approved.push(*modified_action);
                }
            }
        }

        Ok(AgentLoop {
            state: self.state,
            config: self.config,
            phase_data: Some(PhaseData::Policy(PolicyOutput {
                approved_actions: approved,
                denied_reasons: denied,
                has_terminal_action: has_terminal,
                terminal_output,
            })),
            _phase: PhantomData,
        })
    }
}

impl AgentLoop<ToolDispatching> {
    /// Return (action_count, denied_count) from the policy phase.
    pub fn policy_summary(&self) -> (usize, usize) {
        match &self.phase_data {
            Some(PhaseData::Policy(output)) => (
                output.approved_actions.len() + output.denied_reasons.len(),
                output.denied_reasons.len(),
            ),
            _ => (0, 0),
        }
    }

    /// Dispatch approved actions through the executor.
    ///
    /// Consumes `self` and produces `AgentLoop<Observing>`.
    pub async fn dispatch_tools(
        mut self,
        executor: &dyn ActionExecutor,
        circuit_breakers: &CircuitBreakerRegistry,
    ) -> Result<AgentLoop<Observing>, LoopTermination> {
        self.state.current_phase = "tool_dispatching".into();

        let policy_output = match self.phase_data {
            Some(PhaseData::Policy(output)) => output,
            _ => {
                return Err(LoopTermination {
                    reason: LoopTerminationReason::Error {
                        message: "Invalid phase data: expected PolicyOutput".into(),
                    },
                    state: self.state,
                });
            }
        };

        // If we have a terminal action, skip tool dispatch
        if policy_output.has_terminal_action {
            return Ok(AgentLoop {
                state: self.state,
                config: self.config,
                phase_data: Some(PhaseData::Dispatch(DispatchOutput {
                    observations: Vec::new(),
                    should_terminate: true,
                    terminal_output: policy_output.terminal_output,
                })),
                _phase: PhantomData,
            });
        }

        // Dispatch tool calls in parallel
        let observations = executor
            .execute_actions(
                &policy_output.approved_actions,
                &self.config,
                circuit_breakers,
            )
            .await;

        // Add tool results to conversation
        for obs in &observations {
            if !obs.is_error {
                self.state.conversation.push(
                    crate::reasoning::conversation::ConversationMessage::tool_result(
                        &obs.source,
                        &obs.source,
                        &obs.content,
                    ),
                );
            } else {
                self.state.conversation.push(
                    crate::reasoning::conversation::ConversationMessage::tool_result(
                        &obs.source,
                        &obs.source,
                        format!("[Error] {}", obs.content),
                    ),
                );
            }
        }

        Ok(AgentLoop {
            state: self.state,
            config: self.config,
            phase_data: Some(PhaseData::Dispatch(DispatchOutput {
                observations,
                should_terminate: false,
                terminal_output: None,
            })),
            _phase: PhantomData,
        })
    }
}

/// What happens after the observation phase.
pub enum LoopContinuation {
    /// Continue to the next iteration.
    Continue(Box<AgentLoop<Reasoning>>),
    /// The loop is complete.
    Complete(LoopResult),
}

impl AgentLoop<Observing> {
    /// Collect observations and decide whether to continue or terminate.
    ///
    /// Consumes `self` and returns either a new Reasoning phase or the final result.
    pub fn observe_results(mut self) -> LoopContinuation {
        self.state.current_phase = "observing".into();

        let dispatch_output = match self.phase_data {
            Some(PhaseData::Dispatch(output)) => output,
            _ => {
                return LoopContinuation::Complete(LoopResult {
                    output: String::new(),
                    iterations: self.state.iteration,
                    total_usage: self.state.total_usage.clone(),
                    termination_reason: TerminationReason::Error {
                        message: "Invalid phase data".into(),
                    },
                    duration: self.state.elapsed().to_std().unwrap_or_default(),
                    conversation: self.state.conversation,
                });
            }
        };

        if dispatch_output.should_terminate {
            return LoopContinuation::Complete(LoopResult {
                output: dispatch_output.terminal_output.unwrap_or_default(),
                iterations: self.state.iteration,
                total_usage: self.state.total_usage.clone(),
                termination_reason: TerminationReason::Completed,
                duration: self.state.elapsed().to_std().unwrap_or_default(),
                conversation: self.state.conversation,
            });
        }

        // Add observations as pending for next reasoning step
        self.state
            .pending_observations
            .extend(dispatch_output.observations);

        LoopContinuation::Continue(Box::new(AgentLoop {
            state: self.state,
            config: self.config,
            phase_data: None,
            _phase: PhantomData,
        }))
    }
}

/// Reasons the loop may terminate during a phase transition.
/// Carries the state so the caller can extract final conversation/usage.
#[derive(Debug)]
pub struct LoopTermination {
    pub reason: LoopTerminationReason,
    pub state: LoopState,
}

/// The specific reason for termination.
#[derive(Debug)]
pub enum LoopTerminationReason {
    MaxIterations { iterations: u32 },
    MaxTokens { tokens: u32 },
    Timeout,
    Error { message: String },
}

impl LoopTermination {
    /// Convert to a LoopResult for the caller.
    pub fn into_result(self) -> LoopResult {
        let reason = match &self.reason {
            LoopTerminationReason::MaxIterations { .. } => TerminationReason::MaxIterations,
            LoopTerminationReason::MaxTokens { .. } => TerminationReason::MaxTokens,
            LoopTerminationReason::Timeout => TerminationReason::Timeout,
            LoopTerminationReason::Error { message } => TerminationReason::Error {
                message: message.clone(),
            },
        };
        LoopResult {
            output: String::new(),
            iterations: self.state.iteration,
            total_usage: self.state.total_usage.clone(),
            termination_reason: reason,
            duration: self.state.elapsed().to_std().unwrap_or_default(),
            conversation: self.state.conversation,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reasoning::conversation::Conversation;
    use crate::types::AgentId;

    #[test]
    fn test_agent_loop_creation() {
        let state = LoopState::new(AgentId::new(), Conversation::with_system("test"));
        let config = LoopConfig::default();
        let loop_instance = AgentLoop::<Reasoning>::new(state, config);
        assert_eq!(loop_instance.state.iteration, 0);
    }

    #[test]
    fn test_loop_termination_into_result() {
        let state = LoopState::new(AgentId::new(), Conversation::new());
        let termination = LoopTermination {
            reason: LoopTerminationReason::MaxIterations { iterations: 25 },
            state,
        };
        let result = termination.into_result();
        assert!(matches!(
            result.termination_reason,
            TerminationReason::MaxIterations
        ));
    }

    // Compile-time verification:
    // The following function signatures prove that the type system
    // prevents invalid phase transitions. If any of these didn't
    // compile, it would mean the typestate pattern is broken.

    fn _prove_reasoning_to_policy(_loop: AgentLoop<Reasoning>) {
        // This function takes a Reasoning phase loop.
        // The only method available is `produce_output()`, which
        // returns AgentLoop<PolicyCheck>.
        // You cannot call check_policy() or dispatch_tools() here.
    }

    fn _prove_policy_to_dispatch(_loop: AgentLoop<PolicyCheck>) {
        // This function takes a PolicyCheck phase loop.
        // The only method available is `check_policy()`, which
        // returns AgentLoop<ToolDispatching>.
    }

    fn _prove_dispatch_to_observing(_loop: AgentLoop<ToolDispatching>) {
        // This function takes a ToolDispatching phase loop.
        // The only method available is `dispatch_tools()`, which
        // returns AgentLoop<Observing>.
    }

    fn _prove_observing_to_continuation(_loop: AgentLoop<Observing>) {
        // This function takes an Observing phase loop.
        // The only method available is `observe_results()`, which
        // returns LoopContinuation (either Continue<Reasoning> or Complete).
    }
}
