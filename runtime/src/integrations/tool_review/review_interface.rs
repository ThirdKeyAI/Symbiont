//! Human Review Interface
//! 
//! This module provides a streamlined interface for human operators to review
//! AI security analysis results and make approval/rejection decisions.

use super::types::*;
use crate::integrations::mcp::McpTool;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::time::SystemTime;

/// Trait for human review interface operations
#[async_trait]
pub trait HumanReviewInterface: Send + Sync {
    /// Present analysis results for human review
    async fn present_for_review(&self, session: &ToolReviewSession) -> ToolReviewResult<ReviewPresentation>;
    
    /// Submit human decision
    async fn submit_decision(&self, decision: HumanDecisionInput) -> ToolReviewResult<HumanDecision>;
    
    /// Get pending reviews for an operator
    async fn get_pending_reviews(&self, operator_id: &str) -> ToolReviewResult<Vec<ReviewSummary>>;
    
    /// Get review statistics
    async fn get_review_stats(&self, operator_id: &str) -> ToolReviewResult<ReviewStats>;
}

/// Streamlined presentation of analysis results for human review
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewPresentation {
    /// Review session ID
    pub review_id: ReviewId,
    /// Tool being reviewed
    pub tool_summary: ToolSummary,
    /// Overall risk assessment
    pub risk_assessment: RiskAssessment,
    /// Critical findings requiring attention
    pub critical_findings: Vec<CriticalFinding>,
    /// AI recommendation
    pub ai_recommendation: ReviewRecommendation,
    /// Estimated review time
    pub estimated_review_time_minutes: u32,
    /// Review deadline
    pub review_deadline: SystemTime,
}

/// Simplified tool summary for review
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSummary {
    /// Tool name
    pub name: String,
    /// Tool description
    pub description: String,
    /// Tool provider
    pub provider: String,
    /// Key capabilities
    pub capabilities: Vec<String>,
    /// Parameter count
    pub parameter_count: u32,
    /// Has sensitive parameters
    pub has_sensitive_params: bool,
}

/// Risk assessment summary
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RiskAssessment {
    /// Overall risk level
    pub risk_level: RiskLevel,
    /// Risk score (0.0 to 1.0)
    pub risk_score: f32,
    /// Primary risk categories
    pub primary_risks: Vec<SecurityCategory>,
    /// Confidence in assessment
    pub confidence: f32,
}

/// Risk levels for human review
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum RiskLevel {
    Low,
    Medium,
    High,
    Critical,
}

/// Critical finding for human attention
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CriticalFinding {
    /// Finding title
    pub title: String,
    /// Severity level
    pub severity: SecuritySeverity,
    /// Category
    pub category: SecurityCategory,
    /// Brief description
    pub description: String,
    /// Impact assessment
    pub impact: String,
    /// Recommended action
    pub recommended_action: String,
    /// Confidence level
    pub confidence: f32,
}

/// Human decision input
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HumanDecisionInput {
    /// Review session ID
    pub review_id: ReviewId,
    /// Operator making the decision
    pub operator_id: String,
    /// Decision type
    pub decision: HumanDecisionType,
    /// Reasoning for the decision
    pub reasoning: String,
    /// Additional notes
    pub notes: Option<String>,
    /// Time spent on review (seconds)
    pub time_spent_seconds: u32,
}

/// Summary of a review for operator dashboard
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewSummary {
    /// Review ID
    pub review_id: ReviewId,
    /// Tool name
    pub tool_name: String,
    /// Risk level
    pub risk_level: RiskLevel,
    /// Number of critical findings
    pub critical_findings_count: u32,
    /// AI recommendation
    pub ai_recommendation: ReviewRecommendation,
    /// Time since submitted
    pub submitted_hours_ago: u32,
    /// Review deadline
    pub deadline: SystemTime,
    /// Priority score
    pub priority_score: f32,
}

/// Review statistics for an operator
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewStats {
    /// Total reviews completed
    pub total_reviews: u32,
    /// Reviews completed today
    pub reviews_today: u32,
    /// Average review time (minutes)
    pub avg_review_time_minutes: f32,
    /// Approval rate
    pub approval_rate: f32,
    /// Agreement rate with AI recommendations
    pub ai_agreement_rate: f32,
    /// Pending reviews count
    pub pending_reviews: u32,
}

/// Configuration for the review interface
#[derive(Debug, Clone)]
pub struct ReviewInterfaceConfig {
    /// Maximum findings to show in critical list
    pub max_critical_findings: usize,
    /// Default review time limit (minutes)
    pub default_review_time_minutes: u32,
    /// Auto-escalation threshold (hours)
    pub escalation_threshold_hours: u32,
    /// Show detailed technical information
    pub show_technical_details: bool,
}

impl Default for ReviewInterfaceConfig {
    fn default() -> Self {
        Self {
            max_critical_findings: 5,
            default_review_time_minutes: 30,
            escalation_threshold_hours: 24,
            show_technical_details: false,
        }
    }
}

/// Standard implementation of human review interface
pub struct StandardReviewInterface {
    config: ReviewInterfaceConfig,
}

impl StandardReviewInterface {
    /// Create a new review interface
    pub fn new() -> Self {
        Self {
            config: ReviewInterfaceConfig::default(),
        }
    }
    
    /// Create with custom configuration
    pub fn with_config(config: ReviewInterfaceConfig) -> Self {
        Self { config }
    }
    
    /// Extract tool capabilities from schema
    fn extract_capabilities(&self, tool: &McpTool) -> Vec<String> {
        let mut capabilities = Vec::new();
        
        // Analyze schema to determine capabilities
        if let Some(properties) = tool.schema.get("properties").and_then(|p| p.as_object()) {
            for (param_name, _) in properties {
                let param_lower = param_name.to_lowercase();
                
                if param_lower.contains("file") || param_lower.contains("path") {
                    capabilities.push("File System Access".to_string());
                }
                if param_lower.contains("url") || param_lower.contains("http") {
                    capabilities.push("Network Access".to_string());
                }
                if param_lower.contains("command") || param_lower.contains("exec") {
                    capabilities.push("Command Execution".to_string());
                }
                if param_lower.contains("data") || param_lower.contains("content") {
                    capabilities.push("Data Processing".to_string());
                }
            }
        }
        
        if capabilities.is_empty() {
            capabilities.push("General Purpose".to_string());
        }
        
        capabilities.sort();
        capabilities.dedup();
        capabilities
    }
    
    /// Check if tool has sensitive parameters
    fn has_sensitive_parameters(&self, tool: &McpTool) -> bool {
        if let Some(properties) = tool.schema.get("properties").and_then(|p| p.as_object()) {
            for (param_name, _) in properties {
                let param_lower = param_name.to_lowercase();
                if param_lower.contains("password") 
                    || param_lower.contains("secret") 
                    || param_lower.contains("key") 
                    || param_lower.contains("token") {
                    return true;
                }
            }
        }
        false
    }
    
    /// Convert security findings to critical findings for review
    fn extract_critical_findings(&self, findings: &[SecurityFinding]) -> Vec<CriticalFinding> {
        let mut critical_findings = Vec::new();
        
        for finding in findings {
            // Only include high and critical severity findings
            if matches!(finding.severity, SecuritySeverity::High | SecuritySeverity::Critical) {
                let impact = match finding.category {
                    SecurityCategory::MaliciousCode => "Could execute malicious code on the system".to_string(),
                    SecurityCategory::PrivilegeEscalation => "Could gain unauthorized system privileges".to_string(),
                    SecurityCategory::DataExfiltration => "Could access and steal sensitive data".to_string(),
                    SecurityCategory::SchemaInjection => "Could manipulate tool behavior through injection".to_string(),
                    SecurityCategory::SuspiciousParameters => "Parameters may enable malicious behavior".to_string(),
                    SecurityCategory::UnvalidatedInput => "Could be exploited through malicious input".to_string(),
                    SecurityCategory::InsecureDefaults => "Default configuration may be insecure".to_string(),
                    SecurityCategory::Other(_) => "Could pose security risks to the system".to_string(),
                };
                
                let recommended_action = match finding.severity {
                    SecuritySeverity::Critical => "REJECT - Critical security risk".to_string(),
                    SecuritySeverity::High => "REVIEW CAREFULLY - High security risk".to_string(),
                    _ => "Review and assess risk".to_string(),
                };
                
                critical_findings.push(CriticalFinding {
                    title: finding.title.clone(),
                    severity: finding.severity.clone(),
                    category: finding.category.clone(),
                    description: finding.description.clone(),
                    impact,
                    recommended_action,
                    confidence: finding.confidence,
                });
            }
        }
        
        // Sort by severity and confidence
        critical_findings.sort_by(|a, b| {
            b.severity.cmp(&a.severity)
                .then_with(|| b.confidence.partial_cmp(&a.confidence).unwrap_or(std::cmp::Ordering::Equal))
        });
        
        // Limit to configured maximum
        critical_findings.truncate(self.config.max_critical_findings);
        critical_findings
    }
    
    /// Determine risk level from risk score
    fn determine_risk_level(&self, risk_score: f32) -> RiskLevel {
        if risk_score >= 0.8 {
            RiskLevel::Critical
        } else if risk_score >= 0.6 {
            RiskLevel::High
        } else if risk_score >= 0.3 {
            RiskLevel::Medium
        } else {
            RiskLevel::Low
        }
    }
    
    /// Calculate priority score for review ordering
    fn calculate_priority_score(&self, risk_level: &RiskLevel, hours_since_submission: u32, ai_recommendation: &ReviewRecommendation) -> f32 {
        let mut score = 0.0;
        
        // Risk level weight
        score += match risk_level {
            RiskLevel::Critical => 100.0,
            RiskLevel::High => 75.0,
            RiskLevel::Medium => 50.0,
            RiskLevel::Low => 25.0,
        };
        
        // Time urgency weight
        score += hours_since_submission as f32 * 2.0;
        
        // AI recommendation weight
        score += match ai_recommendation {
            ReviewRecommendation::Reject { .. } => 20.0,
            ReviewRecommendation::RequiresHumanJudgment { .. } => 15.0,
            ReviewRecommendation::Approve { .. } => 5.0,
        };
        
        score
    }
}

impl Default for StandardReviewInterface {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl HumanReviewInterface for StandardReviewInterface {
    async fn present_for_review(&self, session: &ToolReviewSession) -> ToolReviewResult<ReviewPresentation> {
        let analysis = session.security_analysis.as_ref()
            .ok_or_else(|| ToolReviewError::AnalysisFailed("No security analysis available".to_string()))?;
        
        // Extract tool summary
        let capabilities = self.extract_capabilities(&session.tool);
        let parameter_count = session.tool.schema
            .get("properties")
            .and_then(|p| p.as_object())
            .map(|props| props.len() as u32)
            .unwrap_or(0);
        
        let tool_summary = ToolSummary {
            name: session.tool.name.clone(),
            description: session.tool.description.clone(),
            provider: session.tool.provider.name.clone(),
            capabilities,
            parameter_count,
            has_sensitive_params: self.has_sensitive_parameters(&session.tool),
        };
        
        // Create risk assessment
        let risk_level = self.determine_risk_level(analysis.risk_score);
        let primary_risks: Vec<SecurityCategory> = analysis.findings
            .iter()
            .map(|f| f.category.clone())
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();
        
        let risk_assessment = RiskAssessment {
            risk_level: risk_level.clone(),
            risk_score: analysis.risk_score,
            primary_risks,
            confidence: analysis.confidence_score,
        };
        
        // Extract critical findings
        let critical_findings = self.extract_critical_findings(&analysis.findings);
        
        // Determine AI recommendation
        let ai_recommendation = if analysis.risk_score >= 0.8 {
            ReviewRecommendation::Reject {
                confidence: analysis.confidence_score,
                reasoning: "High risk score indicates significant security concerns".to_string(),
            }
        } else if analysis.risk_score >= 0.5 {
            ReviewRecommendation::RequiresHumanJudgment {
                reasoning: "Moderate risk requires human assessment".to_string(),
            }
        } else {
            ReviewRecommendation::Approve {
                confidence: analysis.confidence_score,
                reasoning: "Low risk score indicates tool is likely safe".to_string(),
            }
        };
        
        // Calculate estimated review time
        let estimated_review_time_minutes = match risk_level {
            RiskLevel::Critical => 45,
            RiskLevel::High => 30,
            RiskLevel::Medium => 20,
            RiskLevel::Low => 10,
        };
        
        // Set review deadline
        let review_deadline = SystemTime::now() + std::time::Duration::from_secs(
            self.config.default_review_time_minutes as u64 * 60
        );
        
        Ok(ReviewPresentation {
            review_id: session.review_id,
            tool_summary,
            risk_assessment,
            critical_findings,
            ai_recommendation,
            estimated_review_time_minutes,
            review_deadline,
        })
    }
    
    async fn submit_decision(&self, decision_input: HumanDecisionInput) -> ToolReviewResult<HumanDecision> {
        Ok(HumanDecision {
            decision_id: uuid::Uuid::new_v4().to_string(),
            operator_id: decision_input.operator_id,
            decision: decision_input.decision,
            reasoning: decision_input.reasoning,
            decided_at: SystemTime::now(),
            time_spent_seconds: decision_input.time_spent_seconds,
        })
    }
    
    async fn get_pending_reviews(&self, _operator_id: &str) -> ToolReviewResult<Vec<ReviewSummary>> {
        // In a real implementation, this would query a database
        // For now, return empty list
        Ok(vec![])
    }
    
    async fn get_review_stats(&self, _operator_id: &str) -> ToolReviewResult<ReviewStats> {
        // In a real implementation, this would query a database
        // For now, return mock stats
        Ok(ReviewStats {
            total_reviews: 0,
            reviews_today: 0,
            avg_review_time_minutes: 0.0,
            approval_rate: 0.0,
            ai_agreement_rate: 0.0,
            pending_reviews: 0,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::integrations::mcp::{McpTool, ToolProvider, VerificationStatus};
    use std::collections::HashMap;
    
    fn create_test_session_with_findings(findings: Vec<SecurityFinding>) -> ToolReviewSession {
        let tool = McpTool {
            name: "test_tool".to_string(),
            description: "A test tool for review".to_string(),
            schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "description": "Command to execute"
                    }
                }
            }),
            provider: ToolProvider {
                name: "test-provider".to_string(),
                public_key_url: "https://test.example.com/pubkey".to_string(),
            },
            verification_status: VerificationStatus::Pending,
            metadata: Some(HashMap::new()),
        };
        
        let analysis = SecurityAnalysis {
            analysis_id: AnalysisId::new(),
            tool_id: "test_tool".to_string(),
            analyzed_at: SystemTime::now(),
            analyzer_version: "test-v1.0".to_string(),
            risk_score: 0.7,
            findings,
            recommendations: vec!["Review carefully".to_string()],
            confidence_score: 0.8,
            analysis_metadata: AnalysisMetadata {
                processing_time_ms: 1000,
                rag_queries_performed: 3,
                knowledge_sources_consulted: vec!["test_source".to_string()],
                patterns_matched: vec!["command_parameter".to_string()],
                false_positive_likelihood: 0.1,
            },
        };
        
        ToolReviewSession {
            review_id: ReviewId::new(),
            tool,
            state: ToolReviewState::AwaitingHumanReview {
                analysis_id: analysis.analysis_id,
                analysis_completed_at: SystemTime::now(),
                critical_findings: vec![],
                risk_score: 0.7,
                ai_recommendation: ReviewRecommendation::RequiresHumanJudgment {
                    reasoning: "Test".to_string(),
                },
            },
            security_analysis: Some(analysis),
            human_decisions: vec![],
            audit_trail: vec![],
            created_at: SystemTime::now(),
            updated_at: SystemTime::now(),
        }
    }
    
    #[tokio::test]
    async fn test_review_interface_creation() {
        let interface = StandardReviewInterface::new();
        assert_eq!(interface.config.max_critical_findings, 5);
        assert_eq!(interface.config.default_review_time_minutes, 30);
    }
    
    #[tokio::test]
    async fn test_present_for_review() {
        let interface = StandardReviewInterface::new();
        let findings = vec![
            SecurityFinding {
                finding_id: "TEST_1".to_string(),
                severity: SecuritySeverity::High,
                category: SecurityCategory::SchemaInjection,
                title: "Command Injection Risk".to_string(),
                description: "Tool accepts unvalidated command input".to_string(),
                location: Some("command parameter".to_string()),
                confidence: 0.9,
                remediation_suggestion: Some("Add input validation".to_string()),
                cve_references: vec![],
            }
        ];
        
        let session = create_test_session_with_findings(findings);
        let presentation = interface.present_for_review(&session).await.unwrap();
        
        assert_eq!(presentation.tool_summary.name, "test_tool");
        assert_eq!(presentation.critical_findings.len(), 1);
        assert_eq!(presentation.risk_assessment.risk_level, RiskLevel::High);
    }
    
    #[tokio::test]
    async fn test_submit_decision() {
        let interface = StandardReviewInterface::new();
        let decision_input = HumanDecisionInput {
            review_id: ReviewId::new(),
            operator_id: "test_operator".to_string(),
            decision: HumanDecisionType::Approve,
            reasoning: "Tool appears safe after review".to_string(),
            notes: None,
            time_spent_seconds: 300,
        };
        
        let decision = interface.submit_decision(decision_input).await.unwrap();
        assert_eq!(decision.operator_id, "test_operator");
        assert_eq!(decision.decision, HumanDecisionType::Approve);
        assert_eq!(decision.time_spent_seconds, 300);
    }
    
    #[test]
    fn test_risk_level_determination() {
        let interface = StandardReviewInterface::new();
        
        assert_eq!(interface.determine_risk_level(0.9), RiskLevel::Critical);
        assert_eq!(interface.determine_risk_level(0.7), RiskLevel::High);
        assert_eq!(interface.determine_risk_level(0.4), RiskLevel::Medium);
        assert_eq!(interface.determine_risk_level(0.1), RiskLevel::Low);
    }
}