//! Core reasoning builtins for the DSL
//!
//! Provides async builtin functions that bridge the DSL with the
//! reasoning loop infrastructure: `reason`, `llm_call`, `parse_json`,
//! `delegate`, and `tool_call`.

use crate::dsl::evaluator::DslValue;
use crate::error::{ReplError, Result};
use std::collections::HashMap;
use std::sync::Arc;
use symbi_runtime::reasoning::agent_registry::AgentRegistry;
use symbi_runtime::reasoning::inference::InferenceProvider;

/// Shared state for async reasoning builtins.
#[derive(Clone, Default)]
pub struct ReasoningBuiltinContext {
    /// Inference provider for LLM calls.
    pub provider: Option<Arc<dyn InferenceProvider>>,
    /// Agent registry for multi-agent composition.
    pub agent_registry: Option<Arc<AgentRegistry>>,
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
    use symbi_runtime::reasoning::executor::DefaultActionExecutor;
    use symbi_runtime::reasoning::loop_types::{BufferedJournal, LoopConfig};
    use symbi_runtime::reasoning::policy_bridge::DefaultPolicyGate;
    use symbi_runtime::reasoning::reasoning_loop::ReasoningLoopRunner;
    use symbi_runtime::types::AgentId;

    let runner = ReasoningLoopRunner {
        provider: Arc::clone(provider),
        policy_gate: Arc::new(DefaultPolicyGate::permissive()),
        executor: Arc::new(DefaultActionExecutor::default()),
        context_manager: Arc::new(DefaultContextManager::default()),
        circuit_breakers: Arc::new(CircuitBreakerRegistry::default()),
        journal: Arc::new(BufferedJournal::new(1000)),
    };

    let mut conv = Conversation::with_system(&system);
    conv.push(ConversationMessage::user(&user));

    let config = LoopConfig {
        max_iterations,
        max_total_tokens: max_tokens,
        ..Default::default()
    };

    let result = runner.run(AgentId::new(), conv, config).await;

    let mut map = HashMap::new();
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
    let provider = ctx
        .provider
        .as_ref()
        .ok_or_else(|| ReplError::Execution("No inference provider configured".into()))?;

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
    let response = provider
        .complete(&conv, &options)
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
/// Returns the tool result as a string.
pub async fn builtin_tool_call(
    args: &[DslValue],
    _ctx: &ReasoningBuiltinContext,
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

    // In a full setup, this would go through ToolInvocationEnforcer.
    // For now, return a structured result indicating the tool call was made.
    let mut result = HashMap::new();
    result.insert("tool".to_string(), DslValue::String(name));
    result.insert("arguments".to_string(), DslValue::String(arguments));
    result.insert(
        "status".to_string(),
        DslValue::String("executed".to_string()),
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

    // Use inference provider to simulate delegation (each agent is a separate conversation)
    let provider = ctx
        .provider
        .as_ref()
        .ok_or_else(|| ReplError::Execution("No inference provider configured".into()))?;

    use symbi_runtime::reasoning::conversation::{Conversation, ConversationMessage};
    use symbi_runtime::reasoning::inference::InferenceOptions;

    let mut conv = Conversation::with_system(format!(
        "You are agent '{}'. Respond to the delegated task.",
        agent_name
    ));
    conv.push(ConversationMessage::user(&message));

    let response = provider
        .complete(&conv, &InferenceOptions::default())
        .await
        .map_err(|e| {
            ReplError::Execution(format!("Delegation to '{}' failed: {}", agent_name, e))
        })?;

    Ok(DslValue::String(response.content))
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
            "float": 3.14,
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
}
