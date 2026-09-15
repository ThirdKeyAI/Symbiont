//! Core reasoning builtins for the DSL
//!
//! Provides async builtin functions that bridge the DSL with the
//! reasoning loop infrastructure: `reason`, `llm_call`, `parse_json`,
//! `delegate`, and `tool_call`.

use crate::dsl::evaluator::DslValue;
use crate::error::{ReplError, Result};
use std::collections::HashMap;
use std::sync::Arc;
use symbi_runtime::communication::policy_gate::CommunicationPolicyGate;
use symbi_runtime::communication::CommunicationBus;
use symbi_runtime::reasoning::agent_registry::AgentRegistry;
use symbi_runtime::reasoning::inference::InferenceProvider;
use symbi_runtime::reasoning::policy_bridge::ReasoningPolicyGate;
use symbi_runtime::types::AgentId;

/// Operator configuration for async reasoning builtins. The evaluator clones it
/// for each invocation and binds that call's execution principal; sender identity
/// must not be stored in shared mutable state.
#[derive(Clone)]
pub struct ReasoningBuiltinContext {
    /// Canonical project captured from trusted startup or explicit SDK configuration.
    /// A failed capture remains an error; later working-directory changes cannot replace it.
    pub project_root: std::result::Result<std::path::PathBuf, String>,
    /// Inference provider for LLM calls.
    pub provider: Option<Arc<dyn InferenceProvider>>,
    /// Agent registry for multi-agent composition.
    pub agent_registry: Option<Arc<AgentRegistry>>,
    /// The calling agent, used for reasoning authorization and communication.
    pub sender_agent_id: Option<AgentId>,
    /// Communication bus for message tracking and audit.
    pub comm_bus: Option<Arc<dyn CommunicationBus + Send + Sync>>,
    /// Communication policy gate for inter-agent authorization.
    pub comm_policy: Option<Arc<CommunicationPolicyGate>>,
    /// Policy gate for the reasoning loop — governs tool calls and
    /// delegations inside `reason()`. If None, `DefaultPolicyGate::new()`
    /// is used (production-default, non-permissive). Production callers
    /// should install [`OpaPolicyGateBridge`] or another concrete gate
    /// instead of relying on the default.
    pub reasoning_policy_gate: Option<Arc<dyn ReasoningPolicyGate>>,
    /// Operator-installed tool backend, shared by reason() and tool_call().
    pub tool_executor: Option<Arc<dyn symbi_runtime::reasoning::executor::ActionExecutor>>,
    /// Required pre-effect and outcome journal for reasoning and explicit calls.
    pub reasoning_journal: Option<Arc<dyn symbi_runtime::reasoning::loop_types::JournalWriter>>,
    /// Trusted execution configuration, including advertised tools and budgets.
    pub reasoning_config: Option<symbi_runtime::reasoning::LoopConfig>,
    /// Public references to required direct-inference journals. This bounded
    /// display history is not the durable store and never grants authority.
    pub audit_references: Arc<crate::dsl::inference_audit::AuditReferenceLog>,
    /// Active session id (shared cell; settable after the context is frozen).
    #[cfg(feature = "session")]
    pub active_session: std::sync::Arc<std::sync::Mutex<Option<symbi_session::monitor::SessionId>>>,
    /// Session monitor for label derivation. Always present once a bridge exists.
    #[cfg(feature = "session")]
    pub session_monitor: Option<std::sync::Arc<symbi_session::monitor::SessionMonitor>>,
}

pub(crate) fn capture_project_root() -> std::result::Result<std::path::PathBuf, String> {
    std::env::current_dir()
        .and_then(|path| path.canonicalize())
        .map_err(|error| error.to_string())
}

impl Default for ReasoningBuiltinContext {
    fn default() -> Self {
        Self {
            project_root: capture_project_root(),
            provider: None,
            agent_registry: None,
            sender_agent_id: None,
            comm_bus: None,
            comm_policy: None,
            reasoning_policy_gate: None,
            tool_executor: None,
            reasoning_journal: None,
            reasoning_config: None,
            audit_references: Arc::new(Default::default()),
            #[cfg(feature = "session")]
            active_session: Default::default(),
            #[cfg(feature = "session")]
            session_monitor: None,
        }
    }
}

impl ReasoningBuiltinContext {
    /// Bind an explicit operator-selected project before constructing the evaluator.
    pub fn with_project_root(mut self, project: impl AsRef<std::path::Path>) -> Result<Self> {
        let root = project.as_ref().canonicalize().map_err(|error| {
            ReplError::Execution(format!("Cannot resolve DSL project: {error}"))
        })?;
        if !root.is_dir() {
            return Err(ReplError::Execution(
                "DSL project must be a directory".into(),
            ));
        }
        self.project_root = Ok(root);
        Ok(self)
    }

    fn project(&self) -> Result<&std::path::Path> {
        let root = self.project_root.as_ref().map_err(|error| {
            ReplError::Execution(format!("DSL project configuration unavailable: {error}"))
        })?;
        if !root.is_absolute() || !root.is_dir() {
            return Err(ReplError::Execution(
                "Configured DSL project is unavailable or not absolute".into(),
            ));
        }
        Ok(root)
    }

    fn executor(&self) -> Result<Arc<dyn symbi_runtime::reasoning::executor::ActionExecutor>> {
        match &self.tool_executor {
            Some(executor) => Ok(executor.clone()),
            None => Ok(symbi_runtime::reasoning::build_tool_executor(
                &self.project()?.join("tools"),
            )),
        }
    }

    pub(crate) async fn journal(
        &self,
        agent: AgentId,
    ) -> Result<(
        Arc<dyn symbi_runtime::reasoning::loop_types::JournalWriter>,
        Option<symbi_runtime::reasoning::run_audit::RunAuditReference>,
    )> {
        if let Some(journal) = &self.reasoning_journal {
            return Ok((journal.clone(), None));
        }
        let (journal, reference) =
            symbi_runtime::reasoning::run_audit::open_run_journal(self.project()?, agent)
                .await
                .map_err(|error| {
                    ReplError::Execution(format!(
                        "Required DSL audit initialization failed: {error}"
                    ))
                })?;
        Ok((journal, Some(reference)))
    }
}

fn include_audit(
    result: &mut HashMap<String, DslValue>,
    audit: &Option<symbi_runtime::reasoning::run_audit::RunAuditReference>,
) {
    if let Some(audit) = audit {
        result.insert(
            "audit".into(),
            DslValue::Map(HashMap::from([
                ("run_id".into(), DslValue::String(audit.run_id.to_string())),
                (
                    "path".into(),
                    DslValue::String(audit.path.display().to_string()),
                ),
                (
                    "public_key".into(),
                    DslValue::String(audit.public_key.clone()),
                ),
            ])),
        );
    }
}

pub(crate) async fn append_event(
    journal: &dyn symbi_runtime::reasoning::loop_types::JournalWriter,
    agent_id: AgentId,
    event: symbi_runtime::reasoning::loop_types::LoopEvent,
) -> Result<()> {
    journal
        .append(symbi_runtime::reasoning::loop_types::JournalEntry {
            sequence: journal.next_sequence().await,
            timestamp: chrono::Utc::now(),
            agent_id,
            iteration: 0,
            event,
        })
        .await
        .map_err(|error| {
            ReplError::Execution(format!("Required DSL journal write failed: {error}"))
        })
}

/// Execute the `reason` builtin: runs a full reasoning loop.
///
/// Arguments (positional or named):
/// - system: string — system prompt
/// - user: string — user message
/// - max_iterations: integer (optional, default 10)
/// - max_tokens: integer (optional, default 100000)
///
/// Returns a map with keys: response, iterations, total_tokens, termination_reason.
pub async fn builtin_reason(args: &[DslValue], ctx: &ReasoningBuiltinContext) -> Result<DslValue> {
    let provider = ctx
        .provider
        .as_ref()
        .ok_or_else(|| ReplError::Execution("No inference provider configured".into()))?;

    let (system, user, max_iterations, max_tokens) = parse_reason_args(args)?;

    use symbi_runtime::reasoning::circuit_breaker::CircuitBreakerRegistry;
    use symbi_runtime::reasoning::context_manager::DefaultContextManager;
    use symbi_runtime::reasoning::conversation::{Conversation, ConversationMessage};
    use symbi_runtime::reasoning::policy_bridge::DefaultPolicyGate;
    use symbi_runtime::reasoning::reasoning_loop::ReasoningLoopRunner;

    // Prefer a caller-provided policy gate (e.g. OpaPolicyGateBridge wired
    // from the runtime). Fall back to the non-permissive default rather
    // than `DefaultPolicyGate::permissive_for_dev_only()` so `reason()`
    // no longer opts every DSL program into unrestricted tool calls
    // regardless of how the runtime was configured.
    let policy_gate: Arc<dyn ReasoningPolicyGate> = match ctx.reasoning_policy_gate.clone() {
        Some(gate) => gate,
        None => Arc::new(DefaultPolicyGate::new()),
    };

    let agent_id = ctx.sender_agent_id.unwrap_or_default();
    let (journal, audit) = ctx.journal(agent_id).await?;
    let runner = ReasoningLoopRunner {
        provider: Arc::clone(provider),
        policy_gate,
        executor: ctx.executor()?,
        context_manager: Arc::new(DefaultContextManager::default()),
        circuit_breakers: Arc::new(CircuitBreakerRegistry::default()),
        journal,
        knowledge_bridge: None,
        delegation: None,
    };

    let mut conv = Conversation::with_system(&system);
    conv.push(ConversationMessage::user(&user));

    let mut config = ctx.reasoning_config.clone().unwrap_or_default();
    config.max_iterations = config.max_iterations.min(max_iterations);
    config.max_total_tokens = config.max_total_tokens.min(max_tokens);

    let result = runner.run(agent_id, conv, config).await;

    let mut map = HashMap::new();
    include_audit(&mut map, &audit);
    map.insert("response".to_string(), DslValue::String(result.output));
    map.insert(
        "iterations".to_string(),
        DslValue::Integer(result.iterations as i64),
    );
    map.insert(
        "total_tokens".to_string(),
        DslValue::Integer(result.total_usage.total_tokens as i64),
    );
    map.insert(
        "termination_reason".to_string(),
        DslValue::String(format!("{:?}", result.termination_reason)),
    );

    Ok(DslValue::Map(map))
}

/// Execute the `llm_call` builtin: one-shot LLM call.
///
/// Arguments:
/// - prompt: string — the prompt to send
/// - model: string (optional) — model override
/// - temperature: number (optional)
/// - max_tokens: integer (optional)
///
/// Returns a string.
pub async fn builtin_llm_call(
    args: &[DslValue],
    ctx: &ReasoningBuiltinContext,
) -> Result<DslValue> {
    let prompt = match args.first() {
        Some(DslValue::String(s)) => s.clone(),
        Some(DslValue::Map(map)) => map
            .get("prompt")
            .and_then(|v| match v {
                DslValue::String(s) => Some(s.clone()),
                _ => None,
            })
            .ok_or_else(|| ReplError::Execution("llm_call requires 'prompt' argument".into()))?,
        _ => {
            return Err(ReplError::Execution(
                "llm_call requires a string prompt".into(),
            ))
        }
    };

    use symbi_runtime::reasoning::conversation::{Conversation, ConversationMessage};
    use symbi_runtime::reasoning::inference::InferenceOptions;

    let mut conv = Conversation::new();
    conv.push(ConversationMessage::user(&prompt));

    let options = InferenceOptions::default();
    let response = ctx
        .infer("llm_call", &conv, &options, None)
        .await
        .map_err(|e| ReplError::Execution(format!("LLM call failed: {}", e)))?;

    Ok(DslValue::String(response.content))
}

/// Execute the `parse_json` builtin: parse a string as JSON.
///
/// Arguments:
/// - text: string — the JSON text to parse
///
/// Returns a DslValue (Map, List, String, Number, Boolean, or Null).
pub fn builtin_parse_json(args: &[DslValue]) -> Result<DslValue> {
    let text = match args.first() {
        Some(DslValue::String(s)) => s,
        _ => {
            return Err(ReplError::Execution(
                "parse_json requires a string argument".into(),
            ))
        }
    };

    let value: serde_json::Value = serde_json::from_str(text)
        .map_err(|e| ReplError::Execution(format!("JSON parse error: {}", e)))?;

    Ok(json_to_dsl_value(&value))
}

/// Execute the `tool_call` builtin: explicit tool invocation.
///
/// Arguments:
/// - name: string — tool name
/// - args: map — tool arguments
///
/// Each call is normalized, approved when required, checked by policy, and
/// journaled before dispatch through the shared runtime dispatcher. Policy
/// modifications are new proposals and must pass every check again.
///
/// Returns a status map and the public audit reference for default journals.
pub async fn builtin_tool_call(
    args: &[DslValue],
    ctx: &ReasoningBuiltinContext,
) -> Result<DslValue> {
    let (name, arguments) = match args {
        [DslValue::String(name), DslValue::Map(args_map)] => {
            let json_args: serde_json::Map<String, serde_json::Value> = args_map
                .iter()
                .map(|(k, v)| (k.clone(), v.to_json()))
                .collect();
            (
                name.clone(),
                serde_json::Value::Object(json_args).to_string(),
            )
        }
        [DslValue::String(name), DslValue::String(args_str)] => (name.clone(), args_str.clone()),
        [DslValue::String(name)] => (name.clone(), "{}".to_string()),
        _ => {
            return Err(ReplError::Execution(
                "tool_call requires (name: string, args?: map|string)".into(),
            ))
        }
    };

    use symbi_runtime::reasoning::circuit_breaker::CircuitBreakerRegistry;
    use symbi_runtime::reasoning::conversation::Conversation;
    use symbi_runtime::reasoning::dispatch::GovernedToolDispatcher;
    use symbi_runtime::reasoning::loop_types::{
        LoopEvent, LoopState, ProposedAction, TerminationReason,
    };
    use symbi_runtime::reasoning::policy_bridge::DefaultPolicyGate;

    let gate = ctx
        .reasoning_policy_gate
        .clone()
        .unwrap_or_else(|| Arc::new(DefaultPolicyGate::new()));
    let agent_id = ctx.sender_agent_id.unwrap_or_default();
    let (journal, audit) = ctx.journal(agent_id).await?;
    let executor = ctx.executor()?;
    let mut config = ctx.reasoning_config.clone().unwrap_or_default();
    if config.tool_definitions.is_empty() {
        config.tool_definitions = executor.tool_definitions();
    }
    let mut state = LoopState::new(agent_id, Conversation::new());
    state.trusted_context = executor.execution_context();
    let action = ProposedAction::ToolCall {
        call_id: "dsl_tool_call".into(),
        name: name.clone(),
        arguments: arguments.clone(),
    };
    let circuit_breakers = CircuitBreakerRegistry::default();
    // Each explicit builtin call owns a run. Multi-step interactive workflows
    // use reason() or a governed SDK run with a shared LoopState and guard.
    let started = std::time::Instant::now();
    append_event(
        journal.as_ref(),
        agent_id,
        LoopEvent::Started {
            agent_id,
            config: Box::new(config.clone()),
            execution_context: state.trusted_context.clone(),
        },
    )
    .await?;
    let outcome = async {
        let cleanup = symbi_runtime::reasoning::executor::ExecutionRunGuard::new(
            executor.clone(),
            &state,
            &config,
        )
        .map_err(ReplError::Execution)?;
        let observations = GovernedToolDispatcher {
            executor: executor.as_ref(),
            gate: gate.as_ref(),
            journal: journal.as_ref(),
            circuit_breakers: &circuit_breakers,
        }
        .dispatch(&[action], &state, &config)
        .await;
        // Cleanup is required even when dispatch or its journal checkpoint failed.
        cleanup.close().await.map_err(|error| {
            ReplError::Execution(format!("Required worker cleanup failed: {error}"))
        })?;
        observations
            .map_err(|error| ReplError::Execution(error.to_string()))?
            .into_iter()
            .next()
            .ok_or_else(|| ReplError::Execution("tool dispatcher returned no result".into()))
    }
    .await;
    let termination = match &outcome {
        Err(error) => TerminationReason::Error {
            message: error.to_string(),
        },
        Ok(obs)
            if obs
                .metadata
                .get("error_type")
                .is_some_and(|kind| kind == "policy_denied") =>
        {
            TerminationReason::PolicyDenial {
                reason: obs.content.clone(),
            }
        }
        Ok(obs) if obs.is_error => TerminationReason::Error {
            message: obs.content.clone(),
        },
        Ok(_) => TerminationReason::Completed,
    };
    append_event(
        journal.as_ref(),
        agent_id,
        LoopEvent::Terminated {
            reason: termination,
            iterations: 1,
            total_usage: Default::default(),
            duration: started.elapsed(),
        },
    )
    .await?;
    let obs = outcome.map_err(|error| match &audit {
        Some(audit) => ReplError::Execution(format!(
            "{error}; audit run {} at {} (public key {})",
            audit.run_id,
            audit.path.display(),
            audit.public_key
        )),
        None => error,
    })?;
    let denied = obs
        .metadata
        .get("error_type")
        .is_some_and(|kind| kind == "policy_denied");
    let mut result = HashMap::new();
    include_audit(&mut result, &audit);
    result.insert(
        "tool".into(),
        DslValue::String(obs.metadata.get("authorized_tool").cloned().unwrap_or(name)),
    );
    result.insert(
        "arguments".into(),
        DslValue::String(
            obs.metadata
                .get("authorized_arguments")
                .cloned()
                .unwrap_or(arguments),
        ),
    );
    result.insert(
        "status".into(),
        DslValue::String(
            if denied {
                "denied"
            } else if obs.is_error {
                "error"
            } else {
                "success"
            }
            .into(),
        ),
    );
    result.insert(
        if denied { "reason" } else { "result" }.into(),
        DslValue::String(obs.content),
    );
    Ok(DslValue::Map(result))
}

/// Execute the `delegate` builtin: send a message to another agent.
///
/// Arguments:
/// - agent: string — agent name
/// - message: string — message to send
/// - timeout: duration (optional)
///
/// Returns the agent's response as a string.
pub async fn builtin_delegate(
    args: &[DslValue],
    ctx: &ReasoningBuiltinContext,
) -> Result<DslValue> {
    let (agent_name, message) = match args {
        [DslValue::String(agent), DslValue::String(msg)] => (agent.clone(), msg.clone()),
        [DslValue::Map(map)] => {
            let agent = map
                .get("agent")
                .and_then(|v| match v {
                    DslValue::String(s) => Some(s.clone()),
                    _ => None,
                })
                .ok_or_else(|| ReplError::Execution("delegate requires 'agent' argument".into()))?;
            let msg = map
                .get("message")
                .and_then(|v| match v {
                    DslValue::String(s) => Some(s.clone()),
                    _ => None,
                })
                .ok_or_else(|| {
                    ReplError::Execution("delegate requires 'message' argument".into())
                })?;
            (agent, msg)
        }
        _ => {
            return Err(ReplError::Execution(
                "delegate requires (agent: string, message: string)".into(),
            ))
        }
    };

    let label = optional_protocol_label(args);
    crate::dsl::agent_composition::governed_ask(ctx, &agent_name, &message, label.as_deref())
        .await
        .map(DslValue::String)
}

// --- Helper functions ---

fn parse_reason_args(args: &[DslValue]) -> Result<(String, String, u32, u32)> {
    match args {
        // Named arguments via map
        [DslValue::Map(map)] => {
            let system = map
                .get("system")
                .and_then(|v| match v {
                    DslValue::String(s) => Some(s.clone()),
                    _ => None,
                })
                .ok_or_else(|| ReplError::Execution("reason requires 'system' argument".into()))?;
            let user = map
                .get("user")
                .and_then(|v| match v {
                    DslValue::String(s) => Some(s.clone()),
                    _ => None,
                })
                .ok_or_else(|| ReplError::Execution("reason requires 'user' argument".into()))?;
            let max_iterations = map
                .get("max_iterations")
                .and_then(|v| match v {
                    DslValue::Integer(i) => Some(*i as u32),
                    DslValue::Number(n) => Some(*n as u32),
                    _ => None,
                })
                .unwrap_or(10);
            let max_tokens = map
                .get("max_tokens")
                .and_then(|v| match v {
                    DslValue::Integer(i) => Some(*i as u32),
                    DslValue::Number(n) => Some(*n as u32),
                    _ => None,
                })
                .unwrap_or(100_000);
            Ok((system, user, max_iterations, max_tokens))
        }
        // Positional: system, user
        [DslValue::String(system), DslValue::String(user)] => {
            Ok((system.clone(), user.clone(), 10, 100_000))
        }
        // Positional: system, user, max_iterations
        [DslValue::String(system), DslValue::String(user), DslValue::Integer(max_iter)] => {
            Ok((system.clone(), user.clone(), *max_iter as u32, 100_000))
        }
        _ => Err(ReplError::Execution(
            "reason requires (system: string, user: string, [max_iterations?, max_tokens?])".into(),
        )),
    }
}

/// Extract an optional `protocol_label` string from a DSL argument list.
///
/// Named args arrive as a single `DslValue::Map`. This helper looks up the
/// `"protocol_label"` key in that map and returns its value when it is a
/// `DslValue::String`. Returns `None` when the key is absent, has the wrong
/// type, or the args are positional-only (no map present).
pub(crate) fn optional_protocol_label(args: &[DslValue]) -> Option<String> {
    for arg in args {
        if let DslValue::Map(map) = arg {
            if let Some(DslValue::String(label)) = map.get("protocol_label") {
                return Some(label.clone());
            }
        }
    }
    None
}

/// Convert a serde_json::Value to a DslValue.
pub fn json_to_dsl_value(value: &serde_json::Value) -> DslValue {
    match value {
        serde_json::Value::Null => DslValue::Null,
        serde_json::Value::Bool(b) => DslValue::Boolean(*b),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                DslValue::Integer(i)
            } else if let Some(f) = n.as_f64() {
                DslValue::Number(f)
            } else {
                DslValue::Number(0.0)
            }
        }
        serde_json::Value::String(s) => DslValue::String(s.clone()),
        serde_json::Value::Array(arr) => {
            DslValue::List(arr.iter().map(json_to_dsl_value).collect())
        }
        serde_json::Value::Object(obj) => {
            let map: HashMap<String, DslValue> = obj
                .iter()
                .map(|(k, v)| (k.clone(), json_to_dsl_value(v)))
                .collect();
            DslValue::Map(map)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_json_valid() {
        let result =
            builtin_parse_json(&[DslValue::String(r#"{"key": "value", "num": 42}"#.into())])
                .unwrap();
        match result {
            DslValue::Map(map) => {
                assert_eq!(map.get("key"), Some(&DslValue::String("value".into())));
                assert_eq!(map.get("num"), Some(&DslValue::Integer(42)));
            }
            _ => panic!("Expected Map"),
        }
    }

    #[test]
    fn test_parse_json_array() {
        let result = builtin_parse_json(&[DslValue::String("[1, 2, 3]".into())]).unwrap();
        match result {
            DslValue::List(items) => {
                assert_eq!(items.len(), 3);
                assert_eq!(items[0], DslValue::Integer(1));
            }
            _ => panic!("Expected List"),
        }
    }

    #[test]
    fn test_parse_json_invalid() {
        let result = builtin_parse_json(&[DslValue::String("not json".into())]);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_json_nested() {
        let json = r#"{"tasks": [{"id": 1, "done": false}], "count": 1}"#;
        let result = builtin_parse_json(&[DslValue::String(json.into())]).unwrap();
        match result {
            DslValue::Map(map) => match map.get("tasks") {
                Some(DslValue::List(tasks)) => {
                    assert_eq!(tasks.len(), 1);
                    match &tasks[0] {
                        DslValue::Map(task) => {
                            assert_eq!(task.get("id"), Some(&DslValue::Integer(1)));
                            assert_eq!(task.get("done"), Some(&DslValue::Boolean(false)));
                        }
                        _ => panic!("Expected Map in list"),
                    }
                }
                _ => panic!("Expected List for tasks"),
            },
            _ => panic!("Expected Map"),
        }
    }

    #[test]
    fn test_json_to_dsl_value_all_types() {
        let json = serde_json::json!({
            "str": "hello",
            "int": 42,
            "float": 1.5,
            "bool": true,
            "null": null,
            "arr": [1, 2],
            "obj": {"nested": "value"}
        });

        let dsl = json_to_dsl_value(&json);
        match dsl {
            DslValue::Map(map) => {
                assert_eq!(map.get("str"), Some(&DslValue::String("hello".into())));
                assert_eq!(map.get("int"), Some(&DslValue::Integer(42)));
                assert_eq!(map.get("bool"), Some(&DslValue::Boolean(true)));
                assert_eq!(map.get("null"), Some(&DslValue::Null));
            }
            _ => panic!("Expected Map"),
        }
    }

    #[test]
    fn test_parse_reason_args_positional() {
        let args = vec![
            DslValue::String("system prompt".into()),
            DslValue::String("user message".into()),
        ];
        let (system, user, max_iter, max_tokens) = parse_reason_args(&args).unwrap();
        assert_eq!(system, "system prompt");
        assert_eq!(user, "user message");
        assert_eq!(max_iter, 10);
        assert_eq!(max_tokens, 100_000);
    }

    #[test]
    fn test_parse_reason_args_named() {
        let mut map = HashMap::new();
        map.insert("system".into(), DslValue::String("sys".into()));
        map.insert("user".into(), DslValue::String("usr".into()));
        map.insert("max_iterations".into(), DslValue::Integer(5));

        let args = vec![DslValue::Map(map)];
        let (system, user, max_iter, max_tokens) = parse_reason_args(&args).unwrap();
        assert_eq!(system, "sys");
        assert_eq!(user, "usr");
        assert_eq!(max_iter, 5);
        assert_eq!(max_tokens, 100_000);
    }

    #[test]
    fn test_parse_reason_args_missing_required() {
        let mut map = HashMap::new();
        map.insert("system".into(), DslValue::String("sys".into()));
        // Missing "user"

        let args = vec![DslValue::Map(map)];
        assert!(parse_reason_args(&args).is_err());
    }

    #[test]
    fn extracts_optional_protocol_label_named_arg() {
        // Named-arg map containing protocol_label alongside agent/message
        let mut map_with = HashMap::new();
        map_with.insert("agent".into(), DslValue::String("Worker".into()));
        map_with.insert("message".into(), DslValue::String("go".into()));
        map_with.insert("protocol_label".into(), DslValue::String("fast".into()));
        let with = vec![DslValue::Map(map_with)];
        assert_eq!(optional_protocol_label(&with), Some("fast".to_string()));

        // Positional-only — no protocol_label
        let without = vec![
            DslValue::String("Worker".into()),
            DslValue::String("go".into()),
        ];
        assert_eq!(optional_protocol_label(&without), None);

        // Named-arg map without protocol_label key
        let mut map_absent = HashMap::new();
        map_absent.insert("agent".into(), DslValue::String("Bot".into()));
        map_absent.insert("message".into(), DslValue::String("hi".into()));
        let no_label = vec![DslValue::Map(map_absent)];
        assert_eq!(optional_protocol_label(&no_label), None);

        // Wrong type for protocol_label — treated as absent
        let mut map_wrong = HashMap::new();
        map_wrong.insert("agent".into(), DslValue::String("Bot".into()));
        map_wrong.insert("message".into(), DslValue::String("hi".into()));
        map_wrong.insert("protocol_label".into(), DslValue::Integer(42));
        let wrong_type = vec![DslValue::Map(map_wrong)];
        assert_eq!(optional_protocol_label(&wrong_type), None);
    }

    // Real marker effects distinguish authorization from a cosmetic status.
    // Explicit fixture roots avoid changing the process-wide working directory.
    use symbi_runtime::reasoning::policy_bridge::DefaultPolicyGate;

    pub(super) fn setup_side_effecting_tool() -> (std::path::PathBuf, tempfile::TempDir) {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let tools_dir = tempdir.path().join("tools");
        std::fs::create_dir_all(&tools_dir).expect("mkdir tools/");
        std::fs::write(
            tools_dir.join("touch_marker.clad.toml"),
            r#"
[tool]
name = "touch_marker"
version = "1.0.0"
binary = "touch"
description = "test-only: creates a marker file to prove the tool actually ran"

[args.path]
position = 1
required = true
type = "string"
description = "marker file path"

[command]
template = "touch {path}"

[output]
format = "text"

[output.schema]
type = "object"
"#,
        )
        .expect("write manifest");
        let marker = tempdir.path().join("marker.txt");
        (marker, tempdir)
    }

    fn touch_marker_call(marker: &std::path::Path) -> Vec<DslValue> {
        let mut args_map = HashMap::new();
        args_map.insert(
            "path".to_string(),
            DslValue::String(marker.display().to_string()),
        );
        vec![
            DslValue::String("touch_marker".to_string()),
            DslValue::Map(args_map),
        ]
    }

    #[tokio::test]
    async fn tool_call_denied_by_fail_closed_gate_never_executes() {
        let (marker, project) = setup_side_effecting_tool();
        let ctx = ReasoningBuiltinContext {
            project_root: Ok(project.path().to_owned()),
            reasoning_policy_gate: Some(Arc::new(DefaultPolicyGate::new())),
            ..Default::default()
        };

        let result = builtin_tool_call(&touch_marker_call(&marker), &ctx)
            .await
            .expect("builtin_tool_call should not error, just report denial");

        match result {
            DslValue::Map(map) => {
                assert_eq!(
                    map.get("status"),
                    Some(&DslValue::String("denied".to_string()))
                );
            }
            other => panic!("expected Map, got {other:?}"),
        }
        assert!(
            !marker.exists(),
            "fail-closed gate must prevent the tool from actually running, \
             not just report an error"
        );
    }

    #[tokio::test]
    async fn tool_call_with_no_gate_in_context_defaults_fail_closed() {
        // Mirrors `builtin_reason`'s default: a context that never had a
        // gate installed must NOT fall back to permissive.
        let (marker, project) = setup_side_effecting_tool();
        let ctx = ReasoningBuiltinContext::default()
            .with_project_root(project.path())
            .unwrap();

        builtin_tool_call(&touch_marker_call(&marker), &ctx)
            .await
            .expect("builtin_tool_call should not error, just report denial");

        assert!(
            !marker.exists(),
            "no policy gate configured must default to fail-closed, never permissive"
        );
    }

    #[tokio::test]
    async fn tool_call_allowed_by_permissive_gate_executes() {
        let (marker, project) = setup_side_effecting_tool();
        let ctx = ReasoningBuiltinContext {
            project_root: Ok(project.path().to_owned()),
            tool_executor: Some(Arc::new(
                symbi_runtime::toolclad::ToolCladExecutor::new(
                    symbi_runtime::toolclad::manifest::load_manifests_from_dir(
                        &project.path().join("tools"),
                    ),
                )
                .with_development_host_execution(),
            )),
            reasoning_policy_gate: Some(Arc::new(DefaultPolicyGate::permissive_for_dev_only())),
            ..Default::default()
        };

        let result = builtin_tool_call(&touch_marker_call(&marker), &ctx)
            .await
            .expect("builtin_tool_call should not error");

        match result {
            DslValue::Map(map) => {
                assert_eq!(
                    map.get("status"),
                    Some(&DslValue::String("success".to_string()))
                );
            }
            other => panic!("expected Map, got {other:?}"),
        }
        assert!(
            marker.exists(),
            "permissive gate should allow the tool to actually run"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn tool_call_binds_normalized_cedar_input_and_audit_to_real_execution() {
        use symbi_runtime::reasoning::loop_types::{BufferedJournal, LoopEvent};
        use symbi_runtime::reasoning::{CedarPolicy, CedarPolicyGate};
        for (tool, arguments, approval, expected_effect) in [
            ("count_fixture", r#"{"count":"999"}"#, false, true),
            ("count_fixture", r#"{"count":"1"}"#, false, false),
            (
                "count_fixture",
                r#"{"count":"999","extra":"ignored"}"#,
                false,
                false,
            ),
            ("unknown_fixture", "{}", false, false),
            ("count_fixture", r#"{"count":"999"}"#, true, false),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let tools = dir.path().join("tools");
            std::fs::create_dir(&tools).unwrap();
            let manifest_path = tools.join("count_fixture.clad.toml");
            std::fs::write(
                &manifest_path,
                format!(
                    r#"
[tool]
name = "count_fixture"
version = "1"
binary = "/usr/bin/touch"
description = "DSL prepared call fixture"
human_approval = {approval}
[tool.cedar]
resource = "Tool::Fixture"
action = "execute"
[args.count]
position = 1
required = true
type = "integer"
min = 1
max = 5
clamp = true
[command]
template = "/usr/bin/touch '{}/{{count}}'"
[output]
format = "text"
"#,
                    dir.path().display()
                ),
            )
            .unwrap();
            let manifest =
                symbi_runtime::toolclad::manifest::load_manifest(&manifest_path).unwrap();
            let gate = CedarPolicyGate::deny_by_default();
            gate.add_policy(CedarPolicy {
                name: "fixture".into(), active: true,
                source: format!("{}\nforbid(principal, action, resource) when {{ context.invocation.arguments.count != \"5\" }};", symbi_runtime::toolclad::cedar_gen::generate_policy(&manifest).unwrap()),
            }).await;
            let journal = Arc::new(BufferedJournal::new(100));
            let ctx = ReasoningBuiltinContext {
                reasoning_policy_gate: Some(Arc::new(gate)),
                tool_executor: Some(Arc::new(
                    symbi_runtime::toolclad::ToolCladExecutor::new(
                        symbi_runtime::toolclad::manifest::load_manifests_from_dir(&tools),
                    )
                    .with_development_host_execution(),
                )),
                reasoning_journal: Some(journal.clone()),
                ..Default::default()
            };
            let result = builtin_tool_call(
                &[
                    DslValue::String(tool.into()),
                    DslValue::String(arguments.into()),
                ],
                &ctx,
            )
            .await
            .unwrap();
            let DslValue::Map(result) = result else {
                panic!("expected result map")
            };
            assert_eq!(
                result["status"],
                DslValue::String(if expected_effect { "success" } else { "denied" }.into()),
                "{result:?}"
            );
            assert_eq!(dir.path().join("5").exists(), expected_effect, "{result:?}");
            assert!(!dir.path().join("999").exists());
            assert!(!dir.path().join("1").exists());
            let entries = journal.entries().await;
            // Dispatch start/finish are journalled only for an approved call; a
            // denied one is never dispatched, so that pair is absent.
            assert_eq!(entries.len(), if expected_effect { 6 } else { 4 });
            assert!(matches!(entries[0].event, LoopEvent::Started { .. }));
            assert!(matches!(
                entries[entries.len() - 1].event,
                LoopEvent::Terminated { .. }
            ));
            if expected_effect {
                assert!(matches!(
                    entries[2].event,
                    LoopEvent::ToolDispatchStarted { .. }
                ));
                assert!(matches!(
                    entries[3].event,
                    LoopEvent::ToolDispatchFinished { .. }
                ));
            }
            let LoopEvent::PolicyEvaluated { approved_calls, .. } = &entries[1].event else {
                panic!("missing pre-effect checkpoint")
            };
            if expected_effect {
                assert_eq!(approved_calls[0]["arguments"]["count"], "5");
                assert_eq!(
                    result["arguments"],
                    DslValue::String(r#"{"count":"5"}"#.into())
                );
            } else {
                assert!(approved_calls.is_empty());
            }
            assert!(matches!(
                entries[entries.len() - 2].event,
                LoopEvent::ToolBatchCompleted { .. }
            ));
        }
    }

    #[tokio::test]
    async fn tool_call_returns_required_journal_failure() {
        use symbi_runtime::reasoning::loop_types::{JournalEntry, JournalError, JournalWriter};
        struct FailingJournal;
        #[async_trait::async_trait]
        impl JournalWriter for FailingJournal {
            async fn append(&self, _: JournalEntry) -> std::result::Result<(), JournalError> {
                Err(JournalError::WriteFailed("DSL journal fixture".into()))
            }
            async fn next_sequence(&self) -> u64 {
                0
            }
        }
        let ctx = ReasoningBuiltinContext {
            reasoning_journal: Some(Arc::new(FailingJournal)),
            ..Default::default()
        };
        let error = builtin_tool_call(&[DslValue::String("missing".into())], &ctx)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("DSL journal fixture"));
    }

    #[cfg(unix)]
    #[tokio::test]
    #[ignore = "requires Docker, cached Python fixture image, and runtime toolclad-session feature"]
    async fn explicit_dsl_terminal_call_awaits_contained_worker_removal() {
        use std::os::unix::fs::PermissionsExt;
        use symbi_runtime::{
            reasoning::{CedarPolicy, CedarPolicyGate},
            sandbox::command::CommandBoundary,
            toolclad::ToolCladExecutor,
        };
        let dir = tempfile::tempdir().unwrap();
        let output = dir.path().join("output");
        std::fs::create_dir(&output).unwrap();
        std::fs::set_permissions(&output, std::fs::Permissions::from_mode(0o777)).unwrap();
        let script = dir.path().join("terminal.py");
        std::fs::write(
            &script,
            r#"
import os, pathlib, sys, time
assert sys.stdin.isatty() and sys.stdout.isatty()
assert os.getuid() == 65534
print('READY>', flush=True)
for line in sys.stdin:
    assert line.strip() == 'run'
    pathlib.Path('/workspace/allowed').write_text('real terminal effect')
    if os.fork() == 0:
        os.setsid()
        time.sleep(30)
        pathlib.Path('/workspace/late-effect').touch()
        os._exit(0)
    print('executed\nREADY>', flush=True)
"#,
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o444)).unwrap();
        let manifest_path = dir.path().join("terminal.clad.toml");
        std::fs::write(
            &manifest_path,
            r#"
[tool]
name = "terminal"
version = "1"
description = "DSL terminal fixture"
mode = "session"
timeout_seconds = 10
[session]
startup_command = "/usr/local/bin/python3 -u /opt/terminal.py"
ready_pattern = "READY>"
startup_timeout_seconds = 5
idle_timeout_seconds = 20
session_timeout_seconds = 20
max_interactions = 5
[session.commands.send]
pattern = "run"
description = "Execute fixture"
[output]
format = "text"
"#,
        )
        .unwrap();
        let manifest = symbi_runtime::toolclad::manifest::load_manifest(&manifest_path).unwrap();
        let mut profile = CommandBoundary::default();
        profile.docker.image =
            "sha256:7bf61a5ec2a4631b240bc8cf83404e2dd37fb06a4b4bbd34b2c36906a5fb4aee".into();
        profile.docker.volumes = vec![
            format!("{}:/workspace:rw", output.display()),
            format!("{}:/opt/terminal.py:ro", script.display()),
        ];
        let label = format!("symbi.dsl-pty-e2e={}", uuid::Uuid::new_v4());
        profile.docker.extra_flags.push(format!("--label={label}"));
        let executor = Arc::new(
            ToolCladExecutor::new(vec![("terminal".into(), manifest)])
                .with_command_boundary(profile),
        );
        let gate = CedarPolicyGate::deny_by_default();
        gate.add_policy(CedarPolicy { name: "fixture".into(), active: true,
            source: "permit(principal, action == Action::\"tool_call::terminal.send\", resource) when { context.invocation.arguments.command == \"run\" };".into()
        }).await;
        let ctx = ReasoningBuiltinContext {
            project_root: Ok(dir.path().to_owned()),
            tool_executor: Some(executor.clone()),
            reasoning_policy_gate: Some(Arc::new(gate)),
            ..Default::default()
        };
        let result = builtin_tool_call(
            &[
                DslValue::String("terminal.send".into()),
                DslValue::String(r#"{"command":"run"}"#.into()),
            ],
            &ctx,
        )
        .await
        .unwrap();
        let DslValue::Map(result) = result else {
            panic!("expected tool result")
        };
        assert_eq!(
            result["status"],
            DslValue::String("success".into()),
            "{result:?}"
        );
        assert_eq!(
            std::fs::read_to_string(output.join("allowed")).unwrap(),
            "real terminal effect"
        );
        let workers = tokio::process::Command::new("docker")
            .args(["ps", "-aq", "--filter", &format!("label={label}")])
            .output()
            .await
            .unwrap();
        assert!(workers.status.success());
        assert!(
            workers.stdout.is_empty(),
            "builtin must await cleanup even while executor is retained"
        );
        assert!(!output.join("late-effect").exists());
        let entries =
            super::audit_tests::verified_result(&DslValue::Map(result), ctx.sender_agent_id);
        assert!(matches!(
            entries.last().unwrap().event,
            symbi_runtime::reasoning::loop_types::LoopEvent::Terminated {
                reason: symbi_runtime::reasoning::loop_types::TerminationReason::Completed,
                ..
            }
        ));
        drop(executor);
    }
}

#[cfg(all(test, unix))]
mod audit_tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use symbi_runtime::reasoning::{
        inference::{FinishReason, InferenceOptions, InferenceResponse, Usage},
        loop_types::{JournalEntry, LoopEvent, TerminationReason},
        protected_journal::ProtectedJournal,
    };

    struct Provider {
        calls: AtomicUsize,
        pending: bool,
    }
    #[async_trait::async_trait]
    impl InferenceProvider for Provider {
        async fn complete(
            &self,
            _: &symbi_runtime::reasoning::conversation::Conversation,
            _: &InferenceOptions,
        ) -> std::result::Result<
            InferenceResponse,
            symbi_runtime::reasoning::inference::InferenceError,
        > {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if self.pending {
                std::future::pending::<()>().await;
            }
            Ok(InferenceResponse {
                content: "audit fixture response".into(),
                tool_calls: vec![],
                finish_reason: FinishReason::Stop,
                usage: Usage::default(),
                model: "fixture".into(),
            })
        }
        fn provider_name(&self) -> &str {
            "audit-fixture"
        }
        fn default_model(&self) -> &str {
            "fixture"
        }
        fn supports_native_tools(&self) -> bool {
            false
        }
        fn supports_structured_output(&self) -> bool {
            false
        }
    }
    fn provider(pending: bool) -> Arc<Provider> {
        Arc::new(Provider {
            calls: AtomicUsize::new(0),
            pending,
        })
    }
    fn arguments() -> Vec<DslValue> {
        vec![
            DslValue::String("Synthetic system".into()),
            DslValue::String("Synthetic request".into()),
        ]
    }
    pub(super) fn verified_result(
        value: &DslValue,
        principal: Option<AgentId>,
    ) -> Vec<JournalEntry> {
        use std::os::unix::fs::PermissionsExt;
        let json = value.to_json();
        let audit = &json["audit"];
        let path = std::path::Path::new(audit["path"].as_str().unwrap());
        let key = audit["public_key"].as_str().unwrap();
        let key: [u8; 32] = (0..32)
            .map(|i| u8::from_str_radix(&key[i * 2..i * 2 + 2], 16).unwrap())
            .collect::<Vec<_>>()
            .try_into()
            .unwrap();
        let run = audit["run_id"].as_str().unwrap().parse().unwrap();
        let entries = ProtectedJournal::verify_run(path, &key, run).unwrap();
        assert!(ProtectedJournal::verify_run(path, &key, uuid::Uuid::new_v4()).is_err());
        assert_eq!(
            std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let id = principal.unwrap_or(entries[0].agent_id);
        assert!(entries.iter().all(|entry| entry.agent_id == id));
        assert!(matches!(entries[0].event, LoopEvent::Started { agent_id, .. } if agent_id == id));
        entries
    }

    #[tokio::test]
    async fn default_reason_audit_has_distinct_runs_and_one_bound_principal() {
        let project = tempfile::tempdir().unwrap();
        let id = AgentId::new();
        let inference = provider(false);
        let ctx = ReasoningBuiltinContext {
            provider: Some(inference.clone()),
            sender_agent_id: Some(id),
            ..Default::default()
        }
        .with_project_root(project.path())
        .unwrap();
        let first = builtin_reason(&arguments(), &ctx).await.unwrap();
        let second = builtin_reason(&arguments(), &ctx).await.unwrap();
        for value in [&first, &second] {
            let entries = verified_result(value, Some(id));
            assert!(matches!(
                entries.last().unwrap().event,
                LoopEvent::Terminated {
                    reason: TerminationReason::Completed,
                    ..
                }
            ));
        }
        assert_ne!(
            first.to_json()["audit"]["run_id"],
            second.to_json()["audit"]["run_id"]
        );
        assert_eq!(
            first.to_json()["audit"]["public_key"],
            second.to_json()["audit"]["public_key"]
        );
        assert_eq!(inference.calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn unsafe_default_audit_blocks_inference_and_dispatch() {
        use std::os::unix::fs::PermissionsExt;
        let (marker, project) = super::tests::setup_side_effecting_tool();
        let audit = project.path().join(".symbiont/governed");
        std::fs::create_dir_all(&audit).unwrap();
        std::fs::set_permissions(&audit, std::fs::Permissions::from_mode(0o777)).unwrap();
        let inference = provider(false);
        let ctx = ReasoningBuiltinContext {
            provider: Some(inference.clone()),
            reasoning_policy_gate: Some(Arc::new(
                symbi_runtime::reasoning::policy_bridge::DefaultPolicyGate::permissive_for_dev_only(
                ),
            )),
            tool_executor: Some(Arc::new(
                symbi_runtime::toolclad::ToolCladExecutor::new(
                    symbi_runtime::toolclad::manifest::load_manifests_from_dir(
                        &project.path().join("tools"),
                    ),
                )
                .with_development_host_execution(),
            )),
            ..Default::default()
        }
        .with_project_root(project.path())
        .unwrap();
        let error = builtin_reason(&arguments(), &ctx).await.unwrap_err();
        assert!(error
            .to_string()
            .contains("Required DSL audit initialization failed"));
        let error = builtin_tool_call(
            &[
                DslValue::String("touch_marker".into()),
                DslValue::String(serde_json::json!({"path": marker}).to_string()),
            ],
            &ctx,
        )
        .await
        .unwrap_err();
        assert!(error
            .to_string()
            .contains("Required DSL audit initialization failed"));
        assert_eq!(inference.calls.load(Ordering::SeqCst), 0);
        assert!(!marker.exists());
        assert_eq!(std::fs::read_dir(audit).unwrap().count(), 0);
    }

    #[tokio::test]
    async fn missing_project_capture_is_never_replaced_by_later_cwd() {
        let inference = provider(false);
        let ctx = ReasoningBuiltinContext {
            provider: Some(inference.clone()),
            project_root: Err("startup fixture failure".into()),
            ..Default::default()
        };
        assert!(builtin_reason(&arguments(), &ctx)
            .await
            .unwrap_err()
            .to_string()
            .contains("startup fixture failure"));
        assert!(
            builtin_tool_call(&[DslValue::String("missing".into())], &ctx)
                .await
                .unwrap_err()
                .to_string()
                .contains("startup fixture failure")
        );
        assert_eq!(inference.calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn cancelled_reason_leaves_a_signed_incomplete_prefix() {
        let project = tempfile::tempdir().unwrap();
        let (anchor, reference) =
            symbi_runtime::reasoning::run_audit::open_run_journal(project.path(), AgentId::new())
                .await
                .unwrap();
        let key: [u8; 32] = (0..32)
            .map(|i| u8::from_str_radix(&reference.public_key[i * 2..i * 2 + 2], 16).unwrap())
            .collect::<Vec<_>>()
            .try_into()
            .unwrap();
        drop(anchor);
        let inference = provider(true);
        let ctx = ReasoningBuiltinContext {
            provider: Some(inference.clone()),
            ..Default::default()
        }
        .with_project_root(project.path())
        .unwrap();
        let args = arguments();
        let future = builtin_reason(&args, &ctx);
        tokio::pin!(future);
        // Drive the call until the provider has actually been entered, rather
        // than assuming a fixed delay is long enough to get there. A 300ms
        // budget was not always enough under parallel load, and the call-count
        // assertion then failed for a reason unrelated to what this covers.
        // The provider stalls forever once entered, so the future must not
        // finish first.
        tokio::select! {
            _ = &mut future => panic!("reason() must not complete while the provider stalls"),
            entered = tokio::time::timeout(std::time::Duration::from_secs(10), async {
                while inference.calls.load(Ordering::SeqCst) == 0 {
                    tokio::task::yield_now().await;
                }
            }) => entered.expect("the provider must be entered"),
        }
        // Having been entered, it must stay pending.
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(100), &mut future)
                .await
                .is_err()
        );
        assert_eq!(inference.calls.load(Ordering::SeqCst), 1);
        let audit = project.path().join(".symbiont/governed");
        let file = std::fs::read_dir(&audit)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .find(|path| {
                path != &reference.path && path.extension().is_some_and(|ext| ext == "jsonl")
            })
            .unwrap();
        let run_id = file
            .file_stem()
            .unwrap()
            .to_str()
            .unwrap()
            .rsplit('.')
            .next()
            .unwrap()
            .parse()
            .unwrap();
        let entries = ProtectedJournal::verify_run(&file, &key, run_id).unwrap();
        assert!(matches!(entries[0].event, LoopEvent::Started { .. }));
        assert!(entries
            .iter()
            .all(|entry| !matches!(entry.event, LoopEvent::Terminated { .. })));
    }

    #[test]
    fn bridge_project_survives_process_working_directory_change() {
        // Only the isolated subprocess changes CWD; the parallel test runner never does.
        const CHILD: &str = "SYMBI_DSL_CWD_FIXTURE_CHILD";
        if std::env::var_os(CHILD).is_none() {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "dsl::reasoning_builtins::audit_tests::bridge_project_survives_process_working_directory_change", "--test-threads=1"])
                .env(CHILD, "1").output().unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stdout)
            );
            return;
        }
        let (_, project) = super::tests::setup_side_effecting_tool();
        let decoy = tempfile::tempdir().unwrap();
        std::env::set_current_dir(project.path()).unwrap();
        let bridge = crate::runtime_bridge::RuntimeBridge::new();
        let initial = ReasoningBuiltinContext::default();
        std::env::set_current_dir(decoy.path()).unwrap();
        let ctx = bridge.reasoning_context();
        assert_eq!(ctx.project().unwrap(), project.path());
        assert_eq!(initial.project().unwrap(), project.path());
        for context in [&initial, &ctx] {
            assert!(context
                .executor()
                .unwrap()
                .tool_definitions()
                .iter()
                .any(|tool| tool.name == "touch_marker"));
        }
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            let value = builtin_tool_call(&[DslValue::String("missing".into())], &ctx)
                .await
                .unwrap();
            let entries = verified_result(&value, None);
            assert!(matches!(
                entries.last().unwrap().event,
                LoopEvent::Terminated {
                    reason: TerminationReason::PolicyDenial { .. },
                    ..
                }
            ));
            assert!(
                std::path::Path::new(value.to_json()["audit"]["path"].as_str().unwrap())
                    .starts_with(project.path())
            );
        });
        assert!(!decoy.path().join(".symbiont").exists());
    }
    #[tokio::test]
    async fn failed_terminal_write_cannot_report_an_effect_as_success() {
        use symbi_runtime::reasoning::loop_types::{JournalError, JournalWriter};
        struct TerminalFailure;
        #[async_trait::async_trait]
        impl JournalWriter for TerminalFailure {
            async fn append(&self, entry: JournalEntry) -> std::result::Result<(), JournalError> {
                if matches!(entry.event, LoopEvent::Terminated { .. }) {
                    Err(JournalError::WriteFailed("terminal write fixture".into()))
                } else {
                    Ok(())
                }
            }
            async fn next_sequence(&self) -> u64 {
                0
            }
        }
        let (marker, project) = super::tests::setup_side_effecting_tool();
        let executor = symbi_runtime::toolclad::ToolCladExecutor::new(
            symbi_runtime::toolclad::manifest::load_manifests_from_dir(
                &project.path().join("tools"),
            ),
        )
        .with_development_host_execution();
        let ctx = ReasoningBuiltinContext {
            tool_executor: Some(Arc::new(executor)),
            reasoning_policy_gate: Some(Arc::new(
                symbi_runtime::reasoning::policy_bridge::DefaultPolicyGate::permissive_for_dev_only(
                ),
            )),
            reasoning_journal: Some(Arc::new(TerminalFailure)),
            ..Default::default()
        };
        let error = builtin_tool_call(
            &[
                DslValue::String("touch_marker".into()),
                DslValue::String(serde_json::json!({"path": marker}).to_string()),
            ],
            &ctx,
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("terminal write fixture"));
        assert!(
            marker.exists(),
            "the effect happened but must not be reported as durably completed"
        );
    }

    #[tokio::test]
    async fn cleanup_failure_is_recorded_as_error_before_return() {
        use symbi_runtime::reasoning::{
            circuit_breaker::CircuitBreakerRegistry,
            executor::ActionExecutor,
            loop_types::{BufferedJournal, Observation, ProposedAction},
            LoopConfig,
        };
        struct CleanupFailure;
        #[async_trait::async_trait]
        impl ActionExecutor for CleanupFailure {
            async fn execute_actions(
                &self,
                _: &[ProposedAction],
                _: &LoopConfig,
                _: &CircuitBreakerRegistry,
            ) -> Vec<Observation> {
                panic!("unregistered tool must not execute")
            }
            async fn close_run(
                &self,
                _: &str,
                _: std::time::Instant,
            ) -> std::result::Result<(), String> {
                Err("cleanup fixture".into())
            }
        }
        let journal = Arc::new(BufferedJournal::new(100));
        let ctx = ReasoningBuiltinContext {
            tool_executor: Some(Arc::new(CleanupFailure)),
            reasoning_journal: Some(journal.clone()),
            ..Default::default()
        };
        let error = builtin_tool_call(&[DslValue::String("missing".into())], &ctx)
            .await
            .unwrap_err();
        assert!(error
            .to_string()
            .contains("Required worker cleanup failed: cleanup fixture"));
        let entries = journal.entries().await;
        assert!(matches!(&entries.last().unwrap().event,
            LoopEvent::Terminated { reason: TerminationReason::Error { message }, .. } if message.contains("cleanup fixture")));
    }
    #[tokio::test]
    async fn parsed_evaluator_builtins_preserve_project_audit_and_caller() {
        use crate::dsl::{
            ast::*,
            evaluator::{DslEvaluator, ExecutionContext},
            lexer::Lexer,
            parser::Parser,
        };
        let project = tempfile::tempdir().unwrap();
        let inference = provider(false);
        let bridge = Arc::new(
            crate::runtime_bridge::RuntimeBridge::new()
                .with_project_root(project.path())
                .unwrap(),
        );
        bridge.set_inference_provider(inference.clone());
        let evaluator = DslEvaluator::new(bridge);
        let id = uuid::Uuid::new_v4();
        for (source, completed) in [
            (
                r#"function run_fixture() { return reason("synthetic system", "synthetic request") }"#,
                true,
            ),
            (
                r#"function run_fixture() { return tool_call("missing") }"#,
                false,
            ),
        ] {
            let program = Parser::new(Lexer::new(source).tokenize().unwrap())
                .parse()
                .unwrap();
            let mut context = ExecutionContext {
                agent_id: Some(id),
                ..Default::default()
            };
            for declaration in program.declarations {
                if let Declaration::Function(function) = declaration {
                    context.functions.insert(function.name.clone(), function);
                }
            }
            let expression = Expression::FunctionCall(FunctionCall {
                function: "run_fixture".into(),
                arguments: vec![],
                span: program.span,
            });
            let value = evaluator
                .evaluate_expression(&expression, &mut context)
                .await
                .unwrap();
            let entries = verified_result(&value, Some(AgentId(id)));
            assert!(
                std::path::Path::new(value.to_json()["audit"]["path"].as_str().unwrap())
                    .starts_with(project.path())
            );
            match &entries.last().unwrap().event {
                LoopEvent::Terminated {
                    reason: TerminationReason::Completed,
                    ..
                } => assert!(completed),
                LoopEvent::Terminated {
                    reason: TerminationReason::PolicyDenial { .. },
                    ..
                } => assert!(!completed),
                other => panic!("unexpected terminal record: {other:?}"),
            }
        }
        assert_eq!(inference.calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn registered_behaviors_keep_distinct_principals_in_protected_audit() {
        let project = tempfile::tempdir().unwrap();
        let inference = provider(false);
        let bridge = Arc::new(
            crate::RuntimeBridge::new()
                .with_project_root(project.path())
                .unwrap(),
        );
        bridge.set_inference_provider(inference.clone());
        let engine = crate::ReplEngine::new(bridge);
        engine
            .evaluate("agent First {} agent Second {}")
            .await
            .unwrap();
        engine
            .evaluate(
                r#"function helper() { return reason("synthetic system", "synthetic request") }"#,
            )
            .await
            .unwrap();
        engine
            .evaluate("behavior Work { steps { return helper() } }")
            .await
            .unwrap();
        let agents = engine.evaluator().list_agents().await;
        for agent in &agents {
            engine
                .evaluate(&format!(":agent start {}", agent.id))
                .await
                .unwrap();
        }
        let (first, second) = tokio::join!(
            engine
                .evaluator()
                .execute_agent_behavior(agents[0].id, "Work", ""),
            engine
                .evaluator()
                .execute_agent_behavior(agents[1].id, "Work", ""),
        );
        for (value, agent) in [first.unwrap(), second.unwrap()].iter().zip(&agents) {
            let entries = verified_result(value, Some(AgentId(agent.id)));
            assert!(matches!(
                entries.last().unwrap().event,
                LoopEvent::Terminated {
                    reason: TerminationReason::Completed,
                    ..
                }
            ));
        }
        assert_eq!(inference.calls.load(Ordering::SeqCst), 2);
        assert!(engine
            .evaluator()
            .monitor()
            .get_active_executions()
            .is_empty());
    }

    #[tokio::test]
    #[ignore = "requires Docker and cached Python fixture image"]
    async fn registered_repl_behavior_enforces_real_policy_boundary_and_audit() {
        use std::os::unix::fs::PermissionsExt;
        use symbi_runtime::reasoning::{CedarPolicy, CedarPolicyGate};
        let project = tempfile::tempdir().unwrap();
        let output = project.path().join("output");
        std::fs::create_dir(&output).unwrap();
        std::fs::set_permissions(&output, std::fs::Permissions::from_mode(0o777)).unwrap();
        let canary = project.path().join("host-canary");
        std::fs::write(&canary, "synthetic private canary").unwrap();
        let label = format!("symbi.repl-e2e={}", uuid::Uuid::new_v4());
        let config = format!(
            r#"
[sandbox]
tier = "docker"
[sandbox.docker]
image = "sha256:7bf61a5ec2a4631b240bc8cf83404e2dd37fb06a4b4bbd34b2c36906a5fb4aee"
volumes = [{}]
extra_flags = ["--label={label}"]
"#,
            serde_json::to_string(&format!("{}:/workspace:rw", output.display())).unwrap()
        );
        std::fs::write(project.path().join("symbiont.toml"), config).unwrap();
        std::fs::create_dir(project.path().join("tools")).unwrap();
        std::fs::write(project.path().join("tools/write.clad.toml"), format!(r#"
[tool]
name = "write"
version = "1"
description = "Write an isolated test result"
binary = "/bin/sh"
timeout_seconds = 15
[command]
template = "/bin/sh -c 'test ! -e {} && test `id -u` = 65534 && printf registered > /workspace/allowed'"
[output]
format = "text"
"#, canary.display())).unwrap();
        let gate = Arc::new(CedarPolicyGate::deny_by_default());
        let bridge = Arc::new(
            crate::RuntimeBridge::new()
                .with_project_root(project.path())
                .unwrap(),
        );
        bridge.set_reasoning_policy_gate(gate.clone());
        let engine = crate::ReplEngine::new(bridge);
        for requirement in [
            "security { tier: Tier3 }",
            "security { sandbox: strict }",
            "resources { memory: 1MB network: false }",
            "policies { timeout: 1ms }",
        ] {
            let error = engine
                .evaluate(&format!("agent Unsupported {{ {requirement} }}"))
                .await
                .unwrap_err();
            assert!(error.to_string().contains("Unsupported legacy agent"));
            assert!(engine.evaluator().list_agents().await.is_empty());
            assert!(!project.path().join(".symbiont/governed").exists());
            assert!(!output.join("allowed").exists());
        }
        engine
            .evaluate("agent Permitted {} agent Sibling {}")
            .await
            .unwrap();
        engine
            .evaluate(r#"function write_fixture() { return tool_call("write") }"#)
            .await
            .unwrap();
        engine
            .evaluate("behavior Work { steps { return write_fixture() } }")
            .await
            .unwrap();
        let agents = engine.evaluator().list_agents().await;
        let permitted = agents
            .iter()
            .find(|a| a.definition.name == "Permitted")
            .unwrap()
            .id;
        let sibling = agents
            .iter()
            .find(|a| a.definition.name == "Sibling")
            .unwrap()
            .id;
        gate.add_policy(CedarPolicy {
            name: "fixture".into(), active: true,
            source: format!("permit(principal == Agent::\"{permitted}\", action == Action::\"tool_call::write\", resource);"),
        }).await;
        for id in [permitted, sibling] {
            engine
                .evaluate(&format!(":agent start {id}"))
                .await
                .unwrap();
        }
        let denied = engine
            .evaluator()
            .execute_agent_behavior(sibling, "Work", "")
            .await
            .unwrap();
        assert_eq!(denied.to_json()["status"], "denied");
        assert!(!output.join("allowed").exists());
        let denied_entries = verified_result(&denied, Some(AgentId(sibling)));
        assert!(matches!(
            denied_entries.last().unwrap().event,
            LoopEvent::Terminated {
                reason: TerminationReason::PolicyDenial { .. },
                ..
            }
        ));
        let allowed = engine
            .evaluator()
            .execute_agent_behavior(permitted, "Work", "")
            .await
            .unwrap();
        assert_eq!(allowed.to_json()["status"], "success", "{allowed:?}");
        assert_eq!(
            std::fs::read_to_string(output.join("allowed")).unwrap(),
            "registered"
        );
        let entries = verified_result(&allowed, Some(AgentId(permitted)));
        assert!(matches!(
            entries.last().unwrap().event,
            LoopEvent::Terminated {
                reason: TerminationReason::Completed,
                ..
            }
        ));
        std::fs::remove_file(output.join("allowed")).unwrap();
        std::fs::write(
            project.path().join("symbiont.toml"),
            "[sandbox]\ntier = \"firecracker\"\n",
        )
        .unwrap();
        let unavailable = engine
            .evaluator()
            .execute_agent_behavior(permitted, "Work", "")
            .await
            .unwrap();
        assert_ne!(unavailable.to_json()["status"], "success");
        assert!(!output.join("allowed").exists());
        verified_result(&unavailable, Some(AgentId(permitted)));
        let workers = tokio::process::Command::new("docker")
            .args(["ps", "-aq", "--filter", &format!("label={label}")])
            .output()
            .await
            .unwrap();
        assert!(workers.status.success());
        assert!(workers.stdout.is_empty(), "owned workers remain");
    }
}
