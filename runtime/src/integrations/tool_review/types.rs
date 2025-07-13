//! Tool Review Workflow Types
//! 
//! Data structures and enums for the AI-driven tool review and signing workflow.

use crate::integrations::mcp::McpTool;
use crate::integrations::schemapin::SignatureInfo;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::SystemTime;
use uuid::Uuid;

/// Unique identifier for tool review sessions
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ReviewId(pub Uuid);

impl Default for ReviewId {
    fn default() -> Self {
        Self::new()
    }
}

impl ReviewId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

/// Unique identifier for security analyses
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AnalysisId(pub Uuid);

impl Default for AnalysisId {
    fn default() -> Self {
        Self::new()
    }
}

impl AnalysisId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

/// Tool review workflow states
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ToolReviewState {
    /// Tool submitted and waiting for analysis
    PendingReview {
        submitted_at: SystemTime,
        submitted_by: String,
    },
    /// AI analysis in progress
    UnderReview {
        started_at: SystemTime,
        analyzer_id: String,
        analysis_id: AnalysisId,
    },
    /// Waiting for human operator decision
    AwaitingHumanReview {
        analysis_id: AnalysisId,
        analysis_completed_at: SystemTime,
        critical_findings: Vec<SecurityFinding>,
        risk_score: f32,
        ai_recommendation: ReviewRecommendation,
    },
    /// Tool approved by human operator
    Approved {
        approved_by: String,
        approved_at: SystemTime,
        approval_notes: Option<String>,
    },
    /// Tool rejected by human operator
    Rejected {
        rejected_by: String,
        rejected_at: SystemTime,
        rejection_reason: String,
    },
    /// Tool successfully signed
    Signed {
        signature_info: SignatureInfo,
        signed_at: SystemTime,
        signed_by: String,
    },
    /// Signing failed
    SigningFailed {
        error: String,
        failed_at: SystemTime,
        retry_count: u32,
    },
}

/// AI recommendation for tool approval
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ReviewRecommendation {
    Approve { confidence: f32, reasoning: String },
    Reject { confidence: f32, reasoning: String },
    RequiresHumanJudgment { reasoning: String },
}

/// Security finding from AI analysis
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SecurityFinding {
    pub finding_id: String,
    pub severity: SecuritySeverity,
    pub category: SecurityCategory,
    pub title: String,
    pub description: String,
    pub location: Option<String>,
    pub confidence: f32,
    pub remediation_suggestion: Option<String>,
    pub cve_references: Vec<String>,
}

/// Security severity levels
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SecuritySeverity {
    Low,
    Medium,
    High,
    Critical,
}

/// Security finding categories
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum SecurityCategory {
    SchemaInjection,
    PrivilegeEscalation,
    DataExfiltration,
    MaliciousCode,
    SuspiciousParameters,
    UnvalidatedInput,
    InsecureDefaults,
    Other(String),
}

/// Complete security analysis result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecurityAnalysis {
    pub analysis_id: AnalysisId,
    pub tool_id: String,
    pub analyzed_at: SystemTime,
    pub analyzer_version: String,
    pub risk_score: f32,
    pub findings: Vec<SecurityFinding>,
    pub recommendations: Vec<String>,
    pub confidence_score: f32,
    pub analysis_metadata: AnalysisMetadata,
}

/// Metadata about the analysis process
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalysisMetadata {
    pub processing_time_ms: u64,
    pub rag_queries_performed: u32,
    pub knowledge_sources_consulted: Vec<String>,
    pub patterns_matched: Vec<String>,
    pub false_positive_likelihood: f32,
}

/// Tool review session containing all workflow data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolReviewSession {
    pub review_id: ReviewId,
    pub tool: McpTool,
    pub state: ToolReviewState,
    pub security_analysis: Option<SecurityAnalysis>,
    pub human_decisions: Vec<HumanDecision>,
    pub audit_trail: Vec<AuditEvent>,
    pub created_at: SystemTime,
    pub updated_at: SystemTime,
}

/// Human operator decision record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HumanDecision {
    pub decision_id: String,
    pub operator_id: String,
    pub decision: HumanDecisionType,
    pub reasoning: String,
    pub decided_at: SystemTime,
    pub time_spent_seconds: u32,
}

/// Types of human decisions
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum HumanDecisionType {
    Approve,
    Reject,
    RequestReanalysis,
    EscalateToSenior,
}

/// Audit event for traceability
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEvent {
    pub event_id: String,
    pub event_type: AuditEventType,
    pub timestamp: SystemTime,
    pub actor: String,
    pub details: HashMap<String, serde_json::Value>,
}

/// Types of audit events
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum AuditEventType {
    ToolSubmitted,
    AnalysisStarted,
    AnalysisCompleted,
    HumanReviewStarted,
    HumanDecisionMade,
    SigningStarted,
    SigningCompleted,
    SigningFailed,
    StateTransition,
}

/// Configuration for the tool review workflow
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolReviewConfig {
    pub max_analysis_time_seconds: u64,
    pub max_human_review_time_seconds: u64,
    pub auto_approve_threshold: f32,
    pub auto_reject_threshold: f32,
    pub require_human_review_for_high_risk: bool,
    pub max_signing_retries: u32,
    pub security_knowledge_sources: Vec<String>,
}

impl Default for ToolReviewConfig {
    fn default() -> Self {
        Self {
            max_analysis_time_seconds: 300, // 5 minutes
            max_human_review_time_seconds: 3600, // 1 hour
            auto_approve_threshold: 0.9,
            auto_reject_threshold: 0.1,
            require_human_review_for_high_risk: true,
            max_signing_retries: 3,
            security_knowledge_sources: vec![
                "cve_database".to_string(),
                "malware_signatures".to_string(),
                "vulnerability_patterns".to_string(),
            ],
        }
    }
}

/// Errors that can occur during tool review workflow
#[derive(Debug, thiserror::Error)]
pub enum ToolReviewError {
    #[error("Analysis failed: {0}")]
    AnalysisFailed(String),

    #[error("Invalid state transition from {from:?} to {to:?}")]
    InvalidStateTransition {
        from: ToolReviewState,
        to: ToolReviewState,
    },

    #[error("Review session not found: {review_id:?}")]
    ReviewSessionNotFound { review_id: ReviewId },

    #[error("Analysis timeout after {seconds} seconds")]
    AnalysisTimeout { seconds: u64 },

    #[error("Human review timeout after {seconds} seconds")]
    HumanReviewTimeout { seconds: u64 },

    #[error("Signing failed: {reason}")]
    SigningFailed { reason: String },

    #[error("Configuration error: {0}")]
    ConfigurationError(String),

    #[error("Security analyzer error: {0}")]
    SecurityAnalyzerError(String),

    #[error("RAG engine error: {0}")]
    RAGEngineError(String),

    #[error("SchemaPin error: {0}")]
    SchemaPinError(String),

    #[error("Serialization error: {0}")]
    SerializationError(String),

    #[error("Database error: {0}")]
    DatabaseError(String),
}

/// Result type for tool review operations
pub type ToolReviewResult<T> = Result<T, ToolReviewError>;

/// Statistics for the tool review system
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolReviewStats {
    pub total_reviews: u64,
    pub approved_tools: u64,
    pub rejected_tools: u64,
    pub signed_tools: u64,
    pub avg_analysis_time_ms: u64,
    pub avg_human_review_time_ms: u64,
    pub auto_approval_rate: f32,
    pub false_positive_rate: f32,
    pub top_security_categories: Vec<(SecurityCategory, u32)>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_review_id_generation() {
        let id1 = ReviewId::new();
        let id2 = ReviewId::new();
        assert_ne!(id1, id2);
    }

    #[test]
    fn test_analysis_id_generation() {
        let id1 = AnalysisId::new();
        let id2 = AnalysisId::new();
        assert_ne!(id1, id2);
    }

    #[test]
    fn test_security_severity_ordering() {
        assert!(SecuritySeverity::Critical > SecuritySeverity::High);
        assert!(SecuritySeverity::High > SecuritySeverity::Medium);
        assert!(SecuritySeverity::Medium > SecuritySeverity::Low);
    }

    #[test]
    fn test_default_config() {
        let config = ToolReviewConfig::default();
        assert_eq!(config.max_analysis_time_seconds, 300);
        assert_eq!(config.auto_approve_threshold, 0.9);
        assert!(config.require_human_review_for_high_risk);
    }
}