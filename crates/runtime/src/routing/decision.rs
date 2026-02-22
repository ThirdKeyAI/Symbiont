//! Routing decision types and structures

use super::error::TaskType;
use crate::config::ResourceConstraints;
use crate::sandbox::SandboxTier;
use crate::types::AgentId;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Duration;

/// Routing decision outcome
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RouteDecision {
    /// Route to SLM with specific model
    UseSLM {
        model_id: String,
        monitoring: MonitoringLevel,
        fallback_on_failure: bool,
        sandbox_tier: Option<SandboxTier>,
    },
    /// Route to LLM provider
    UseLLM {
        provider: LLMProvider,
        reason: String,
        sandbox_tier: Option<SandboxTier>,
    },
    /// Deny the request
    Deny {
        reason: String,
        policy_violated: String,
    },
}

/// Monitoring level for SLM execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MonitoringLevel {
    None,
    Basic,
    Enhanced { confidence_threshold: f64 },
}

/// LLM provider specification
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum LLMProvider {
    OpenAI {
        model: Option<String>,
    },
    Anthropic {
        model: Option<String>,
    },
    Custom {
        endpoint: String,
        model: Option<String>,
    },
}

impl std::fmt::Display for LLMProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LLMProvider::OpenAI { model } => {
                let model_name = model.as_deref().unwrap_or("gpt-3.5-turbo");
                write!(f, "OpenAI({})", model_name)
            }
            LLMProvider::Anthropic { model } => {
                let model_name = model.as_deref().unwrap_or("claude-3-haiku");
                write!(f, "Anthropic({})", model_name)
            }
            LLMProvider::Custom { endpoint, model } => {
                let model_name = model.as_deref().unwrap_or("unknown");
                write!(f, "Custom({}, {})", endpoint, model_name)
            }
        }
    }
}

/// Context for routing decisions
#[derive(Debug, Clone)]
pub struct RoutingContext {
    /// Request identification
    pub request_id: String,
    pub agent_id: AgentId,
    pub timestamp: chrono::DateTime<chrono::Utc>,

    /// Task information
    pub task_type: TaskType,
    pub prompt: String,
    pub expected_output_type: OutputType,

    /// Resource constraints
    pub max_execution_time: Option<Duration>,
    pub resource_limits: Option<ResourceConstraints>,

    /// Agent context
    pub agent_capabilities: Vec<String>,
    pub agent_security_level: SecurityLevel,

    /// Additional metadata
    pub metadata: HashMap<String, String>,
}

/// Expected output type for the task
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum OutputType {
    Text,
    Code,
    Json,
    Structured,
    Binary,
}

/// Security level for agent operations
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum SecurityLevel {
    Low = 1,
    Medium = 2,
    High = 3,
    Critical = 4,
}

/// Model request structure
#[derive(Debug, Clone)]
pub struct ModelRequest {
    pub prompt: String,
    pub parameters: HashMap<String, serde_json::Value>,
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
    pub stop_sequences: Option<Vec<String>>,
}

/// Model response structure
#[derive(Debug, Clone)]
pub struct ModelResponse {
    pub content: String,
    pub finish_reason: FinishReason,
    pub token_usage: Option<TokenUsage>,
    pub metadata: HashMap<String, serde_json::Value>,
    pub confidence_score: Option<f64>,
}

/// Reason why model generation finished
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FinishReason {
    Stop,
    Length,
    ContentFilter,
    Error,
}

/// Token usage statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenUsage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
}

/// Routing statistics for monitoring with lock-free atomic counters
///
/// All counter fields use `AtomicU64` for lock-free concurrent updates.
/// Only the confidence score circular buffer requires a `Mutex`, which is
/// held very briefly during score insertion or averaging.
pub struct RoutingStatistics {
    total_requests: AtomicU64,
    slm_routes: AtomicU64,
    llm_routes: AtomicU64,
    denied_routes: AtomicU64,
    fallback_routes: AtomicU64,
    /// Cumulative response time in nanoseconds (truncated to u64)
    cumulative_response_time_nanos: AtomicU64,
    /// Number of successful requests (for computing success_rate)
    successful_requests: AtomicU64,
    /// Confidence scores protected by a std Mutex (held briefly)
    confidence_state: Mutex<ConfidenceState>,
}

/// Internal state for confidence score tracking
struct ConfidenceState {
    scores: VecDeque<f64>,
    max_scores: usize,
}

impl Default for RoutingStatistics {
    fn default() -> Self {
        Self {
            total_requests: AtomicU64::new(0),
            slm_routes: AtomicU64::new(0),
            llm_routes: AtomicU64::new(0),
            denied_routes: AtomicU64::new(0),
            fallback_routes: AtomicU64::new(0),
            cumulative_response_time_nanos: AtomicU64::new(0),
            successful_requests: AtomicU64::new(0),
            confidence_state: Mutex::new(ConfidenceState {
                scores: VecDeque::new(),
                max_scores: 1000,
            }),
        }
    }
}

impl std::fmt::Debug for RoutingStatistics {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RoutingStatistics")
            .field("total_requests", &self.total_requests())
            .field("slm_routes", &self.slm_routes())
            .field("llm_routes", &self.llm_routes())
            .field("denied_routes", &self.denied_routes())
            .field("fallback_routes", &self.fallback_routes())
            .field("average_response_time", &self.average_response_time())
            .field("success_rate", &self.success_rate())
            .finish()
    }
}

impl Clone for RoutingStatistics {
    fn clone(&self) -> Self {
        let confidence_state = self.confidence_state.lock().unwrap();
        Self {
            total_requests: AtomicU64::new(self.total_requests.load(Ordering::Relaxed)),
            slm_routes: AtomicU64::new(self.slm_routes.load(Ordering::Relaxed)),
            llm_routes: AtomicU64::new(self.llm_routes.load(Ordering::Relaxed)),
            denied_routes: AtomicU64::new(self.denied_routes.load(Ordering::Relaxed)),
            fallback_routes: AtomicU64::new(self.fallback_routes.load(Ordering::Relaxed)),
            cumulative_response_time_nanos: AtomicU64::new(
                self.cumulative_response_time_nanos.load(Ordering::Relaxed),
            ),
            successful_requests: AtomicU64::new(self.successful_requests.load(Ordering::Relaxed)),
            confidence_state: Mutex::new(ConfidenceState {
                scores: confidence_state.scores.clone(),
                max_scores: confidence_state.max_scores,
            }),
        }
    }
}

impl RoutingContext {
    /// Create a new routing context
    pub fn new(agent_id: AgentId, task_type: TaskType, prompt: String) -> Self {
        Self {
            request_id: uuid::Uuid::new_v4().to_string(),
            agent_id,
            timestamp: chrono::Utc::now(),
            task_type,
            prompt,
            expected_output_type: OutputType::Text,
            max_execution_time: None,
            resource_limits: None,
            agent_capabilities: Vec::new(),
            agent_security_level: SecurityLevel::Medium,
            metadata: HashMap::new(),
        }
    }

    /// Set expected output type
    pub fn with_output_type(mut self, output_type: OutputType) -> Self {
        self.expected_output_type = output_type;
        self
    }

    /// Set resource limits
    pub fn with_resource_limits(mut self, limits: ResourceConstraints) -> Self {
        self.resource_limits = Some(limits);
        self
    }

    /// Set security level
    pub fn with_security_level(mut self, level: SecurityLevel) -> Self {
        self.agent_security_level = level;
        self
    }

    /// Add metadata
    pub fn with_metadata(mut self, key: String, value: String) -> Self {
        self.metadata.insert(key, value);
        self
    }
}

impl ModelRequest {
    /// Create a new model request from a task
    pub fn from_task(prompt: String) -> Self {
        Self {
            prompt,
            parameters: HashMap::new(),
            max_tokens: None,
            temperature: None,
            stop_sequences: None,
        }
    }

    /// Set temperature parameter
    pub fn with_temperature(mut self, temperature: f32) -> Self {
        self.temperature = Some(temperature);
        self
    }

    /// Set max tokens
    pub fn with_max_tokens(mut self, max_tokens: u32) -> Self {
        self.max_tokens = Some(max_tokens);
        self
    }
}

impl RoutingStatistics {
    // --- Accessor methods (lock-free reads) ---

    /// Get total number of requests processed
    pub fn total_requests(&self) -> u64 {
        self.total_requests.load(Ordering::Relaxed)
    }

    /// Get number of requests routed to SLM
    pub fn slm_routes(&self) -> u64 {
        self.slm_routes.load(Ordering::Relaxed)
    }

    /// Get number of requests routed to LLM
    pub fn llm_routes(&self) -> u64 {
        self.llm_routes.load(Ordering::Relaxed)
    }

    /// Get number of denied requests
    pub fn denied_routes(&self) -> u64 {
        self.denied_routes.load(Ordering::Relaxed)
    }

    /// Get number of fallback routes (SLM -> LLM)
    pub fn fallback_routes(&self) -> u64 {
        self.fallback_routes.load(Ordering::Relaxed)
    }

    /// Compute average response time from cumulative nanos and total requests
    pub fn average_response_time(&self) -> Duration {
        let total = self.total_requests.load(Ordering::Relaxed);
        if total == 0 {
            return Duration::ZERO;
        }
        let cumulative = self.cumulative_response_time_nanos.load(Ordering::Relaxed);
        Duration::from_nanos(cumulative / total)
    }

    /// Compute success rate from successful and total requests
    pub fn success_rate(&self) -> f64 {
        let total = self.total_requests.load(Ordering::Relaxed);
        if total == 0 {
            return 0.0;
        }
        let successful = self.successful_requests.load(Ordering::Relaxed);
        successful as f64 / total as f64
    }

    // --- Mutation methods (lock-free atomic increments) ---

    /// Record a completed request, updating all relevant counters atomically
    pub fn record_request(&self, decision: &RouteDecision, response_time: Duration, success: bool) {
        self.total_requests.fetch_add(1, Ordering::Relaxed);

        match decision {
            RouteDecision::UseSLM { .. } => {
                self.slm_routes.fetch_add(1, Ordering::Relaxed);
            }
            RouteDecision::UseLLM { .. } => {
                self.llm_routes.fetch_add(1, Ordering::Relaxed);
            }
            RouteDecision::Deny { .. } => {
                self.denied_routes.fetch_add(1, Ordering::Relaxed);
            }
        }

        // Truncate to u64 nanos (good for ~584 years of cumulative time)
        let nanos = response_time.as_nanos() as u64;
        self.cumulative_response_time_nanos
            .fetch_add(nanos, Ordering::Relaxed);

        if success {
            self.successful_requests.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Record a fallback from SLM to LLM
    pub fn record_fallback(&self) {
        self.fallback_routes.fetch_add(1, Ordering::Relaxed);
    }

    /// Add a confidence score (takes Mutex briefly)
    pub fn add_confidence_score(&self, score: f64) {
        let mut state = self.confidence_state.lock().unwrap();
        state.scores.push_back(score);
        if state.scores.len() > state.max_scores {
            state.scores.pop_front();
        }
    }

    /// Get average confidence score (takes Mutex briefly)
    pub fn average_confidence(&self) -> Option<f64> {
        let state = self.confidence_state.lock().unwrap();
        if state.scores.is_empty() {
            None
        } else {
            Some(state.scores.iter().sum::<f64>() / state.scores.len() as f64)
        }
    }
}
