//! Human-in-the-loop critic
//!
//! Provides a `HumanCritic` that suspends the reasoning loop to wait for
//! human approval/rejection. Unifies automated and manual review under
//! a single auditable abstraction.

use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};

/// Result of a critic evaluation (automated or human).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CriticResult {
    /// Whether the content was approved.
    pub approved: bool,
    /// Overall score (0.0 - 1.0).
    pub score: f64,
    /// Per-dimension scores for rubric evaluations.
    pub dimension_scores: std::collections::HashMap<String, f64>,
    /// Free-text feedback.
    pub feedback: String,
    /// Identity of the reviewer.
    pub reviewer: ReviewerIdentity,
}

/// Identity of who performed the review.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ReviewerIdentity {
    /// An LLM model performed the review.
    #[serde(rename = "llm")]
    Llm { model_id: String },
    /// A human performed the review.
    #[serde(rename = "human")]
    Human { user_id: String, name: String },
}

/// A review request sent to human reviewers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewRequest {
    /// Unique identifier for this review.
    pub review_id: String,
    /// Content to be reviewed.
    pub content: String,
    /// Context about what produced this content.
    pub context: String,
    /// Rubric dimensions to evaluate (if any).
    pub rubric_dimensions: Vec<String>,
    /// When this review was requested.
    pub requested_at: chrono::DateTime<chrono::Utc>,
    /// Deadline for the review.
    pub deadline: chrono::DateTime<chrono::Utc>,
}

/// A review response from a human reviewer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewResponse {
    /// The review request ID this responds to.
    pub review_id: String,
    /// The critic result.
    pub result: CriticResult,
}

/// The Critic trait, implementable by both automated and human reviewers.
#[async_trait::async_trait]
pub trait Critic: Send + Sync {
    /// Evaluate content and return a critic result.
    async fn evaluate(&self, content: &str, context: &str) -> Result<CriticResult, CriticError>;

    /// Returns the type of critic for logging.
    fn critic_type(&self) -> &str;
}

/// Human critic that sends reviews to a channel and waits for responses.
pub struct HumanCritic {
    /// Channel to send review requests out (to webhook, UI, etc.).
    review_sender: mpsc::Sender<(ReviewRequest, oneshot::Sender<ReviewResponse>)>,
    /// Default timeout for human reviews.
    timeout: Duration,
    /// Default reviewer identity.
    default_reviewer: String,
}

impl HumanCritic {
    /// Create a new human critic.
    ///
    /// Returns the critic and a receiver that dispatches review requests
    /// to whatever transport the application uses (webhook, websocket, etc.).
    pub fn new(
        timeout: Duration,
    ) -> (
        Self,
        mpsc::Receiver<(ReviewRequest, oneshot::Sender<ReviewResponse>)>,
    ) {
        let (tx, rx) = mpsc::channel(32);
        let critic = Self {
            review_sender: tx,
            timeout,
            default_reviewer: "human".into(),
        };
        (critic, rx)
    }

    /// Set the default reviewer name.
    pub fn with_reviewer_name(mut self, name: impl Into<String>) -> Self {
        self.default_reviewer = name.into();
        self
    }
}

#[async_trait::async_trait]
impl Critic for HumanCritic {
    async fn evaluate(&self, content: &str, context: &str) -> Result<CriticResult, CriticError> {
        let review_id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now();

        let request = ReviewRequest {
            review_id: review_id.clone(),
            content: content.to_string(),
            context: context.to_string(),
            rubric_dimensions: Vec::new(),
            requested_at: now,
            deadline: now + chrono::Duration::from_std(self.timeout).unwrap_or_default(),
        };

        let (response_tx, response_rx) = oneshot::channel();

        // Send the review request
        self.review_sender
            .send((request, response_tx))
            .await
            .map_err(|_| CriticError::ChannelClosed)?;

        // Wait for the response with timeout
        match tokio::time::timeout(self.timeout, response_rx).await {
            Ok(Ok(response)) => Ok(response.result),
            Ok(Err(_)) => Err(CriticError::ChannelClosed),
            Err(_) => Err(CriticError::Timeout {
                review_id,
                timeout: self.timeout,
            }),
        }
    }

    fn critic_type(&self) -> &str {
        "human"
    }
}

/// An automated LLM-based critic.
pub struct LlmCritic {
    /// The inference provider to use.
    provider: Arc<dyn crate::reasoning::inference::InferenceProvider>,
    /// System prompt for the critic.
    system_prompt: String,
    /// Model ID for identification.
    model_id: String,
}

impl LlmCritic {
    /// Create a new LLM critic.
    pub fn new(
        provider: Arc<dyn crate::reasoning::inference::InferenceProvider>,
        system_prompt: impl Into<String>,
    ) -> Self {
        let model_id = provider.default_model().to_string();
        Self {
            provider,
            system_prompt: system_prompt.into(),
            model_id,
        }
    }
}

#[async_trait::async_trait]
impl Critic for LlmCritic {
    async fn evaluate(&self, content: &str, context: &str) -> Result<CriticResult, CriticError> {
        use crate::reasoning::conversation::{Conversation, ConversationMessage};
        use crate::reasoning::inference::{InferenceOptions, ResponseFormat};

        let mut conv = Conversation::with_system(&self.system_prompt);
        conv.push(ConversationMessage::user(format!(
            "Context: {}\n\nContent to review:\n{}",
            context, content
        )));

        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "approved": {"type": "boolean"},
                "score": {"type": "number", "minimum": 0.0, "maximum": 1.0},
                "feedback": {"type": "string"}
            },
            "required": ["approved", "score", "feedback"]
        });

        let options = InferenceOptions {
            response_format: ResponseFormat::JsonSchema {
                schema,
                name: Some("critic_evaluation".into()),
            },
            ..InferenceOptions::default()
        };

        let response = self.provider.complete(&conv, &options).await.map_err(|e| {
            CriticError::InferenceError {
                message: e.to_string(),
            }
        })?;

        // Parse the structured response
        let parsed: serde_json::Value =
            serde_json::from_str(&response.content).map_err(|e| CriticError::ParseError {
                message: e.to_string(),
            })?;

        Ok(CriticResult {
            approved: parsed["approved"].as_bool().unwrap_or(false),
            score: parsed["score"].as_f64().unwrap_or(0.0),
            dimension_scores: std::collections::HashMap::new(),
            feedback: parsed["feedback"].as_str().unwrap_or("").to_string(),
            reviewer: ReviewerIdentity::Llm {
                model_id: self.model_id.clone(),
            },
        })
    }

    fn critic_type(&self) -> &str {
        "llm"
    }
}

/// Errors from the critic system.
#[derive(Debug, thiserror::Error)]
pub enum CriticError {
    #[error("Review timed out (review_id={review_id}, timeout={timeout:?})")]
    Timeout {
        review_id: String,
        timeout: Duration,
    },

    #[error("Review channel closed")]
    ChannelClosed,

    #[error("Inference error: {message}")]
    InferenceError { message: String },

    #[error("Failed to parse critic response: {message}")]
    ParseError { message: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_critic_result_serde() {
        let result = CriticResult {
            approved: true,
            score: 0.85,
            dimension_scores: {
                let mut m = std::collections::HashMap::new();
                m.insert("accuracy".into(), 0.9);
                m.insert("clarity".into(), 0.8);
                m
            },
            feedback: "Good analysis.".into(),
            reviewer: ReviewerIdentity::Llm {
                model_id: "claude-sonnet".into(),
            },
        };

        let json = serde_json::to_string(&result).unwrap();
        let restored: CriticResult = serde_json::from_str(&json).unwrap();
        assert!(restored.approved);
        assert!((restored.score - 0.85).abs() < f64::EPSILON);
        assert_eq!(restored.dimension_scores.len(), 2);
    }

    #[test]
    fn test_review_request_serde() {
        let request = ReviewRequest {
            review_id: "test-123".into(),
            content: "Content to review".into(),
            context: "Generated by agent X".into(),
            rubric_dimensions: vec!["accuracy".into(), "completeness".into()],
            requested_at: chrono::Utc::now(),
            deadline: chrono::Utc::now() + chrono::Duration::minutes(5),
        };

        let json = serde_json::to_string(&request).unwrap();
        let restored: ReviewRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.review_id, "test-123");
        assert_eq!(restored.rubric_dimensions.len(), 2);
    }

    #[test]
    fn test_reviewer_identity_serde() {
        let llm = ReviewerIdentity::Llm {
            model_id: "gpt-4".into(),
        };
        let json = serde_json::to_string(&llm).unwrap();
        assert!(json.contains("\"type\":\"llm\""));

        let human = ReviewerIdentity::Human {
            user_id: "user-1".into(),
            name: "Alice".into(),
        };
        let json = serde_json::to_string(&human).unwrap();
        assert!(json.contains("\"type\":\"human\""));
    }

    #[tokio::test]
    async fn test_human_critic_timeout() {
        let (critic, _rx) = HumanCritic::new(Duration::from_millis(50));
        // Don't send a response — should timeout
        let result = critic.evaluate("test content", "test context").await;
        assert!(result.is_err());
        match result.unwrap_err() {
            CriticError::Timeout { .. } => {}
            other => panic!("Expected Timeout, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_human_critic_response() {
        let (critic, mut rx) = HumanCritic::new(Duration::from_secs(5));

        // Spawn a task to respond to the review
        tokio::spawn(async move {
            if let Some((request, response_tx)) = rx.recv().await {
                let _ = response_tx.send(ReviewResponse {
                    review_id: request.review_id,
                    result: CriticResult {
                        approved: true,
                        score: 0.9,
                        dimension_scores: std::collections::HashMap::new(),
                        feedback: "Looks good!".into(),
                        reviewer: ReviewerIdentity::Human {
                            user_id: "tester".into(),
                            name: "Test User".into(),
                        },
                    },
                });
            }
        });

        let result = critic
            .evaluate("test content", "test context")
            .await
            .unwrap();
        assert!(result.approved);
        assert!((result.score - 0.9).abs() < f64::EPSILON);
        assert_eq!(result.feedback, "Looks good!");
    }

    #[tokio::test]
    async fn test_human_critic_channel_closed() {
        let (critic, rx) = HumanCritic::new(Duration::from_secs(5));
        drop(rx); // Close the receiver

        let result = critic.evaluate("test", "context").await;
        assert!(matches!(result.unwrap_err(), CriticError::ChannelClosed));
    }
}
