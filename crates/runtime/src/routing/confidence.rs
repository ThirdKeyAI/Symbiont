//! Confidence monitoring trait and types
//!
//! Defines `ConfidenceMonitorTrait` for evaluating SLM response quality
//! and `NoOpConfidenceMonitor` as the default pass-through implementation.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::fmt;

use super::decision::{ModelRequest, ModelResponse};
use crate::context::AgentContext as Context;

/// Result of confidence evaluation
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct ConfidenceEvaluation {
    /// Whether the evaluation meets the confidence threshold
    pub meets_threshold: bool,
    /// Recommendation based on confidence analysis
    pub recommendation: ConfidenceRecommendation,
}

/// Recommendation based on confidence evaluation
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConfidenceRecommendation {
    #[default]
    UseResponse,
    FallbackToLLM,
}

/// Errors that can occur during confidence evaluation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ConfidenceError {
    EvaluationFailed,
}

impl fmt::Display for ConfidenceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConfidenceError::EvaluationFailed => write!(f, "Confidence evaluation failed"),
        }
    }
}

impl std::error::Error for ConfidenceError {}

/// Trait for confidence monitoring implementations
#[async_trait]
pub trait ConfidenceMonitorTrait: Send + Sync {
    /// Evaluates the confidence of a model's response.
    async fn evaluate(
        &self,
        _context: &Context,
        _request: &ModelRequest,
        _response: &ModelResponse,
    ) -> Result<ConfidenceEvaluation, ConfidenceError>;
}

/// No-operation confidence monitor implementation for trait usage
#[derive(Debug, Default, Clone)]
pub struct NoOpConfidenceMonitor;

#[async_trait]
impl ConfidenceMonitorTrait for NoOpConfidenceMonitor {
    async fn evaluate(
        &self,
        _context: &Context,
        _request: &ModelRequest,
        _response: &ModelResponse,
    ) -> Result<ConfidenceEvaluation, ConfidenceError> {
        Ok(ConfidenceEvaluation {
            meets_threshold: true,
            recommendation: ConfidenceRecommendation::UseResponse,
        })
    }
}
