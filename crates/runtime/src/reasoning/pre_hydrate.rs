//! Deterministic Context Pre-Fetch (Pre-Hydration)
//!
//! Extracts references (URLs, file paths, GitHub issues/PRs) from task input
//! via regex, resolves them in parallel via the executor with timeout, prunes
//! to a token budget, and formats as a system message.
//! Part of the orga-adaptive feature gate.

use std::collections::HashSet;
use std::time::Duration;

use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::reasoning::dispatch::GovernedToolDispatcher;
use crate::reasoning::loop_types::{JournalError, LoopConfig, LoopState, ProposedAction};

/// A compiled pattern for extracting references from input text.
#[derive(Debug, Clone)]
struct CompiledPattern {
    ref_type: String,
    regex: Regex,
}

/// A user-defined pattern for reference extraction.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReferencePattern {
    /// The type label for matched references (e.g., "jira_ticket").
    pub ref_type: String,
    /// The regex pattern string. Must have at least one capture group.
    pub pattern: String,
}

/// Configuration for deterministic context pre-fetch.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreHydrationConfig {
    /// Custom patterns for reference extraction (in addition to built-ins).
    #[serde(default)]
    pub custom_patterns: Vec<ReferencePattern>,
    /// Mapping from reference type to tool name for resolution.
    /// e.g., `{"url" -> "web_fetch", "file" -> "file_read"}`
    #[serde(default)]
    pub resolution_tools: std::collections::HashMap<String, String>,
    /// Timeout for the entire resolution phase.
    #[serde(default = "default_timeout", with = "humantime_serde")]
    pub timeout: Duration,
    /// Maximum number of references to extract.
    #[serde(default = "default_max_references")]
    pub max_references: usize,
    /// Maximum tokens for the hydrated context (1 token ~ 4 chars).
    #[serde(default = "default_max_context_tokens")]
    pub max_context_tokens: usize,
}

fn default_timeout() -> Duration {
    Duration::from_secs(15)
}

fn default_max_references() -> usize {
    10
}

fn default_max_context_tokens() -> usize {
    4000
}

impl Default for PreHydrationConfig {
    fn default() -> Self {
        Self {
            custom_patterns: Vec::new(),
            resolution_tools: std::collections::HashMap::new(),
            timeout: default_timeout(),
            max_references: default_max_references(),
            max_context_tokens: default_max_context_tokens(),
        }
    }
}

/// A reference extracted from the task input.
#[derive(Debug, Clone)]
pub struct ExtractedReference {
    /// The type of reference (e.g., "url", "file", "issue", "pr").
    pub ref_type: String,
    /// The raw matched text.
    pub value: String,
}

/// A resolved reference with its content.
#[derive(Debug, Clone)]
pub struct ResolvedReference {
    /// The original reference.
    pub reference: ExtractedReference,
    /// The resolved content (may be truncated).
    pub content: String,
    /// Estimated token count (chars / 4).
    pub token_estimate: usize,
}

/// Result of the hydration process.
#[derive(Debug)]
pub struct HydratedContext {
    /// Successfully resolved references.
    pub resolved: Vec<ResolvedReference>,
    /// References that failed to resolve.
    pub failed: Vec<(ExtractedReference, String)>,
    /// Total estimated tokens used.
    pub total_tokens: usize,
}

/// Engine for deterministic context pre-fetch.
pub struct PreHydrationEngine {
    config: PreHydrationConfig,
    builtin_patterns: Vec<CompiledPattern>,
    custom_compiled: Vec<CompiledPattern>,
}

impl PreHydrationEngine {
    /// Create a new pre-hydration engine with the given configuration.
    pub fn new(config: PreHydrationConfig) -> Self {
        let builtin_patterns = vec![
            CompiledPattern {
                ref_type: "url".to_string(),
                regex: Regex::new(r"https?://[^\s\)>\]]+").unwrap(),
            },
            CompiledPattern {
                ref_type: "file".to_string(),
                regex: Regex::new(r"(?:^|\s)([./~][a-zA-Z0-9_/.\-]+\.[a-zA-Z0-9]+)").unwrap(),
            },
            CompiledPattern {
                ref_type: "issue".to_string(),
                regex: Regex::new(r"#(\d+)").unwrap(),
            },
            CompiledPattern {
                ref_type: "pr".to_string(),
                regex: Regex::new(r"(?i)PR\s*#(\d+)").unwrap(),
            },
        ];

        let custom_compiled = config
            .custom_patterns
            .iter()
            .filter_map(|p| {
                Regex::new(&p.pattern).ok().map(|regex| CompiledPattern {
                    ref_type: p.ref_type.clone(),
                    regex,
                })
            })
            .collect();

        Self {
            config,
            builtin_patterns,
            custom_compiled,
        }
    }

    /// Extract references from the task input text.
    pub fn extract_references(&self, input: &str) -> Vec<ExtractedReference> {
        let mut seen = HashSet::new();
        let mut refs = Vec::new();

        let all_patterns = self
            .builtin_patterns
            .iter()
            .chain(self.custom_compiled.iter());

        for pattern in all_patterns {
            for cap in pattern.regex.find_iter(input) {
                let value = cap.as_str().trim().to_string();
                if !value.is_empty() && seen.insert(value.clone()) {
                    refs.push(ExtractedReference {
                        ref_type: pattern.ref_type.clone(),
                        value,
                    });
                    if refs.len() >= self.config.max_references {
                        return refs;
                    }
                }
            }
        }

        refs
    }

    /// Resolve task references through the same preparation, approval, policy,
    /// and required journal checkpoint as explicit tool calls.
    pub async fn hydrate(
        &self,
        refs: &[ExtractedReference],
        dispatcher: &GovernedToolDispatcher<'_>,
        state: &LoopState,
        loop_config: &LoopConfig,
    ) -> Result<HydratedContext, JournalError> {
        let mut hydrated = HydratedContext {
            resolved: Vec::new(),
            failed: Vec::new(),
            total_tokens: 0,
        };
        let mut actions = Vec::new();
        let mut ref_map = Vec::new();
        for (i, reference) in refs.iter().enumerate() {
            if let Some(tool_name) = self.config.resolution_tools.get(&reference.ref_type) {
                let call_id = format!("prehydrate_{}", i);
                actions.push(ProposedAction::ToolCall {
                    call_id: call_id.clone(),
                    name: tool_name.clone(),
                    arguments: serde_json::json!({"input": reference.value}).to_string(),
                });
                ref_map.push((call_id, reference));
            } else {
                hydrated
                    .failed
                    .push((reference.clone(), "No resolution tool configured".into()));
            }
        }
        if actions.is_empty() {
            return Ok(hydrated);
        }

        let mut config = loop_config.clone();
        let elapsed = state.elapsed().to_std().unwrap_or(Duration::ZERO);
        config.timeout = config
            .timeout
            .min(elapsed.saturating_add(self.config.timeout));
        config.tool_timeout = config.tool_timeout.min(self.config.timeout);
        let observations = dispatcher.dispatch(&actions, state, &config).await?;
        let mut by_id: std::collections::HashMap<_, _> = observations
            .into_iter()
            .filter_map(|obs| obs.call_id.clone().map(|id| (id, obs)))
            .collect();
        let mut remaining_bytes = self.config.max_context_tokens.saturating_mul(4);
        for (id, reference) in ref_map {
            match by_id.remove(&id) {
                None => hydrated.failed.push((
                    reference.clone(),
                    "Resolution returned no correlated result".into(),
                )),
                Some(obs) if obs.is_error => hydrated.failed.push((reference.clone(), obs.content)),
                Some(_) if remaining_bytes == 0 => hydrated
                    .failed
                    .push((reference.clone(), "Token budget exhausted".into())),
                Some(obs) => {
                    let mut content = obs.content;
                    let mut end = remaining_bytes.min(content.len());
                    while !content.is_char_boundary(end) {
                        end -= 1;
                    }
                    content.truncate(end);
                    let token_estimate = content.len().div_ceil(4);
                    remaining_bytes =
                        remaining_bytes.saturating_sub(token_estimate.saturating_mul(4));
                    hydrated.total_tokens += token_estimate;
                    hydrated.resolved.push(ResolvedReference {
                        reference: reference.clone(),
                        content,
                        token_estimate,
                    });
                }
            }
        }
        Ok(hydrated)
    }

    /// Format hydrated context as a system message string.
    pub fn format_context(hydrated: &HydratedContext) -> String {
        if hydrated.resolved.is_empty() {
            return String::new();
        }

        let mut lines = vec!["[PRE_HYDRATED_CONTEXT]".to_string()];
        lines.push("The following references were resolved from the task input:".to_string());

        for resolved in &hydrated.resolved {
            lines.push(format!(
                "\n--- {} ({}) ---",
                resolved.reference.value, resolved.reference.ref_type
            ));
            lines.push(resolved.content.clone());
        }

        if !hydrated.failed.is_empty() {
            lines.push(format!(
                "\n({} references could not be resolved)",
                hydrated.failed.len()
            ));
        }

        lines.join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_urls() {
        let engine = PreHydrationEngine::new(PreHydrationConfig::default());
        let input = "Check https://example.com/api and https://docs.rs/tokio for details";
        let refs = engine.extract_references(input);
        let urls: Vec<_> = refs.iter().filter(|r| r.ref_type == "url").collect();
        assert_eq!(urls.len(), 2);
        assert!(urls[0].value.contains("example.com"));
        assert!(urls[1].value.contains("docs.rs"));
    }

    #[test]
    fn test_extract_file_paths() {
        let engine = PreHydrationEngine::new(PreHydrationConfig::default());
        let input = "Read ./src/main.rs and ~/config.toml";
        let refs = engine.extract_references(input);
        let files: Vec<_> = refs.iter().filter(|r| r.ref_type == "file").collect();
        assert!(files.len() >= 2);
    }

    #[test]
    fn test_extract_issues() {
        let engine = PreHydrationEngine::new(PreHydrationConfig::default());
        let input = "Related to #42 and #100";
        let refs = engine.extract_references(input);
        let issues: Vec<_> = refs.iter().filter(|r| r.ref_type == "issue").collect();
        assert_eq!(issues.len(), 2);
    }

    #[test]
    fn test_extract_prs() {
        let engine = PreHydrationEngine::new(PreHydrationConfig::default());
        let input = "See PR #55 for context";
        let refs = engine.extract_references(input);
        let prs: Vec<_> = refs.iter().filter(|r| r.ref_type == "pr").collect();
        assert_eq!(prs.len(), 1);
    }

    #[test]
    fn test_max_references_cap() {
        let config = PreHydrationConfig {
            max_references: 2,
            ..Default::default()
        };
        let engine = PreHydrationEngine::new(config);
        let input = "Issues #1, #2, #3, #4, #5";
        let refs = engine.extract_references(input);
        assert!(refs.len() <= 2);
    }

    #[test]
    fn test_deduplication() {
        let engine = PreHydrationEngine::new(PreHydrationConfig::default());
        let input = "Visit https://example.com and again https://example.com";
        let refs = engine.extract_references(input);
        let urls: Vec<_> = refs.iter().filter(|r| r.ref_type == "url").collect();
        assert_eq!(urls.len(), 1);
    }

    #[test]
    fn test_empty_input() {
        let engine = PreHydrationEngine::new(PreHydrationConfig::default());
        let refs = engine.extract_references("");
        assert!(refs.is_empty());
    }

    #[test]
    fn test_format_context_empty() {
        let hydrated = HydratedContext {
            resolved: Vec::new(),
            failed: Vec::new(),
            total_tokens: 0,
        };
        let formatted = PreHydrationEngine::format_context(&hydrated);
        assert!(formatted.is_empty());
    }

    #[test]
    fn test_format_context_with_content() {
        let hydrated = HydratedContext {
            resolved: vec![ResolvedReference {
                reference: ExtractedReference {
                    ref_type: "url".to_string(),
                    value: "https://example.com".to_string(),
                },
                content: "Example content".to_string(),
                token_estimate: 4,
            }],
            failed: Vec::new(),
            total_tokens: 4,
        };
        let formatted = PreHydrationEngine::format_context(&hydrated);
        assert!(formatted.contains("[PRE_HYDRATED_CONTEXT]"));
        assert!(formatted.contains("https://example.com"));
        assert!(formatted.contains("Example content"));
    }

    #[test]
    fn test_custom_patterns() {
        let config = PreHydrationConfig {
            custom_patterns: vec![ReferencePattern {
                ref_type: "jira".to_string(),
                pattern: r"[A-Z]+-\d+".to_string(),
            }],
            ..Default::default()
        };
        let engine = PreHydrationEngine::new(config);
        let input = "Check PROJ-123 for details";
        let refs = engine.extract_references(input);
        let jira: Vec<_> = refs.iter().filter(|r| r.ref_type == "jira").collect();
        assert_eq!(jira.len(), 1);
        assert!(jira[0].value.contains("PROJ-123"));
    }

    #[tokio::test]
    async fn test_hydrate_empty_refs() {
        let engine = PreHydrationEngine::new(PreHydrationConfig::default());
        let executor = crate::reasoning::executor::UnavailableToolExecutor;
        let gate = crate::reasoning::policy_bridge::DefaultPolicyGate::new();
        let journal = crate::reasoning::loop_types::BufferedJournal::new(100);
        let circuit_breakers = crate::reasoning::circuit_breaker::CircuitBreakerRegistry::default();
        let dispatcher = GovernedToolDispatcher {
            executor: &executor,
            gate: &gate,
            journal: &journal,
            circuit_breakers: &circuit_breakers,
        };
        let state = LoopState::new(
            crate::types::AgentId::new(),
            crate::reasoning::conversation::Conversation::new(),
        );
        let result = engine
            .hydrate(&[], &dispatcher, &state, &LoopConfig::default())
            .await
            .unwrap();
        assert!(result.resolved.is_empty());
        assert!(result.failed.is_empty());
        assert_eq!(result.total_tokens, 0);
    }

    #[tokio::test]
    async fn hydration_correlates_results_and_preserves_utf8_within_budget() {
        use crate::reasoning::circuit_breaker::CircuitBreakerRegistry;
        use crate::reasoning::executor::ActionExecutor;
        use crate::reasoning::inference::ToolDefinition;
        use crate::reasoning::loop_types::{BufferedJournal, Observation};
        use crate::reasoning::policy_bridge::DefaultPolicyGate;
        use crate::reasoning::prepared::AuthorizedAction;
        struct Resolver;
        #[async_trait::async_trait]
        impl ActionExecutor for Resolver {
            fn tool_definitions(&self) -> Vec<ToolDefinition> {
                vec![ToolDefinition {
                    name: "resolve".into(),
                    description: "UTF-8 fixture".into(),
                    parameters: serde_json::json!({"type":"object", "properties":{"input":{"type":"string"}}}),
                }]
            }
            async fn execute_actions(
                &self,
                _: &[ProposedAction],
                _: &LoopConfig,
                _: &CircuitBreakerRegistry,
            ) -> Vec<Observation> {
                panic!("pre-fetch bypassed authorization")
            }
            async fn execute_authorized(
                &self,
                actions: Vec<AuthorizedAction>,
                _: &LoopConfig,
                _: &CircuitBreakerRegistry,
            ) -> Vec<Observation> {
                actions
                    .into_iter()
                    .rev()
                    .map(|grant| {
                        let ProposedAction::ToolCall {
                            call_id, arguments, ..
                        } = grant.action()
                        else {
                            unreachable!()
                        };
                        let args: serde_json::Value = serde_json::from_str(arguments).unwrap();
                        Observation::tool_result("resolve", args["input"].as_str().unwrap())
                            .with_call_id(call_id)
                    })
                    .collect()
            }
        }
        let executor = Resolver;
        let gate = DefaultPolicyGate::permissive_for_dev_only();
        let journal = BufferedJournal::new(100);
        let circuit_breakers = CircuitBreakerRegistry::default();
        let dispatcher = GovernedToolDispatcher {
            executor: &executor,
            gate: &gate,
            journal: &journal,
            circuit_breakers: &circuit_breakers,
        };
        let state = LoopState::new(
            crate::types::AgentId::new(),
            crate::reasoning::conversation::Conversation::new(),
        );
        let config = LoopConfig {
            tool_definitions: executor.tool_definitions(),
            ..Default::default()
        };
        let refs = [
            ExtractedReference {
                ref_type: "fixture".into(),
                value: "😀é".into(),
            },
            ExtractedReference {
                ref_type: "fixture".into(),
                value: "second".into(),
            },
            ExtractedReference {
                ref_type: "unsupported".into(),
                value: "third".into(),
            },
        ];
        let engine = PreHydrationEngine::new(PreHydrationConfig {
            resolution_tools: [("fixture".into(), "resolve".into())].into(),
            max_context_tokens: 1,
            ..Default::default()
        });
        let result = engine
            .hydrate(&refs, &dispatcher, &state, &config)
            .await
            .unwrap();
        assert_eq!(result.resolved.len(), 1);
        assert_eq!(result.resolved[0].reference.value, "😀é");
        assert_eq!(result.resolved[0].content, "😀");
        assert_eq!(result.total_tokens, 1);
        assert_eq!(result.failed.len(), 2);
    }

    #[test]
    fn test_default_config() {
        let config = PreHydrationConfig::default();
        assert_eq!(config.timeout, Duration::from_secs(15));
        assert_eq!(config.max_references, 10);
        assert_eq!(config.max_context_tokens, 4000);
        assert!(config.custom_patterns.is_empty());
        assert!(config.resolution_tools.is_empty());
    }
}
