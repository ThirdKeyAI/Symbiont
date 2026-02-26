//! Agent composition builtins for the DSL
//!
//! Provides async builtins for spawning agents, sending messages,
//! and executing concurrent patterns: `spawn_agent`, `ask`, `send_to`,
//! `parallel`, and `race`.

use crate::dsl::evaluator::DslValue;
use crate::dsl::reasoning_builtins::ReasoningBuiltinContext;
use crate::error::{ReplError, Result};
use std::collections::HashMap;

/// Execute the `spawn_agent` builtin: register a new named agent.
///
/// Arguments (named via map or positional):
/// - name: string — agent name
/// - system: string — system prompt
/// - tools: list of strings (optional)
/// - response_format: string (optional)
///
/// Returns a map with `agent_id` and `name`.
pub async fn builtin_spawn_agent(
    args: &[DslValue],
    ctx: &ReasoningBuiltinContext,
) -> Result<DslValue> {
    let registry = ctx
        .agent_registry
        .as_ref()
        .ok_or_else(|| ReplError::Execution("No agent registry configured".into()))?;

    let (name, system_prompt, tools, response_format) = parse_spawn_args(args)?;

    let agent_id = registry
        .spawn_agent(&name, &system_prompt, tools, response_format)
        .await;

    let mut result = HashMap::new();
    result.insert(
        "agent_id".to_string(),
        DslValue::String(agent_id.to_string()),
    );
    result.insert("name".to_string(), DslValue::String(name));
    Ok(DslValue::Map(result))
}

/// Execute the `ask` builtin: send a message to a named agent and wait for response.
///
/// Arguments:
/// - agent: string — agent name
/// - message: string
///
/// Returns the agent's response as a string.
pub async fn builtin_ask(args: &[DslValue], ctx: &ReasoningBuiltinContext) -> Result<DslValue> {
    let registry = ctx
        .agent_registry
        .as_ref()
        .ok_or_else(|| ReplError::Execution("No agent registry configured".into()))?;

    let provider = ctx
        .provider
        .as_ref()
        .ok_or_else(|| ReplError::Execution("No inference provider configured".into()))?;

    let (agent_name, message) = parse_ask_args(args)?;

    let response = registry
        .ask_agent(&agent_name, &message, provider.as_ref())
        .await
        .map_err(|e| ReplError::Execution(format!("ask({}) failed: {}", agent_name, e)))?;

    Ok(DslValue::String(response))
}

/// Execute the `send_to` builtin: fire-and-forget message to a named agent.
///
/// Arguments:
/// - agent: string — agent name
/// - message: string
///
/// Returns null (fire-and-forget).
pub async fn builtin_send_to(args: &[DslValue], ctx: &ReasoningBuiltinContext) -> Result<DslValue> {
    let registry = ctx
        .agent_registry
        .as_ref()
        .ok_or_else(|| ReplError::Execution("No agent registry configured".into()))?;

    let provider = ctx
        .provider
        .as_ref()
        .ok_or_else(|| ReplError::Execution("No inference provider configured".into()))?;

    let (agent_name, message) = parse_ask_args(args)?;

    if !registry.has_agent(&agent_name).await {
        return Err(ReplError::Execution(format!(
            "Agent '{}' not found",
            agent_name
        )));
    }

    // Fire-and-forget: spawn a background task
    let registry = registry.clone();
    let provider = provider.clone();
    tokio::spawn(async move {
        let _ = registry
            .ask_agent(&agent_name, &message, provider.as_ref())
            .await;
    });

    Ok(DslValue::Null)
}

/// Execute the `parallel` builtin: run multiple agent calls concurrently.
///
/// Arguments:
/// - tasks: list of maps, each with `{agent: string, message: string}`
///
/// Returns a list of results (strings or error maps).
pub async fn builtin_parallel(
    args: &[DslValue],
    ctx: &ReasoningBuiltinContext,
) -> Result<DslValue> {
    let registry = ctx
        .agent_registry
        .as_ref()
        .ok_or_else(|| ReplError::Execution("No agent registry configured".into()))?;

    let provider = ctx
        .provider
        .as_ref()
        .ok_or_else(|| ReplError::Execution("No inference provider configured".into()))?;

    let tasks = parse_parallel_args(args)?;

    let mut handles = Vec::new();
    for (agent_name, message) in tasks {
        let registry = registry.clone();
        let provider = provider.clone();
        handles.push(tokio::spawn(async move {
            registry
                .ask_agent(&agent_name, &message, provider.as_ref())
                .await
                .map_err(|e| format!("{}", e))
        }));
    }

    let mut results = Vec::new();
    for handle in handles {
        match handle.await {
            Ok(Ok(response)) => results.push(DslValue::String(response)),
            Ok(Err(e)) => {
                let mut error_map = HashMap::new();
                error_map.insert("error".to_string(), DslValue::String(e));
                results.push(DslValue::Map(error_map));
            }
            Err(e) => {
                let mut error_map = HashMap::new();
                error_map.insert("error".to_string(), DslValue::String(e.to_string()));
                results.push(DslValue::Map(error_map));
            }
        }
    }

    Ok(DslValue::List(results))
}

/// Execute the `race` builtin: run multiple agent calls, return first to complete.
///
/// Arguments:
/// - tasks: list of maps, each with `{agent: string, message: string}`
///
/// Returns the first successful result as a string.
pub async fn builtin_race(args: &[DslValue], ctx: &ReasoningBuiltinContext) -> Result<DslValue> {
    let registry = ctx
        .agent_registry
        .as_ref()
        .ok_or_else(|| ReplError::Execution("No agent registry configured".into()))?;

    let provider = ctx
        .provider
        .as_ref()
        .ok_or_else(|| ReplError::Execution("No inference provider configured".into()))?;

    let tasks = parse_parallel_args(args)?;

    if tasks.is_empty() {
        return Err(ReplError::Execution(
            "race requires at least one task".into(),
        ));
    }

    let mut join_set = tokio::task::JoinSet::new();
    for (agent_name, message) in tasks {
        let registry = registry.clone();
        let provider = provider.clone();
        join_set.spawn(async move {
            registry
                .ask_agent(&agent_name, &message, provider.as_ref())
                .await
                .map_err(|e| format!("{}", e))
        });
    }

    // Return the first completed result
    match join_set.join_next().await {
        Some(Ok(Ok(response))) => {
            join_set.abort_all();
            Ok(DslValue::String(response))
        }
        Some(Ok(Err(e))) => {
            join_set.abort_all();
            Err(ReplError::Execution(format!(
                "race: first completed with error: {}",
                e
            )))
        }
        Some(Err(e)) => {
            join_set.abort_all();
            Err(ReplError::Execution(format!("race: task panic: {}", e)))
        }
        None => Err(ReplError::Execution("race: no tasks to run".into())),
    }
}

// --- Argument parsing helpers ---

fn parse_spawn_args(args: &[DslValue]) -> Result<(String, String, Vec<String>, Option<String>)> {
    match args {
        [DslValue::Map(map)] => {
            let name = extract_string(map, "name")?;
            let system = extract_string(map, "system")?;
            let tools = map
                .get("tools")
                .and_then(|v| match v {
                    DslValue::List(items) => Some(
                        items
                            .iter()
                            .filter_map(|i| match i {
                                DslValue::String(s) => Some(s.clone()),
                                _ => None,
                            })
                            .collect(),
                    ),
                    _ => None,
                })
                .unwrap_or_default();
            let response_format = map.get("response_format").and_then(|v| match v {
                DslValue::String(s) => Some(s.clone()),
                _ => None,
            });
            Ok((name, system, tools, response_format))
        }
        [DslValue::String(name), DslValue::String(system)] => {
            Ok((name.clone(), system.clone(), Vec::new(), None))
        }
        [DslValue::String(name), DslValue::String(system), DslValue::List(tools)] => {
            let tool_names = tools
                .iter()
                .filter_map(|t| match t {
                    DslValue::String(s) => Some(s.clone()),
                    _ => None,
                })
                .collect();
            Ok((name.clone(), system.clone(), tool_names, None))
        }
        _ => Err(ReplError::Execution(
            "spawn_agent requires (name: string, system: string, [tools?, response_format?])"
                .into(),
        )),
    }
}

fn parse_ask_args(args: &[DslValue]) -> Result<(String, String)> {
    match args {
        [DslValue::String(agent), DslValue::String(message)] => {
            Ok((agent.clone(), message.clone()))
        }
        [DslValue::Map(map)] => {
            let agent = extract_string(map, "agent")?;
            let message = extract_string(map, "message")?;
            Ok((agent, message))
        }
        _ => Err(ReplError::Execution(
            "requires (agent: string, message: string)".into(),
        )),
    }
}

fn parse_parallel_args(args: &[DslValue]) -> Result<Vec<(String, String)>> {
    match args {
        [DslValue::List(items)] => {
            let mut tasks = Vec::new();
            for item in items {
                match item {
                    DslValue::Map(map) => {
                        let agent = extract_string(map, "agent")?;
                        let message = extract_string(map, "message")?;
                        tasks.push((agent, message));
                    }
                    _ => {
                        return Err(ReplError::Execution(
                            "parallel/race items must be maps with {agent, message}".into(),
                        ))
                    }
                }
            }
            Ok(tasks)
        }
        _ => Err(ReplError::Execution(
            "parallel/race requires a list of {agent, message} maps".into(),
        )),
    }
}

fn extract_string(map: &HashMap<String, DslValue>, key: &str) -> Result<String> {
    map.get(key)
        .and_then(|v| match v {
            DslValue::String(s) => Some(s.clone()),
            _ => None,
        })
        .ok_or_else(|| ReplError::Execution(format!("Missing required string argument '{}'", key)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_spawn_args_named() {
        let mut map = HashMap::new();
        map.insert("name".into(), DslValue::String("researcher".into()));
        map.insert("system".into(), DslValue::String("You research.".into()));
        map.insert(
            "tools".into(),
            DslValue::List(vec![DslValue::String("search".into())]),
        );

        let (name, system, tools, format) = parse_spawn_args(&[DslValue::Map(map)]).unwrap();
        assert_eq!(name, "researcher");
        assert_eq!(system, "You research.");
        assert_eq!(tools, vec!["search"]);
        assert!(format.is_none());
    }

    #[test]
    fn test_parse_spawn_args_positional() {
        let args = vec![
            DslValue::String("coder".into()),
            DslValue::String("You code.".into()),
        ];
        let (name, system, tools, format) = parse_spawn_args(&args).unwrap();
        assert_eq!(name, "coder");
        assert_eq!(system, "You code.");
        assert!(tools.is_empty());
        assert!(format.is_none());
    }

    #[test]
    fn test_parse_spawn_args_with_tools() {
        let args = vec![
            DslValue::String("worker".into()),
            DslValue::String("You work.".into()),
            DslValue::List(vec![
                DslValue::String("read".into()),
                DslValue::String("write".into()),
            ]),
        ];
        let (name, system, tools, _) = parse_spawn_args(&args).unwrap();
        assert_eq!(name, "worker");
        assert_eq!(system, "You work.");
        assert_eq!(tools, vec!["read", "write"]);
    }

    #[test]
    fn test_parse_spawn_args_with_response_format() {
        let mut map = HashMap::new();
        map.insert("name".into(), DslValue::String("parser".into()));
        map.insert("system".into(), DslValue::String("Parse data.".into()));
        map.insert("response_format".into(), DslValue::String("json".into()));

        let (_, _, _, format) = parse_spawn_args(&[DslValue::Map(map)]).unwrap();
        assert_eq!(format, Some("json".into()));
    }

    #[test]
    fn test_parse_ask_args_positional() {
        let args = vec![
            DslValue::String("agent1".into()),
            DslValue::String("hello".into()),
        ];
        let (agent, msg) = parse_ask_args(&args).unwrap();
        assert_eq!(agent, "agent1");
        assert_eq!(msg, "hello");
    }

    #[test]
    fn test_parse_ask_args_named() {
        let mut map = HashMap::new();
        map.insert("agent".into(), DslValue::String("bot".into()));
        map.insert("message".into(), DslValue::String("hi".into()));
        let (agent, msg) = parse_ask_args(&[DslValue::Map(map)]).unwrap();
        assert_eq!(agent, "bot");
        assert_eq!(msg, "hi");
    }

    #[test]
    fn test_parse_parallel_args() {
        let mut task1 = HashMap::new();
        task1.insert("agent".into(), DslValue::String("a".into()));
        task1.insert("message".into(), DslValue::String("m1".into()));

        let mut task2 = HashMap::new();
        task2.insert("agent".into(), DslValue::String("b".into()));
        task2.insert("message".into(), DslValue::String("m2".into()));

        let args = vec![DslValue::List(vec![
            DslValue::Map(task1),
            DslValue::Map(task2),
        ])];
        let tasks = parse_parallel_args(&args).unwrap();
        assert_eq!(tasks.len(), 2);
        assert_eq!(tasks[0], ("a".into(), "m1".into()));
        assert_eq!(tasks[1], ("b".into(), "m2".into()));
    }

    #[test]
    fn test_parse_spawn_args_missing_name() {
        let map = HashMap::new();
        assert!(parse_spawn_args(&[DslValue::Map(map)]).is_err());
    }

    #[test]
    fn test_parse_ask_args_invalid() {
        assert!(parse_ask_args(&[DslValue::Integer(42)]).is_err());
    }

    #[test]
    fn test_parse_parallel_args_empty_list() {
        let args = vec![DslValue::List(vec![])];
        let tasks = parse_parallel_args(&args).unwrap();
        assert!(tasks.is_empty());
    }

    #[test]
    fn test_parse_parallel_args_invalid_item() {
        let args = vec![DslValue::List(vec![DslValue::String("not a map".into())])];
        assert!(parse_parallel_args(&args).is_err());
    }
}
