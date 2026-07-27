//! Multi-agent pattern builtins for the DSL
//!
//! Provides convenience builtins for common multi-agent patterns:
//! `chain`, `debate`, `map_reduce`, and `director`.

use crate::dsl::evaluator::DslValue;
use crate::dsl::reasoning_builtins::ReasoningBuiltinContext;
use crate::error::{ReplError, Result};
use std::collections::HashMap;
use std::sync::Arc;
use symbi_runtime::reasoning::conversation::{Conversation, ConversationMessage};
use symbi_runtime::reasoning::inference::InferenceOptions;

// --- Fan-out / iteration caps ---
//
// `director`, `debate`, and `map_reduce` are not host actions — a policy gate
// has nothing to evaluate here, since no tool executes and no side effect
// occurs outside the inference provider. What they *can* do is turn one DSL
// call into an unbounded number of downstream inference calls, driven by
// either model-authored output (`director`'s plan) or an unbounded
// caller-supplied count (`debate`'s `rounds`, `map_reduce`'s `inputs`). That
// is cost amplification and, for model-authored fields, prompt-injection
// amplification (long injected strings re-fed into further prompts
// verbatim). The fix here is a hard cap surfaced as an honest error, not a
// `ReasoningLoopRunner` port.

/// Maximum number of `assignments` a `director` plan may propose. The model
/// authors this list (the JSON plan response), and `builtin_director` issues
/// one inference call per assignment — an unbounded list is unbounded cost
/// chosen entirely by model output, not the DSL caller.
const MAX_DIRECTOR_ASSIGNMENTS: usize = 20;

/// Maximum length (bytes) of a single `assignments[].worker` or `.subtask`
/// string in a `director` plan. Both are model-authored and are later
/// interpolated verbatim into a worker's prompt (`subtask` as the user
/// message, `worker` as a system-prompt lookup key) — an unbounded length is
/// unbounded prompt-injection amplification.
const MAX_DIRECTOR_FIELD_LEN: usize = 4_000;

/// Maximum `rounds` accepted by `debate`. Each round issues two inference
/// calls (writer + critic) plus one final synthesis call at the end, so cost
/// scales linearly with `rounds` — which is caller-supplied with no default
/// upper bound.
const MAX_DEBATE_ROUNDS: u32 = 20;

/// Maximum number of `map_reduce` map-phase inference calls allowed in
/// flight at once. `inputs` has no length limit, and the map phase used to
/// fan every input out via `try_join_all` simultaneously — an unbounded
/// concurrent burst against the inference provider (rate limits, cost
/// spikes, thundering herd). This bounds concurrency only, via chunked
/// joins: every input is still processed and no result is dropped or
/// truncated, it just runs in batches of this size rather than all at once.
/// Unlike the caps above, exceeding it does not error — see the doc comment
/// on `builtin_map_reduce`'s map phase.
const MAX_MAP_REDUCE_CONCURRENCY: usize = 8;

/// Execute the `chain` builtin: sequential execution where each step's
/// output feeds into the next step's input.
///
/// Arguments:
/// - steps: list of maps, each with keys: system, prompt_template (optional)
///   OR
/// - steps: list of strings (prompts, executed sequentially)
///
/// Returns the final step's output as a string.
pub async fn builtin_chain(args: &[DslValue], ctx: &ReasoningBuiltinContext) -> Result<DslValue> {
    let provider = ctx
        .provider
        .as_ref()
        .ok_or_else(|| ReplError::Execution("No inference provider configured".into()))?;

    let steps = match args.first() {
        Some(DslValue::List(steps)) => steps.clone(),
        Some(DslValue::Map(map)) => map
            .get("steps")
            .and_then(|v| match v {
                DslValue::List(l) => Some(l.clone()),
                _ => None,
            })
            .ok_or_else(|| ReplError::Execution("chain requires 'steps' as a list".into()))?,
        _ => {
            return Err(ReplError::Execution(
                "chain requires a list of steps".into(),
            ))
        }
    };

    if steps.is_empty() {
        return Err(ReplError::Execution(
            "chain requires at least one step".into(),
        ));
    }

    let mut current_output = String::new();
    let mut results = Vec::new();

    for (i, step) in steps.iter().enumerate() {
        let (system, user_template) = match step {
            DslValue::String(prompt) => (
                "You are a helpful assistant. Process the input and respond.".to_string(),
                prompt.clone(),
            ),
            DslValue::Map(map) => {
                let system = map
                    .get("system")
                    .and_then(|v| match v {
                        DslValue::String(s) => Some(s.clone()),
                        _ => None,
                    })
                    .unwrap_or_else(|| "You are a helpful assistant.".to_string());
                let template = map
                    .get("prompt")
                    .and_then(|v| match v {
                        DslValue::String(s) => Some(s.clone()),
                        _ => None,
                    })
                    .unwrap_or_else(|| "Process the following input:".to_string());
                (system, template)
            }
            _ => {
                return Err(ReplError::Execution(format!(
                    "chain step {} must be a string or map",
                    i
                )))
            }
        };

        let user_msg = if current_output.is_empty() {
            user_template
        } else {
            format!("{}\n\nPrevious output:\n{}", user_template, current_output)
        };

        let mut conv = Conversation::with_system(&system);
        conv.push(ConversationMessage::user(&user_msg));

        let response = provider
            .complete(&conv, &InferenceOptions::default())
            .await
            .map_err(|e| ReplError::Execution(format!("Chain step {} failed: {}", i, e)))?;

        current_output = response.content.clone();
        results.push(DslValue::String(response.content));
    }

    // Return a map with the final output and all intermediate results
    let mut result_map = HashMap::new();
    result_map.insert("output".to_string(), DslValue::String(current_output));
    result_map.insert("steps".to_string(), DslValue::List(results));

    Ok(DslValue::Map(result_map))
}

/// Execute the `debate` builtin: alternating two-agent critique with
/// optional model routing and adaptive convergence.
///
/// Arguments (named via map):
/// - writer_prompt: string — system prompt for the writer
/// - critic_prompt: string — system prompt for the critic
/// - topic: string — initial topic/content to debate
/// - rounds: integer (optional, default 3)
/// - writer_model: string (optional) — model for writer
/// - critic_model: string (optional) — model for critic
/// - convergence: string (optional) — "fixed" or "adaptive"
///
/// Returns a map with keys: final_answer, rounds_completed, history.
pub async fn builtin_debate(args: &[DslValue], ctx: &ReasoningBuiltinContext) -> Result<DslValue> {
    let provider = ctx
        .provider
        .as_ref()
        .ok_or_else(|| ReplError::Execution("No inference provider configured".into()))?;

    let params = match args.first() {
        Some(DslValue::Map(map)) => map.clone(),
        _ => {
            return Err(ReplError::Execution(
                "debate requires named arguments as a map".into(),
            ))
        }
    };

    let writer_prompt = get_string_param(&params, "writer_prompt")?;
    let critic_prompt = get_string_param(&params, "critic_prompt")?;
    let topic = get_string_param(&params, "topic")?;
    let rounds = params
        .get("rounds")
        .and_then(|v| match v {
            DslValue::Integer(i) => Some(*i as u32),
            DslValue::Number(n) => Some(*n as u32),
            _ => None,
        })
        .unwrap_or(3);

    if rounds > MAX_DEBATE_ROUNDS {
        return Err(ReplError::Execution(format!(
            "debate: rounds ({}) exceeds the cap of {} (MAX_DEBATE_ROUNDS); each round issues \
             two inference calls, so this bounds total cost",
            rounds, MAX_DEBATE_ROUNDS
        )));
    }

    let mut history = Vec::new();
    let mut current_content = topic.clone();

    for round in 0..rounds {
        // Writer phase
        let mut writer_conv = Conversation::with_system(&writer_prompt);
        if round == 0 {
            writer_conv.push(ConversationMessage::user(format!(
                "Topic: {}",
                current_content
            )));
        } else {
            writer_conv.push(ConversationMessage::user(format!(
                "Revise your response based on this critique:\n\n{}\n\nOriginal topic: {}",
                current_content, topic
            )));
        }

        let writer_response = provider
            .complete(&writer_conv, &InferenceOptions::default())
            .await
            .map_err(|e| {
                ReplError::Execution(format!("Debate writer round {} failed: {}", round, e))
            })?;

        let mut round_map = HashMap::new();
        round_map.insert("round".to_string(), DslValue::Integer(round as i64 + 1));
        round_map.insert(
            "writer".to_string(),
            DslValue::String(writer_response.content.clone()),
        );

        // Critic phase
        let mut critic_conv = Conversation::with_system(&critic_prompt);
        critic_conv.push(ConversationMessage::user(format!(
            "Evaluate the following response:\n\n{}",
            writer_response.content
        )));

        let critic_response = provider
            .complete(&critic_conv, &InferenceOptions::default())
            .await
            .map_err(|e| {
                ReplError::Execution(format!("Debate critic round {} failed: {}", round, e))
            })?;

        round_map.insert(
            "critic".to_string(),
            DslValue::String(critic_response.content.clone()),
        );
        history.push(DslValue::Map(round_map));

        current_content = critic_response.content;
    }

    // Final writer response incorporating all critique
    let mut final_conv = Conversation::with_system(&writer_prompt);
    final_conv.push(ConversationMessage::user(format!(
        "Provide your final, refined response incorporating all critiques.\n\nLatest critique: {}\n\nOriginal topic: {}",
        current_content, topic
    )));

    let final_response = provider
        .complete(&final_conv, &InferenceOptions::default())
        .await
        .map_err(|e| ReplError::Execution(format!("Debate final response failed: {}", e)))?;

    let mut result = HashMap::new();
    result.insert(
        "final_answer".to_string(),
        DslValue::String(final_response.content),
    );
    result.insert(
        "rounds_completed".to_string(),
        DslValue::Integer(rounds as i64),
    );
    result.insert("history".to_string(), DslValue::List(history));

    Ok(DslValue::Map(result))
}

/// Execute the `map_reduce` builtin: parallel fan-out + aggregate.
///
/// Arguments (named via map):
/// - inputs: list — items to process
/// - mapper: string — system prompt for the mapper
/// - reducer: string — system prompt for the reducer
///
/// Returns a map with keys: result, mapped_results.
pub async fn builtin_map_reduce(
    args: &[DslValue],
    ctx: &ReasoningBuiltinContext,
) -> Result<DslValue> {
    let provider = ctx
        .provider
        .as_ref()
        .ok_or_else(|| ReplError::Execution("No inference provider configured".into()))?;

    let params = match args.first() {
        Some(DslValue::Map(map)) => map.clone(),
        _ => {
            return Err(ReplError::Execution(
                "map_reduce requires named arguments as a map".into(),
            ))
        }
    };

    let inputs = match params.get("inputs") {
        Some(DslValue::List(items)) => items.clone(),
        _ => {
            return Err(ReplError::Execution(
                "map_reduce requires 'inputs' as a list".into(),
            ))
        }
    };
    let mapper_prompt = get_string_param(&params, "mapper")?;
    let reducer_prompt = get_string_param(&params, "reducer")?;

    // Map phase: process inputs concurrently, but in chunks of at most
    // `MAX_MAP_REDUCE_CONCURRENCY` so a large `inputs` list cannot fan out
    // an unbounded burst of simultaneous inference calls. Every input is
    // still processed — later chunks simply wait for earlier ones — so
    // nothing here is dropped or truncated, unlike the count/length caps
    // above which reject outright.
    let mut mapped_results: Vec<String> = Vec::with_capacity(inputs.len());
    for chunk in inputs.chunks(MAX_MAP_REDUCE_CONCURRENCY) {
        let mut map_futures = Vec::with_capacity(chunk.len());
        for input in chunk {
            let input_str = match input {
                DslValue::String(s) => s.clone(),
                other => format!("{:?}", other),
            };
            let provider = Arc::clone(provider);
            let mapper_prompt = mapper_prompt.clone();

            map_futures.push(async move {
                let mut conv = Conversation::with_system(&mapper_prompt);
                conv.push(ConversationMessage::user(&input_str));
                provider
                    .complete(&conv, &InferenceOptions::default())
                    .await
                    .map(|r| r.content)
                    .map_err(|e| ReplError::Execution(format!("Map failed: {}", e)))
            });
        }

        let chunk_results: Vec<String> = futures::future::try_join_all(map_futures).await?;
        mapped_results.extend(chunk_results);
    }

    // Reduce phase: aggregate all mapped results
    let combined = mapped_results
        .iter()
        .enumerate()
        .map(|(i, r)| format!("Result {}: {}", i + 1, r))
        .collect::<Vec<_>>()
        .join("\n\n");

    let mut reduce_conv = Conversation::with_system(&reducer_prompt);
    reduce_conv.push(ConversationMessage::user(format!(
        "Aggregate the following results:\n\n{}",
        combined
    )));

    let reduce_response = provider
        .complete(&reduce_conv, &InferenceOptions::default())
        .await
        .map_err(|e| ReplError::Execution(format!("Reduce failed: {}", e)))?;

    let mut result = HashMap::new();
    result.insert(
        "result".to_string(),
        DslValue::String(reduce_response.content),
    );
    result.insert(
        "mapped_results".to_string(),
        DslValue::List(mapped_results.into_iter().map(DslValue::String).collect()),
    );

    Ok(DslValue::Map(result))
}

/// Execute the `director` builtin: decompose + delegate + synthesize.
///
/// Arguments (named via map):
/// - orchestrator_prompt: string — system prompt for the director
/// - workers: list of maps with {name, system} — worker agent definitions
/// - task: string — the task to accomplish
///
/// Returns a map with keys: result, plan, worker_results.
pub async fn builtin_director(
    args: &[DslValue],
    ctx: &ReasoningBuiltinContext,
) -> Result<DslValue> {
    let provider = ctx
        .provider
        .as_ref()
        .ok_or_else(|| ReplError::Execution("No inference provider configured".into()))?;

    let params = match args.first() {
        Some(DslValue::Map(map)) => map.clone(),
        _ => {
            return Err(ReplError::Execution(
                "director requires named arguments as a map".into(),
            ))
        }
    };

    let orchestrator_prompt = get_string_param(&params, "orchestrator_prompt")?;
    let task = get_string_param(&params, "task")?;

    let workers = match params.get("workers") {
        Some(DslValue::List(items)) => items.clone(),
        _ => {
            return Err(ReplError::Execution(
                "director requires 'workers' as a list".into(),
            ))
        }
    };

    // Parse worker definitions
    let worker_defs: Vec<(String, String)> = workers
        .iter()
        .map(|w| match w {
            DslValue::Map(map) => {
                let name = map
                    .get("name")
                    .and_then(|v| match v {
                        DslValue::String(s) => Some(s.clone()),
                        _ => None,
                    })
                    .unwrap_or_else(|| "worker".to_string());
                let system = map
                    .get("system")
                    .and_then(|v| match v {
                        DslValue::String(s) => Some(s.clone()),
                        _ => None,
                    })
                    .unwrap_or_else(|| "You are a helpful assistant.".to_string());
                Ok((name, system))
            }
            _ => Err(ReplError::Execution(
                "Each worker must be a map with 'name' and 'system'".into(),
            )),
        })
        .collect::<Result<Vec<_>>>()?;

    // Step 1: Director creates a plan
    let worker_names: Vec<String> = worker_defs.iter().map(|(n, _)| n.clone()).collect();
    let mut plan_conv = Conversation::with_system(&orchestrator_prompt);
    plan_conv.push(ConversationMessage::user(format!(
        "Task: {}\n\nAvailable workers: {}\n\nCreate a plan assigning subtasks to each worker. Respond with a JSON object like: {{\"assignments\": [{{\"worker\": \"name\", \"subtask\": \"description\"}}]}}",
        task,
        worker_names.join(", ")
    )));

    let plan_options = InferenceOptions {
        response_format: symbi_runtime::reasoning::inference::ResponseFormat::JsonObject,
        ..Default::default()
    };

    let plan_response = provider
        .complete(&plan_conv, &plan_options)
        .await
        .map_err(|e| ReplError::Execution(format!("Director planning failed: {}", e)))?;

    let plan_text = plan_response.content.clone();

    // Parse assignments. `parse_assignments` enforces
    // `MAX_DIRECTOR_ASSIGNMENTS` / `MAX_DIRECTOR_FIELD_LEN` and errors rather
    // than truncating, so a plan that exceeds either cap is rejected before
    // any worker inference below is issued.
    let assignments = parse_assignments(&plan_text, &worker_defs)?;

    // Step 2: Execute worker subtasks
    let mut worker_results = Vec::new();
    for (worker_name, worker_system, subtask) in &assignments {
        let mut worker_conv = Conversation::with_system(worker_system);
        worker_conv.push(ConversationMessage::user(subtask));

        let response = provider
            .complete(&worker_conv, &InferenceOptions::default())
            .await
            .map_err(|e| ReplError::Execution(format!("Worker '{}' failed: {}", worker_name, e)))?;

        let mut r = HashMap::new();
        r.insert("worker".to_string(), DslValue::String(worker_name.clone()));
        r.insert("subtask".to_string(), DslValue::String(subtask.clone()));
        r.insert("result".to_string(), DslValue::String(response.content));
        worker_results.push(DslValue::Map(r));
    }

    // Step 3: Director synthesizes results
    let results_summary = worker_results
        .iter()
        .map(|r| match r {
            DslValue::Map(m) => {
                let worker = m
                    .get("worker")
                    .and_then(|v| match v {
                        DslValue::String(s) => Some(s.as_str()),
                        _ => None,
                    })
                    .unwrap_or("unknown");
                let result = m
                    .get("result")
                    .and_then(|v| match v {
                        DslValue::String(s) => Some(s.as_str()),
                        _ => None,
                    })
                    .unwrap_or("");
                format!("Worker '{}': {}", worker, result)
            }
            _ => String::new(),
        })
        .collect::<Vec<_>>()
        .join("\n\n");

    let mut synth_conv = Conversation::with_system(&orchestrator_prompt);
    synth_conv.push(ConversationMessage::user(format!(
        "Synthesize the following worker results into a final answer:\n\n{}\n\nOriginal task: {}",
        results_summary, task
    )));

    let synth_response = provider
        .complete(&synth_conv, &InferenceOptions::default())
        .await
        .map_err(|e| ReplError::Execution(format!("Director synthesis failed: {}", e)))?;

    let mut result = HashMap::new();
    result.insert(
        "result".to_string(),
        DslValue::String(synth_response.content),
    );
    result.insert("plan".to_string(), DslValue::String(plan_text));
    result.insert("worker_results".to_string(), DslValue::List(worker_results));

    Ok(DslValue::Map(result))
}

// --- Helpers ---

fn get_string_param(map: &HashMap<String, DslValue>, key: &str) -> Result<String> {
    map.get(key)
        .and_then(|v| match v {
            DslValue::String(s) => Some(s.clone()),
            _ => None,
        })
        .ok_or_else(|| ReplError::Execution(format!("Missing required parameter '{}'", key)))
}

/// Parse a `director` plan's `assignments` into `(worker, system, subtask)`
/// triples, then enforce `MAX_DIRECTOR_ASSIGNMENTS` / `MAX_DIRECTOR_FIELD_LEN`
/// on the result. `builtin_director` issues one inference call per returned
/// assignment, so both caps bound cost/injection amplification driven by
/// model output — exceeding either is an honest error, never a silent
/// truncation of the plan.
fn parse_assignments(
    plan_text: &str,
    worker_defs: &[(String, String)],
) -> Result<Vec<(String, String, String)>> {
    let assignments = parse_assignments_unchecked(plan_text, worker_defs);

    if assignments.len() > MAX_DIRECTOR_ASSIGNMENTS {
        return Err(ReplError::Execution(format!(
            "director: plan proposes {} assignments, exceeding the cap of {} \
             (MAX_DIRECTOR_ASSIGNMENTS)",
            assignments.len(),
            MAX_DIRECTOR_ASSIGNMENTS
        )));
    }
    for (worker, _system, subtask) in &assignments {
        if worker.len() > MAX_DIRECTOR_FIELD_LEN {
            return Err(ReplError::Execution(format!(
                "director: assignment 'worker' field ({} bytes) exceeds the cap of {} bytes \
                 (MAX_DIRECTOR_FIELD_LEN)",
                worker.len(),
                MAX_DIRECTOR_FIELD_LEN
            )));
        }
        if subtask.len() > MAX_DIRECTOR_FIELD_LEN {
            return Err(ReplError::Execution(format!(
                "director: assignment 'subtask' field ({} bytes) exceeds the cap of {} bytes \
                 (MAX_DIRECTOR_FIELD_LEN)",
                subtask.len(),
                MAX_DIRECTOR_FIELD_LEN
            )));
        }
    }

    Ok(assignments)
}

fn parse_assignments_unchecked(
    plan_text: &str,
    worker_defs: &[(String, String)],
) -> Vec<(String, String, String)> {
    // Try to parse as JSON
    if let Ok(plan_json) = serde_json::from_str::<serde_json::Value>(plan_text) {
        if let Some(assignments) = plan_json["assignments"].as_array() {
            return assignments
                .iter()
                .filter_map(|a| {
                    let worker = a["worker"].as_str()?;
                    let subtask = a["subtask"].as_str()?;
                    let system = worker_defs
                        .iter()
                        .find(|(n, _)| n == worker)
                        .map(|(_, s)| s.clone())
                        .unwrap_or_else(|| "You are a helpful assistant.".to_string());
                    Some((worker.to_string(), system, subtask.to_string()))
                })
                .collect();
        }
    }

    // Fallback: assign the entire task to each worker
    worker_defs
        .iter()
        .map(|(name, system)| {
            (
                name.clone(),
                system.clone(),
                format!("Complete this task: {}", plan_text),
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_assignments_valid_json() {
        let plan = r#"{"assignments": [{"worker": "researcher", "subtask": "Find data"}, {"worker": "writer", "subtask": "Write report"}]}"#;
        let workers = vec![
            ("researcher".to_string(), "Research system".to_string()),
            ("writer".to_string(), "Writer system".to_string()),
        ];

        let assignments = parse_assignments(plan, &workers).expect("within caps");
        assert_eq!(assignments.len(), 2);
        assert_eq!(assignments[0].0, "researcher");
        assert_eq!(assignments[0].2, "Find data");
        assert_eq!(assignments[1].0, "writer");
        assert_eq!(assignments[1].2, "Write report");
    }

    #[test]
    fn test_parse_assignments_fallback() {
        let plan = "This is not JSON";
        let workers = vec![
            ("a".to_string(), "System A".to_string()),
            ("b".to_string(), "System B".to_string()),
        ];

        let assignments = parse_assignments(plan, &workers).expect("within caps");
        assert_eq!(assignments.len(), 2);
        assert!(assignments[0].2.contains("This is not JSON"));
    }

    #[test]
    fn test_get_string_param() {
        let mut map = HashMap::new();
        map.insert("key".into(), DslValue::String("value".into()));

        assert_eq!(get_string_param(&map, "key").unwrap(), "value");
        assert!(get_string_param(&map, "missing").is_err());
    }

    #[test]
    fn test_get_string_param_wrong_type() {
        let mut map = HashMap::new();
        map.insert("key".into(), DslValue::Integer(42));

        assert!(get_string_param(&map, "key").is_err());
    }

    // --- Fan-out cap tests ---
    //
    // Each test drives the real builtin (not just the parsing helpers) with
    // a mock `InferenceProvider` so the assertion is "no inference beyond
    // the cap actually ran", not just "the error string looks right".

    use std::sync::atomic::{AtomicUsize, Ordering};
    use symbi_runtime::reasoning::inference::{
        FinishReason, InferenceError, InferenceProvider, InferenceResponse, Usage,
    };

    /// A provider that returns a fixed canned response, counts total calls,
    /// and tracks the peak number of calls it had in flight at once (an
    /// optional artificial delay widens the window in which concurrent
    /// calls overlap, so the concurrency cap test can actually observe
    /// overlap rather than accidentally-serialized calls).
    struct CountingProvider {
        response: String,
        delay: std::time::Duration,
        call_count: AtomicUsize,
        in_flight: AtomicUsize,
        peak_in_flight: AtomicUsize,
    }

    impl CountingProvider {
        fn new(response: impl Into<String>) -> Self {
            Self {
                response: response.into(),
                delay: std::time::Duration::ZERO,
                call_count: AtomicUsize::new(0),
                in_flight: AtomicUsize::new(0),
                peak_in_flight: AtomicUsize::new(0),
            }
        }

        fn with_delay(mut self, delay: std::time::Duration) -> Self {
            self.delay = delay;
            self
        }

        fn calls(&self) -> usize {
            self.call_count.load(Ordering::SeqCst)
        }

        fn peak_concurrency(&self) -> usize {
            self.peak_in_flight.load(Ordering::SeqCst)
        }
    }

    #[async_trait::async_trait]
    impl InferenceProvider for CountingProvider {
        async fn complete(
            &self,
            _conversation: &Conversation,
            _options: &InferenceOptions,
        ) -> std::result::Result<InferenceResponse, InferenceError> {
            self.call_count.fetch_add(1, Ordering::SeqCst);
            let now = self.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
            self.peak_in_flight.fetch_max(now, Ordering::SeqCst);
            if !self.delay.is_zero() {
                tokio::time::sleep(self.delay).await;
            }
            self.in_flight.fetch_sub(1, Ordering::SeqCst);

            Ok(InferenceResponse {
                content: self.response.clone(),
                tool_calls: vec![],
                finish_reason: FinishReason::Stop,
                usage: Usage {
                    prompt_tokens: 1,
                    completion_tokens: 1,
                    total_tokens: 2,
                },
                model: "mock".to_string(),
            })
        }

        fn provider_name(&self) -> &str {
            "mock"
        }
        fn default_model(&self) -> &str {
            "mock-model"
        }
        fn supports_native_tools(&self) -> bool {
            false
        }
        fn supports_structured_output(&self) -> bool {
            true
        }
    }

    fn director_worker(name: &str) -> DslValue {
        let mut m = HashMap::new();
        m.insert("name".to_string(), DslValue::String(name.to_string()));
        m.insert(
            "system".to_string(),
            DslValue::String("You are a helpful assistant.".to_string()),
        );
        DslValue::Map(m)
    }

    #[tokio::test]
    async fn director_rejects_plan_with_too_many_assignments() {
        // One more assignment than the cap allows.
        let assignments: Vec<serde_json::Value> = (0..=MAX_DIRECTOR_ASSIGNMENTS)
            .map(|i| serde_json::json!({"worker": format!("w{i}"), "subtask": "do it"}))
            .collect();
        let plan = serde_json::json!({ "assignments": assignments }).to_string();

        let provider = Arc::new(CountingProvider::new(plan));
        let ctx = ReasoningBuiltinContext {
            provider: Some(provider.clone() as Arc<dyn InferenceProvider>),
            ..Default::default()
        };

        let mut params = HashMap::new();
        params.insert(
            "orchestrator_prompt".to_string(),
            DslValue::String("You direct.".to_string()),
        );
        params.insert(
            "task".to_string(),
            DslValue::String("Do the thing".to_string()),
        );
        params.insert(
            "workers".to_string(),
            DslValue::List(vec![director_worker("w0")]),
        );

        let err = builtin_director(&[DslValue::Map(params)], &ctx)
            .await
            .expect_err("a plan exceeding MAX_DIRECTOR_ASSIGNMENTS must be rejected");
        let msg = format!("{err}");
        assert!(
            msg.contains("assignments") && msg.contains("exceeding"),
            "expected an assignment-count cap error, got: {msg}"
        );

        assert_eq!(
            provider.calls(),
            1,
            "only the planning call should have run; no per-worker inference beyond the cap"
        );
    }

    #[tokio::test]
    async fn director_rejects_oversized_assignment_field() {
        let oversized = "x".repeat(MAX_DIRECTOR_FIELD_LEN + 1);
        let plan = serde_json::json!({
            "assignments": [{"worker": "w0", "subtask": oversized}]
        })
        .to_string();

        let provider = Arc::new(CountingProvider::new(plan));
        let ctx = ReasoningBuiltinContext {
            provider: Some(provider.clone() as Arc<dyn InferenceProvider>),
            ..Default::default()
        };

        let mut params = HashMap::new();
        params.insert(
            "orchestrator_prompt".to_string(),
            DslValue::String("You direct.".to_string()),
        );
        params.insert("task".to_string(), DslValue::String("Do it".to_string()));
        params.insert(
            "workers".to_string(),
            DslValue::List(vec![director_worker("w0")]),
        );

        let err = builtin_director(&[DslValue::Map(params)], &ctx)
            .await
            .expect_err("an oversized assignment field must be rejected");
        let msg = format!("{err}");
        assert!(
            msg.contains("subtask") && msg.contains("exceeds"),
            "expected a field-length cap error, got: {msg}"
        );

        assert_eq!(
            provider.calls(),
            1,
            "only the planning call should have run; no per-worker inference beyond the cap"
        );
    }

    #[tokio::test]
    async fn debate_rejects_rounds_above_cap() {
        let provider = Arc::new(CountingProvider::new("response"));
        let ctx = ReasoningBuiltinContext {
            provider: Some(provider.clone() as Arc<dyn InferenceProvider>),
            ..Default::default()
        };

        let mut params = HashMap::new();
        params.insert(
            "writer_prompt".to_string(),
            DslValue::String("write".to_string()),
        );
        params.insert(
            "critic_prompt".to_string(),
            DslValue::String("critique".to_string()),
        );
        params.insert("topic".to_string(), DslValue::String("topic".to_string()));
        params.insert(
            "rounds".to_string(),
            DslValue::Integer((MAX_DEBATE_ROUNDS + 1) as i64),
        );

        let err = builtin_debate(&[DslValue::Map(params)], &ctx)
            .await
            .expect_err("rounds above MAX_DEBATE_ROUNDS must be rejected");
        let msg = format!("{err}");
        assert!(
            msg.contains("rounds") && msg.contains("exceeds"),
            "expected a rounds cap error, got: {msg}"
        );

        assert_eq!(
            provider.calls(),
            0,
            "no writer/critic inference should run once rounds exceeds the cap"
        );
    }

    #[tokio::test]
    async fn map_reduce_bounds_concurrent_inferences_without_dropping_inputs() {
        // Enough inputs to span several chunks at the concurrency cap.
        let n = MAX_MAP_REDUCE_CONCURRENCY * 3;
        let provider = Arc::new(
            CountingProvider::new("mapped").with_delay(std::time::Duration::from_millis(20)),
        );
        let ctx = ReasoningBuiltinContext {
            provider: Some(provider.clone() as Arc<dyn InferenceProvider>),
            ..Default::default()
        };

        let inputs: Vec<DslValue> = (0..n)
            .map(|i| DslValue::String(format!("item {i}")))
            .collect();
        let mut params = HashMap::new();
        params.insert("inputs".to_string(), DslValue::List(inputs));
        params.insert(
            "mapper".to_string(),
            DslValue::String("map sys".to_string()),
        );
        params.insert(
            "reducer".to_string(),
            DslValue::String("reduce sys".to_string()),
        );

        let result = builtin_map_reduce(&[DslValue::Map(params)], &ctx)
            .await
            .expect("map_reduce must not error under the concurrency cap");

        match result {
            DslValue::Map(map) => match map.get("mapped_results") {
                Some(DslValue::List(items)) => {
                    assert_eq!(
                        items.len(),
                        n,
                        "every input must be processed — the concurrency cap clamps \
                         scheduling, not data"
                    );
                }
                other => panic!("expected 'mapped_results' list, got {other:?}"),
            },
            other => panic!("expected Map, got {other:?}"),
        }

        assert!(
            provider.peak_concurrency() <= MAX_MAP_REDUCE_CONCURRENCY,
            "peak concurrent inferences ({}) exceeded MAX_MAP_REDUCE_CONCURRENCY ({})",
            provider.peak_concurrency(),
            MAX_MAP_REDUCE_CONCURRENCY
        );
        assert!(
            provider.peak_concurrency() > 1,
            "test should exercise real overlap, not accidental full serialization"
        );
        // n map calls + 1 reduce call.
        assert_eq!(provider.calls(), n + 1);
    }
}
