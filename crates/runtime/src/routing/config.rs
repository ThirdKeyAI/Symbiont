//! Configuration types for the routing module

use super::decision::{LLMProvider, MonitoringLevel, SecurityLevel};
use super::error::TaskType;
use crate::config::ResourceConstraints;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Duration;

/// Complete routing configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutingConfig {
    /// Enable the policy-driven router
    pub enabled: bool,
    /// Routing policy configuration
    pub policy: RoutingPolicyConfig,
    /// Task classification settings
    pub classification: TaskClassificationConfig,
    /// LLM provider configurations
    pub llm_providers: HashMap<String, LLMProviderConfig>,
}

/// Core routing policy configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutingPolicyConfig {
    /// Global routing settings
    pub global_settings: GlobalRoutingSettings,
    /// Ordered list of routing rules
    pub rules: Vec<RoutingRule>,
    /// Default action when no rules match
    pub default_action: RouteAction,
    /// LLM fallback configuration
    pub fallback_config: FallbackConfig,
}

/// Global routing settings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GlobalRoutingSettings {
    /// Enable/disable SLM routing globally
    pub slm_routing_enabled: bool,
    /// Always audit routing decisions
    pub always_audit: bool,
    /// Global confidence threshold for SLM responses
    pub global_confidence_threshold: f64,
    /// Maximum retry attempts for failed SLM calls
    pub max_slm_retries: u32,
}

/// Individual routing rule
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutingRule {
    /// Rule identifier
    pub name: String,
    /// Rule priority (higher = evaluated first)
    pub priority: u32,
    /// Conditions that must be met
    pub conditions: RoutingConditions,
    /// Action to take if conditions match
    pub action: RouteAction,
    /// Action extensions for additional configuration
    #[serde(default)]
    pub action_extension: Option<ActionExtension>,
    /// Whether this rule can be overridden
    pub override_allowed: bool,
}

/// Conditions for routing rules
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutingConditions {
    /// Task types this rule applies to
    pub task_types: Option<Vec<TaskType>>,
    /// Agent IDs this rule applies to
    pub agent_ids: Option<Vec<String>>,
    /// Resource requirements
    pub resource_constraints: Option<ResourceConstraints>,
    /// Security level requirements
    pub security_level: Option<SecurityLevel>,
    /// Custom condition expressions
    pub custom_conditions: Option<Vec<String>>,
}

/// Action to take when routing conditions are met
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RouteAction {
    /// Use SLM with specified preferences
    UseSLM {
        model_preference: ModelPreference,
        monitoring_level: MonitoringLevel,
        fallback_on_low_confidence: bool,
        confidence_threshold: Option<f64>,
    },
    /// Use LLM provider
    UseLLM {
        provider: LLMProvider,
        model: Option<String>,
    },
    /// Deny request
    Deny { reason: String },
}

/// Model preference for SLM selection
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ModelPreference {
    /// Prefer specialist models for the task type
    Specialist,
    /// Prefer general-purpose models
    Generalist,
    /// Use specific model by ID
    Specific { model_id: String },
    /// Use best available model for requirements
    BestAvailable,
}

/// Action extensions for additional routing configuration
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ActionExtension {
    /// Preferred sandbox tier for execution
    pub sandbox: Option<String>,
}

/// Fallback configuration for LLM providers
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FallbackConfig {
    /// Enable fallback mechanism
    pub enabled: bool,
    /// Maximum fallback attempts
    pub max_attempts: u32,
    /// Timeout for fallback operations
    #[serde(with = "humantime_serde")]
    pub timeout: Duration,
    /// Provider priority order
    pub providers: HashMap<String, LLMProviderConfig>,
}

/// LLM provider configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LLMProviderConfig {
    /// API key environment variable name
    pub api_key_env: String,
    /// Base URL for the provider
    pub base_url: String,
    /// Default model for this provider
    pub default_model: String,
    /// Request timeout
    #[serde(with = "humantime_serde")]
    pub timeout: Duration,
    /// Maximum retries
    pub max_retries: u32,
    /// Rate limiting settings
    pub rate_limit: Option<RateLimitConfig>,
}

/// Rate limiting configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RateLimitConfig {
    /// Requests per minute
    pub requests_per_minute: u32,
    /// Tokens per minute
    pub tokens_per_minute: Option<u32>,
    /// Burst allowance
    pub burst_allowance: Option<u32>,
}

/// Task classification configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskClassificationConfig {
    /// Enable automatic task classification
    pub enabled: bool,
    /// Classification patterns for different task types
    pub patterns: HashMap<TaskType, ClassificationPattern>,
    /// Confidence threshold for classification
    pub confidence_threshold: f64,
    /// Fallback task type when classification fails
    pub default_task_type: TaskType,
}

/// Pattern for task classification
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClassificationPattern {
    /// Keywords that indicate this task type
    pub keywords: Vec<String>,
    /// Regex patterns for classification
    pub patterns: Vec<String>,
    /// Weight for this pattern
    pub weight: f64,
}

impl Default for RoutingConfig {
    fn default() -> Self {
        let mut llm_providers = HashMap::new();

        llm_providers.insert(
            "openai".to_string(),
            LLMProviderConfig {
                api_key_env: "OPENAI_API_KEY".to_string(),
                base_url: "https://api.openai.com/v1".to_string(),
                default_model: "gpt-3.5-turbo".to_string(),
                timeout: Duration::from_secs(60),
                max_retries: 3,
                rate_limit: Some(RateLimitConfig {
                    requests_per_minute: 60,
                    tokens_per_minute: Some(10000),
                    burst_allowance: Some(10),
                }),
            },
        );

        llm_providers.insert(
            "anthropic".to_string(),
            LLMProviderConfig {
                api_key_env: "ANTHROPIC_API_KEY".to_string(),
                base_url: "https://api.anthropic.com".to_string(),
                default_model: "claude-3-sonnet-20240229".to_string(),
                timeout: Duration::from_secs(60),
                max_retries: 3,
                rate_limit: Some(RateLimitConfig {
                    requests_per_minute: 60,
                    tokens_per_minute: Some(10000),
                    burst_allowance: Some(10),
                }),
            },
        );

        Self {
            enabled: true,
            policy: RoutingPolicyConfig::default(),
            classification: TaskClassificationConfig::default(),
            llm_providers,
        }
    }
}

impl Default for RoutingPolicyConfig {
    fn default() -> Self {
        Self {
            global_settings: GlobalRoutingSettings::default(),
            rules: Vec::new(),
            default_action: RouteAction::UseLLM {
                provider: LLMProvider::OpenAI { model: None },
                model: Some("gpt-3.5-turbo".to_string()),
            },
            fallback_config: FallbackConfig::default(),
        }
    }
}

impl Default for GlobalRoutingSettings {
    fn default() -> Self {
        Self {
            slm_routing_enabled: true,
            always_audit: true,
            global_confidence_threshold: 0.85,
            max_slm_retries: 2,
        }
    }
}

impl Default for FallbackConfig {
    fn default() -> Self {
        let mut providers = HashMap::new();
        providers.insert(
            "primary".to_string(),
            LLMProviderConfig {
                api_key_env: "OPENAI_API_KEY".to_string(),
                base_url: "https://api.openai.com/v1".to_string(),
                default_model: "gpt-3.5-turbo".to_string(),
                timeout: Duration::from_secs(60),
                max_retries: 3,
                rate_limit: None,
            },
        );

        Self {
            enabled: true,
            max_attempts: 3,
            timeout: Duration::from_secs(30),
            providers,
        }
    }
}

impl Default for TaskClassificationConfig {
    fn default() -> Self {
        let mut patterns = HashMap::new();

        patterns.insert(
            TaskType::Intent,
            ClassificationPattern {
                keywords: vec![
                    "intent".to_string(),
                    "intention".to_string(),
                    "purpose".to_string(),
                ],
                patterns: vec![r"what.*intent".to_string(), r"user.*wants".to_string()],
                weight: 1.0,
            },
        );

        patterns.insert(
            TaskType::CodeGeneration,
            ClassificationPattern {
                keywords: vec![
                    "code".to_string(),
                    "function".to_string(),
                    "implement".to_string(),
                    "generate".to_string(),
                ],
                patterns: vec![
                    r"write.*code".to_string(),
                    r"implement.*function".to_string(),
                ],
                weight: 1.0,
            },
        );

        patterns.insert(
            TaskType::Analysis,
            ClassificationPattern {
                keywords: vec![
                    "analyze".to_string(),
                    "analysis".to_string(),
                    "examine".to_string(),
                    "review".to_string(),
                ],
                patterns: vec![
                    r"analyze.*data".to_string(),
                    r"perform.*analysis".to_string(),
                ],
                weight: 1.0,
            },
        );

        Self {
            enabled: true,
            patterns,
            confidence_threshold: 0.7,
            default_task_type: TaskType::Custom("unknown".to_string()),
        }
    }
}

impl RoutingRule {
    /// Check if this rule's conditions match the given context
    pub fn matches(&self, context: &super::decision::RoutingContext) -> bool {
        // Check task types
        if let Some(ref task_types) = self.conditions.task_types {
            if !task_types.contains(&context.task_type) {
                return false;
            }
        }

        // Check agent IDs
        if let Some(ref agent_ids) = self.conditions.agent_ids {
            if !agent_ids.contains(&context.agent_id.to_string()) {
                return false;
            }
        }

        // Check security level
        if let Some(ref required_level) = self.conditions.security_level {
            if context.agent_security_level < *required_level {
                return false;
            }
        }

        // Check resource constraints
        if let Some(ref rule_constraints) = self.conditions.resource_constraints {
            if let Some(ref context_limits) = context.resource_limits {
                if context_limits.max_memory_mb > rule_constraints.max_memory_mb {
                    return false;
                }
            }
        }

        // Implement custom condition evaluation
        if let Some(ref custom_conditions) = self.conditions.custom_conditions {
            for condition_expr in custom_conditions {
                if !self.evaluate_custom_condition(condition_expr, context) {
                    return false;
                }
            }
        }

        true
    }

    /// Evaluate a custom condition expression
    fn evaluate_custom_condition(
        &self,
        condition_expr: &str,
        context: &super::decision::RoutingContext,
    ) -> bool {
        // Simple expression evaluator for basic conditions
        // In a real implementation, this could use a proper expression parser

        // Handle common condition patterns
        if condition_expr.contains("agent_id") {
            if let Some(expected_id) = condition_expr.strip_prefix("agent_id == ") {
                let expected_id = expected_id.trim_matches('"');
                return context.agent_id.to_string() == expected_id;
            }
        }

        if condition_expr.contains("task_type") {
            if let Some(expected_type) = condition_expr.strip_prefix("task_type == ") {
                let expected_type = expected_type.trim_matches('"');
                return format!("{:?}", context.task_type)
                    .to_lowercase()
                    .contains(&expected_type.to_lowercase());
            }
        }

        if condition_expr.contains("security_level") && condition_expr.contains(">=") {
            if let Some(level_str) = condition_expr.strip_prefix("security_level >= ") {
                if let Ok(required_level) = level_str.trim().parse::<u8>() {
                    let current_level = match context.agent_security_level {
                        SecurityLevel::Low => 1,
                        SecurityLevel::Medium => 2,
                        SecurityLevel::High => 3,
                        SecurityLevel::Critical => 4,
                    };
                    return current_level >= required_level;
                }
            }
        }

        if condition_expr.contains("memory_limit") {
            if let Some(ref resource_limits) = context.resource_limits {
                if condition_expr.contains("<=") {
                    if let Some(limit_str) = condition_expr.strip_prefix("memory_limit <= ") {
                        if let Ok(max_memory) = limit_str.trim().parse::<u64>() {
                            return resource_limits.max_memory_mb <= max_memory;
                        }
                    }
                }
            }
        }

        // Handle boolean expressions
        if condition_expr == "true" {
            return true;
        }
        if condition_expr == "false" {
            return false;
        }

        // Default: log unrecognized condition and return true to be permissive
        tracing::warn!("Unrecognized custom condition: {}", condition_expr);
        true
    }
}

impl RoutingPolicyConfig {
    /// Validate the routing policy configuration
    pub fn validate(&self) -> Result<(), super::error::RoutingError> {
        // Validate rules are sorted by priority
        let mut prev_priority = u32::MAX;
        for rule in &self.rules {
            if rule.priority > prev_priority {
                return Err(super::error::RoutingError::ConfigurationError {
                    key: "policy.rules".to_string(),
                    reason: "Rules must be ordered by priority (highest first)".to_string(),
                });
            }
            prev_priority = rule.priority;
        }

        // Validate confidence thresholds
        if self.global_settings.global_confidence_threshold < 0.0
            || self.global_settings.global_confidence_threshold > 1.0
        {
            return Err(super::error::RoutingError::ConfigurationError {
                key: "policy.global_settings.global_confidence_threshold".to_string(),
                reason: "Confidence threshold must be between 0.0 and 1.0".to_string(),
            });
        }

        Ok(())
    }
}
