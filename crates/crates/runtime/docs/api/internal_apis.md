# Internal API Specifications

## Overview

This document describes the internal APIs and trait specifications for the Tool Review Workflow components within the Symbiont runtime. These APIs define how the various Rust components communicate with each other.

## Core Traits

### SecurityAnalyzer Trait

The [`SecurityAnalyzer`](../../src/integrations/tool_review/analyzer.rs:17) trait defines the interface for security analysis components.

```rust
#[async_trait]
pub trait SecurityAnalyzer: Send + Sync {
    /// Analyze a tool for security vulnerabilities
    async fn analyze_tool(&self, tool: &McpTool) -> ToolReviewResult<SecurityAnalysis>;
    
    /// Get analyzer configuration
    fn get_config(&self) -> &SecurityAnalyzerConfig;
    
    /// Update analyzer configuration
    async fn update_config(&mut self, config: SecurityAnalyzerConfig) -> ToolReviewResult<()>;
}
```

**Implementations:**
- [`AISecurityAnalyzer`](../../src/integrations/tool_review/analyzer.rs:60) - RAG-powered security analyzer

**Usage Example:**
```rust
let analyzer = AISecurityAnalyzer::new(rag_engine, agent_id);
let analysis = analyzer.analyze_tool(&tool).await?;
```

### HumanReviewInterface Trait

The [`HumanReviewInterface`](../../src/integrations/tool_review/review_interface.rs:18) trait defines the interface for human review operations.

```rust
#[async_trait]
pub trait HumanReviewInterface: Send + Sync {
    /// Present security analysis for human review
    async fn present_for_review(
        &self,
        session: &ToolReviewSession,
        analysis: &SecurityAnalysis,
    ) -> ToolReviewResult<ReviewPresentation>;
    
    /// Wait for human decision with timeout
    async fn wait_for_decision(
        &self,
        review_id: ReviewId,
        timeout: Duration,
    ) -> ToolReviewResult<HumanDecision>;
    
    /// Get interface configuration
    fn get_config(&self) -> &ReviewInterfaceConfig;
}
```

**Implementations:**
- [`StandardReviewInterface`](../../src/integrations/tool_review/review_interface.rs:50) - Standard human review interface

### SchemaPinCli Trait

The [`SchemaPinCli`](../../src/integrations/schemapin/cli_wrapper.rs:16) trait defines the interface for SchemaPin operations.

```rust
#[async_trait]
pub trait SchemaPinCli: Send + Sync {
    /// Initialize SchemaPin with configuration
    async fn init(&self, config: SchemaPinConfig) -> Result<(), SchemaPinError>;
    
    /// Add a new key to the keystore
    async fn add_key(&self, args: AddKeyArgs) -> Result<KeyInfo, SchemaPinError>;
    
    /// List available keys
    async fn list_keys(&self) -> Result<Vec<KeyInfo>, SchemaPinError>;
    
    /// Sign a schema with the specified key
    async fn sign_schema(&self, args: SignArgs) -> Result<SigningResult, SchemaPinError>;
    
    /// Verify a signed schema
    async fn verify_schema(&self, args: VerifyArgs) -> Result<VerificationResult, SchemaPinError>;
}
```

**Implementations:**
- [`SchemaPinCliWrapper`](../../src/integrations/schemapin/cli_wrapper.rs:38) - CLI wrapper implementation

## Component APIs

### ToolReviewOrchestrator

The [`ToolReviewOrchestrator`](../../src/integrations/tool_review/orchestrator.rs:31) is the central coordinator for the workflow.

#### Constructor
```rust
impl ToolReviewOrchestrator {
    pub fn new(
        security_analyzer: Box<dyn SecurityAnalyzer>,
        review_interface: Box<dyn HumanReviewInterface>,
        schemapin_cli: Box<dyn SchemaPinCli>,
        config: ToolReviewConfig,
    ) -> Self
}
```

#### Core Methods
```rust
impl ToolReviewOrchestrator {
    /// Submit a tool for review
    pub async fn submit_tool(&self, tool: McpTool, submitted_by: String) -> ToolReviewResult<ReviewId>;
    
    /// Get the current state of a review session
    pub async fn get_review_state(&self, review_id: ReviewId) -> ToolReviewResult<ToolReviewState>;
    
    /// Get complete review session details
    pub async fn get_review_session(&self, review_id: ReviewId) -> ToolReviewResult<ToolReviewSession>;
    
    /// List all review sessions with optional filtering
    pub async fn list_sessions(&self, filter: Option<SessionFilter>) -> ToolReviewResult<Vec<ToolReviewSession>>;
    
    /// Get workflow statistics
    pub async fn get_stats(&self) -> ToolReviewResult<ToolReviewStats>;
    
    /// Process the workflow for a specific session
    async fn process_workflow(&self, review_id: ReviewId) -> ToolReviewResult<()>;
}
```

#### Event Handling
```rust
#[async_trait]
pub trait WorkflowEventHandler: Send + Sync {
    async fn handle_event(&self, event: WorkflowEvent) -> Result<(), Box<dyn std::error::Error>>;
}

impl ToolReviewOrchestrator {
    /// Register an event handler
    pub fn add_event_handler(&mut self, handler: Box<dyn WorkflowEventHandler>);
}
```

### SecurityKnowledgeBase

The [`SecurityKnowledgeBase`](../../src/integrations/tool_review/knowledge_base.rs:103) provides security pattern matching capabilities.

#### Core Methods
```rust
impl SecurityKnowledgeBase {
    /// Create a new knowledge base with default patterns
    pub fn new() -> Self;
    
    /// Load patterns from a JSON file
    pub fn load_from_file(&mut self, file_path: &str) -> Result<(), SecurityKnowledgeError>;
    
    /// Add a vulnerability pattern
    pub fn add_vulnerability_pattern(&mut self, pattern: VulnerabilityPattern) -> Result<(), SecurityKnowledgeError>;
    
    /// Add a malicious signature
    pub fn add_malicious_signature(&mut self, signature: MaliciousSignature) -> Result<(), SecurityKnowledgeError>;
    
    /// Analyze schema for vulnerabilities
    pub fn analyze_schema(&self, schema: &serde_json::Value) -> Vec<SecurityFinding>;
    
    /// Check vulnerability patterns against content
    pub fn check_vulnerability_patterns(&self, content: &str) -> Vec<VulnerabilityMatch>;
    
    /// Check malicious signatures against content
    pub fn check_malicious_signatures(&self, content: &str) -> Vec<SignatureMatch>;
    
    /// Get pattern statistics
    pub fn get_stats(&self) -> &PatternStats;
}
```

## Data Models

### Core Types

All types are defined in [`types.rs`](../../src/integrations/tool_review/types.rs).

#### ReviewId and AnalysisId
```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ReviewId(pub Uuid);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AnalysisId(pub Uuid);
```

#### ToolReviewState Enum
```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ToolReviewState {
    PendingReview { submitted_at: SystemTime, submitted_by: String },
    UnderReview { started_at: SystemTime, analyzer_id: String, analysis_id: AnalysisId },
    AwaitingHumanReview { analysis_id: AnalysisId, analysis_completed_at: SystemTime, critical_findings: Vec<SecurityFinding>, risk_score: f32, ai_recommendation: ReviewRecommendation },
    Approved { approved_by: String, approved_at: SystemTime, approval_notes: Option<String> },
    Rejected { rejected_by: String, rejected_at: SystemTime, rejection_reason: String },
    Signed { signature_info: SignatureInfo, signed_at: SystemTime, signed_by: String },
    SigningFailed { error: String, failed_at: SystemTime, retry_count: u32 },
}
```

#### SecurityFinding
```rust
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
```

#### SecurityAnalysis
```rust
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
```

### Configuration Types

#### ToolReviewConfig
```rust
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
```

#### SecurityAnalyzerConfig
```rust
#[derive(Debug, Clone)]
pub struct SecurityAnalyzerConfig {
    pub max_analysis_time_seconds: u64,
    pub confidence_threshold: f32,
    pub include_low_severity: bool,
    pub knowledge_sources: Vec<String>,
    pub max_rag_queries: u32,
}
```

## Error Handling

### ToolReviewError
```rust
#[derive(Debug, thiserror::Error)]
pub enum ToolReviewError {
    #[error("Analysis failed: {0}")]
    AnalysisFailed(String),

    #[error("Invalid state transition from {from:?} to {to:?}")]
    InvalidStateTransition { from: ToolReviewState, to: ToolReviewState },

    #[error("Review session not found: {review_id:?}")]
    ReviewSessionNotFound { review_id: ReviewId },

    #[error("Analysis timeout after {seconds} seconds")]
    AnalysisTimeout { seconds: u64 },

    #[error("Human review timeout after {seconds} seconds")]
    HumanReviewTimeout { seconds: u64 },

    #[error("Signing failed: {reason}")]
    SigningFailed { reason: String },

    // ... additional error variants
}

pub type ToolReviewResult<T> = Result<T, ToolReviewError>;
```

## Usage Patterns

### Basic Workflow Setup

```rust
use symbiont_runtime::integrations::tool_review::*;

// Create components
let rag_engine = Arc::new(StandardRAGEngine::new(context_manager));
let security_analyzer = Box::new(AISecurityAnalyzer::new(rag_engine, agent_id));
let review_interface = Box::new(StandardReviewInterface::new(config));
let schemapin_cli = Box::new(SchemaPinCliWrapper::new()?);

// Create orchestrator
let orchestrator = ToolReviewOrchestrator::new(
    security_analyzer,
    review_interface,
    schemapin_cli,
    ToolReviewConfig::default(),
);

// Submit tool for review
let review_id = orchestrator.submit_tool(tool, "user@example.com".to_string()).await?;

// Monitor progress
loop {
    let state = orchestrator.get_review_state(review_id).await?;
    match state {
        ToolReviewState::Signed { .. } => break,
        ToolReviewState::Rejected { .. } => return Err("Tool rejected".into()),
        _ => tokio::time::sleep(Duration::from_secs(5)).await,
    }
}
```

### Custom Event Handling

```rust
struct CustomEventHandler;

#[async_trait]
impl WorkflowEventHandler for CustomEventHandler {
    async fn handle_event(&self, event: WorkflowEvent) -> Result<(), Box<dyn std::error::Error>> {
        match event.event_type {
            WorkflowEventType::ToolSubmitted => {
                println!("Tool submitted: {}", event.review_id);
            }
            WorkflowEventType::AnalysisCompleted => {
                println!("Analysis completed for: {}", event.review_id);
            }
            WorkflowEventType::HumanDecisionMade => {
                println!("Human decision made for: {}", event.review_id);
            }
            WorkflowEventType::ToolSigned => {
                println!("Tool signed: {}", event.review_id);
            }
        }
        Ok(())
    }
}

// Register event handler
let mut orchestrator = ToolReviewOrchestrator::new(/* ... */);
orchestrator.add_event_handler(Box::new(CustomEventHandler));
```

### Advanced Security Analysis

```rust
// Custom analyzer configuration
let config = SecurityAnalyzerConfig {
    max_analysis_time_seconds: 180,
    confidence_threshold: 0.8,
    include_low_severity: true,
    knowledge_sources: vec![
        "custom_patterns".to_string(),
        "industry_standards".to_string(),
    ],
    max_rag_queries: 15,
};

let analyzer = AISecurityAnalyzer::with_config(rag_engine, agent_id, config);

// Direct analysis
let analysis = analyzer.analyze_tool(&tool).await?;
println!("Risk score: {}", analysis.risk_score);
for finding in &analysis.findings {
    println!("Finding: {} ({})", finding.title, finding.severity);
}
```

### Knowledge Base Management

```rust
// Load custom patterns
let mut knowledge_base = SecurityKnowledgeBase::new();
knowledge_base.load_from_file("custom_patterns.json")?;

// Add custom pattern
let pattern = VulnerabilityPattern {
    id: "custom_001".to_string(),
    name: "Custom Security Pattern".to_string(),
    description: "Detects custom vulnerability patterns".to_string(),
    category: SecurityCategory::Other("custom".to_string()),
    severity: SecuritySeverity::High,
    rules: vec![/* ... */],
    cve_references: vec![],
    remediation: Some("Apply custom mitigation".to_string()),
    false_positive_indicators: vec![],
};

knowledge_base.add_vulnerability_pattern(pattern)?;

// Analyze with custom knowledge base
let findings = knowledge_base.analyze_schema(&tool.schema);
```

## Testing

### Unit Testing Traits

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use mockall::{automock, predicate::*};

    #[automock]
    impl SecurityAnalyzer for MockSecurityAnalyzer {}

    #[tokio::test]
    async fn test_workflow_with_mock_analyzer() {
        let mut mock_analyzer = MockSecurityAnalyzer::new();
        mock_analyzer
            .expect_analyze_tool()
            .returning(|_| Ok(create_test_analysis()));

        let orchestrator = ToolReviewOrchestrator::new(
            Box::new(mock_analyzer),
            Box::new(create_mock_review_interface()),
            Box::new(create_mock_schemapin_cli()),
            ToolReviewConfig::default(),
        );

        let review_id = orchestrator.submit_tool(create_test_tool(), "test@example.com".to_string()).await?;
        assert!(!review_id.0.is_nil());
    }
}
```

### Integration Testing

```rust
#[tokio::test]
async fn test_full_workflow_integration() {
    let context_manager = Arc::new(MockContextManager::new());
    let rag_engine = Arc::new(StandardRAGEngine::new(context_manager));
    
    let orchestrator = create_test_orchestrator(rag_engine).await?;
    
    // Submit a tool with known vulnerabilities
    let tool = create_vulnerable_test_tool();
    let review_id = orchestrator.submit_tool(tool, "test@example.com".to_string()).await?;
    
    // Wait for analysis to complete
    let mut attempts = 0;
    loop {
        let state = orchestrator.get_review_state(review_id).await?;
        match state {
            ToolReviewState::AwaitingHumanReview { .. } => break,
            _ if attempts > 10 => panic!("Analysis took too long"),
            _ => {
                attempts += 1;
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
    }
    
    // Verify analysis results
    let session = orchestrator.get_review_session(review_id).await?;
    assert!(session.security_analysis.is_some());
    let analysis = session.security_analysis.unwrap();
    assert!(analysis.risk_score > 0.5);
    assert!(!analysis.findings.is_empty());
}
```

## Performance Considerations

### Async Design
- All trait methods that perform I/O are async
- State management uses Arc<RwLock<T>> for thread-safe shared access
- Background tasks use tokio::spawn for non-blocking execution

### Memory Management
- Large data structures (SecurityAnalysis, ToolReviewSession) use Arc for efficient sharing
- Knowledge base patterns are compiled once and reused
- Event handlers avoid blocking the main workflow

### Scalability
- Stateless trait implementations enable horizontal scaling
- Session storage can be backed by external databases
- Knowledge base can be shared across multiple analyzer instances

## Security Considerations

### Input Validation
- All external inputs are validated before processing
- Schema validation uses strict JSON Schema rules
- Pattern matching includes false positive detection

### Access Control
- Trait implementations should verify caller permissions
- Sensitive operations (signing) require elevated privileges
- Audit trails track all security-relevant operations

### Data Protection
- Private keys are never exposed in API responses
- Sensitive analysis data has configurable retention periods
- All inter-component communication uses authenticated channels