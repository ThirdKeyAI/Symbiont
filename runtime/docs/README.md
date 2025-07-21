# Symbiont Tool Review Workflow

An AI-driven system for automated security analysis, human review, and digital signing of MCP (Model Context Protocol) tools within the Symbiont platform.

## Overview

The Tool Review Workflow provides a complete solution for ensuring the security and integrity of MCP tools before they are deployed in production environments. The system combines advanced AI-powered security analysis with human oversight and cryptographic signing to create a robust, auditable workflow.

### Key Features

- 🤖 **AI-Powered Security Analysis**: RAG-enhanced vulnerability detection using comprehensive security knowledge base
- 👤 **Human-in-the-Loop Review**: Streamlined interface for security experts to review critical findings
- 🔐 **Cryptographic Signing**: Digital signing of approved tools using SchemaPin integration
- 📊 **Complete Audit Trail**: Immutable logging of all review decisions and state changes
- ⚡ **High Performance**: Async Rust implementation supporting concurrent workflows
- 🔄 **State Management**: Robust state machine with error recovery and retry mechanisms

## Quick Start

### Prerequisites

- Rust 1.70+ with cargo
- PostgreSQL 14+
- Redis 6+
- Qdrant vector database

### Installation

1. **Clone and build**:
   ```bash
   git clone https://github.com/symbiont-platform/symbiont.git
   cd symbiont/runtime
   cargo build --release --features full
   ```

2. **Start dependencies**:
   ```bash
   docker-compose up -d postgres redis qdrant
   ```

3. **Configure environment**:
   ```bash
   export DATABASE_URL="postgresql://symbiont:password@localhost/symbiont_tool_review"
   export REDIS_URL="redis://localhost:6379"
   export QDRANT_URL="http://localhost:6333"
   export OPENAI_API_KEY="your_openai_api_key"
   ```

4. **Run the system**:
   ```bash
   cargo run --bin tool-review-api
   ```

### Basic Usage

```rust
use symbiont_runtime::integrations::tool_review::*;

// Submit a tool for review
let tool = McpTool {
    name: "my-tool".to_string(),
    description: "Custom MCP tool".to_string(),
    schema: serde_json::json!({
        "type": "object",
        "properties": {
            "input": {"type": "string"}
        }
    }),
    // ... other fields
};

let orchestrator = ToolReviewOrchestrator::new(/* components */);
let review_id = orchestrator.submit_tool(tool, "user@example.com".to_string()).await?;

// Monitor progress
let state = orchestrator.get_review_state(review_id).await?;
println!("Review status: {:?}", state);
```

## Architecture

The system consists of several key components:

```mermaid
graph TB
    subgraph "API Layer"
        API[REST API Server]
        WS[WebSocket Handler]
    end
    
    subgraph "Core Workflow"
        ORCH[Tool Review Orchestrator]
        SM[State Manager]
    end
    
    subgraph "Analysis Components"
        SA[Security Analyzer]
        KB[Knowledge Base]
        RAG[RAG Engine]
    end
    
    subgraph "Review Components"
        HRI[Human Review Interface]
        QUEUE[Review Queue]
    end
    
    subgraph "Signing Components"
        SP[SchemaPin CLI]
        KS[Key Store]
    end
    
    API --> ORCH
    ORCH --> SA
    ORCH --> HRI
    ORCH --> SP
    SA --> KB
    SA --> RAG
```

### Workflow States

The review process follows a well-defined state machine:

1. **PendingReview** → Tool submitted, waiting for analysis
2. **UnderReview** → AI security analysis in progress
3. **AwaitingHumanReview** → High-risk findings require human decision
4. **Approved** → Tool approved, ready for signing
5. **Signed** → Tool successfully signed and published
6. **Rejected** → Tool rejected due to security concerns

## Components

### [Security Analyzer](../src/integrations/tool_review/analyzer.rs)
AI-powered component that analyzes MCP tool schemas for security vulnerabilities using:
- Pattern matching against known vulnerability signatures
- RAG-enhanced context retrieval for threat intelligence
- Risk scoring based on finding severity and confidence

### [Human Review Interface](../src/integrations/tool_review/review_interface.rs)
Streamlined interface for security experts to:
- Review AI analysis findings
- Make approval/rejection decisions
- Add manual review notes
- Escalate complex cases

### [Tool Review Orchestrator](../src/integrations/tool_review/orchestrator.rs)
Central coordinator that:
- Manages workflow state transitions
- Handles timeout and retry logic
- Emits audit events
- Coordinates between all components

### [Security Knowledge Base](../src/integrations/tool_review/knowledge_base.rs)
Comprehensive database of:
- Vulnerability patterns and signatures
- Malicious code detection rules
- False positive indicators
- Remediation recommendations

### [SchemaPin Integration](../src/integrations/schemapin/)
Cryptographic signing service that:
- Manages signing keys securely
- Generates digital signatures for approved tools
- Provides verification capabilities
- Integrates with hardware security modules

## API Reference

### REST API

Submit a tool for review:
```http
POST /v1/tool-review/sessions
Content-Type: application/json

{
  "tool": {
    "name": "example-tool",
    "description": "Example MCP tool",
    "schema": { /* JSON schema */ }
  },
  "submitted_by": "user@example.com"
}
```

Get review status:
```http
GET /v1/tool-review/sessions/{review_id}
```

Submit human decision:
```http
POST /v1/tool-review/review/{review_id}/decision
Content-Type: application/json

{
  "decision": "approve",
  "reasoning": "Tool appears safe after manual review",
  "operator_id": "reviewer@example.com"
}
```

### Internal APIs

The system exposes several Rust traits for customization:

- [`SecurityAnalyzer`](api/internal_apis.md#securityanalyzer-trait) - Custom security analysis implementations
- [`HumanReviewInterface`](api/internal_apis.md#humanreviewinterface-trait) - Custom review interfaces
- [`SchemaPinCli`](api/internal_apis.md#schemapincli-trait) - Custom signing implementations

## Configuration

### Environment Variables

| Variable | Description | Required |
|----------|-------------|----------|
| `DATABASE_URL` | PostgreSQL connection string | Yes |
| `REDIS_URL` | Redis connection string | No |
| `QDRANT_URL` | Qdrant server URL | No |
| `OPENAI_API_KEY` | OpenAI API key for RAG | Yes |
| `SCHEMAPIN_KEY_PATH` | Path to signing keys | No |

### Configuration File

```toml
[workflow]
max_analysis_time_seconds = 300
max_human_review_time_seconds = 3600
auto_approve_threshold = 0.9
auto_reject_threshold = 0.1
max_signing_retries = 3

[security_analyzer]
confidence_threshold = 0.6
include_low_severity = false
max_rag_queries = 10

[review_interface]
max_queue_size = 100
priority_boost_hours = 24
```

## Security

The system implements multiple security layers:

- **Authentication**: Bearer token authentication for all API endpoints
- **Authorization**: Role-based access control (submitters, reviewers, admins)
- **Encryption**: TLS 1.3 for transport, AES-256 for data at rest
- **Key Management**: HSM integration for signing key protection
- **Audit Logging**: Immutable audit trail for all security-relevant operations

## Monitoring

Key metrics to monitor:

- `tool_review.submissions.total` - Total tool submissions
- `tool_review.analysis.duration_seconds` - Analysis processing time
- `tool_review.queue.pending_count` - Pending human reviews
- `tool_review.security.findings.total` - Security findings by severity
- `tool_review.signing.success_rate` - Signing success rate

Health check endpoint:
```http
GET /health
```

## Deployment

### Docker

```bash
docker-compose up -d
```

### Kubernetes

```bash
kubectl apply -f k8s/
```

### Monitoring Stack

The system integrates with:
- **Prometheus** for metrics collection
- **Grafana** for dashboards
- **Jaeger** for distributed tracing
- **ELK Stack** for centralized logging

## Documentation

- 📋 [Complete Workflow Documentation](tool_review_workflow.md) - Comprehensive system documentation
- 🔌 [API Specifications](api/) - REST API and internal API documentation
- 🏗️ [Architecture Guide](tool_review_workflow.md#architecture) - System architecture and design decisions
- 🚀 [Deployment Guide](tool_review_workflow.md#deployment-guide) - Production deployment instructions
- 🔧 [Configuration Reference](tool_review_workflow.md#configuration) - All configuration options
- 🛡️ [Security Guide](tool_review_workflow.md#security-considerations) - Security best practices
- 📊 [Monitoring Guide](tool_review_workflow.md#monitoring-and-troubleshooting) - Monitoring and troubleshooting

## Examples

### Complete Workflow Example

```rust
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize system components
    let orchestrator = ToolReviewOrchestrator::new(
        Box::new(AISecurityAnalyzer::new(rag_engine, agent_id)),
        Box::new(StandardReviewInterface::new(config)),
        Box::new(SchemaPinCliWrapper::new()?),
        ToolReviewConfig::default(),
    );
    
    // Submit tool for review
    let review_id = orchestrator.submit_tool(tool, "user@example.com".to_string()).await?;
    
    // Wait for completion
    loop {
        match orchestrator.get_review_state(review_id).await? {
            ToolReviewState::Signed { .. } => {
                println!("✅ Tool successfully signed!");
                break;
            }
            ToolReviewState::Rejected { rejection_reason, .. } => {
                println!("❌ Tool rejected: {}", rejection_reason);
                break;
            }
            _ => tokio::time::sleep(Duration::from_secs(1)).await,
        }
    }
    
    Ok(())
}
```

### Custom Event Handler

```rust
struct NotificationHandler;

#[async_trait]
impl WorkflowEventHandler for NotificationHandler {
    async fn handle_event(&self, event: WorkflowEvent) -> Result<(), Box<dyn std::error::Error>> {
        match event.event_type {
            WorkflowEventType::HumanReviewRequired => {
                send_slack_notification("Security team review needed").await?;
            }
            WorkflowEventType::ToolSigned => {
                send_email_notification("Tool approved and signed").await?;
            }
            _ => {}
        }
        Ok(())
    }
}
```

## Contributing

1. Fork the repository
2. Create a feature branch
3. Add tests for new functionality
4. Ensure all tests pass: `cargo test`
5. Submit a pull request

### Development Setup

```bash
# Install development dependencies
cargo install cargo-watch
cargo install cargo-tarpaulin

# Run tests with coverage
cargo tarpaulin --out html

# Watch for changes during development
cargo watch -x test
```

## Testing

The system includes comprehensive testing:

```bash
# Unit tests
cargo test --lib

# Integration tests
cargo test --test integration

# End-to-end tests
cargo test --test e2e

# Performance tests
cargo test --release --test performance
```

## Performance

Typical performance characteristics:

- **Analysis Time**: 2-5 seconds for standard tools
- **Throughput**: 100+ concurrent reviews
- **Queue Processing**: < 1 second per item
- **API Response Time**: < 100ms (95th percentile)

## Troubleshooting

Common issues and solutions:

### Analysis Timeouts
- Increase `max_analysis_time_seconds` configuration
- Check RAG engine connectivity
- Monitor vector database performance

### Human Review Backlog
- Scale review team
- Implement auto-escalation rules
- Optimize review interface workflow

### Signing Failures
- Verify key availability and permissions
- Check HSM connectivity
- Monitor certificate expiration

## License

MIT License - see [LICENSE](../LICENSE) file for details.

## Support

- 📖 **Documentation**: [Complete workflow docs](tool_review_workflow.md)
- 🐛 **Issues**: [GitHub Issues](https://github.com/symbiont-platform/symbiont/issues)
- 💬 **Discussion**: [GitHub Discussions](https://github.com/symbiont-platform/symbiont/discussions)
- 📧 **Email**: tool-review-support@symbiont.platform

---

**Built with ❤️ by the Symbiont Platform Team**