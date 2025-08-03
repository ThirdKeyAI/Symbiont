# Symbi Agent Runtime System

A high-performance, secure runtime system for managing autonomous agents in the Symbi platform. Built in Rust with comprehensive security, context management, knowledge systems, and secure external tool integration.

## Overview

The Symbi Agent Runtime System provides a complete infrastructure for executing and managing autonomous agents with:

- **Multi-tier Security**: Tier1 (Docker), Tier2 (gVisor), Tier3 (Firecracker) sandboxing
- **Resource Management**: Memory, CPU, disk I/O, and network bandwidth allocation and monitoring
- **Priority-based Scheduling**: Efficient task scheduling with load balancing
- **Encrypted Communication**: Secure inter-agent messaging with Ed25519 signatures and AES-256-GCM encryption
- **Error Recovery**: Circuit breakers, retry strategies, and automatic recovery mechanisms
- **Audit Trail**: Cryptographic audit logging for compliance and debugging
- **Context Management**: Persistent agent memory and knowledge storage
- **RAG Engine**: Retrieval-augmented generation with semantic search
- **Vector Database**: Qdrant integration for embedding storage and similarity search
- **Secure MCP Integration**: Cryptographically verified external tool access
- **SchemaPin Security**: Tool verification with Trust-On-First-Use (TOFU)
- **AI Tool Review**: Automated security analysis and signing workflow
- **Policy Engine**: Resource access control with YAML-based policies
- **Basic Secrets Management**: Local encrypted file storage for secure configurations
- **Cryptographic CLI**: Tool for encrypting/decrypting secret files locally
- **Optional HTTP API**: RESTful API interface for external system integration (feature-gated)

## Architecture

```
┌───────────────────────────────────────────────────────────────────────────────┐
│                         Agent Runtime System                                  │
├─────────────────┬─────────────────┬─────────────────┬─────────────────────────┤
│   Scheduler     │   Lifecycle     │ Resource Manager│   Context Manager       │
│                 │   Controller    │                 │                         │
├─────────────────┼─────────────────┼─────────────────┼─────────────────────────┤
│ Communication   │ Error Handler   │   RAG Engine    │   Vector Database       │
│     Bus         │                 │                 │                         │
├─────────────────┼─────────────────┼─────────────────┼─────────────────────────┤
│  MCP Client     │ SchemaPin       │ Tool Review     │   Policy Engine         │
│                 │ Integration     │ Workflow        │                         │
├─────────────────┼─────────────────┼─────────────────┼─────────────────────────┤
│ HTTP API Server │  HTTP-Input     │                 │     (Optional)          │
│   (Optional)    │  (Optional)     │                 │                         │
└─────────────────┴─────────────────┴─────────────────┴─────────────────────────┘
```

## Quick Start

### Installation

Add to your `Cargo.toml`:

```toml
[dependencies]
symbi-runtime = "0.1.0"
tokio = { version = "1.0", features = ["full"] }
```

### Basic Usage

```rust
use symbi_runtime::*;
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create agent configuration
    let agent_config = AgentConfig {
        id: AgentId::new(),
        name: "example_agent".to_string(),
        dsl_source: "agent logic here".to_string(),
        execution_mode: ExecutionMode::Persistent,
        security_tier: SecurityTier::Tier2,
        resource_limits: ResourceLimits {
            memory_mb: 512,
            cpu_cores: 1.0,
            disk_io_mbps: 50,
            network_io_mbps: 10,
            execution_timeout: Duration::from_secs(3600),
            idle_timeout: Duration::from_secs(300),
        },
        capabilities: vec![Capability::FileSystem, Capability::Network],
        policies: vec![],
        metadata: std::collections::HashMap::new(),
        priority: Priority::Normal,
    };

    // Create and start agent
    let agent = AgentInstance::new(agent_config);
    println!("Agent {} created with state: {:?}", agent.id, agent.state);

    Ok(())
}
```

## Core Components

### 1. Agent Lifecycle Management

Manages agent states and transitions:

```rust
use symbi_runtime::lifecycle::*;

let config = LifecycleConfig {
    initialization_timeout: Duration::from_secs(60),
    termination_timeout: Duration::from_secs(30),
    state_check_interval: Duration::from_secs(5),
    enable_auto_recovery: true,
    max_restart_attempts: 3,
    max_agents: 100,
};

let lifecycle_controller = DefaultLifecycleController::new(config).await?;
```

**Agent States:**
- `Created` → `Initializing` → `Ready` → `Running`
- `Suspended`, `Waiting`, `Failed`, `Terminating`, `Terminated`

### 2. Resource Management

Tracks and enforces resource limits with policy integration:

```rust
use symbi_runtime::resource::*;

let config = ResourceManagerConfig {
    total_memory: 8192 * 1024 * 1024, // 8GB
    total_cpu_cores: 8,
    enforcement_enabled: true,
    monitoring_interval: Duration::from_secs(30),
    policy_enforcement: Some(PolicyEnforcementConfig {
        enabled: true,
        policy_file: "access_policies.yaml".to_string(),
    }),
    // ... other config
};

let resource_manager = DefaultResourceManager::new(config).await?;

// Allocate resources with policy checks
let allocation = resource_manager.allocate_resources(agent_id, limits).await?;
```

### 3. Context Management

Persistent agent memory and knowledge:

```rust
use symbi_runtime::context::*;

let config = ContextManagerConfig {
    storage_path: "./agent_contexts".to_string(),
    enable_compression: true,
    max_context_size_mb: 100,
    cleanup_interval: Duration::from_hours(24),
};

let context_manager = StandardContextManager::new(config).await?;

// Store agent context
let context = AgentContext {
    agent_id,
    conversation_history: vec![],
    knowledge_base: KnowledgeBase::new(),
    // ... other fields
};

context_manager.store_context(agent_id, context).await?;

// Add knowledge
context_manager.add_knowledge_item(agent_id, KnowledgeItem {
    content: "Important information".to_string(),
    metadata: HashMap::new(),
    timestamp: SystemTime::now(),
}).await?;
```

### 4. RAG Engine

Retrieval-augmented generation capabilities:

```rust
use symbi_runtime::rag::*;

let config = RAGConfig {
    max_documents: 10,
    relevance_threshold: 0.7,
    response_timeout: Duration::from_secs(30),
};

let rag_engine = StandardRAGEngine::new(
    config,
    context_manager,
    vector_db,
).await?;

// Process query with RAG
let request = RAGRequest {
    query: "What is machine learning?".to_string(),
    agent_id,
    max_results: 5,
    include_metadata: true,
};

let response = rag_engine.process_query(request).await?;
println!("RAG Response: {}", response.content);
```

### 5. Vector Database Integration

Semantic search with Qdrant:

```rust
use symbi_runtime::context::vector_db::*;

let config = VectorDbConfig {
    qdrant_url: "http://localhost:6333".to_string(),
    collection_name: "agent_knowledge".to_string(),
    vector_dimension: 384,
    distance_metric: DistanceMetric::Cosine,
};

let vector_db = QdrantClientWrapper::new(config).await?;

// Store knowledge with embeddings
vector_db.store_knowledge_item(KnowledgeItem {
    content: "Machine learning is a subset of AI".to_string(),
    metadata: metadata! {
        "topic" => "AI",
        "source" => "documentation"
    },
    timestamp: SystemTime::now(),
}).await?;

// Semantic search
let results = vector_db.semantic_search(
    "What is artificial intelligence?",
    5
).await?;
```

### 6. Secure MCP Integration

Cryptographically verified external tools:

```rust
use symbi_runtime::integrations::mcp::*;
use symbi_runtime::integrations::schemapin::*;

// Initialize SchemaPin for tool verification
let schemapin_config = SchemaPinConfig {
    binary_path: "/path/to/schemapin-cli".to_string(),
    timeout: Duration::from_secs(30),
    env_vars: HashMap::new(),
};

let schemapin = SchemaPinCliWrapper::new(schemapin_config).await?;

// Initialize key store for TOFU
let key_store_config = KeyStoreConfig {
    storage_path: "./keys".to_string(),
    file_permissions: 0o600,
};

let key_store = LocalKeyStore::new(key_store_config).await?;

// Create secure MCP client
let mcp_config = McpClientConfig {
    verification_enabled: true,
    connection_timeout: Duration::from_secs(30),
    max_concurrent_connections: 10,
};

let mcp_client = SecureMcpClient::new(
    mcp_config,
    schemapin,
    key_store,
).await?;

// Discover and verify tools
let tools = mcp_client.discover_tools("wss://tool-provider.com").await?;
for tool in tools {
    println!("Verified tool: {} (status: {:?})", tool.name, tool.verification_status);
}
```

### 7. AI Tool Review Workflow

Automated security analysis and signing:

```rust
use symbi_runtime::integrations::tool_review::*;

let review_config = ToolReviewConfig {
    enable_ai_analysis: true,
    require_human_review: true,
    auto_sign_threshold: 0.9,
};

let orchestrator = ToolReviewOrchestrator::new(
    review_config,
    ai_analyzer,
    review_interface,
    schemapin_cli,
).await?;

// Submit tool for review
let review_request = ToolReviewRequest {
    tool_name: "file_processor".to_string(),
    schema_content: tool_schema,
    provider_info: ProviderInfo {
        name: "TrustedProvider".to_string(),
        domain: "trusted.com".to_string(),
        public_key_url: "https://trusted.com/.well-known/keys".to_string(),
    },
    priority: ReviewPriority::Normal,
};

let review_id = orchestrator.submit_for_review(review_request).await?;

// Check review status
let status = orchestrator.get_review_status(review_id).await?;
match status.state {
    ReviewState::Approved => println!("Tool approved and signed"),
    ReviewState::Rejected => println!("Tool rejected: {}", status.rejection_reason.unwrap()),
    _ => println!("Review in progress"),
}
```

### 8. Policy Engine

Resource access control:

```rust
use symbi_runtime::integrations::policy_engine::*;

// Load policies from YAML
let policy_config = PolicyEnforcementConfig {
    policy_file: "access_policies.yaml".to_string(),
    enable_caching: true,
    cache_ttl: Duration::from_secs(300),
};

let policy_engine = DefaultPolicyEnforcementPoint::new(policy_config).await?;

// Check resource access
let access_request = ResourceAccessRequest {
    agent_id,
    resource_type: ResourceType::File,
    resource_path: "/sensitive/data.txt".to_string(),
    operation: Operation::Read,
    metadata: HashMap::new(),
};

let decision = policy_engine.evaluate_access(&access_request).await?;
match decision.decision {
    AccessDecision::Allow => {
        // Proceed with resource access
    }
    AccessDecision::Deny => {
        println!("Access denied: {}", decision.reason.unwrap());
    }
    _ => {
        // Handle other decision types
    }
}
### 9. Basic Secrets Management

Local encrypted file storage for secure configuration data:

```rust
use symbi_runtime::secrets::file_backend::*;
use symbi_runtime::crypto::*;

// Configure encrypted file storage
let file_config = FileBackendConfig {
    base_path: "./secrets".to_string(),
    file_extension: "enc".to_string(),
    permissions: 0o600,
};

let crypto = Aes256GcmCrypto::new();
let key_utils = KeyUtils::new();
let master_key = key_utils.get_or_create_key()?;

let file_backend = FileBackend::new(file_config, crypto, master_key).await?;

// Store encrypted secret
let secret = Secret::new("api_key", "secret_value_123")
    .with_metadata("environment", "development");

file_backend.store_secret("app/api_key", secret).await?;

// Retrieve a secret
let retrieved = file_backend.get_secret("app/api_key").await?;
println!("API Key: {}", retrieved.value);
```

#### CLI Usage

Encrypt and decrypt secret files:

```bash
# Encrypt a JSON configuration file
symbiont secrets encrypt --in config.json --out config.json.enc

# Decrypt and view
symbiont secrets decrypt --in config.json.enc

# Edit encrypted file in-place
symbiont secrets edit --file config.json.enc
```

```

### 10. Optional HTTP API

When enabled with the `http-api` feature, the runtime exposes a RESTful API:

```rust
#[cfg(feature = "http-api")]
use symbi_runtime::api::{HttpApiServer, HttpApiConfig};

// Configure HTTP API server
let api_config = HttpApiConfig {
    bind_address: "127.0.0.1".to_string(),
    port: 8080,
    enable_cors: true,
    enable_tracing: true,
};

// Create and start API server
let api_server = HttpApiServer::new(api_config);
api_server.start().await?;
```

#### Available Endpoints

- `GET /api/v1/health` - System health check
- `GET /api/v1/agents` - List all active agents (requires authentication)
- `GET /api/v1/agents/{id}/status` - Get specific agent status (requires authentication)
- `POST /api/v1/agents` - Create a new agent (requires authentication)
- `PUT /api/v1/agents/{id}` - Update an agent (requires authentication)
- `DELETE /api/v1/agents/{id}` - Delete an agent (requires authentication)
- `POST /api/v1/agents/{id}/execute` - Execute an agent (requires authentication)
- `GET /api/v1/agents/{id}/history` - Get agent execution history (requires authentication)
- `POST /api/v1/workflows/execute` - Execute workflows
- `GET /api/v1/metrics` - System performance metrics

> **Note:** All `/api/v1/agents*` endpoints require Bearer token authentication. Set the `API_AUTH_TOKEN` environment variable and use the header:
> `Authorization: Bearer <your-token>`

#### Example Usage

```bash
# Check system health
curl http://localhost:8080/api/v1/health

# List all agents
curl http://localhost:8080/api/v1/agents

# Execute a workflow
curl -X POST http://localhost:8080/api/v1/workflows/execute \
  -H "Content-Type: application/json" \
  -d '{"workflow_id": "example", "parameters": {}}'
```

#### Enable HTTP API

Add to your `Cargo.toml`:

```toml
[dependencies]
symbi-runtime = { version = "0.1.0", features = ["http-api"] }
```

Or build with feature flag:

```bash
cargo build --features http-api
```

## Security Features

### Sandboxing

- **Tier 1 (Docker)**: Container isolation with resource limits and security hardening
- **Enhanced Isolation**: Additional tiers available in Enterprise edition

### SchemaPin Cryptographic Security

- **Tool Verification**: ECDSA P-256 signatures for all external tools
- **Trust-On-First-Use**: Key pinning prevents man-in-the-middle attacks
- **Automated Review**: AI-driven security analysis of tool schemas
- **Human Oversight**: Configurable human review for high-risk tools

### Encryption

- **Message Encryption**: AES-256-GCM for message payloads
- **Digital Signatures**: Ed25519 for message authentication
- **Key Management**: Secure key generation, storage, and rotation
- **Schema Signing**: ECDSA P-256 for tool schema verification

### Policy-Based Access Control

- **YAML Configuration**: Human-readable policy definitions
- **Resource Types**: File, network, database, API access control
- **Dynamic Enforcement**: Real-time policy evaluation
- **Audit Logging**: Complete access decision tracking

### Audit Trail

All operations are logged with cryptographic integrity:

```rust
use symbi_runtime::integrations::audit_trail::*;

let audit_trail = MockAuditTrail::new().await?;

// Record an event
let event = AuditEvent {
    id: AuditId::new(),
    timestamp: SystemTime::now(),
    event_type: AuditEventType::ToolInvoked,
    agent_id: Some(agent_id),
    details: "External tool executed successfully".to_string(),
    metadata: HashMap::new(),
};

audit_trail.record_event(event).await?;
```

## Configuration

### Environment Variables

- `SYMBI_LOG_LEVEL`: Set logging level (debug, info, warn, error)
- `SYMBI_MAX_AGENTS`: Maximum number of concurrent agents
- `SYMBI_RESOURCE_ENFORCEMENT`: Enable/disable resource enforcement
- `SYMBI_QDRANT_URL`: Qdrant vector database URL
- `SYMBI_SCHEMAPIN_PATH`: Path to SchemaPin CLI binary

### Configuration Files

Create `runtime_config.toml`:

```toml
[lifecycle]
initialization_timeout = "60s"
termination_timeout = "30s"
max_agents = 100

[resource]
total_memory = 8589934592  # 8GB
total_cpu_cores = 8
enforcement_enabled = true

[context]
storage_path = "./agent_contexts"
enable_compression = true
max_context_size_mb = 100

[vector_db]
qdrant_url = "http://localhost:6333"
collection_name = "agent_knowledge"
vector_dimension = 384

[mcp]
verification_enabled = true
connection_timeout = "30s"
max_concurrent_connections = 10

[schemapin]
binary_path = "/usr/local/bin/schemapin-cli"
timeout = "30s"

[policy]
policy_file = "access_policies.yaml"
enable_caching = true
cache_ttl = "300s"

[scheduler]
max_concurrent_tasks = 100
load_balancing_strategy = "ResourceBased"

[communication]
enable_encryption = true
max_message_size = 1048576  # 1MB

[error_handler]
enable_auto_recovery = true
max_recovery_attempts = 3

# Optional HTTP API configuration (only used if http-api feature is enabled)
[http_api]
bind_address = "127.0.0.1"
port = 8080
enable_cors = true
enable_tracing = true
```

Create `access_policies.yaml`:

```yaml
version: "1.0"
policies:
  - name: "default_file_access"
    priority: 100
    conditions:
      - resource_type: "File"
      - agent_security_level: "standard"
    effect: "Allow"
    resources:
      - "/tmp/**"
      - "/workspace/**"
    operations: ["Read", "Write"]
    
  - name: "network_restrictions"
    priority: 200
    conditions:
      - resource_type: "Network"
    effect: "Deny"
    resources:
      - "127.0.0.1/**"
      - "localhost/**"
    operations: ["Connect"]
```

## Testing

Run all tests:

```bash
cd crates/runtime
cargo test
```

Run specific test suites:

```bash
# Unit tests only
cargo test --lib

# Integration tests
cargo test --test integration_tests
cargo test --test rag_integration_tests
cargo test --test mcp_client_tests
cargo test --test schemapin_integration_tests
cargo test --test policy_engine_tests

# Specific module tests
cargo test context::tests
cargo test rag::tests
cargo test integrations::mcp::tests
```

## Performance

### Benchmarks

- **Agent Creation**: ~1ms per agent
- **Message Throughput**: 10,000+ messages/second
- **Resource Allocation**: ~100μs per allocation
- **State Transitions**: ~50μs per transition
- **Context Retrieval**: <50ms average
- **Vector Search**: <100ms for 1M+ embeddings
- **RAG Pipeline**: <500ms end-to-end
- **Schema Verification**: <100ms per tool
- **Policy Evaluation**: <1ms per access check

### Memory Usage

- **Base Runtime**: ~10MB
- **Per Agent**: ~1-5MB (depending on configuration)
- **Context Manager**: ~256MB per agent (peak)
- **Vector Database**: Configurable with compression
- **Security Components**: ~2MB overhead per agent
- **Message Buffers**: Configurable (default 1MB per agent)

## Error Codes

| Code | Description |
|------|-------------|
| `AGENT_001` | Agent not found |
| `RESOURCE_001` | Insufficient resources |
| `COMM_001` | Message delivery failed |
| `SCHED_001` | Task scheduling failed |
| `SEC_001` | Security violation |
| `CTX_001` | Context storage error |
| `RAG_001` | RAG processing error |
| `VDB_001` | Vector database error |
| `MCP_001` | MCP connection error |
| `SCHEMA_001` | Schema verification failed |
| `POLICY_001` | Policy violation |

## Examples

### Context and RAG Example

```bash
cargo run --example context_example
cargo run --example rag_example
```

### Persistence Testing

```bash
cargo run --example context_persistence_test
```

### Full System Example

```bash
cargo run --example full_system
```

## Contributing

1. Fork the repository
2. Create a feature branch
3. Add tests for new functionality
4. Ensure all tests pass: `cargo test`
5. Update documentation
6. Submit a pull request

## License

Licensed under the MIT License. See `LICENSE` file for details.

## Support

For issues and questions:
- GitHub Issues: [symbiont-runtime/issues](https://github.com/symbiont/runtime/issues)
- Documentation: [docs.symbiont.dev](https://docs.symbiont.dev)

## Roadmap

### ✅ Phase 1: Core Infrastructure (COMPLETED)
- [x] Agent Runtime Scheduler
- [x] Agent Lifecycle Controller  
- [x] Resource Manager
- [x] Communication Bus
- [x] Error Handler

### ✅ Phase 2: Advanced Features (COMPLETED)
- [x] Multi-tier security integration
- [x] Policy enforcement hooks
- [x] Comprehensive testing
- [x] Performance optimization

### ✅ Phase 3: Production Readiness (COMPLETED)
- [x] Complete audit trail integration
- [x] Advanced monitoring
- [x] Security hardening
- [x] Documentation

### ✅ Phase 4: Context & Knowledge Systems (COMPLETED)
- [x] Agent Context Manager with persistent storage
- [x] Vector Database integration (Qdrant)
- [x] RAG Engine implementation
- [x] Knowledge persistence and sharing
- [x] Semantic search capabilities

### ✅ Phase 5: Secure MCP Integration (COMPLETED)
- [x] SchemaPin cryptographic verification
- [x] Trust-On-First-Use (TOFU) key management
- [x] Secure MCP Client implementation
- [x] AI-driven tool review and signing workflow
- [x] Tool invocation security enforcement
- [x] Resource access management with policy engine
- [x] Complete end-to-end security framework

### ✅ Phase 6: Basic Secrets Management (COMPLETED)
- [x] Encrypted file backend with AES-256-GCM encryption
- [x] CLI tools for secret encryption/decryption operations
- [x] Cross-platform file-based secret storage
- [x] Integration with existing runtime components

### 🚧 Phase 7: Advanced Intelligence (PLANNED)
- [ ] Multi-modal RAG support (images, audio, structured data)
- [ ] Cross-agent knowledge synthesis with knowledge graphs
- [ ] Intelligent context management with adaptive pruning
- [ ] Advanced learning capabilities with federated learning
- [ ] Performance optimization and intelligent caching

## Architecture Achievements

The Symbi Agent Runtime System now represents a complete, production-ready platform for secure, intelligent agent deployment with:

### Security Excellence
- **Zero-Trust Architecture**: All external tools cryptographically verified
- **Defense in Depth**: Multiple security layers from sandboxing to policy enforcement
- **Audit Compliance**: Complete cryptographic audit trails for all operations
- **Attack Prevention**: SchemaPin prevents tool substitution and supply chain attacks

### Intelligence Capabilities  
- **Semantic Understanding**: Vector-based knowledge storage and retrieval
- **Context Awareness**: Persistent agent memory across sessions
- **Knowledge Synthesis**: RAG-powered intelligent responses
- **Learning Systems**: Knowledge sharing between agents

### Enterprise Ready
- **Scalable Architecture**: Support for thousands of concurrent agents
- **Policy Governance**: Fine-grained access control with YAML policies
- **Integration Ready**: Secure external tool and service integration
- **Performance Optimized**: Sub-millisecond operation latencies

The platform successfully bridges the gap between secure execution and intelligent operation, creating a foundation for next-generation AI agent systems that are both powerful and trustworthy.
