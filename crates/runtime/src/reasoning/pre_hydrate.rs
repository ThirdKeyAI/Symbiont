//! Deterministic Context Pre-Fetch (Pre-Hydration)
//!
//! Extracts references (URLs, file paths, GitHub issues/PRs) from task input
//! via regex, resolves them in parallel via the executor with timeout, prunes
//! to a token budget, and formats as a system message.
//! Part of the orga-adaptive feature gate.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::reasoning::circuit_breaker::CircuitBreakerRegistry;
use crate::reasoning::executor::ActionExecutor;
use crate::reasoning::loop_types::{LoopConfig, ProposedAction};

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

    /// Resolve extracted references in parallel via the executor.
    pub async fn hydrate(
        &self,
        refs: &[ExtractedReference],
        executor: &Arc<dyn ActionExecutor>,
        circuit_breakers: &Arc<CircuitBreakerRegistry>,
        loop_config: &LoopConfig,
    ) -> HydratedContext {
        if refs.is_empty() {
            return HydratedContext {
                resolved: Vec::new(),
                failed: Vec::new(),
                total_tokens: 0,
            };
        }

        // Build tool call actions for each reference
        let mut actions = Vec::new();
        let mut ref_map: Vec<&ExtractedReference> = Vec::new();

        for (i, r) in refs.iter().enumerate() {
            if let Some(tool_name) = self.config.resolution_tools.get(&r.ref_type) {
                let arguments = serde_json::json!({"input": r.value}).to_string();
                actions.push(ProposedAction::ToolCall {
                    call_id: format!("prehydrate_{}", i),
                    name: tool_name.clone(),
                    arguments,
                });
                ref_map.push(r);
            }
        }

        if actions.is_empty() {
            // No resolution tools configured for any reference types
            return HydratedContext {
                resolved: Vec::new(),
                failed: refs
                    .iter()
                    .map(|r| (r.clone(), "No resolution tool configured".to_string()))
                    .collect(),
                total_tokens: 0,
            };
        }

        // Execute with timeout
        let observations = match tokio::time::timeout(
            self.config.timeout,
            executor.execute_actions(&actions, loop_config, circuit_breakers),
        )
        .await
        {
            Ok(obs) => obs,
            Err(_) => {
                return HydratedContext {
                    resolved: Vec::new(),
                    failed: refs
                        .iter()
                        .map(|r| (r.clone(), "Resolution timed out".to_string()))
                        .collect(),
                    total_tokens: 0,
                };
            }
        };

        // Process results and prune to token budget
        let mut resolved = Vec::new();
        let mut failed = Vec::new();
        let mut total_tokens = 0;
        let max_chars = self.config.max_context_tokens * 4; // 1 token ~ 4 chars

        for (i, obs) in observations.iter().enumerate() {
            if i >= ref_map.len() {
                break;
            }
            let reference = ref_map[i].clone();

            if obs.is_error {
                failed.push((reference, obs.content.clone()));
            } else {
                let mut content = obs.content.clone();
                let remaining_chars = max_chars.saturating_sub(total_tokens * 4);
                if content.len() > remaining_chars {
                    content.truncate(remaining_chars);
                    content.push_str("...[truncated]");
                }
                let token_estimate = content.len() / 4;
                total_tokens += token_estimate;

                resolved.push(ResolvedReference {
                    reference,
                    content,
                    token_estimate,
                });

                if total_tokens >= self.config.max_context_tokens {
                    // Budget exhausted; remaining refs are "failed" due to budget
                    for r in ref_map.iter().skip(i + 1) {
                        failed.push(((*r).clone(), "Token budget exhausted".to_string()));
                    }
                    break;
                }
            }
        }

        HydratedContext {
            resolved,
            failed,
            total_tokens,
        }
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
        let executor: Arc<dyn ActionExecutor> =
            Arc::new(crate::reasoning::executor::DefaultActionExecutor::default());
        let circuit_breakers = Arc::new(CircuitBreakerRegistry::default());
        let loop_config = LoopConfig::default();

        let result = engine
            .hydrate(&[], &executor, &circuit_breakers, &loop_config)
            .await;
        assert!(result.resolved.is_empty());
        assert!(result.failed.is_empty());
        assert_eq!(result.total_tokens, 0);
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
