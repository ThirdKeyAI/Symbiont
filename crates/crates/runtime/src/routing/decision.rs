//! Routing decision types and structures

use serde::{Deserialize, Serialize};
use std::time::Duration;
use std::collections::{HashMap, VecDeque};
use crate::types::AgentId;
use crate::config::ResourceConstraints;
use super::error::TaskType;

/// Routing decision outcome
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RouteDecision {
    /// Route to SLM with specific model
    UseSLM {
        model_id: String,
        monitoring: MonitoringLevel,
        fallback_on_failure: bool,
    },
    /// Route to LLM provider
    UseLLM {
        provider: LLMProvider,
        reason: String,
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
    OpenAI { model: Option<String> },
    Anthropic { model: Option<String> },
    Custom { endpoint: String, model: Option<String> },
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

/// Routing statistics for monitoring with improved precision and memory efficiency
#[derive(Debug, Clone)]
pub struct RoutingStatistics {
    pub total_requests: u64,
    pub slm_routes: u64,
    pub llm_routes: u64,
    pub denied_routes: u64,
    pub fallback_routes: u64,
    /// Cumulative response time in nanoseconds for precise averaging
    cumulative_response_time_nanos: u128,
    /// Average response time calculated from cumulative time
    pub average_response_time: Duration,
    pub success_rate: f64,
    /// Circular buffer for confidence scores (memory efficient)
    confidence_scores: VecDeque<f64>,
    /// Maximum confidence scores to retain
    max_confidence_scores: usize,
}

impl Default for RoutingStatistics {
    fn default() -> Self {
        Self {
            total_requests: 0,
            slm_routes: 0,
            llm_routes: 0,
            denied_routes: 0,
            fallback_routes: 0,
            cumulative_response_time_nanos: 0,
            average_response_time: Duration::from_millis(0),
            success_rate: 0.0,
            confidence_scores: VecDeque::new(),
            max_confidence_scores: 1000,
        }
    }
}

impl RoutingContext {
    /// Create a new routing context
    pub fn new(
        agent_id: AgentId,
        task_type: TaskType,
        prompt: String,
    ) -> Self {
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
    /// Update statistics with a new routing decision
    pub fn update(&mut self, decision: &RouteDecision, response_time: Duration, success: bool) {
        self.total_requests += 1;
        
        match decision {
            RouteDecision::UseSLM { .. } => self.slm_routes += 1,
            RouteDecision::UseLLM { .. } => self.llm_routes += 1,
            RouteDecision::Deny { .. } => self.denied_routes += 1,
        }
        
        // Update cumulative response time and calculate average
        self.cumulative_response_time_nanos += response_time.as_nanos();
        self.average_response_time = Duration::from_nanos(
            (self.cumulative_response_time_nanos / self.total_requests as u128) as u64
        );
        
        // Update success rate
        let successful_requests = if success { 1 } else { 0 };
        self.success_rate = (self.success_rate * (self.total_requests - 1) as f64 + successful_requests as f64) / self.total_requests as f64;
    }
    
    /// Add confidence score to statistics
    pub fn add_confidence_score(&mut self, score: f64) {
        self.confidence_scores.push_back(score);
        // Keep only last 1000 scores to prevent unbounded growth
        if self.confidence_scores.len() > self.max_confidence_scores {
            self.confidence_scores.pop_front();
        }
    }
    
    /// Get average confidence score
    pub fn average_confidence(&self) -> Option<f64> {
        if self.confidence_scores.is_empty() {
            None
        } else {
            Some(self.confidence_scores.iter().sum::<f64>() / self.confidence_scores.len() as f64)
        }
    }
}