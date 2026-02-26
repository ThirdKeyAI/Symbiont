//! Declarative pipeline configuration
//!
//! Supports TOML-based pipeline definitions for director-critic patterns
//! and other multi-agent workflows. Enterprise admins define mandatory
//! quality gates that policies can reference.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Top-level pipeline configuration, typically loaded from TOML.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineConfig {
    /// Named pipeline definitions.
    pub pipeline: HashMap<String, PipelineDefinition>,
}

/// A single pipeline definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineDefinition {
    /// Pipeline type (e.g., "director_critic", "chain", "map_reduce").
    #[serde(rename = "type")]
    pub pipeline_type: String,

    /// Director/orchestrator configuration.
    #[serde(default)]
    pub director: Option<DirectorConfig>,

    /// Critic configuration.
    #[serde(default)]
    pub critic: Option<CriticConfig>,

    /// Convergence criteria.
    #[serde(default)]
    pub convergence: Option<ConvergenceConfig>,

    /// Chain steps (for chain-type pipelines).
    #[serde(default)]
    pub steps: Vec<StepConfig>,
}

/// Director/orchestrator model configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirectorConfig {
    /// Model to use (e.g., "slm", "claude-sonnet", "gpt-4").
    pub model: String,

    /// Temperature for generation.
    #[serde(default = "default_temperature")]
    pub temperature: f64,

    /// System prompt override.
    #[serde(default)]
    pub system_prompt: Option<String>,

    /// Maximum tokens for each director response.
    #[serde(default)]
    pub max_tokens: Option<u32>,
}

/// Critic evaluation configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CriticConfig {
    /// Model to use for critique.
    pub model: String,

    /// Evaluation mode: "binary", "score", or "rubric".
    #[serde(default = "default_evaluation_mode")]
    pub evaluation_mode: EvaluationMode,

    /// Score threshold for approval (0.0 - 1.0).
    #[serde(default = "default_threshold")]
    pub threshold: f64,

    /// Rubric for multi-dimension evaluation.
    #[serde(default)]
    pub rubric: HashMap<String, RubricDimension>,

    /// System prompt for the critic.
    #[serde(default)]
    pub system_prompt: Option<String>,
}

/// Evaluation mode for critics.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EvaluationMode {
    /// Simple approve/reject.
    Binary,
    /// Single numeric score.
    Score,
    /// Multi-dimension rubric with weights.
    Rubric,
}

/// A single dimension in a rubric-based evaluation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RubricDimension {
    /// Weight of this dimension in the overall score.
    pub weight: f64,

    /// Description of what this dimension evaluates.
    #[serde(default)]
    pub description: Option<String>,
}

/// Convergence criteria for iterative patterns.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ConvergenceConfig {
    /// Run for exactly N rounds.
    FixedRounds { rounds: u32 },

    /// Run until improvement drops below threshold, with min/max bounds.
    AdaptiveBreak {
        min_rounds: u32,
        max_rounds: u32,
        improvement_threshold: f64,
    },

    /// Run until a score threshold is met.
    ScoreThreshold { target_score: f64, max_rounds: u32 },
}

/// A step in a chain-type pipeline.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepConfig {
    /// Step name.
    pub name: String,

    /// Model to use for this step.
    pub model: String,

    /// System prompt for this step.
    #[serde(default)]
    pub system_prompt: Option<String>,

    /// Output format: "text", "json", or a schema name.
    #[serde(default)]
    pub output_format: Option<String>,
}

fn default_temperature() -> f64 {
    0.7
}

fn default_evaluation_mode() -> EvaluationMode {
    EvaluationMode::Score
}

fn default_threshold() -> f64 {
    0.7
}

impl PipelineConfig {
    /// Parse a pipeline configuration from TOML.
    pub fn from_toml(toml_str: &str) -> Result<Self, PipelineConfigError> {
        toml::from_str(toml_str).map_err(|e| PipelineConfigError::ParseError {
            message: e.to_string(),
        })
    }

    /// Serialize to TOML.
    pub fn to_toml(&self) -> Result<String, PipelineConfigError> {
        toml::to_string_pretty(self).map_err(|e| PipelineConfigError::SerializeError {
            message: e.to_string(),
        })
    }

    /// Get a pipeline definition by name.
    pub fn get_pipeline(&self, name: &str) -> Option<&PipelineDefinition> {
        self.pipeline.get(name)
    }

    /// Validate the configuration.
    pub fn validate(&self) -> Result<(), PipelineConfigError> {
        for (name, pipeline) in &self.pipeline {
            // Validate critic threshold
            if let Some(critic) = &pipeline.critic {
                if !(0.0..=1.0).contains(&critic.threshold) {
                    return Err(PipelineConfigError::ValidationError {
                        field: format!("pipeline.{}.critic.threshold", name),
                        message: "Threshold must be between 0.0 and 1.0".into(),
                    });
                }

                // Validate rubric weights sum to ~1.0 if rubric mode
                if critic.evaluation_mode == EvaluationMode::Rubric && !critic.rubric.is_empty() {
                    let total_weight: f64 = critic.rubric.values().map(|d| d.weight).sum();
                    if (total_weight - 1.0).abs() > 0.01 {
                        return Err(PipelineConfigError::ValidationError {
                            field: format!("pipeline.{}.critic.rubric", name),
                            message: format!(
                                "Rubric weights must sum to 1.0, got {}",
                                total_weight
                            ),
                        });
                    }
                }
            }

            // Validate convergence bounds
            if let Some(ConvergenceConfig::AdaptiveBreak {
                min_rounds,
                max_rounds,
                ..
            }) = &pipeline.convergence
            {
                if min_rounds > max_rounds {
                    return Err(PipelineConfigError::ValidationError {
                        field: format!("pipeline.{}.convergence", name),
                        message: "min_rounds must be <= max_rounds".into(),
                    });
                }
            }
        }

        Ok(())
    }
}

/// Errors from pipeline configuration.
#[derive(Debug, thiserror::Error)]
pub enum PipelineConfigError {
    #[error("TOML parse error: {message}")]
    ParseError { message: String },

    #[error("TOML serialization error: {message}")]
    SerializeError { message: String },

    #[error("Validation error in '{field}': {message}")]
    ValidationError { field: String, message: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_director_critic_pipeline() {
        let toml = r#"
[pipeline.compliance_review]
type = "director_critic"

[pipeline.compliance_review.director]
model = "slm"
temperature = 0.7

[pipeline.compliance_review.critic]
model = "claude-sonnet"
evaluation_mode = "rubric"
threshold = 0.85

[pipeline.compliance_review.critic.rubric.accuracy]
weight = 0.4

[pipeline.compliance_review.critic.rubric.compliance]
weight = 0.3

[pipeline.compliance_review.critic.rubric.completeness]
weight = 0.3

[pipeline.compliance_review.convergence]
type = "adaptive_break"
min_rounds = 1
max_rounds = 3
improvement_threshold = 0.05
"#;

        let config = PipelineConfig::from_toml(toml).unwrap();
        assert!(config.validate().is_ok());

        let pipeline = config.get_pipeline("compliance_review").unwrap();
        assert_eq!(pipeline.pipeline_type, "director_critic");

        let critic = pipeline.critic.as_ref().unwrap();
        assert_eq!(critic.model, "claude-sonnet");
        assert_eq!(critic.evaluation_mode, EvaluationMode::Rubric);
        assert_eq!(critic.rubric.len(), 3);

        let director = pipeline.director.as_ref().unwrap();
        assert_eq!(director.model, "slm");
        assert!((director.temperature - 0.7).abs() < f64::EPSILON);
    }

    #[test]
    fn test_parse_fixed_rounds_convergence() {
        let toml = r#"
[pipeline.simple]
type = "director_critic"

[pipeline.simple.convergence]
type = "fixed_rounds"
rounds = 3
"#;

        let config = PipelineConfig::from_toml(toml).unwrap();
        let pipeline = config.get_pipeline("simple").unwrap();
        match pipeline.convergence.as_ref().unwrap() {
            ConvergenceConfig::FixedRounds { rounds } => assert_eq!(*rounds, 3),
            _ => panic!("Expected FixedRounds"),
        }
    }

    #[test]
    fn test_parse_score_threshold_convergence() {
        let toml = r#"
[pipeline.quality]
type = "director_critic"

[pipeline.quality.convergence]
type = "score_threshold"
target_score = 0.9
max_rounds = 5
"#;

        let config = PipelineConfig::from_toml(toml).unwrap();
        let pipeline = config.get_pipeline("quality").unwrap();
        match pipeline.convergence.as_ref().unwrap() {
            ConvergenceConfig::ScoreThreshold {
                target_score,
                max_rounds,
            } => {
                assert!((target_score - 0.9).abs() < f64::EPSILON);
                assert_eq!(*max_rounds, 5);
            }
            _ => panic!("Expected ScoreThreshold"),
        }
    }

    #[test]
    fn test_validate_invalid_threshold() {
        let toml = r#"
[pipeline.bad]
type = "director_critic"

[pipeline.bad.critic]
model = "test"
threshold = 1.5
"#;

        let config = PipelineConfig::from_toml(toml).unwrap();
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_validate_rubric_weights() {
        let toml = r#"
[pipeline.bad]
type = "director_critic"

[pipeline.bad.critic]
model = "test"
evaluation_mode = "rubric"
threshold = 0.5

[pipeline.bad.critic.rubric.a]
weight = 0.3

[pipeline.bad.critic.rubric.b]
weight = 0.3
"#;

        let config = PipelineConfig::from_toml(toml).unwrap();
        let err = config.validate().unwrap_err();
        assert!(err.to_string().contains("sum to 1.0"));
    }

    #[test]
    fn test_validate_convergence_bounds() {
        let toml = r#"
[pipeline.bad]
type = "director_critic"

[pipeline.bad.convergence]
type = "adaptive_break"
min_rounds = 5
max_rounds = 3
improvement_threshold = 0.05
"#;

        let config = PipelineConfig::from_toml(toml).unwrap();
        let err = config.validate().unwrap_err();
        assert!(err.to_string().contains("min_rounds"));
    }

    #[test]
    fn test_roundtrip_serialization() {
        let toml = r#"
[pipeline.test]
type = "chain"

[[pipeline.test.steps]]
name = "summarize"
model = "slm"

[[pipeline.test.steps]]
name = "refine"
model = "claude-sonnet"
"#;

        let config = PipelineConfig::from_toml(toml).unwrap();
        let serialized = config.to_toml().unwrap();
        let restored = PipelineConfig::from_toml(&serialized).unwrap();

        let pipeline = restored.get_pipeline("test").unwrap();
        assert_eq!(pipeline.steps.len(), 2);
        assert_eq!(pipeline.steps[0].name, "summarize");
        assert_eq!(pipeline.steps[1].name, "refine");
    }

    #[test]
    fn test_default_values() {
        let toml = r#"
[pipeline.minimal]
type = "director_critic"

[pipeline.minimal.director]
model = "slm"

[pipeline.minimal.critic]
model = "test"
"#;

        let config = PipelineConfig::from_toml(toml).unwrap();
        let pipeline = config.get_pipeline("minimal").unwrap();

        let director = pipeline.director.as_ref().unwrap();
        assert!((director.temperature - 0.7).abs() < f64::EPSILON);

        let critic = pipeline.critic.as_ref().unwrap();
        assert_eq!(critic.evaluation_mode, EvaluationMode::Score);
        assert!((critic.threshold - 0.7).abs() < f64::EPSILON);
    }
}
