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

// clippy::result_large_err — the Err type is `LoopTermination` (~240 bytes),
// returned by every typestate transition below. Boxing it would put an
// allocation on the loop's hot path and change the signature of the whole
// Reasoning -> PolicyCheck -> ToolDispatching -> Observing chain for a value
// that is moved once and immediately matched on. Deliberate; revisit if
// LoopTermination grows.
#![allow(clippy::result_large_err)]
use std::marker::PhantomData;

use crate::reasoning::circuit_breaker::CircuitBreakerRegistry;
use crate::reasoning::context_manager::ContextManager;
use crate::reasoning::conversation::Conversation;
use crate::reasoning::executor::ActionExecutor;
use crate::reasoning::inference::{InferenceProvider, ToolDefinition};
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
    /// `delegation_available` reflects whether the runner holds a delegation
    /// handle; it gates the `delegate` tool-call conversion (see
    /// [`tool_call_to_action`]) so runners that implement `delegate` in their
    /// own executor keep receiving it as a tool call.
    pub async fn produce_output(
        mut self,
        provider: &dyn InferenceProvider,
        context_manager: &dyn ContextManager,
        delegation_available: bool,
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

        // Apply context management (truncate conversation to fit budget).
        // Debug-level visibility into truncation: fires when the manager
        // actually drops messages. Useful for spotting "context budget too
        // small for this workload" (the conversation churns every turn) vs
        // the healthy case (rare truncation). Debug, not warn — for large
        // workloads truncation is expected and per-iteration, so warn would
        // be pure noise.
        let before_len = self.state.conversation.len();
        let before_tokens = self.state.conversation.estimate_tokens();
        context_manager.manage_context(
            &mut self.state.conversation,
            self.config.context_token_budget,
        );
        let after_len = self.state.conversation.len();
        if after_len != before_len {
            tracing::debug!(
                iter = self.state.iteration,
                before_len,
                after_len,
                before_tokens,
                budget = self.config.context_token_budget,
                "context_manager truncated conversation"
            );
        }

        // Drain pending observations. Tool results are already in the conversation
        // (added by dispatch_tools for approved actions, and by check_policy for
        // denied tool calls). Policy denials are also already added as tool_result
        // messages in check_policy, so we don't need to add them again here.
        self.state.pending_observations.clear();

        // Build inference options
        let options = crate::reasoning::inference::InferenceOptions {
            max_tokens: self
                .config
                .max_total_tokens
                .saturating_sub(self.state.total_usage.total_tokens)
                .min(self.config.max_output_tokens),
            temperature: self.config.temperature,
            tool_definitions: self.config.tool_definitions.clone(),
            // Propagate the loop's tool_choice preference so providers
            // honor `LoopConfig::tool_choice` (e.g. ToolChoice::Any to
            // force tool_use on every turn — required for iterate-until-
            // done agents).
            tool_choice: self.config.tool_choice.clone(),
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

        // A refusal, or a turn that produced neither tool calls nor any text, is a
        // no-progress turn. Terminate distinctly instead of returning an empty
        // Respond (which reads as a silent successful completion and no-ops
        // tool-driven loops). Callers can then retry / fail over.
        if response.finish_reason == crate::reasoning::inference::FinishReason::Refusal {
            return Err(LoopTermination {
                reason: LoopTerminationReason::Error {
                    message: "model refused the request (stop_reason=refusal)".to_string(),
                },
                state: self.state,
            });
        }
        if !response.has_tool_calls() && response.content.trim().is_empty() {
            return Err(LoopTermination {
                reason: LoopTerminationReason::Error {
                    message: "model produced no tool calls and no text (no-progress turn)"
                        .to_string(),
                },
                state: self.state,
            });
        }

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
                .map(|tc| tool_call_to_action(tc.id, tc.name, tc.arguments, delegation_available))
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
    /// Return a clone of the proposed actions from the reasoning phase.
    /// Used by the loop driver to emit `ReasoningComplete` journal events
    /// before the policy check consumes the data.
    pub fn proposed_actions(&self) -> Vec<ProposedAction> {
        match &self.phase_data {
            Some(PhaseData::Reasoning(output)) => output.proposed_actions.clone(),
            _ => Vec::new(),
        }
    }

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
            // M4 mitigation: validate tool-call arguments against the
            // declared JSON schema (if any) before handing the action to
            // the policy gate. The LLM controls `arguments` as a free-form
            // string, so we treat a schema violation — or anything that
            // isn't a JSON object — as a policy denial. This closes the
            // simplest prompt-injection-to-exec path where the model is
            // coerced into emitting malformed args.
            //
            // If no schema is registered for the named tool we still
            // require the args to parse as a JSON object (the minimum bar
            // — MCP servers and the local executor both expect an object
            // payload). Reject strings, numbers, arrays, booleans, etc.
            // Compute schema-validation result up-front so the immutable
            // borrow of `action` ends before we (potentially) move it into
            // `denied`.
            let schema_check: Option<(String, String, String)> = match &action {
                ProposedAction::ToolCall {
                    call_id,
                    name,
                    arguments,
                } => match validate_tool_call_arguments(
                    name,
                    arguments,
                    &self.config.tool_definitions,
                ) {
                    Ok(()) => None,
                    Err(reason) => Some((call_id.clone(), name.clone(), reason)),
                },
                _ => None,
            };

            if let Some((call_id, name, reason)) = schema_check {
                self.state.conversation.push(
                    crate::reasoning::conversation::ConversationMessage::tool_result(
                        call_id,
                        name,
                        format!("[Schema validation failed] {}", reason),
                    ),
                );
                self.state
                    .pending_observations
                    .push(Observation::policy_denial(&reason));
                denied.push((action, reason));
                continue;
            }

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
                    push_denial_tool_result(&mut self.state.conversation, &action, &reason);
                    // Also feed denial back as pending observation for the loop driver
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

/// Push a `tool_result` for a denied action so the model sees the denial and no
/// `tool_use` is left orphaned. Covers both tool calls and delegations — both
/// originate from a model tool call and therefore carry a `call_id`.
pub(crate) fn push_denial_tool_result(
    conversation: &mut Conversation,
    action: &ProposedAction,
    reason: &str,
) {
    let (call_id, name) = match action {
        ProposedAction::ToolCall { call_id, name, .. } => (call_id, name.clone()),
        ProposedAction::Delegate {
            call_id, target, ..
        } => (call_id, format!("delegate:{}", target)),
        _ => return,
    };
    conversation.push(
        crate::reasoning::conversation::ConversationMessage::tool_result(
            call_id,
            &name,
            format!("[Policy denied] {}", reason),
        ),
    );
}

/// The tool name the model calls to delegate work to another agent.
pub(crate) const DELEGATE_TOOL_NAME: &str = "delegate";

/// Convert one model tool call into a proposed action.
///
/// A well-formed `delegate` call becomes a `Delegate` action carrying the tool
/// call's id, so its result returns to the model as a correlated `tool_result`.
/// Everything else — including a `delegate` call with malformed arguments —
/// stays a `ToolCall`, so the executor surfaces a precise error rather than the
/// call being silently dropped.
///
/// `delegation_available` must be true only when the runner actually has a
/// delegation handle. Runners that implement their own `delegate` tool through
/// their executor (symbi-shell's orchestrator does) have no handle, and
/// converting their call would route it to a dispatcher that can only answer
/// "delegation is not available" — hijacking a tool that works. When false the
/// call stays a `ToolCall` and reaches the executor that implements it.
pub(crate) fn tool_call_to_action(
    call_id: String,
    name: String,
    arguments: String,
    delegation_available: bool,
) -> ProposedAction {
    if delegation_available && name == DELEGATE_TOOL_NAME {
        if let Ok(serde_json::Value::Object(map)) =
            serde_json::from_str::<serde_json::Value>(&arguments)
        {
            if let (Some(agent), Some(task)) = (
                map.get("agent").and_then(|v| v.as_str()),
                map.get("task").and_then(|v| v.as_str()),
            ) {
                return ProposedAction::Delegate {
                    call_id,
                    target: agent.to_string(),
                    message: task.to_string(),
                };
            }
        }
    }
    ProposedAction::ToolCall {
        call_id,
        name,
        arguments,
    }
}

/// Dispatch approved `Delegate` actions through the delegation handle, returning
/// one Observation per delegate. Never silently drops: a missing handle or any
/// `DelegationError` becomes an honest `is_error` Observation.
pub(crate) async fn dispatch_delegations(
    actions: &[ProposedAction],
    delegation: Option<&dyn crate::reasoning::delegation::DelegationExecutor>,
    config: &LoopConfig,
) -> Vec<Observation> {
    use crate::reasoning::delegation::DelegationContext;
    let depth = config.delegation_depth;
    let chain = &config.delegation_chain;
    let mut observations = Vec::new();
    for action in actions {
        if let ProposedAction::Delegate {
            call_id,
            target,
            message,
        } = action
        {
            let source = format!("delegate:{}", target);
            let obs = match delegation {
                None => Observation::tool_error(
                    source,
                    "agent delegation is not available in this runner",
                ),
                Some(d) => {
                    let ctx = DelegationContext {
                        depth,
                        chain: chain.to_vec(),
                        max_iterations: config.max_iterations,
                        max_total_tokens: config.max_total_tokens,
                        timeout: config.timeout,
                    };
                    match d.delegate(target, message, ctx).await {
                        Ok(output) => Observation::tool_result(source, output),
                        Err(e) => Observation::tool_error(source, e.to_string()),
                    }
                }
            };
            observations.push(obs.with_call_id(call_id.clone()));
        }
    }
    observations
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
        delegation: Option<&dyn crate::reasoning::delegation::DelegationExecutor>,
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

        // Dispatch approved Delegate actions (tool calls are handled above).
        let mut observations = observations;
        let delegate_obs =
            dispatch_delegations(&policy_output.approved_actions, delegation, &self.config).await;
        observations.extend(delegate_obs);

        // Add tool results to conversation
        for obs in &observations {
            let tool_call_id = obs.call_id.as_deref().unwrap_or(&obs.source);
            if !obs.is_error {
                self.state.conversation.push(
                    crate::reasoning::conversation::ConversationMessage::tool_result(
                        tool_call_id,
                        &obs.source,
                        &obs.content,
                    ),
                );
            } else {
                self.state.conversation.push(
                    crate::reasoning::conversation::ConversationMessage::tool_result(
                        tool_call_id,
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
    /// Return observation count from the dispatch phase.
    /// Used by the loop driver to emit `ObservationsCollected` journal events.
    pub fn observation_count(&self) -> usize {
        match &self.phase_data {
            Some(PhaseData::Dispatch(output)) => output.observations.len(),
            _ => 0,
        }
    }

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

/// Validate an LLM-supplied tool-call argument blob.
///
/// `arguments` is a JSON-encoded string controlled by the model, so it
/// can be malformed, of the wrong shape, or intentionally adversarial.
/// We enforce two layers of defence:
///
/// 1. The string MUST parse as a JSON object. Strings, arrays, numbers
///    and other primitives are rejected. This is the minimum bar even
///    when no schema is registered for the named tool — every tool
///    dispatcher in the runtime (the default executor, the MCP bridge)
///    expects an object payload.
/// 2. If the named tool is registered in `tool_definitions` and ships a
///    JSON Schema, the parsed args are validated against that schema
///    via the `jsonschema` crate. The first violation is returned as
///    the denial reason.
///
/// On success, returns `Ok(())`. On failure, returns the human-readable
/// reason that `check_policy` will fold into a `LoopDecision::Deny`.
fn validate_tool_call_arguments(
    name: &str,
    arguments: &str,
    tool_definitions: &[ToolDefinition],
) -> Result<(), String> {
    // Layer 1: parse and require an object.
    let parsed: serde_json::Value = serde_json::from_str(arguments)
        .map_err(|e| format!("tool '{}' arguments are not valid JSON: {}", name, e))?;
    if !parsed.is_object() {
        return Err(format!(
            "tool '{}' arguments must be a JSON object, got {}",
            name,
            json_type_of(&parsed)
        ));
    }

    // Layer 2: schema validation when a definition is registered.
    if let Some(def) = tool_definitions.iter().find(|d| d.name == name) {
        // Treat a missing/empty schema the same as "no schema" — the
        // object-shape check above is the floor in that case.
        if !def.parameters.is_null() {
            match jsonschema::validator_for(&def.parameters) {
                Ok(validator) => {
                    let errors: Vec<String> = validator
                        .iter_errors(&parsed)
                        .map(|e| {
                            let path = e.instance_path.to_string();
                            if path.is_empty() {
                                e.to_string()
                            } else {
                                format!("at '{}': {}", path, e)
                            }
                        })
                        .collect();
                    if !errors.is_empty() {
                        return Err(format!(
                            "tool '{}' arguments failed schema validation: {}",
                            name,
                            errors.join("; ")
                        ));
                    }
                }
                Err(e) => {
                    // A tool with a broken schema shouldn't dispatch
                    // unvalidated args — fail closed.
                    return Err(format!(
                        "tool '{}' has an invalid declared schema (refusing to dispatch): {}",
                        name, e
                    ));
                }
            }
        }
    }

    Ok(())
}

fn json_type_of(v: &serde_json::Value) -> &'static str {
    match v {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "boolean",
        serde_json::Value::Number(_) => "number",
        serde_json::Value::String(_) => "string",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "object",
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

    struct FakeDelegation;
    #[async_trait::async_trait]
    impl crate::reasoning::delegation::DelegationExecutor for FakeDelegation {
        async fn delegate(
            &self,
            target: &str,
            message: &str,
            ctx: crate::reasoning::delegation::DelegationContext,
        ) -> Result<String, crate::reasoning::delegation::DelegationError> {
            Ok(format!(
                "handled '{}' for '{}' at depth {}",
                message, target, ctx.depth
            ))
        }
    }

    #[tokio::test]
    async fn dispatch_routes_delegate_through_delegation_handle() {
        let obs = super::dispatch_delegations(
            &[ProposedAction::Delegate {
                call_id: "test-call".into(),
                target: "reviewer".into(),
                message: "check this".into(),
            }],
            Some(&FakeDelegation),
            &LoopConfig {
                delegation_depth: 2,
                delegation_chain: vec!["planner".into()],
                ..Default::default()
            },
        )
        .await;
        assert_eq!(obs.len(), 1);
        assert!(!obs[0].is_error, "delegation should succeed: {:?}", obs[0]);
        assert!(obs[0].content.contains("check this"));
        assert!(obs[0].content.contains("depth 2"));
    }

    #[tokio::test]
    async fn dispatch_delegate_without_handle_is_honest_error() {
        let obs = super::dispatch_delegations(
            &[ProposedAction::Delegate {
                call_id: "test-call".into(),
                target: "reviewer".into(),
                message: "hi".into(),
            }],
            None,
            &LoopConfig::default(),
        )
        .await;
        assert_eq!(obs.len(), 1);
        assert!(obs[0].is_error);
        assert!(obs[0].content.to_lowercase().contains("delegation"));
    }

    #[tokio::test]
    async fn delegate_observations_carry_the_originating_call_id() {
        // Without a call_id the conversation push falls back to the source
        // string, emitting a tool_result whose id matches no tool_use — which
        // the provider rejects. The observation must carry the real call id.
        let obs = super::dispatch_delegations(
            &[ProposedAction::Delegate {
                call_id: "toolu_abc123".into(),
                target: "reviewer".into(),
                message: "check this".into(),
            }],
            Some(&FakeDelegation),
            &LoopConfig::default(),
        )
        .await;
        assert_eq!(obs.len(), 1);
        assert_eq!(obs[0].call_id.as_deref(), Some("toolu_abc123"));

        // The honest-error path must correlate too, or a failed delegation
        // orphans the tool_use just as badly as a successful one.
        let err_obs = super::dispatch_delegations(
            &[ProposedAction::Delegate {
                call_id: "toolu_xyz789".into(),
                target: "reviewer".into(),
                message: "check this".into(),
            }],
            None,
            &LoopConfig::default(),
        )
        .await;
        assert_eq!(err_obs.len(), 1);
        assert!(err_obs[0].is_error);
        assert_eq!(err_obs[0].call_id.as_deref(), Some("toolu_xyz789"));
    }

    #[test]
    fn delegate_tool_call_converts_to_a_delegate_action() {
        let action = super::tool_call_to_action(
            "toolu_1".to_string(),
            "delegate".to_string(),
            r#"{"agent":"reviewer","task":"check src/main.rs"}"#.to_string(),
            true,
        );
        match action {
            ProposedAction::Delegate {
                call_id,
                target,
                message,
            } => {
                assert_eq!(call_id, "toolu_1");
                assert_eq!(target, "reviewer");
                assert_eq!(message, "check src/main.rs");
            }
            other => panic!("expected Delegate, got {other:?}"),
        }
    }

    #[test]
    fn delegate_tool_call_is_left_alone_when_the_runner_has_no_delegation_handle() {
        // Regression guard: symbi-shell's orchestrator advertises its OWN
        // `delegate` tool and implements it in its executor, and builds its
        // runner without a delegation handle. Converting its call would route it
        // to a dispatcher that can only answer "delegation is not available",
        // silently breaking a tool that works. It must stay a ToolCall so the
        // executor that implements it still receives it.
        let action = super::tool_call_to_action(
            "toolu_shell".to_string(),
            "delegate".to_string(),
            r#"{"agent":"reviewer","task":"check src/main.rs"}"#.to_string(),
            false,
        );
        match action {
            ProposedAction::ToolCall {
                call_id,
                name,
                arguments,
            } => {
                assert_eq!(call_id, "toolu_shell");
                assert_eq!(name, "delegate");
                assert!(arguments.contains("reviewer"));
            }
            other => panic!("expected the call to stay a ToolCall, got {other:?}"),
        }
    }

    #[test]
    fn non_delegate_tool_call_stays_a_tool_call() {
        let action = super::tool_call_to_action(
            "toolu_2".to_string(),
            "list_agents".to_string(),
            "{}".to_string(),
            true,
        );
        assert!(matches!(action, ProposedAction::ToolCall { .. }));
    }

    #[test]
    fn malformed_delegate_args_fall_through_to_a_tool_call() {
        // Never silently dropped: it stays a ToolCall so the executor returns a
        // precise argument error the model can act on.
        for bad in [
            "not json",
            r#"{"agent":"reviewer"}"#,
            r#"{"task":"do it"}"#,
            r#"{"agent":42,"task":"do it"}"#,
        ] {
            let action = super::tool_call_to_action(
                "toolu_3".to_string(),
                "delegate".to_string(),
                bad.to_string(),
                true,
            );
            assert!(
                matches!(action, ProposedAction::ToolCall { .. }),
                "malformed delegate args {bad:?} must stay a ToolCall"
            );
        }
    }

    #[test]
    fn denied_delegate_pushes_a_correlated_tool_result() {
        // A denied Delegate must not vanish: the model needs a tool_result
        // correlated to its tool_use, exactly like a denied ToolCall gets.
        let mut conversation = Conversation::new();
        super::push_denial_tool_result(
            &mut conversation,
            &ProposedAction::Delegate {
                call_id: "toolu_deny".into(),
                target: "reviewer".into(),
                message: "check".into(),
            },
            "no cedar permit",
        );
        let rendered = format!("{:?}", conversation);
        assert!(
            rendered.contains("toolu_deny"),
            "denial must be correlated to the originating call id: {rendered}"
        );
        assert!(rendered.contains("no cedar permit"));
    }
}
