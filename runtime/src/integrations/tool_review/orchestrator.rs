//! Tool Review Workflow Orchestrator
//! 
//! Central coordinator for the AI-driven tool review and signing workflow.
//! Manages state transitions, coordinates between components, and ensures proper workflow execution.

use crate::integrations::mcp::{McpClient, McpTool};
use crate::integrations::schemapin::{SchemaPinCli, SignArgs};
use super::analyzer::SecurityAnalyzer;
use super::review_interface::HumanReviewInterface;
use super::types::*;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, Duration};
use tokio::sync::{RwLock, Mutex};
use tokio::time::{sleep, timeout};
use uuid::Uuid;

/// Central orchestrator for the tool review workflow
pub struct ToolReviewOrchestrator {
    /// Active review sessions
    sessions: Arc<RwLock<HashMap<ReviewId, ToolReviewSession>>>,
    /// AI security analyzer
    analyzer: Arc<dyn SecurityAnalyzer>,
    /// Human review interface
    review_interface: Arc<dyn HumanReviewInterface>,
    /// SchemaPin CLI for signing
    schemapin_cli: Arc<dyn SchemaPinCli>,
    /// MCP client for tool discovery
    mcp_client: Arc<dyn McpClient>,
    /// Configuration
    config: ToolReviewConfig,
    /// Background task handles
    background_tasks: Arc<Mutex<Vec<tokio::task::JoinHandle<()>>>>,
}

/// Events that can occur during workflow execution
#[derive(Debug, Clone)]
pub enum WorkflowEvent {
    /// Tool submitted for review
    ToolSubmitted {
        review_id: ReviewId,
        tool: McpTool,
        submitted_by: String,
    },
    /// Analysis started
    AnalysisStarted {
        review_id: ReviewId,
        analysis_id: AnalysisId,
    },
    /// Analysis completed
    AnalysisCompleted {
        review_id: ReviewId,
        analysis: SecurityAnalysis,
    },
    /// Human review required
    HumanReviewRequired {
        review_id: ReviewId,
        critical_findings: Vec<SecurityFinding>,
    },
    /// Human decision made
    HumanDecisionMade {
        review_id: ReviewId,
        decision: HumanDecision,
    },
    /// Tool approved
    ToolApproved {
        review_id: ReviewId,
        approved_by: String,
    },
    /// Tool rejected
    ToolRejected {
        review_id: ReviewId,
        rejected_by: String,
        reason: String,
    },
    /// Signing started
    SigningStarted {
        review_id: ReviewId,
    },
    /// Tool signed successfully
    ToolSigned {
        review_id: ReviewId,
        signature_info: crate::integrations::schemapin::SignatureInfo,
    },
    /// Signing failed
    SigningFailed {
        review_id: ReviewId,
        error: String,
        retry_count: u32,
    },
    /// Workflow completed
    WorkflowCompleted {
        review_id: ReviewId,
        final_state: ToolReviewState,
    },
}

/// Workflow event handler trait
pub trait WorkflowEventHandler: Send + Sync {
    /// Handle a workflow event
    fn handle_event(&self, event: WorkflowEvent) -> impl std::future::Future<Output = ()> + Send;
}

/// Statistics about workflow execution
#[derive(Debug, Clone)]
pub struct WorkflowStats {
    pub active_sessions: usize,
    pub completed_reviews: u64,
    pub average_analysis_time: Duration,
    pub average_human_review_time: Duration,
    pub auto_approval_rate: f32,
    pub signing_success_rate: f32,
}

impl ToolReviewOrchestrator {
    /// Create a new tool review orchestrator
    pub fn new(
        analyzer: Arc<dyn SecurityAnalyzer>,
        review_interface: Arc<dyn HumanReviewInterface>,
        schemapin_cli: Arc<dyn SchemaPinCli>,
        mcp_client: Arc<dyn McpClient>,
        config: ToolReviewConfig,
    ) -> Self {
        Self {
            sessions: Arc::new(RwLock::new(HashMap::new())),
            analyzer,
            review_interface,
            schemapin_cli,
            mcp_client,
            config,
            background_tasks: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Submit a tool for review
    pub async fn submit_tool_for_review(
        &self,
        tool: McpTool,
        submitted_by: String,
    ) -> ToolReviewResult<ReviewId> {
        let review_id = ReviewId::new();
        let now = SystemTime::now();

        // Create initial session
        let session = ToolReviewSession {
            review_id,
            tool: tool.clone(),
            state: ToolReviewState::PendingReview {
                submitted_at: now,
                submitted_by: submitted_by.clone(),
            },
            security_analysis: None,
            human_decisions: Vec::new(),
            audit_trail: vec![AuditEvent {
                event_id: Uuid::new_v4().to_string(),
                event_type: AuditEventType::ToolSubmitted,
                timestamp: now,
                actor: submitted_by.clone(),
                details: HashMap::new(),
            }],
            created_at: now,
            updated_at: now,
        };

        // Store session
        {
            let mut sessions = self.sessions.write().await;
            sessions.insert(review_id, session);
        }

        // Start workflow processing
        self.start_workflow_processing(review_id).await?;

        Ok(review_id)
    }

    /// Get the current state of a review session
    pub async fn get_review_state(&self, review_id: ReviewId) -> ToolReviewResult<ToolReviewState> {
        let sessions = self.sessions.read().await;
        let session = sessions
            .get(&review_id)
            .ok_or(ToolReviewError::ReviewSessionNotFound { review_id })?;
        Ok(session.state.clone())
    }

    /// Get a complete review session
    pub async fn get_review_session(&self, review_id: ReviewId) -> ToolReviewResult<ToolReviewSession> {
        let sessions = self.sessions.read().await;
        let session = sessions
            .get(&review_id)
            .ok_or(ToolReviewError::ReviewSessionNotFound { review_id })?
            .clone();
        Ok(session)
    }

    /// List all active review sessions
    pub async fn list_active_sessions(&self) -> Vec<ReviewId> {
        let sessions = self.sessions.read().await;
        sessions.keys().copied().collect()
    }

    /// Get workflow statistics
    pub async fn get_workflow_stats(&self) -> WorkflowStats {
        let sessions = self.sessions.read().await;
        let active_sessions = sessions.len();
        
        // Calculate statistics from sessions
        let mut completed_reviews = 0u64;
        let total_analysis_time = Duration::ZERO;
        let total_human_review_time = Duration::ZERO;
        let mut auto_approvals = 0u64;
        let mut successful_signings = 0u64;
        let mut total_signings = 0u64;

        for session in sessions.values() {
            match &session.state {
                ToolReviewState::Signed { .. } => {
                    completed_reviews += 1;
                    successful_signings += 1;
                    total_signings += 1;
                }
                ToolReviewState::Rejected { .. } => {
                    completed_reviews += 1;
                }
                ToolReviewState::SigningFailed { .. } => {
                    completed_reviews += 1;
                    total_signings += 1;
                }
                _ => {}
            }

            // Check if auto-approved (no human decisions)
            if session.human_decisions.is_empty() && matches!(session.state, ToolReviewState::Signed { .. }) {
                auto_approvals += 1;
            }
        }

        WorkflowStats {
            active_sessions,
            completed_reviews,
            average_analysis_time: if completed_reviews > 0 {
                total_analysis_time / completed_reviews as u32
            } else {
                Duration::ZERO
            },
            average_human_review_time: if completed_reviews > 0 {
                total_human_review_time / completed_reviews as u32
            } else {
                Duration::ZERO
            },
            auto_approval_rate: if completed_reviews > 0 {
                auto_approvals as f32 / completed_reviews as f32
            } else {
                0.0
            },
            signing_success_rate: if total_signings > 0 {
                successful_signings as f32 / total_signings as f32
            } else {
                0.0
            },
        }
    }

    /// Start workflow processing for a review session
    async fn start_workflow_processing(&self, review_id: ReviewId) -> ToolReviewResult<()> {
        let orchestrator = self.clone_for_task();
        let handle = tokio::spawn(async move {
            if let Err(e) = orchestrator.process_workflow(review_id).await {
                eprintln!("Workflow processing failed for {}: {}", review_id.0, e);
            }
        });

        let mut tasks = self.background_tasks.lock().await;
        tasks.push(handle);
        Ok(())
    }

    /// Main workflow processing logic
    async fn process_workflow(&self, review_id: ReviewId) -> ToolReviewResult<()> {
        loop {
            let current_state = self.get_review_state(review_id).await?;
            
            match current_state {
                ToolReviewState::PendingReview { .. } => {
                    self.start_analysis(review_id).await?;
                }
                ToolReviewState::UnderReview { analysis_id, .. } => {
                    // Wait for analysis to complete or timeout
                    self.wait_for_analysis_completion(review_id, analysis_id).await?;
                }
                ToolReviewState::AwaitingHumanReview { .. } => {
                    self.wait_for_human_decision(review_id).await?;
                }
                ToolReviewState::Approved { .. } => {
                    self.start_signing(review_id).await?;
                }
                ToolReviewState::Rejected { .. } => {
                    // Workflow completed
                    self.complete_workflow(review_id).await?;
                    break;
                }
                ToolReviewState::Signed { .. } => {
                    // Workflow completed
                    self.complete_workflow(review_id).await?;
                    break;
                }
                ToolReviewState::SigningFailed { retry_count, .. } if retry_count >= self.config.max_signing_retries => {
                    // Workflow completed
                    self.complete_workflow(review_id).await?;
                    break;
                }
                ToolReviewState::SigningFailed { retry_count, .. } => {
                    // Retry signing
                    sleep(Duration::from_secs(2_u64.pow(retry_count))).await; // Exponential backoff
                    self.start_signing(review_id).await?;
                }
            }
        }

        Ok(())
    }

    /// Start AI security analysis
    async fn start_analysis(&self, review_id: ReviewId) -> ToolReviewResult<()> {
        let analysis_id = AnalysisId::new();
        let now = SystemTime::now();

        // Update state to UnderReview
        self.update_session_state(
            review_id,
            ToolReviewState::UnderReview {
                started_at: now,
                analyzer_id: "ai_security_analyzer".to_string(),
                analysis_id,
            },
        ).await?;

        // Get tool for analysis
        let tool = {
            let sessions = self.sessions.read().await;
            let session = sessions
                .get(&review_id)
                .ok_or(ToolReviewError::ReviewSessionNotFound { review_id })?;
            session.tool.clone()
        };

        // Start analysis in background
        let analyzer = self.analyzer.clone();
        let orchestrator = self.clone_for_task();
        
        tokio::spawn(async move {
            match analyzer.analyze_tool(&tool).await {
                Ok(analysis) => {
                    if let Err(e) = orchestrator.handle_analysis_completion(review_id, analysis).await {
                        eprintln!("Failed to handle analysis completion: {}", e);
                    }
                }
                Err(e) => {
                    if let Err(e) = orchestrator.handle_analysis_failure(review_id, e.to_string()).await {
                        eprintln!("Failed to handle analysis failure: {}", e);
                    }
                }
            }
        });

        Ok(())
    }

    /// Handle analysis completion
    async fn handle_analysis_completion(
        &self,
        review_id: ReviewId,
        analysis: SecurityAnalysis,
    ) -> ToolReviewResult<()> {
        let now = SystemTime::now();

        // Determine if human review is required
        let requires_human_review = self.requires_human_review(&analysis);

        if requires_human_review {
            // Extract critical findings
            let critical_findings: Vec<SecurityFinding> = analysis
                .findings
                .iter()
                .filter(|f| matches!(f.severity, SecuritySeverity::High | SecuritySeverity::Critical))
                .cloned()
                .collect();

            // Update state to AwaitingHumanReview
            self.update_session_state(
                review_id,
                ToolReviewState::AwaitingHumanReview {
                    analysis_id: analysis.analysis_id,
                    analysis_completed_at: now,
                    critical_findings,
                    risk_score: analysis.risk_score,
                    ai_recommendation: self.generate_ai_recommendation(&analysis),
                },
            ).await?;
        } else {
            // Auto-approve or auto-reject based on analysis
            if analysis.risk_score >= self.config.auto_approve_threshold {
                self.update_session_state(
                    review_id,
                    ToolReviewState::Approved {
                        approved_by: "ai_auto_approval".to_string(),
                        approved_at: now,
                        approval_notes: Some("Automatically approved by AI analysis".to_string()),
                    },
                ).await?;
            } else {
                self.update_session_state(
                    review_id,
                    ToolReviewState::Rejected {
                        rejected_by: "ai_auto_rejection".to_string(),
                        rejected_at: now,
                        rejection_reason: "Tool failed automated security analysis".to_string(),
                    },
                ).await?;
            }
        }

        // Store analysis in session
        {
            let mut sessions = self.sessions.write().await;
            if let Some(session) = sessions.get_mut(&review_id) {
                session.security_analysis = Some(analysis);
                session.updated_at = now;
            }
        }

        Ok(())
    }

    /// Handle analysis failure
    async fn handle_analysis_failure(
        &self,
        review_id: ReviewId,
        error: String,
    ) -> ToolReviewResult<()> {
        let now = SystemTime::now();
        
        self.update_session_state(
            review_id,
            ToolReviewState::Rejected {
                rejected_by: "system".to_string(),
                rejected_at: now,
                rejection_reason: format!("Analysis failed: {}", error),
            },
        ).await?;

        Ok(())
    }

    /// Wait for analysis completion with timeout
    async fn wait_for_analysis_completion(
        &self,
        review_id: ReviewId,
        _analysis_id: AnalysisId,
    ) -> ToolReviewResult<()> {
        let timeout_duration = Duration::from_secs(self.config.max_analysis_time_seconds);
        
        let result = timeout(timeout_duration, async {
            loop {
                let state = self.get_review_state(review_id).await?;
                if !matches!(state, ToolReviewState::UnderReview { .. }) {
                    break;
                }
                sleep(Duration::from_secs(1)).await;
            }
            Ok::<(), ToolReviewError>(())
        }).await;

        match result {
            Ok(Ok(())) => Ok(()),
            Ok(Err(e)) => Err(e),
            Err(_) => {
                // Timeout occurred
                let now = SystemTime::now();
                self.update_session_state(
                    review_id,
                    ToolReviewState::Rejected {
                        rejected_by: "system".to_string(),
                        rejected_at: now,
                        rejection_reason: format!("Analysis timeout after {} seconds", self.config.max_analysis_time_seconds),
                    },
                ).await?;
                Err(ToolReviewError::AnalysisTimeout {
                    seconds: self.config.max_analysis_time_seconds,
                })
            }
        }
    }

    /// Wait for human decision
    async fn wait_for_human_decision(&self, review_id: ReviewId) -> ToolReviewResult<()> {
        let timeout_duration = Duration::from_secs(self.config.max_human_review_time_seconds);
        
        let result = timeout(timeout_duration, async {
            loop {
                let state = self.get_review_state(review_id).await?;
                if !matches!(state, ToolReviewState::AwaitingHumanReview { .. }) {
                    break;
                }
                sleep(Duration::from_secs(5)).await; // Check every 5 seconds
            }
            Ok::<(), ToolReviewError>(())
        }).await;

        match result {
            Ok(Ok(())) => Ok(()),
            Ok(Err(e)) => Err(e),
            Err(_) => {
                // Timeout occurred
                let now = SystemTime::now();
                self.update_session_state(
                    review_id,
                    ToolReviewState::Rejected {
                        rejected_by: "system".to_string(),
                        rejected_at: now,
                        rejection_reason: format!("Human review timeout after {} seconds", self.config.max_human_review_time_seconds),
                    },
                ).await?;
                Err(ToolReviewError::HumanReviewTimeout {
                    seconds: self.config.max_human_review_time_seconds,
                })
            }
        }
    }

    /// Start signing process
    async fn start_signing(&self, review_id: ReviewId) -> ToolReviewResult<()> {
        let _now = SystemTime::now();

        // Get tool schema for signing
        let tool = {
            let sessions = self.sessions.read().await;
            let session = sessions
                .get(&review_id)
                .ok_or(ToolReviewError::ReviewSessionNotFound { review_id })?;
            session.tool.clone()
        };

        // Create temporary schema file
        let schema_path = format!("/tmp/tool_schema_{}.json", review_id.0);
        let schema_content = serde_json::to_string_pretty(&tool)
            .map_err(|e| ToolReviewError::SerializationError(e.to_string()))?;
        
        tokio::fs::write(&schema_path, schema_content).await
            .map_err(|e| ToolReviewError::SerializationError(e.to_string()))?;

        // Sign the schema
        let sign_args = SignArgs::new(
            schema_path.clone(),
            "/path/to/trusted/private.key".to_string(), // TODO: Make configurable
        );

        let schemapin_cli = self.schemapin_cli.clone();
        let orchestrator = self.clone_for_task();

        tokio::spawn(async move {
            match schemapin_cli.sign_schema(sign_args).await {
                Ok(signing_result) => {
                    if let Err(e) = orchestrator.handle_signing_success(review_id, signing_result).await {
                        eprintln!("Failed to handle signing success: {}", e);
                    }
                }
                Err(e) => {
                    if let Err(e) = orchestrator.handle_signing_failure(review_id, e.to_string()).await {
                        eprintln!("Failed to handle signing failure: {}", e);
                    }
                }
            }

            // Clean up temporary file
            let _ = tokio::fs::remove_file(&schema_path).await;
        });

        Ok(())
    }

    /// Handle successful signing
    async fn handle_signing_success(
        &self,
        review_id: ReviewId,
        signing_result: crate::integrations::schemapin::SigningResult,
    ) -> ToolReviewResult<()> {
        let now = SystemTime::now();

        if let Some(signature_info) = signing_result.signature {
            self.update_session_state(
                review_id,
                ToolReviewState::Signed {
                    signature_info,
                    signed_at: now,
                    signed_by: "trusted_key".to_string(),
                },
            ).await?;
        } else {
            self.handle_signing_failure(review_id, "No signature information returned".to_string()).await?;
        }

        Ok(())
    }

    /// Handle signing failure
    async fn handle_signing_failure(
        &self,
        review_id: ReviewId,
        error: String,
    ) -> ToolReviewResult<()> {
        let now = SystemTime::now();
        
        // Get current retry count
        let retry_count = {
            let sessions = self.sessions.read().await;
            let session = sessions
                .get(&review_id)
                .ok_or(ToolReviewError::ReviewSessionNotFound { review_id })?;
            
            match &session.state {
                ToolReviewState::SigningFailed { retry_count, .. } => *retry_count + 1,
                _ => 1,
            }
        };

        self.update_session_state(
            review_id,
            ToolReviewState::SigningFailed {
                error,
                failed_at: now,
                retry_count,
            },
        ).await?;

        Ok(())
    }

    /// Complete the workflow
    async fn complete_workflow(&self, _review_id: ReviewId) -> ToolReviewResult<()> {
        // Workflow is complete, session can be archived or cleaned up
        // For now, we'll just leave it in the sessions map for historical purposes
        Ok(())
    }

    /// Update session state and add audit event
    async fn update_session_state(
        &self,
        review_id: ReviewId,
        new_state: ToolReviewState,
    ) -> ToolReviewResult<()> {
        let now = SystemTime::now();
        
        let mut sessions = self.sessions.write().await;
        let session = sessions
            .get_mut(&review_id)
            .ok_or(ToolReviewError::ReviewSessionNotFound { review_id })?;

        let old_state = session.state.clone();
        session.state = new_state.clone();
        session.updated_at = now;

        // Add audit event
        session.audit_trail.push(AuditEvent {
            event_id: Uuid::new_v4().to_string(),
            event_type: AuditEventType::StateTransition,
            timestamp: now,
            actor: "orchestrator".to_string(),
            details: {
                let mut details = HashMap::new();
                details.insert("from_state".to_string(), serde_json::to_value(&old_state).unwrap_or_default());
                details.insert("to_state".to_string(), serde_json::to_value(&new_state).unwrap_or_default());
                details
            },
        });

        Ok(())
    }

    /// Determine if human review is required
    fn requires_human_review(&self, analysis: &SecurityAnalysis) -> bool {
        // Require human review for high-risk findings
        if self.config.require_human_review_for_high_risk {
            for finding in &analysis.findings {
                if matches!(finding.severity, SecuritySeverity::High | SecuritySeverity::Critical) {
                    return true;
                }
            }
        }

        // Require human review if risk score is in the middle range
        analysis.risk_score > self.config.auto_reject_threshold 
            && analysis.risk_score < self.config.auto_approve_threshold
    }

    /// Generate AI recommendation based on analysis
    fn generate_ai_recommendation(&self, analysis: &SecurityAnalysis) -> ReviewRecommendation {
        if analysis.risk_score >= self.config.auto_approve_threshold {
            ReviewRecommendation::Approve {
                confidence: analysis.confidence_score,
                reasoning: "Tool passed all security checks with high confidence".to_string(),
            }
        } else if analysis.risk_score <= self.config.auto_reject_threshold {
            ReviewRecommendation::Reject {
                confidence: analysis.confidence_score,
                reasoning: "Tool has significant security concerns".to_string(),
            }
        } else {
            ReviewRecommendation::RequiresHumanJudgment {
                reasoning: "Tool has moderate risk that requires human evaluation".to_string(),
            }
        }
    }

    /// Clone orchestrator for background tasks
    fn clone_for_task(&self) -> Self {
        Self {
            sessions: self.sessions.clone(),
            analyzer: self.analyzer.clone(),
            review_interface: self.review_interface.clone(),
            schemapin_cli: self.schemapin_cli.clone(),
            mcp_client: self.mcp_client.clone(),
            config: self.config.clone(),
            background_tasks: self.background_tasks.clone(),
        }
    }

    /// Shutdown the orchestrator and wait for background tasks
    pub async fn shutdown(&self) -> ToolReviewResult<()> {
        let mut tasks = self.background_tasks.lock().await;
        
        // Cancel all background tasks
        for task in tasks.drain(..) {
            task.abort();
        }

        Ok(())
    }
}

/// Public API for human operators to make decisions
impl ToolReviewOrchestrator {
    /// Submit a human decision for a review session
    pub async fn submit_human_decision(
        &self,
        review_id: ReviewId,
        operator_id: String,
        decision_type: HumanDecisionType,
        reasoning: String,
    ) -> ToolReviewResult<()> {
        let now = SystemTime::now();

        // Create human decision record
        let decision = HumanDecision {
            decision_id: Uuid::new_v4().to_string(),
            operator_id: operator_id.clone(),
            decision: decision_type.clone(),
            reasoning: reasoning.clone(),
            decided_at: now,
            time_spent_seconds: 0, // TODO: Track actual time spent
        };

        // Update session with decision
        {
            let mut sessions = self.sessions.write().await;
            let session = sessions
                .get_mut(&review_id)
                .ok_or(ToolReviewError::ReviewSessionNotFound { review_id })?;

            session.human_decisions.push(decision);
            session.updated_at = now;

            // Add audit event
            session.audit_trail.push(AuditEvent {
                event_id: Uuid::new_v4().to_string(),
                event_type: AuditEventType::HumanDecisionMade,
                timestamp: now,
                actor: operator_id.clone(),
                details: {
                    let mut details = HashMap::new();
                    details.insert("decision".to_string(), serde_json::to_value(&decision_type).unwrap_or_default());
                    details.insert("reasoning".to_string(), serde_json::Value::String(reasoning.clone()));
                    details
                },
            });
        }

        // Update state based on decision
        match decision_type {
            HumanDecisionType::Approve => {
                self.update_session_state(
                    review_id,
                    ToolReviewState::Approved {
                        approved_by: operator_id,
                        approved_at: now,
                        approval_notes: Some(reasoning),
                    },
                ).await?;
            }
            HumanDecisionType::Reject => {
                self.update_session_state(
                    review_id,
                    ToolReviewState::Rejected {
                        rejected_by: operator_id,
                        rejected_at: now,
                        rejection_reason: reasoning,
                    },
                ).await?;
            }
            HumanDecisionType::RequestReanalysis => {
                // Restart analysis
                self.update_session_state(
                    review_id,
                    ToolReviewState::PendingReview {
                        submitted_at: now,
                        submitted_by: format!("reanalysis_requested_by_{}", operator_id),
                    },
                ).await?;
            }
            HumanDecisionType::EscalateToSenior => {
                // Keep in AwaitingHumanReview state but mark as escalated
                // Implementation would depend on escalation workflow
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::integrations::mcp::MockMcpClient;
    use crate::integrations::schemapin::MockSchemaPinCli;
    use crate::rag::MockRAGEngine;
    use super::analyzer::AISecurityAnalyzer;
    use super::review_interface::StandardReviewInterface;
    use std::sync::Arc;

    fn create_test_orchestrator() -> ToolReviewOrchestrator {
        let rag_engine = Arc::new(MockRAGEngine::new());
        let analyzer = Arc::new(AISecurityAnalyzer::new(rag_engine, Default::default()));
        let review_interface = Arc::new(StandardReviewInterface::new(Default::default()));
        let schemapin_cli = Arc::new(MockSchemaPinCli::new());
        let mcp_client = Arc::new(MockMcpClient::new());
        
        ToolReviewOrchestrator::new(
            analyzer,
            review_interface,
            schemapin_cli,
            mcp_client,
            ToolReviewConfig::default(),
        )
    }

    #[tokio::test]
    async fn test_tool_submission() {
        let orchestrator = create_test_orchestrator();
        
        let tool = McpTool {
            name: "test_tool".to_string(),
            description: Some("A test tool".to_string()),
            input_schema: serde_json::json!({}),
        };

        let review_id = orchestrator
            .submit_tool_for_review(tool, "test_user".to_string())
            .await
            .unwrap();

        let state = orchestrator.get_review_state(review_id).await.unwrap();
        assert!(matches!(state, ToolReviewState::PendingReview { .. }));
    }

    #[tokio::test]
    async fn test_workflow_stats() {
        let orchestrator = create_test_orchestrator();
        let stats = orchestrator.get_workflow_stats().await;
        
        assert_eq!(stats.active_sessions, 0);
        assert_eq!(stats.completed_reviews, 0);
    }
}