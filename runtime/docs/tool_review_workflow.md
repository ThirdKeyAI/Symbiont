# Tool Review Workflow Documentation

## Table of Contents

1. [Overview](#overview)
2. [Architecture](#architecture)
3. [Workflow States](#workflow-states)
4. [Component Integration](#component-integration)
5. [Deployment Guide](#deployment-guide)
6. [Configuration](#configuration)
7. [Security Considerations](#security-considerations)
8. [Monitoring and Troubleshooting](#monitoring-and-troubleshooting)
9. [Examples](#examples)

## Overview

The Tool Review Workflow is an AI-driven system for automatically reviewing, analyzing, and signing MCP (Model Context Protocol) tools within the Symbiont platform. The system combines automated security analysis powered by RAG (Retrieval-Augmented Generation) with human oversight to ensure safe deployment of tools.

### Key Features

- **Automated Security Analysis**: RAG-powered vulnerability detection using comprehensive security knowledge base
- **Human-in-the-Loop Review**: Streamlined interface for human reviewers to make final approval decisions
- **Digital Signing**: Cryptographic signing of approved tools using SchemaPin integration
- **Audit Trail**: Complete traceability of all review decisions and state changes
- **Scalable Architecture**: Async Rust implementation supporting high throughput

### Workflow Overview

```mermaid
graph TD
    A[Tool Submitted] --> B[Security Analysis]
    B --> C{Risk Assessment}
    C -->|Low Risk & High Confidence| D[Auto-Approve]
    C -->|High Risk or Low Confidence| E[Human Review]
    D --> F[Digital Signing]
    E --> G{Human Decision}
    G -->|Approve| F
    G -->|Reject| H[Rejected]
    F --> I[Signed & Published]
    F -->|Signing Failed| J[Retry Signing]
    J --> F
```

## Architecture

### System Architecture

```mermaid
graph TB
    subgraph "API Layer"
        API[REST API Server]
        WS[WebSocket Handler]
        WH[Webhook Handler]
    end
    
    subgraph "Core Workflow"
        ORCH[Tool Review Orchestrator]
        SM[State Manager]
        EH[Event Handler]
    end
    
    subgraph "Analysis Components"
        SA[Security Analyzer]
        KB[Knowledge Base]
        RAG[RAG Engine]
    end
    
    subgraph "Review Components"
        HRI[Human Review Interface]
        QUEUE[Review Queue]
        UI[Review UI]
    end
    
    subgraph "Signing Components"
        SP[SchemaPin CLI]
        KS[Key Store]
        SIGN[Signature Service]
    end
    
    subgraph "Storage"
        DB[(Database)]
        CACHE[(Redis Cache)]
        FS[(File Storage)]
    end
    
    subgraph "External Services"
        MCP[MCP Registry]
        NOTIFY[Notification Service]
        METRICS[Metrics Collector]
    end
    
    API --> ORCH
    WS --> ORCH
    ORCH --> SM
    ORCH --> SA
    ORCH --> HRI
    ORCH --> SP
    SA --> KB
    SA --> RAG
    HRI --> QUEUE
    HRI --> UI
    SP --> KS
    ORCH --> EH
    EH --> WH
    EH --> NOTIFY
    SM --> DB
    SM --> CACHE
    KB --> FS
    SP --> FS
    ORCH --> METRICS
```

### Component Dependencies

```mermaid
graph LR
    subgraph "Core Runtime"
        CTX[Context Manager]
        VDB[Vector Database]
        AGT[Agent Runtime]
    end
    
    subgraph "Tool Review System"
        ORCH[Orchestrator]
        ANALYZER[Security Analyzer]
        REVIEW[Review Interface]
        SCHEMA[SchemaPin Wrapper]
    end
    
    subgraph "External Dependencies"
        QDRANT[Qdrant Vector DB]
        OPENAI[OpenAI API]
        REDIS[Redis Cache]
        POSTGRES[PostgreSQL]
    end
    
    ANALYZER --> CTX
    ANALYZER --> VDB
    ORCH --> ANALYZER
    ORCH --> REVIEW
    ORCH --> SCHEMA
    VDB --> QDRANT
    CTX --> OPENAI
    ORCH --> REDIS
    ORCH --> POSTGRES
```

## Workflow States

### State Machine

The workflow follows a well-defined state machine with the following states:

```mermaid
stateDiagram-v2
    [*] --> PendingReview
    PendingReview --> UnderReview : Start Analysis
    UnderReview --> AwaitingHumanReview : Analysis Complete (High Risk)
    UnderReview --> Approved : Analysis Complete (Auto-Approve)
    AwaitingHumanReview --> Approved : Human Approval
    AwaitingHumanReview --> Rejected : Human Rejection
    AwaitingHumanReview --> UnderReview : Request Re-analysis
    Approved --> Signed : Signing Success
    Approved --> SigningFailed : Signing Error
    SigningFailed --> Approved : Retry Signing
    SigningFailed --> Rejected : Max Retries Exceeded
    Signed --> [*]
    Rejected --> [*]
```

### State Descriptions

| State | Description | Next States | Timeout |
|-------|-------------|-------------|---------|
| `PendingReview` | Tool submitted, waiting for analysis | `UnderReview` | N/A |
| `UnderReview` | AI security analysis in progress | `AwaitingHumanReview`, `Approved` | 5 minutes |
| `AwaitingHumanReview` | Waiting for human reviewer decision | `Approved`, `Rejected`, `UnderReview` | 1 hour |
| `Approved` | Tool approved, ready for signing | `Signed`, `SigningFailed` | N/A |
| `Rejected` | Tool rejected, workflow complete | Terminal | N/A |
| `Signed` | Tool successfully signed and published | Terminal | N/A |
| `SigningFailed` | Signing attempt failed | `Approved`, `Rejected` | N/A |

### State Transition Logic

```rust
impl ToolReviewOrchestrator {
    async fn transition_state(&self, review_id: ReviewId, new_state: ToolReviewState) -> ToolReviewResult<()> {
        let current_state = self.get_review_state(review_id).await?;
        
        // Validate state transition
        match (&current_state, &new_state) {
            (ToolReviewState::PendingReview { .. }, ToolReviewState::UnderReview { .. }) => Ok(()),
            (ToolReviewState::UnderReview { .. }, ToolReviewState::AwaitingHumanReview { .. }) => Ok(()),
            (ToolReviewState::UnderReview { .. }, ToolReviewState::Approved { .. }) => Ok(()),
            (ToolReviewState::AwaitingHumanReview { .. }, ToolReviewState::Approved { .. }) => Ok(()),
            (ToolReviewState::AwaitingHumanReview { .. }, ToolReviewState::Rejected { .. }) => Ok(()),
            (ToolReviewState::Approved { .. }, ToolReviewState::Signed { .. }) => Ok(()),
            (ToolReviewState::Approved { .. }, ToolReviewState::SigningFailed { .. }) => Ok(()),
            (ToolReviewState::SigningFailed { .. }, ToolReviewState::Approved { .. }) => Ok(()),
            _ => Err(ToolReviewError::InvalidStateTransition {
                from: current_state,
                to: new_state,
            }),
        }?;
        
        // Update state and emit event
        self.state_manager.update_state(review_id, new_state.clone()).await?;
        self.emit_event(WorkflowEvent::StateTransition { review_id, new_state }).await;
        
        Ok(())
    }
}
```

## Component Integration

### Security Analyzer Integration

The [`SecurityAnalyzer`](../src/integrations/tool_review/analyzer.rs:17) integrates with multiple components:

```mermaid
sequenceDiagram
    participant O as Orchestrator
    participant SA as Security Analyzer
    participant KB as Knowledge Base
    participant RAG as RAG Engine
    participant CM as Context Manager
    
    O->>SA: analyze_tool(tool)
    SA->>KB: check_vulnerability_patterns(tool_json)
    KB-->>SA: pattern_matches[]
    SA->>KB: check_malicious_signatures(tool_json)
    KB-->>SA: signature_matches[]
    SA->>RAG: process_query(security_queries)
    RAG->>CM: retrieve_context(query)
    CM-->>RAG: relevant_documents[]
    RAG-->>SA: analysis_responses[]
    SA->>SA: calculate_risk_score(findings)
    SA-->>O: SecurityAnalysis
```

#### Configuration

```rust
let analyzer_config = SecurityAnalyzerConfig {
    max_analysis_time_seconds: 120,
    confidence_threshold: 0.6,
    include_low_severity: false,
    knowledge_sources: vec![
        "vulnerability_patterns".to_string(),
        "malicious_code_signatures".to_string(),
        "security_best_practices".to_string(),
    ],
    max_rag_queries: 10,
};

let security_analyzer = AISecurityAnalyzer::with_config(
    rag_engine,
    agent_id,
    analyzer_config,
);
```

### Human Review Interface Integration

```mermaid
sequenceDiagram
    participant O as Orchestrator
    participant HRI as Human Review Interface
    participant Q as Review Queue
    participant UI as Review UI
    participant H as Human Reviewer
    
    O->>HRI: present_for_review(session, analysis)
    HRI->>Q: add_to_queue(review_item)
    HRI->>UI: notify_new_review(review_id)
    UI->>H: display_review_dashboard()
    H->>UI: select_review(review_id)
    UI->>HRI: get_review_details(review_id)
    HRI-->>UI: review_presentation
    H->>UI: submit_decision(decision)
    UI->>HRI: record_decision(review_id, decision)
    HRI-->>O: human_decision
```

### SchemaPin Integration

```mermaid
sequenceDiagram
    participant O as Orchestrator
    participant SP as SchemaPin CLI
    participant KS as Key Store
    participant FS as File System
    
    O->>SP: sign_schema(sign_args)
    SP->>KS: get_signing_key(key_id)
    KS-->>SP: private_key
    SP->>SP: generate_signature(schema, private_key)
    SP->>FS: write_signed_schema(signed_tool)
    SP-->>O: SigningResult
    
    Note over O,FS: Error Handling
    SP->>SP: retry_with_backoff()
    SP->>O: SigningError (if max retries exceeded)
```

## Deployment Guide

### Prerequisites

- **Rust**: 1.70+ with async support
- **Database**: PostgreSQL 14+ or compatible
- **Cache**: Redis 6+ for session storage
- **Vector DB**: Qdrant for RAG embeddings
- **File Storage**: Local filesystem or S3-compatible storage

### Installation Steps

1. **Clone Repository**
   ```bash
   git clone https://github.com/symbiont-platform/symbiont.git
   cd symbiont/runtime
   ```

2. **Install Dependencies**
   ```bash
   cargo build --release --features full
   ```

3. **Database Setup**
   ```sql
   CREATE DATABASE symbiont_tool_review;
   CREATE USER symbiont_user WITH PASSWORD 'secure_password';
   GRANT ALL PRIVILEGES ON DATABASE symbiont_tool_review TO symbiont_user;
   ```

4. **Configuration**
   ```toml
   # config/tool_review.toml
   [database]
   url = "postgresql://symbiont_user:secure_password@localhost/symbiont_tool_review"
   max_connections = 20
   
   [redis]
   url = "redis://localhost:6379"
   prefix = "symbiont:tool_review:"
   
   [qdrant]
   url = "http://localhost:6333"
   collection = "security_knowledge"
   
   [workflow]
   max_analysis_time_seconds = 300
   max_human_review_time_seconds = 3600
   auto_approve_threshold = 0.9
   auto_reject_threshold = 0.1
   max_signing_retries = 3
   ```

5. **Run Migrations**
   ```bash
   cargo run --bin migrate -- --config config/tool_review.toml
   ```

6. **Start Services**
   ```bash
   # Start API server
   cargo run --bin tool-review-api -- --config config/tool_review.toml
   
   # Start workflow processor (separate process)
   cargo run --bin workflow-processor -- --config config/tool_review.toml
   ```

### Docker Deployment

```dockerfile
# Dockerfile
FROM rust:1.70 as builder
WORKDIR /app
COPY . .
RUN cargo build --release --features full

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*
COPY --from=builder /app/target/release/tool-review-api /usr/local/bin/
COPY --from=builder /app/target/release/workflow-processor /usr/local/bin/
EXPOSE 8080
CMD ["tool-review-api"]
```

```yaml
# docker-compose.yml
version: '3.8'
services:
  tool-review-api:
    build: .
    ports:
      - "8080:8080"
    environment:
      - DATABASE_URL=postgresql://symbiont:password@postgres/symbiont_tool_review
      - REDIS_URL=redis://redis:6379
      - QDRANT_URL=http://qdrant:6333
    depends_on:
      - postgres
      - redis
      - qdrant
  
  workflow-processor:
    build: .
    command: workflow-processor
    environment:
      - DATABASE_URL=postgresql://symbiont:password@postgres/symbiont_tool_review
      - REDIS_URL=redis://redis:6379
      - QDRANT_URL=http://qdrant:6333
    depends_on:
      - postgres
      - redis
      - qdrant
  
  postgres:
    image: postgres:14
    environment:
      - POSTGRES_DB=symbiont_tool_review
      - POSTGRES_USER=symbiont
      - POSTGRES_PASSWORD=password
    volumes:
      - postgres_data:/var/lib/postgresql/data
  
  redis:
    image: redis:6-alpine
    volumes:
      - redis_data:/data
  
  qdrant:
    image: qdrant/qdrant:latest
    ports:
      - "6333:6333"
    volumes:
      - qdrant_data:/qdrant/storage

volumes:
  postgres_data:
  redis_data:
  qdrant_data:
```

### Kubernetes Deployment

```yaml
# k8s/deployment.yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: tool-review-api
spec:
  replicas: 3
  selector:
    matchLabels:
      app: tool-review-api
  template:
    metadata:
      labels:
        app: tool-review-api
    spec:
      containers:
      - name: api
        image: symbiont/tool-review:latest
        ports:
        - containerPort: 8080
        env:
        - name: DATABASE_URL
          valueFrom:
            secretKeyRef:
              name: database-secret
              key: url
        resources:
          requests:
            memory: "256Mi"
            cpu: "250m"
          limits:
            memory: "512Mi"
            cpu: "500m"
---
apiVersion: v1
kind: Service
metadata:
  name: tool-review-api-service
spec:
  selector:
    app: tool-review-api
  ports:
  - port: 80
    targetPort: 8080
  type: LoadBalancer
```

## Configuration

### Environment Variables

| Variable | Description | Default | Required |
|----------|-------------|---------|----------|
| `DATABASE_URL` | PostgreSQL connection string | - | Yes |
| `REDIS_URL` | Redis connection string | `redis://localhost:6379` | No |
| `QDRANT_URL` | Qdrant server URL | `http://localhost:6333` | No |
| `OPENAI_API_KEY` | OpenAI API key for RAG | - | Yes |
| `SCHEMAPIN_KEY_PATH` | Path to signing keys | `/etc/schemapin/keys` | No |
| `LOG_LEVEL` | Logging level | `info` | No |
| `API_PORT` | API server port | `8080` | No |

### Configuration File Structure

```toml
[server]
host = "0.0.0.0"
port = 8080
workers = 4

[database]
url = "postgresql://user:pass@localhost/db"
max_connections = 20
connection_timeout = 30
idle_timeout = 600

[redis]
url = "redis://localhost:6379"
prefix = "symbiont:tool_review:"
connection_timeout = 5
command_timeout = 10

[qdrant]
url = "http://localhost:6333"
collection = "security_knowledge"
embedding_size = 1536
search_limit = 10

[workflow]
max_analysis_time_seconds = 300
max_human_review_time_seconds = 3600
auto_approve_threshold = 0.9
auto_reject_threshold = 0.1
require_human_review_for_high_risk = true
max_signing_retries = 3

[security_analyzer]
confidence_threshold = 0.6
include_low_severity = false
max_rag_queries = 10
knowledge_sources = ["vulnerability_patterns", "malicious_signatures"]

[review_interface]
max_queue_size = 100
priority_boost_hours = 24
escalation_threshold_hours = 4

[schemapin]
cli_path = "/usr/local/bin/schemapin"
key_store_path = "/etc/schemapin/keys"
default_key_id = "symbiont-signing-key"
signature_algorithm = "Ed25519"

[monitoring]
metrics_enabled = true
metrics_port = 9090
health_check_enabled = true
health_check_path = "/health"

[logging]
level = "info"
format = "json"
file = "/var/log/symbiont/tool-review.log"
max_file_size = "100MB"
max_files = 10
```

### Runtime Configuration Updates

Some configuration can be updated at runtime via API:

```http
PUT /config/security-analyzer
Content-Type: application/json

{
  "confidence_threshold": 0.7,
  "include_low_severity": true,
  "max_rag_queries": 15
}
```

## Security Considerations

### Authentication and Authorization

- **API Authentication**: Bearer tokens with JWT validation
- **Role-Based Access Control**: Different permissions for submitters, reviewers, and administrators
- **Key Management**: Secure storage and rotation of signing keys

### Data Protection

- **Encryption at Rest**: Database encryption for sensitive analysis data
- **Encryption in Transit**: TLS 1.3 for all API communications
- **Key Isolation**: Signing keys stored in hardware security modules (HSM) or secure enclaves

### Audit and Compliance

- **Complete Audit Trail**: All actions logged with immutable timestamps
- **Data Retention**: Configurable retention policies for analysis data
- **Compliance Reporting**: Built-in reports for security compliance frameworks

### Threat Model

```mermaid
graph TD
    subgraph "Trust Boundaries"
        A[External API Clients]
        B[Review UI]
        C[Internal Components]
        D[Database]
        E[File Storage]
    end
    
    subgraph "Threats"
        T1[Malicious Tool Submission]
        T2[Unauthorized Access]
        T3[Data Tampering]
        T4[Key Compromise]
        T5[Insider Threats]
    end
    
    subgraph "Mitigations"
        M1[Input Validation]
        M2[Authentication/Authorization]
        M3[Immutable Audit Logs]
        M4[HSM Key Storage]
        M5[Least Privilege Access]
    end
    
    T1 --> M1
    T2 --> M2
    T3 --> M3
    T4 --> M4
    T5 --> M5
```

## Monitoring and Troubleshooting

### Metrics and Monitoring

Key metrics to monitor:

```rust
// Workflow metrics
counter!("tool_review.submissions.total", "status" => "submitted");
histogram!("tool_review.analysis.duration_seconds");
counter!("tool_review.decisions.total", "type" => "human_approve");
gauge!("tool_review.queue.pending_count");

// Performance metrics
histogram!("tool_review.api.request_duration_seconds", "endpoint" => "/sessions");
counter!("tool_review.errors.total", "component" => "security_analyzer");
gauge!("tool_review.database.connections.active");

// Security metrics
counter!("tool_review.security.findings.total", "severity" => "critical");
counter!("tool_review.signing.attempts.total", "result" => "success");
```

### Health Checks

```rust
#[derive(Serialize)]
struct HealthStatus {
    status: String,
    checks: HashMap<String, ComponentHealth>,
    timestamp: SystemTime,
}

#[derive(Serialize)]
struct ComponentHealth {
    status: String,
    response_time_ms: u64,
    error: Option<String>,
}

async fn health_check() -> Result<HealthStatus, HealthError> {
    let mut checks = HashMap::new();
    
    // Database connectivity
    checks.insert("database".to_string(), check_database().await);
    
    // Redis connectivity
    checks.insert("redis".to_string(), check_redis().await);
    
    // Qdrant connectivity
    checks.insert("qdrant".to_string(), check_qdrant().await);
    
    // SchemaPin availability
    checks.insert("schemapin".to_string(), check_schemapin().await);
    
    let overall_status = if checks.values().all(|c| c.status == "healthy") {
        "healthy"
    } else {
        "unhealthy"
    };
    
    Ok(HealthStatus {
        status: overall_status.to_string(),
        checks,
        timestamp: SystemTime::now(),
    })
}
```

### Common Issues and Solutions

#### Analysis Timeouts

**Symptoms**: Tools stuck in `UnderReview` state
**Causes**: 
- RAG engine overload
- Network connectivity issues
- Large tool schemas

**Solutions**:
```rust
// Increase timeout configuration
let config = SecurityAnalyzerConfig {
    max_analysis_time_seconds: 300, // Increase from default 120
    max_rag_queries: 5, // Reduce to speed up analysis
    // ...
};

// Monitor RAG engine performance
histogram!("rag.query.duration_seconds");
counter!("rag.query.timeout.total");
```

#### Human Review Queue Backlog

**Symptoms**: High `awaiting_human_review` count
**Causes**:
- Insufficient reviewers
- Complex tools requiring extended review

**Solutions**:
- Auto-escalation for long-pending reviews
- Priority scoring based on tool criticality
- Load balancing across reviewer teams

#### Signing Failures

**Symptoms**: Tools stuck in `SigningFailed` state
**Causes**:
- Key rotation issues
- HSM connectivity problems
- Insufficient permissions

**Solutions**:
```rust
// Implement exponential backoff for retries
let retry_delay = Duration::from_secs(2_u64.pow(retry_count));
tokio::time::sleep(retry_delay).await;

// Health check for signing service
async fn check_signing_service() -> ComponentHealth {
    match schemapin_cli.list_keys().await {
        Ok(_) => ComponentHealth {
            status: "healthy".to_string(),
            response_time_ms: 50,
            error: None,
        },
        Err(e) => ComponentHealth {
            status: "unhealthy".to_string(),
            response_time_ms: 0,
            error: Some(e.to_string()),
        },
    }
}
```

### Logging Best Practices

```rust
use tracing::{info, warn, error, instrument};

#[instrument(skip(self), fields(review_id = %review_id))]
async fn process_workflow(&self, review_id: ReviewId) -> ToolReviewResult<()> {
    info!("Starting workflow processing");
    
    match self.execute_analysis(review_id).await {
        Ok(analysis) => {
            info!(
                risk_score = analysis.risk_score,
                findings_count = analysis.findings.len(),
                "Analysis completed successfully"
            );
        }
        Err(e) => {
            error!(error = %e, "Analysis failed");
            return Err(e);
        }
    }
    
    Ok(())
}
```

## Examples

### Complete Workflow Example

```rust
use symbiont_runtime::integrations::tool_review::*;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize components
    let context_manager = Arc::new(StandardContextManager::new(/* config */));
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
    let tool = McpTool {
        name: "example-tool".to_string(),
        description: "Example tool for demonstration".to_string(),
        schema: serde_json::json!({
            "type": "object",
            "properties": {
                "input": {
                    "type": "string",
                    "description": "Tool input"
                }
            },
            "required": ["input"]
        }),
        provider: ToolProvider {
            name: "example-provider".to_string(),
            public_key_url: "https://example.com/pubkey.pem".to_string(),
        },
        verification_status: VerificationStatus::Pending,
        metadata: None,
    };
    
    let review_id = orchestrator.submit_tool(tool, "user@example.com".to_string()).await?;
    println!("Submitted tool for review: {}", review_id.0);
    
    // Monitor progress
    loop {
        let state = orchestrator.get_review_state(review_id).await?;
        println!("Current state: {:?}", state);
        
        match state {
            ToolReviewState::Signed { signature_info, .. } => {
                println!("Tool successfully signed!");
                println!("Signature: {}", signature_info.signature);
                break;
            }
            ToolReviewState::Rejected { rejection_reason, .. } => {
                println!("Tool rejected: {}", rejection_reason);
                break;
            }
            _ => {
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        }
    }
    
    Ok(())
}
```

### Custom Event Handler Example

```rust
struct SlackNotificationHandler {
    slack_client: SlackClient,
}

#[async_trait]
impl WorkflowEventHandler for SlackNotificationHandler {
    async fn handle_event(&self, event: WorkflowEvent) -> Result<(), Box<dyn std::error::Error>> {
        match event.event_type {
            WorkflowEventType::ToolSubmitted => {
                self.slack_client.send_message(
                    "#tool-reviews",
                    &format!("🔍 New tool submitted for review: {}", event.review_id),
                ).await?;
            }
            WorkflowEventType::HumanReviewRequired => {
                self.slack_client.send_message(
                    "#security-team",
                    &format!("👤 Human review required for tool: {}", event.review_id),
                ).await?;
            }
            WorkflowEventType::ToolSigned => {
                self.slack_client.send_message(
                    "#tool-releases",
                    &format!("✅ Tool successfully signed and published: {}", event.review_id),
                ).await?;
            }
            _ => {} // Handle other events as needed
        }
        Ok(())
    }
}
```

### Integration Testing Example

```rust
#[tokio::test]
async fn test_end_to_end_workflow() {
    // Setup test environment
    let test_db = TestDatabase::new().await;
    let test_redis = TestRedis::new().await;
    let mock_rag = MockRAGEngine::new();
    
    // Create test orchestrator
    let orchestrator = create_test_orchestrator(test_db, test_redis, mock_rag).await;
    
    // Test data
    let vulnerable_tool = create_tool_with_vulnerability();
    let safe_tool = create_safe_tool();
    
    // Test vulnerable tool rejection
    let review_id = orchestrator.submit_tool(vulnerable_tool, "test@example.com".to_string()).await.unwrap();
    wait_for_state(&orchestrator, review_id, |state| {
        matches!(state, ToolReviewState::AwaitingHumanReview { .. })
    }).await;
    
    // Simulate human rejection
    let decision = HumanDecision {
        decision: HumanDecisionType::Reject,
        reasoning: "Tool contains security vulnerabilities".to_string(),
        operator_id: "reviewer@example.com".to_string(),
        time_spent_seconds: 300,
    };
    
    orchestrator.handle_human_decision(review_id, decision).await.unwrap();
    
    let final_state = wait_for_state(&orchestrator, review_id, |state| {
        matches!(state, ToolReviewState::Rejected { .. })
    }).await;
    
    assert!(matches!(final_state, ToolReviewState::Rejected { .. }));
    
    // Test safe tool auto-approval
    let review_id = orchestrator.submit_tool(safe_tool, "test@example.com".to_string()).await.unwrap();
    let final_state = wait_for_state(&orchestrator, review_id, |state| {
        matches!(state, ToolReviewState::Signed { .. })
    }).await;
    
    assert!(matches!(final_state, ToolReviewState::Signed { .. }));
}
```

This comprehensive documentation provides complete coverage of the Tool Review Workflow system, including architecture diagrams, deployment instructions, configuration options, security considerations, and practical examples for implementation and troubleshooting.