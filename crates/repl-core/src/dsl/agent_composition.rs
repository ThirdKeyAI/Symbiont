//! Agent composition builtins for the DSL
//!
//! Provides async builtins for spawning agents, sending messages,
//! and executing concurrent patterns: `spawn_agent`, `ask`, `send_to`,
//! `parallel`, and `race`.

use crate::dsl::evaluator::DslValue;
use crate::dsl::reasoning_builtins::{optional_protocol_label, ReasoningBuiltinContext};
use crate::error::{ReplError, Result};
use std::collections::HashMap;
use std::time::Duration;
use symbi_runtime::communication::policy_gate::CommunicationRequest;
use symbi_runtime::types::{AgentId, MessageType, RequestId};

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

/// Governed single-turn delegation: resolve the target agent, run the
/// communication policy gate (and session conformance when a session is open),
/// log both messages, and return the agent's reply text. This is the typed core
/// shared by the `ask` DSL builtin and the shell orchestrator's `delegate` tool.
pub async fn governed_ask(
    ctx: &ReasoningBuiltinContext,
    target: &str,
    message: &str,
    explicit_label: Option<&str>,
) -> Result<String> {
    let registry = ctx
        .agent_registry
        .as_ref()
        .ok_or_else(|| ReplError::Execution("No agent registry configured".into()))?;

    // Resolve the target before requiring a provider: discovering that the
    // target does not exist does not need an inference provider, and surfacing
    // that error first is strictly more useful to callers.
    let recipient_id = resolve_agent_id(target, ctx).await?;

    let provider = ctx
        .provider
        .as_ref()
        .ok_or_else(|| ReplError::Execution("No inference provider configured".into()))?;

    let sender_id = ctx.sender_agent_id.unwrap_or_default();
    let request_id = RequestId::new();

    check_comm_policy(
        ctx,
        sender_id,
        recipient_id,
        MessageType::Request(request_id),
        explicit_label,
    )?;
    log_comm_message(
        ctx,
        sender_id,
        recipient_id,
        message,
        MessageType::Request(request_id),
        Duration::from_secs(30),
    )
    .await;

    let response = registry
        .ask_agent(target, message, provider.as_ref())
        .await
        .map_err(|e| ReplError::Execution(format!("ask({}) failed: {}", target, e)))?;

    log_comm_message(
        ctx,
        recipient_id,
        sender_id,
        &response,
        MessageType::Response(request_id),
        Duration::from_secs(30),
    )
    .await;

    Ok(response)
}

/// Governed multi-turn delegation: resolve `target`, run the comm-policy gate,
/// then complete `conversation` (which already contains the agent's system
/// message + prior turns + the new user message) against the provider. Returns
/// the reply text. Shares target resolution + the gate with `governed_ask`.
pub async fn governed_ask_conversation(
    ctx: &ReasoningBuiltinContext,
    target: &str,
    conversation: &symbi_runtime::reasoning::conversation::Conversation,
) -> Result<String> {
    // Resolve the target before requiring a provider (same ordering as
    // `governed_ask`): an unknown-agent error is strictly more useful than a
    // "no provider" error, and does not need an inference provider to surface.
    let recipient_id = resolve_agent_id(target, ctx).await?;

    let provider = ctx
        .provider
        .as_ref()
        .ok_or_else(|| ReplError::Execution("No inference provider configured".into()))?;

    let sender_id = ctx.sender_agent_id.unwrap_or_default();
    let request_id = RequestId::new();
    check_comm_policy(
        ctx,
        sender_id,
        recipient_id,
        MessageType::Request(request_id),
        None,
    )?;
    let options = symbi_runtime::reasoning::inference::InferenceOptions::default();
    let response = provider
        .complete(conversation, &options)
        .await
        .map_err(|e| ReplError::Execution(format!("ask({}) failed: {}", target, e)))?;
    Ok(response.content)
}

/// Execute the `ask` builtin: send a message to a named agent and wait for response.
///
/// Arguments:
/// - agent: string — agent name
/// - message: string
///
/// Returns the agent's response as a string.
pub async fn builtin_ask(args: &[DslValue], ctx: &ReasoningBuiltinContext) -> Result<DslValue> {
    let (agent_name, message) = parse_ask_args(args)?;
    let plabel = optional_protocol_label(args);
    let response = governed_ask(ctx, &agent_name, &message, plabel.as_deref()).await?;
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

    // Communication bus wiring: policy check + message logging
    let recipient_id = resolve_agent_id(&agent_name, ctx).await?;
    let sender_id = ctx.sender_agent_id.unwrap_or_default();

    check_comm_policy(
        ctx,
        sender_id,
        recipient_id,
        MessageType::Direct(recipient_id),
        None,
    )?;
    log_comm_message(
        ctx,
        sender_id,
        recipient_id,
        &message,
        MessageType::Direct(recipient_id),
        Duration::from_secs(30),
    )
    .await;

    // Fire-and-forget: spawn a background task. Errors are logged so an
    // auditor can trace failed deliveries without the DSL caller needing
    // to await completion.
    let registry = registry.clone();
    let provider = provider.clone();
    tokio::spawn(async move {
        match registry
            .ask_agent(&agent_name, &message, provider.as_ref())
            .await
        {
            Ok(_) => {
                tracing::debug!(
                    agent = %agent_name,
                    sender = %sender_id,
                    "send_to: background ask_agent succeeded",
                );
            }
            Err(e) => {
                tracing::warn!(
                    agent = %agent_name,
                    sender = %sender_id,
                    error = %e,
                    "send_to: background ask_agent failed",
                );
            }
        }
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

    // Pre-spawn policy checks: all must pass before any task is spawned
    let sender_id = ctx.sender_agent_id.unwrap_or_default();
    let mut checked_tasks = Vec::new();
    for (agent_name, message) in &tasks {
        let recipient_id = resolve_agent_id(agent_name, ctx).await?;
        let request_id = RequestId::new();
        check_comm_policy(
            ctx,
            sender_id,
            recipient_id,
            MessageType::Request(request_id),
            None,
        )?;
        checked_tasks.push((
            agent_name.clone(),
            message.clone(),
            recipient_id,
            request_id,
        ));
    }

    // All checks passed — log outbound messages and spawn tasks
    let comm_bus = ctx.comm_bus.clone();
    let mut handles = Vec::new();
    for (agent_name, message, recipient_id, request_id) in checked_tasks {
        log_comm_message(
            ctx,
            sender_id,
            recipient_id,
            &message,
            MessageType::Request(request_id),
            Duration::from_secs(30),
        )
        .await;

        let registry = registry.clone();
        let provider = provider.clone();
        let bus = comm_bus.clone();
        handles.push(tokio::spawn(async move {
            let result = registry
                .ask_agent(&agent_name, &message, provider.as_ref())
                .await
                .map_err(|e| format!("{}", e));

            // Log response via cloned bus
            if let Ok(ref response) = result {
                if let Some(ref bus) = bus {
                    let msg = bus.create_internal_message(
                        recipient_id,
                        sender_id,
                        bytes::Bytes::from(response.clone()),
                        MessageType::Response(request_id),
                        Duration::from_secs(30),
                    );
                    if let Err(e) = bus.send_message(msg).await {
                        tracing::warn!("Failed to log inter-agent response: {}", e);
                    }
                }
            }

            result
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

    // Pre-spawn policy checks: all must pass before any task is spawned
    let sender_id = ctx.sender_agent_id.unwrap_or_default();
    let mut checked_tasks = Vec::new();
    for (agent_name, message) in &tasks {
        let recipient_id = resolve_agent_id(agent_name, ctx).await?;
        let request_id = RequestId::new();
        check_comm_policy(
            ctx,
            sender_id,
            recipient_id,
            MessageType::Request(request_id),
            None,
        )?;
        checked_tasks.push((
            agent_name.clone(),
            message.clone(),
            recipient_id,
            request_id,
        ));
    }

    // All checks passed — log outbound messages and spawn tasks
    let comm_bus = ctx.comm_bus.clone();
    let mut join_set = tokio::task::JoinSet::new();
    for (agent_name, message, recipient_id, request_id) in checked_tasks {
        log_comm_message(
            ctx,
            sender_id,
            recipient_id,
            &message,
            MessageType::Request(request_id),
            Duration::from_secs(30),
        )
        .await;

        let registry = registry.clone();
        let provider = provider.clone();
        let bus = comm_bus.clone();
        join_set.spawn(async move {
            let result = registry
                .ask_agent(&agent_name, &message, provider.as_ref())
                .await
                .map_err(|e| format!("{}", e));

            // Log response via cloned bus
            if let Ok(ref response) = result {
                if let Some(ref bus) = bus {
                    let msg = bus.create_internal_message(
                        recipient_id,
                        sender_id,
                        bytes::Bytes::from(response.clone()),
                        MessageType::Response(request_id),
                        Duration::from_secs(30),
                    );
                    if let Err(e) = bus.send_message(msg).await {
                        tracing::warn!("Failed to log inter-agent response: {}", e);
                    }
                }
            }

            result
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

// --- Communication helpers ---

/// Resolve an agent name to its AgentId via the registry.
pub(crate) async fn resolve_agent_id(name: &str, ctx: &ReasoningBuiltinContext) -> Result<AgentId> {
    let registry = ctx
        .agent_registry
        .as_ref()
        .ok_or_else(|| ReplError::Execution("No agent registry configured".into()))?;

    registry
        .get_agent(name)
        .await
        .map(|agent| agent.agent_id)
        .ok_or_else(|| ReplError::Execution(format!("Unknown agent: {}", name)))
}

/// Check communication policy. Returns Ok(()) if allowed or if no policy gate is configured.
///
/// When a session is active in `ctx`, the protocol label is auto-derived from
/// the monitor using `legal_labels_to`. If the label is unambiguous (exactly
/// one legal option), it is used automatically and the session FSMs are stepped
/// by the gate. Pass `explicit_label` to resolve ambiguity when multiple labels
/// are legal for the same sender→recipient pair.
///
/// v1a note: only `ask` and `delegate` thread an `explicit_label` (via the
/// optional `protocol_label` named arg). The fire-and-forget / fan-out
/// primitives (`send_to`, `parallel`, `race`) pass `None`, so they rely on
/// unambiguous auto-derivation; an ambiguous edge reached through them will
/// error. Wiring the escape hatch for those primitives is a v1b refinement.
pub(crate) fn check_comm_policy(
    ctx: &ReasoningBuiltinContext,
    sender: AgentId,
    recipient: AgentId,
    message_type: MessageType,
    explicit_label: Option<&str>,
) -> Result<()> {
    #[cfg(feature = "session")]
    let (session_id, protocol_label) = match (
        ctx.active_session.lock().unwrap().clone(),
        ctx.session_monitor.as_ref(),
    ) {
        (Some(sid), Some(mon)) => {
            let labels = mon
                .legal_labels_to(&sid, &sender.to_string(), &recipient.to_string())
                .map_err(|e| ReplError::Execution(format!("session: {e}")))?;
            let label = match labels.len() {
                1 => labels.into_iter().next().unwrap(),
                0 => {
                    let opts = mon
                        .legal_next(&sid, &sender.to_string())
                        .map(|evs| {
                            evs.iter()
                                .map(|e| e.to_string())
                                .collect::<Vec<_>>()
                                .join(", ")
                        })
                        .unwrap_or_default();
                    return Err(ReplError::Execution(format!(
                        "session: no legal message to this recipient now; legal next: {opts}"
                    )));
                }
                _ => match explicit_label {
                    Some(l) if labels.iter().any(|x| x == l) => l.to_string(),
                    _ => {
                        return Err(ReplError::Execution(format!(
                            "session: ambiguous label to this recipient; specify protocol_label \
                             as one of: {}",
                            labels.join(", ")
                        )));
                    }
                },
            };
            (Some(sid.to_string()), Some(label))
        }
        _ => (None, None),
    };
    #[cfg(not(feature = "session"))]
    let (session_id, protocol_label): (Option<String>, Option<String>) = {
        let _ = explicit_label; // unused without the session feature
        (None, None)
    };

    if let Some(policy) = &ctx.comm_policy {
        let request = CommunicationRequest {
            sender,
            recipient,
            message_type,
            topic: None,
            session_id,
            protocol_label,
        };
        policy
            .evaluate(&request)
            .map_err(|e| ReplError::Execution(format!("Inter-agent communication denied: {}", e)))
    } else {
        Ok(())
    }
}

/// Log an outbound message via the CommunicationBus. Best-effort (errors logged, not propagated).
pub(crate) async fn log_comm_message(
    ctx: &ReasoningBuiltinContext,
    sender: AgentId,
    recipient: AgentId,
    payload: &str,
    message_type: MessageType,
    ttl: Duration,
) {
    if let Some(bus) = &ctx.comm_bus {
        let msg = bus.create_internal_message(
            sender,
            recipient,
            bytes::Bytes::from(payload.to_string()),
            message_type,
            ttl,
        );
        if let Err(e) = bus.send_message(msg).await {
            tracing::warn!("Failed to log inter-agent message: {}", e);
        }
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

/// Maximum number of tasks accepted by `parallel()` / `race()`.
///
/// Each task spawns a tokio task and issues a policy-gated inter-agent
/// message, so an unbounded list is both a cheap local DoS (fork-bomb of
/// tasks) and an amplification vector into the inference provider. The
/// limit can be widened via `SYMBIONT_MAX_PARALLEL_TASKS` for operators
/// that genuinely need it.
const DEFAULT_MAX_PARALLEL_TASKS: usize = 32;

fn max_parallel_tasks() -> usize {
    std::env::var("SYMBIONT_MAX_PARALLEL_TASKS")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|n| *n > 0)
        .unwrap_or(DEFAULT_MAX_PARALLEL_TASKS)
}

fn parse_parallel_args(args: &[DslValue]) -> Result<Vec<(String, String)>> {
    let cap = max_parallel_tasks();
    match args {
        [DslValue::List(items)] => {
            if items.len() > cap {
                return Err(ReplError::Execution(format!(
                    "parallel/race: too many tasks ({} > {}); raise SYMBIONT_MAX_PARALLEL_TASKS \
                     if intentional",
                    items.len(),
                    cap
                )));
            }
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

    /// Serialise env-var-dependent tests behind a single process-wide lock
    /// so parallel cargo-test execution doesn't race on the global env.
    fn env_test_lock() -> std::sync::MutexGuard<'static, ()> {
        use std::sync::{Mutex, OnceLock};
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    }

    #[test]
    fn test_parse_parallel_args_rejects_oversize_list() {
        let _g = env_test_lock();
        // Ensure env override doesn't leak in from another test run.
        std::env::remove_var("SYMBIONT_MAX_PARALLEL_TASKS");
        let mut items = Vec::new();
        for i in 0..(DEFAULT_MAX_PARALLEL_TASKS + 1) {
            let mut map = HashMap::new();
            map.insert("agent".into(), DslValue::String(format!("a{}", i)));
            map.insert("message".into(), DslValue::String("hi".into()));
            items.push(DslValue::Map(map));
        }
        let args = vec![DslValue::List(items)];
        let err = parse_parallel_args(&args).unwrap_err();
        let msg = format!("{}", err);
        assert!(
            msg.contains("too many tasks"),
            "expected fan-out cap error, got: {}",
            msg
        );
    }

    #[test]
    fn test_parse_parallel_args_env_override_allows_larger_list() {
        let _g = env_test_lock();
        std::env::set_var("SYMBIONT_MAX_PARALLEL_TASKS", "64");
        let mut items = Vec::new();
        for i in 0..40 {
            let mut map = HashMap::new();
            map.insert("agent".into(), DslValue::String(format!("a{}", i)));
            map.insert("message".into(), DslValue::String("hi".into()));
            items.push(DslValue::Map(map));
        }
        let args = vec![DslValue::List(items)];
        let res = parse_parallel_args(&args);
        std::env::remove_var("SYMBIONT_MAX_PARALLEL_TASKS");
        assert!(res.is_ok(), "override must widen the cap");
    }

    #[cfg(feature = "session")]
    #[tokio::test]
    async fn check_comm_policy_auto_derives_label_and_enforces_order() {
        use crate::dsl::reasoning_builtins::ReasoningBuiltinContext;
        use std::sync::{Arc, Mutex};
        use symbi_runtime::communication::policy_gate::CommunicationPolicyGate;
        use symbi_runtime::types::AgentId;
        use symbi_runtime::types::MessageType;
        use symbi_session::examples::coordinator_pipeline;
        use symbi_session::monitor::{SessionId, SessionMonitor};

        let (g, _r) = coordinator_pipeline();
        let monitor = Arc::new(SessionMonitor::new());
        let (coord, validator, processor) = (AgentId::new(), AgentId::new(), AgentId::new());
        let sid = SessionId("cp1".into());
        let mut assign = std::collections::HashMap::new();
        assign.insert(coord.to_string(), "Coordinator".to_string());
        assign.insert(validator.to_string(), "Validator".to_string());
        assign.insert(processor.to_string(), "Processor".to_string());
        monitor.establish(sid.clone(), &g, assign).unwrap();

        let gate =
            Arc::new(CommunicationPolicyGate::permissive().with_session_monitor(monitor.clone()));
        let ctx = ReasoningBuiltinContext {
            comm_policy: Some(gate),
            session_monitor: Some(monitor.clone()),
            active_session: Arc::new(Mutex::new(Some(sid.clone()))),
            ..Default::default()
        };

        // Conforming first step (label auto-derived to "task"); explicit_label None.
        check_comm_policy(&ctx, coord, validator, MessageType::Direct(validator), None).unwrap();
        // Out-of-order: no legal send to Processor yet -> denied with guidance.
        let err = check_comm_policy(&ctx, coord, processor, MessageType::Direct(processor), None)
            .unwrap_err();
        let msg = format!("{err}").to_lowercase();
        assert!(
            msg.contains("session") || msg.contains("legal"),
            "got: {msg}"
        );
    }

    #[cfg(feature = "session")]
    #[test]
    fn dsl_path_enforces_pipeline_with_autoderived_labels() {
        use crate::runtime_bridge::RuntimeBridge;
        use std::time::Duration;
        use symbi_runtime::session::RoleBinding;
        use symbi_runtime::types::AgentId;
        use symbi_runtime::types::MessageType;
        use symbi_session::examples::coordinator_pipeline;

        let bridge = RuntimeBridge::new_permissive_for_dev();
        let (g, _r) = coordinator_pipeline();
        let (c, v, p) = (AgentId::new(), AgentId::new(), AgentId::new());
        let rb = RoleBinding::new()
            .bind(c, "Coordinator")
            .bind(v, "Validator")
            .bind(p, "Processor");
        let _sid = bridge
            .open_session(&g, rb, Duration::from_secs(60))
            .unwrap();
        let ctx = bridge.reasoning_context();

        // Fully auto-derived labels — the caller never names a label (explicit_label = None):
        check_comm_policy(&ctx, c, v, MessageType::Direct(v), None).unwrap(); // -> "task"
        check_comm_policy(&ctx, v, c, MessageType::Direct(c), None).unwrap(); // -> "ok"
        check_comm_policy(&ctx, c, p, MessageType::Direct(p), None).unwrap(); // -> "task"
        check_comm_policy(&ctx, p, c, MessageType::Direct(c), None).unwrap(); // -> "done"

        // Fresh session: out-of-order first move denied with guidance.
        let bridge2 = RuntimeBridge::new_permissive_for_dev();
        let (g2, _r2) = coordinator_pipeline();
        let (c2, v2, p2) = (AgentId::new(), AgentId::new(), AgentId::new());
        let rb2 = RoleBinding::new()
            .bind(c2, "Coordinator")
            .bind(v2, "Validator")
            .bind(p2, "Processor");
        bridge2
            .open_session(&g2, rb2, Duration::from_secs(60))
            .unwrap();
        let ctx2 = bridge2.reasoning_context();
        let err = check_comm_policy(&ctx2, c2, p2, MessageType::Direct(p2), None).unwrap_err();
        let msg = format!("{err}").to_lowercase();
        assert!(
            msg.contains("session") || msg.contains("legal"),
            "got: {msg}"
        );
    }
}
