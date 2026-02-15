# Symbiont Example Agents

This directory contains ten reusable agent examples that demonstrate core Symbiont capabilities and common use cases. These agents serve as both learning resources and production-ready templates for building your own intelligent automation workflows.

## 📋 Overview

| Agent | Purpose | Key Features |
|-------|---------|--------------|
| [NLP Processor](#nlp-processor) | Text analysis and processing | Sentiment analysis, entity extraction, summarization |
| [Data Validator](#data-validator) | Data quality assessment | Schema validation, quality scoring, error reporting |
| [Format Converter](#format-converter) | File format transformation | Multi-format support, error handling, metadata preservation |
| [API Aggregator](#api-aggregator) | External service integration | Multi-source data collection, response normalization |
| [Security Scanner](#security-scanner) | Security assessment | Vulnerability detection, compliance checking, risk scoring |
| [Webhook Handler](#webhook-handler) | HTTP webhook processing | Webhook block, provider presets, event filtering |
| [Workflow Orchestrator](#workflow-orchestrator) | Process automation | Multi-step workflows, dependency management, error recovery |
| [Notification Router](#notification-router) | Event-driven messaging | Multi-channel notifications, routing rules, rate limiting |
| [Knowledge Curator](#knowledge-curator) | Persistent knowledge base | Memory block, hybrid search, fact extraction |
| [Incident Tracker](#incident-tracker) | DevOps incident response | Webhook + memory blocks, deduplication, alert routing |

### v1.4.0 DSL Features

Two new top-level blocks were added in v1.4.0:

- **`memory` block** — Persistent markdown-backed storage with hybrid search (vector + keyword). See [Knowledge Curator](#knowledge-curator) and [Incident Tracker](#incident-tracker).
- **`webhook` block** — First-class webhook ingestion with provider presets (GitHub, Stripe, Slack, Custom) and JSON path filtering. See [Webhook Handler](#webhook-handler) and [Incident Tracker](#incident-tracker).

## 🚀 Quick Start

### Prerequisites

1. **Symbiont Runtime**: Ensure you have the Symbiont runtime installed and configured
2. **Dependencies**: Some agents require external services (detailed below)
3. **Permissions**: Verify your user has appropriate capabilities for agent operations

### Installation

```bash
# Clone the repository
git clone https://github.com/thirdkeyai/symbiont.git
cd symbiont

# Build the runtime
cargo build --release

# Copy example agents to your workspace
cp examples/agents/* ./agents/
```

### Basic Usage

```bash
# Parse and validate an agent definition
cargo run -- dsl parse agents/nlp_processor.dsl

# Run an agent in the runtime
cd crates/runtime
cargo run --example basic_agent -- --agent ../../agents/nlp_processor.dsl
```

---

## 🔤 NLP Processor

**Purpose**: Multi-purpose text analysis agent for natural language processing tasks.

### Features
- Sentiment analysis
- Named entity extraction
- Text summarization
- Keyword extraction
- Language detection

### Usage Example

```bash
# Example: Analyze customer feedback
echo '{
  "text": "I love this product! The customer service was excellent and delivery was fast.",
  "tasks": ["sentiment", "entities", "keywords"]
}' | cargo run --example basic_agent -- --agent agents/nlp_processor.dsl
```

### Expected Output
```json
{
  "sentiment": {
    "score": 0.9,
    "label": "positive",
    "confidence": 0.95
  },
  "entities": [
    {"text": "customer service", "type": "SERVICE"},
    {"text": "delivery", "type": "SERVICE"}
  ],
  "keywords": ["product", "excellent", "fast", "customer service"]
}
```

### Dependencies
- **LLM Access**: Requires configured language model for summarization
- **NLP Libraries**: Sentiment analysis and entity extraction models

### Configuration
```toml
[nlp_processor]
max_text_length = 50000
enable_pii_detection = true
supported_languages = ["en", "es", "fr", "de"]
```

---

## ✅ Data Validator

**Purpose**: Comprehensive data quality assessment and validation agent.

### Features
- Schema validation
- Data type checking
- Quality scoring
- Error reporting with details
- Statistical analysis

### Usage Example

```bash
# Example: Validate customer data
echo '{
  "data": {
    "records": [
      {"name": "John Doe", "email": "john@example.com", "age": 30},
      {"name": "", "email": "invalid-email", "age": -5}
    ]
  },
  "schema": {
    "fields": [
      {"name": "name", "type": "string", "required": true, "min_length": 1},
      {"name": "email", "type": "email", "required": true},
      {"name": "age", "type": "integer", "min": 0, "max": 120}
    ]
  }
}' | cargo run --example basic_agent -- --agent agents/data_validator.dsl
```

### Expected Output
```json
{
  "valid_records": 1,
  "invalid_records": 1,
  "quality_score": 0.5,
  "errors": [
    {
      "record_index": 1,
      "field": "name",
      "error": "Required field is empty"
    },
    {
      "record_index": 1,
      "field": "email", 
      "error": "Invalid email format"
    },
    {
      "record_index": 1,
      "field": "age",
      "error": "Value -5 is below minimum 0"
    }
  ],
  "statistics": {
    "total_records": 2,
    "completion_rate": 0.83
  }
}
```

### Use Cases
- ETL pipeline validation
- API input validation
- Data migration quality checks
- ML dataset preparation

---

## 🔄 Format Converter

**Purpose**: Universal file format conversion with intelligent format detection.

### Features
- Auto-format detection
- Multi-format support (JSON, CSV, XML, YAML)
- Metadata preservation
- Error handling and recovery
- Large file support

### Usage Example

```bash
# Example: Convert CSV to JSON
echo '{
  "input_file": {
    "path": "/data/customers.csv",
    "content": "name,email,age\nJohn,john@example.com,30\nJane,jane@example.com,25"
  },
  "target_format": "json"
}' | cargo run --example basic_agent -- --agent agents/format_converter.dsl
```

### Expected Output
```json
{
  "success": true,
  "message": "Conversion completed successfully",
  "output_file": {
    "path": "/data/customers.json",
    "format": "json",
    "size_bytes": 156
  },
  "source_format": "csv",
  "target_format": "json",
  "conversion_time_ms": 45
}
```

### Supported Formats
- **Input**: CSV, JSON, XML, YAML, TSV
- **Output**: JSON, CSV, XML, YAML
- **Planned**: Parquet, Avro, Protocol Buffers

### Configuration
```toml
[format_converter]
max_file_size_mb = 100
preserve_metadata = true
output_directory = "/tmp/converted"
compression_enabled = false
```

---

## 🌐 API Aggregator

**Purpose**: Collect and merge data from multiple external APIs with unified response format.

### Features
- Multi-source data collection
- Response normalization
- Error handling per source
- Rate limiting compliance
- Parallel processing

### Usage Example

```bash
# Example: Aggregate weather data from multiple sources
echo '{
  "sources": [
    {
      "name": "weather_api",
      "endpoint": "https://api.weather.com/v1/current",
      "auth_key": "vault://api_keys/weather",
      "schema": "weather_v1"
    },
    {
      "name": "backup_weather",
      "endpoint": "https://backup-weather.com/api/current",
      "auth_key": "vault://api_keys/backup_weather", 
      "schema": "weather_v2"
    }
  ],
  "query": "New York, NY"
}' | cargo run --example basic_agent -- --agent agents/api_aggregator.dsl
```

### Expected Output
```json
{
  "query": "New York, NY",
  "sources_queried": 2,
  "successful_responses": 2,
  "aggregated_results": {
    "temperature": {
      "celsius": 22,
      "fahrenheit": 72,
      "sources": ["weather_api", "backup_weather"],
      "confidence": 0.95
    },
    "conditions": "partly cloudy",
    "humidity": 65,
    "last_updated": "2024-01-15T10:30:00Z"
  },
  "source_details": [
    {
      "source": "weather_api",
      "response_time_ms": 245,
      "status": "success"
    },
    {
      "source": "backup_weather", 
      "response_time_ms": 892,
      "status": "success"
    }
  ]
}
```

### Use Cases
- Multi-vendor price comparison
- Social media sentiment aggregation
- Financial data consolidation
- News article compilation

### Dependencies
- **Secret Management**: Vault or file-based secret storage
- **Network Access**: External API connectivity
- **API Keys**: Valid authentication for each source

---

## 🔒 Security Scanner

**Purpose**: Comprehensive security assessment and vulnerability detection.

### Features
- Vulnerability scanning
- Compliance checking (SOC2, HIPAA, GDPR)
- Risk scoring and prioritization
- Remediation recommendations
- Audit trail generation

### Usage Example

```bash
# Example: Scan web application for vulnerabilities
echo '{
  "target": {
    "identifier": "webapp.company.com",
    "type": "web_application",
    "classification": "internal"
  },
  "scan_type": "comprehensive"
}' | cargo run --example basic_agent -- --agent agents/security_scanner.dsl
```

### Expected Output
```json
{
  "target": "webapp.company.com",
  "scan_type": "comprehensive",
  "start_time": "2024-01-15T10:00:00Z",
  "end_time": "2024-01-15T10:15:00Z",
  "vulnerabilities": [
    {
      "id": "CVE-2023-1234",
      "severity": "high",
      "description": "SQL injection vulnerability in user input form",
      "location": "/api/user/search",
      "cvss_score": 8.5
    }
  ],
  "compliance_status": {
    "SOC2": {
      "compliant": true,
      "findings": []
    },
    "HIPAA": {
      "compliant": false,
      "findings": ["Insufficient data encryption"]
    }
  },
  "risk_score": 7.2,
  "recommendations": [
    {
      "priority": "high",
      "action": "Implement parameterized queries",
      "timeline": "immediate"
    }
  ]
}
```

### Scan Types
- **vulnerability**: Focus on security vulnerabilities
- **compliance**: Check regulatory compliance
- **comprehensive**: Full security assessment

### Requirements
- **Security Clearance**: User must have security analyst role
- **Target Authorization**: Scan target must be owned/authorized
- **Approved Scope**: Scan parameters must be pre-approved

---

## 🪝 Webhook Handler

**Purpose**: Process incoming HTTP webhooks with intelligent filtering and event routing. Updated in v1.4.0 with a first-class `webhook` block.

### v1.4.0 Features Demonstrated

- **`webhook` block** — Top-level `webhook slack_alerts` with Slack provider preset and JSON path event filter

```
webhook slack_alerts {
    path     "/hooks/slack"
    provider slack
    secret   "vault://webhooks/slack/secret"
    agent    webhook_handler
    filter {
        json_path "$.type"
        equals    "security_alert"
    }
}
```

### Features
- First-class webhook block with provider presets
- HMAC-SHA256 signature verification
- Event type filtering via JSON path
- Security alert processing
- Topic-based publishing
- Source validation
- Audit trail for all operations

### Usage Example

```bash
# Example: Process security alert webhook
echo '{
  "type": "security_alert",
  "source": "slack",
  "user": "security@company.com",
  "message": "Suspicious login attempt detected",
  "severity": "high",
  "metadata": {
    "ip_address": "192.168.1.100",
    "timestamp": "2024-01-15T10:30:00Z",
    "login_attempts": 5
  }
}' | cargo run --example basic_agent -- --agent agents/webhook_handler.dsl
```

### Expected Output
```json
{
  "summary": "Suspicious login attempt detected",
  "source": "slack",
  "level": "high",
  "user": "security@company.com"
}
```

### Supported Event Types
- **security_alert**: Security incidents and threats
- **system_event**: System status and health events (planned)
- **user_action**: User activity events (planned)

### Security Features
- **Source Validation**: Only allows LLM usage from trusted sources (Slack, company emails)
- **Alert Publishing**: Authorized security alerts are published to alert topics
- **Audit Logging**: All operations are audited for compliance
- **Privacy**: Strict privacy mode with ephemeral memory

### Use Cases
- Security incident ingestion
- Webhook-to-event-stream bridge
- Real-time alert processing
- External system integration

### Configuration
```toml
[webhook_handler]
allowed_sources = ["slack", "github", "monitoring"]
company_domain = "company.com"
alert_topic = "topic://alerts"
audit_enabled = true
```

---

## 🔗 Workflow Orchestrator

**Purpose**: Coordinate complex multi-step workflows with dependency management.

### Features
- Multi-step workflow execution
- Dependency resolution
- Error recovery and rollback
- Progress tracking
- Conditional branching

### Usage Example

```bash
# Example: Data processing pipeline
echo '{
  "workflow_definition": {
    "name": "data_processing_pipeline",
    "steps": [
      {
        "name": "validate_data",
        "agent": "data_validator",
        "input": "raw_data",
        "required": true
      },
      {
        "name": "convert_format",
        "agent": "format_converter", 
        "input": "validated_data",
        "depends_on": ["validate_data"],
        "required": true
      },
      {
        "name": "analyze_text",
        "agent": "nlp_processor",
        "input": "converted_data",
        "depends_on": ["convert_format"],
        "required": false
      }
    ],
    "allowed_agents": ["data_validator", "format_converter", "nlp_processor"]
  }
}' | cargo run --example basic_agent -- --agent agents/workflow_orchestrator.dsl
```

### Expected Output
```json
{
  "success": true,
  "message": "Workflow completed successfully",
  "execution_context": {
    "workflow_id": "wf_20240115_103000_abc123",
    "start_time": "2024-01-15T10:30:00Z",
    "end_time": "2024-01-15T10:33:45Z",
    "steps_completed": 3,
    "total_steps": 3,
    "results": {
      "validate_data": {
        "success": true,
        "quality_score": 0.95
      },
      "convert_format": {
        "success": true,
        "output_format": "json"
      },
      "analyze_text": {
        "success": true,
        "sentiment_score": 0.8
      }
    }
  }
}
```

### Workflow Features
- **Dependency Management**: Steps wait for prerequisites
- **Error Handling**: Optional vs required step failures
- **State Persistence**: Workflow state survives restarts
- **Parallel Execution**: Independent steps run concurrently

---

## 📢 Notification Router

**Purpose**: Event-driven notification delivery across multiple channels.

### Features
- Multi-channel delivery (email, Slack, SMS, webhooks)
- Smart routing based on event severity
- Rate limiting and throttling
- Consent management
- Delivery tracking

### Usage Example

```bash
# Example: Route security alert
echo '{
  "event": {
    "id": "sec_alert_001",
    "type": "security_incident",
    "severity": "high",
    "title": "Suspicious login detected",
    "description": "Multiple failed login attempts from unknown IP",
    "source": "auth_service",
    "metadata": {
      "ip_address": "192.168.1.100",
      "user": "admin@company.com",
      "attempts": 5
    }
  },
  "routing_rules": {
    "channels": [
      {
        "type": "email",
        "condition": "severity >= medium",
        "recipients": ["security@company.com", "admin@company.com"]
      },
      {
        "type": "slack",
        "condition": "severity == high",
        "webhook_url": "vault://webhooks/security_slack"
      },
      {
        "type": "sms",
        "condition": "severity == critical",
        "phone_numbers": ["+1234567890"]
      }
    ]
  }
}' | cargo run --example basic_agent -- --agent agents/notification_router.dsl
```

### Expected Output
```json
{
  "event_id": "sec_alert_001",
  "notifications_sent": 2,
  "delivery_results": [
    {
      "channel": "email",
      "result": {
        "success": true,
        "recipients_reached": 2,
        "message_id": "email_20240115_103000"
      }
    },
    {
      "channel": "slack",
      "result": {
        "success": true,
        "channel": "#security-alerts",
        "timestamp": "2024-01-15T10:30:00Z"
      }
    }
  ],
  "timestamp": "2024-01-15T10:30:00Z"
}
```

### Channel Types
- **Email**: SMTP-based email delivery
- **Slack**: Webhook-based Slack messages
- **SMS**: Twilio or similar SMS providers
- **Webhook**: Custom HTTP endpoints
- **Push**: Mobile push notifications (planned)

---

## 🧠 Knowledge Curator

**Purpose**: Persistent knowledge base agent with hybrid search powered by the v1.4.0 `memory` block.

### v1.4.0 Features Demonstrated

- **`memory` block** — Top-level `memory knowledge_store` with markdown store, 365-day retention, and hybrid search weights
- **Hybrid search** — Configurable vector (0.6) and keyword (0.4) weighting for retrieval
- **Intent classification** — Routes queries to store, search, or summarize paths

### DSL Highlights

```
memory knowledge_store {
    store     markdown
    path      "data/knowledge"
    retention 365d
    search {
        vector_weight  0.6
        keyword_weight 0.4
    }
}
```

### Usage Example

```bash
# Store a document
echo '{
  "query": "Store this architecture decision",
  "context": {
    "document": "We chose PostgreSQL for the primary datastore...",
    "source": "adr-001",
    "user": {"name": "alice", "role": "editor"}
  }
}' | cargo run --example basic_agent -- --agent agents/knowledge_curator.dsl

# Search the knowledge base
echo '{
  "query": "What database did we choose?",
  "context": {"user": {"name": "bob", "role": "viewer"}}
}' | cargo run --example basic_agent -- --agent agents/knowledge_curator.dsl
```

### Use Cases
- Architecture decision records
- Onboarding knowledge bases
- Research paper indexing
- Team runbook management

---

## 🚨 Incident Tracker

**Purpose**: DevOps incident tracking combining webhook ingestion with persistent memory for history, deduplication, and resolution hints. Demonstrates both v1.4.0 features together.

### v1.4.0 Features Demonstrated

- **`memory` block** — Persistent incident history with 2-year retention and balanced hybrid search
- **`webhook` block** — Multiple webhook endpoints (GitHub, Stripe) with provider presets and JSON path filters
- **Deduplication** — Searches memory for existing incidents before creating new ones
- **Resolution hints** — Retrieves similar past incidents to suggest fixes

### DSL Highlights

```
memory incident_history {
    store     markdown
    path      "data/incidents"
    retention 730d
    search {
        vector_weight  0.5
        keyword_weight 0.5
    }
}

webhook github_incidents {
    path     "/hooks/github"
    provider github
    secret   "vault://webhooks/github/secret"
    agent    incident_tracker
    filter {
        json_path "$.action"
        equals    "created"
    }
}

webhook stripe_failures {
    path     "/hooks/stripe"
    provider stripe
    secret   "vault://webhooks/stripe/secret"
    agent    incident_tracker
    filter {
        json_path "$.type"
        contains  "failed"
    }
}
```

### Usage Example

```bash
# Process a GitHub security advisory
echo '{
  "source": "github",
  "action": "created",
  "verified": true,
  "summary": "Critical dependency vulnerability in lodash",
  "severity": "critical"
}' | cargo run --example basic_agent -- --agent agents/incident_tracker.dsl
```

### Expected Output
```json
{
  "status": "created",
  "incident_id": "inc_20260215_001",
  "severity": "critical",
  "similar_past_incidents": 2,
  "resolution_hints": ["Upgrade lodash to 4.17.21", "Run npm audit fix"]
}
```

### Use Cases
- Security advisory tracking
- Payment failure monitoring
- Deployment failure correlation
- On-call incident management

---

## 🛠️ Configuration

### Global Configuration

Create `symbiont.toml` in your project root:

```toml
[runtime]
max_agents = 50
execution_timeout_seconds = 300
audit_enabled = true

[security]
default_sandbox_tier = "docker"
policy_enforcement = "strict"

[secrets]
backend = "vault"
vault_endpoint = "https://vault.company.com"

[integrations]
enable_llm = true
enable_external_apis = true

# Agent-specific configurations
[agents.nlp_processor]
max_text_length = 50000
enable_pii_detection = true

[agents.security_scanner]
require_approval = true
max_scan_duration = 1800

[agents.api_aggregator]
default_timeout = 30
max_concurrent_requests = 10
```

### Environment Variables

```bash
# Core runtime
export SYMBI_LOG_LEVEL=info
export SYMBI_CONFIG_PATH=./symbiont.toml

# Security
export VAULT_ADDR=https://vault.company.com
export VAULT_TOKEN=your_vault_token

# External integrations
export OPENAI_API_KEY=your_openai_key
export SLACK_WEBHOOK_URL=your_slack_webhook
```

## 🔧 Development Tips

### Testing Agents

```bash
# Validate agent syntax
cargo run -- dsl parse agents/your_agent.dsl

# Test with mock data
echo '{"test": "data"}' | cargo run --example basic_agent -- --agent agents/your_agent.dsl

# Run integration tests
cargo test --test agent_integration_tests
```

### Debugging

```bash
# Enable debug logging
export RUST_LOG=debug

# Use the runtime with verbose output
cargo run --example full_system -- --verbose
```

### Custom Agents

Use these examples as templates:

1. Copy an existing agent that's closest to your use case
2. Modify the capabilities and policies
3. Update the agent logic in the `with` block
4. Test thoroughly with representative data
5. Deploy to your Symbiont runtime

## 📚 Next Steps

- **[DSL Guide](https://docs.symbiont.dev/dsl-guide)** - Learn advanced DSL features
- **[Security Model](https://docs.symbiont.dev/security-model)** - Understand security implementation
- **[Runtime Architecture](https://docs.symbiont.dev/runtime-architecture)** - Deep dive into the runtime
- **[API Reference](https://docs.symbiont.dev/api-reference)** - Complete API documentation

## 📄 License

These example agents are provided under the MIT License. See [LICENSE](../LICENSE) for details.