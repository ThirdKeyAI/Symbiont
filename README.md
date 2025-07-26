<img src="logo-hz.png" alt="Symbi">

**Symbi** is an AI-native agent framework for building autonomous, policy-aware agents that can safely collaborate with humans, other agents, and large language models. The Community edition provides core functionality with optional Enterprise features for advanced security, monitoring, and collaboration.

## 🚀 Quick Start

### Prerequisites
- Docker (recommended) or Rust 1.88+
- Qdrant vector database (for semantic search)

### Running with Pre-built Containers

**Using GitHub Container Registry (Recommended):**

```bash
# Run unified symbi CLI
docker run --rm -v $(pwd):/workspace ghcr.io/thirdkeyai/symbi:latest dsl parse /workspace/agent.dsl

# Run MCP Server
docker run --rm -p 8080:8080 ghcr.io/thirdkeyai/symbi:latest mcp

# Interactive development
docker run --rm -it -v $(pwd):/workspace ghcr.io/thirdkeyai/symbi:latest bash
```

### Building from Source

```bash
# Build development environment
docker build -t symbi:latest .
docker run --rm -it -v $(pwd):/workspace symbi:latest bash

# Build the unified symbi binary
cargo build --release

# Test the components
cargo test

# Run example agents (from crates/runtime)
cd crates/runtime && cargo run --example basic_agent
cd crates/runtime && cargo run --example full_system
cd crates/runtime && cargo run --example rag_example

# Use the unified symbi CLI
cargo run -- dsl parse my_agent.dsl
cargo run -- mcp --port 8080

# Enable HTTP API (optional)
cd crates/runtime && cargo run --features http-api --example full_system
```

### Optional HTTP API

Enable RESTful HTTP API for external integration:

```bash
# Build with HTTP API feature
cargo build --features http-api

# Or add to Cargo.toml
[dependencies]
symbi-runtime = { version = "0.1.0", features = ["http-api"] }
```

**Key Endpoints:**
- `GET /api/v1/health` - Health check and system status
- `GET /api/v1/agents` - List all active agents
- `POST /api/v1/workflows/execute` - Execute workflows
- `GET /api/v1/metrics` - System metrics

## 📁 Project Structure

```
symbi/
├── src/                   # Unified symbi CLI binary
├── crates/                # Workspace crates
│   ├── dsl/              # Symbi DSL implementation
│   │   ├── src/          # Parser and library code
│   │   ├── tests/        # DSL test suite
│   │   └── tree-sitter-symbiont/ # Grammar definition
│   └── runtime/          # Agent Runtime System (Community)
│       ├── src/          # Core runtime components
│       ├── examples/     # Usage examples
│       └── tests/        # Integration tests
├── docs/                 # Documentation
└── Cargo.toml           # Workspace configuration
```

## 🔧 Features

### ✅ Community Features (OSS)
- **DSL Grammar**: Complete Tree-sitter grammar for agent definitions
- **Agent Runtime**: Task scheduling, resource management, lifecycle control
- **Tier 1 Sandboxing**: Docker containerized isolation for agent operations
- **MCP Integration**: Model Context Protocol client for external tools
- **SchemaPin Security**: Basic cryptographic tool verification 
- **RAG Engine**: Retrieval-augmented generation with vector search
- **Context Management**: Persistent agent memory and knowledge storage
- **Vector Database**: Qdrant integration for semantic search
- **Basic Secrets Management**: Local encrypted file storage for configurations
- **Cryptographic CLI**: Tool for encrypting/decrypting secret files
- **HTTP API**: Optional RESTful interface (feature-gated)

### 🏢 Enterprise Features (License Required)
- **Advanced Sandboxing**: gVisor and Firecracker isolation **(Enterprise)**
- **AI Tool Review**: Automated security analysis workflow **(Enterprise)**
- **Cryptographic Audit**: Complete audit trails with Ed25519 signatures **(Enterprise)**
- **Multi-Agent Communication**: Encrypted inter-agent messaging **(Enterprise)**
- **Real-time Monitoring**: SLA metrics and performance dashboards **(Enterprise)**
- **Professional Services and Support**: Custom development and support **(Enterprise)**

## 📐 Symbiont DSL

Define intelligent agents with built-in policies and capabilities:

```symbiont
metadata {
    version = "1.0.0"
    author = "Your Name"
    description = "Data analysis agent"
}

agent analyze_data(input: DataSet) -> Result {
    capabilities = ["data_analysis", "visualization"]
    
    policy data_privacy {
        allow: read(input) if input.anonymized == true
        deny: store(input) if input.contains_pii == true
        audit: all_operations
    }
    
    with memory = "persistent", requires = "approval" {
        if (llm_check_safety(input)) {
            result = analyze(input);
            return result;
        } else {
            return reject("Safety check failed");
        }
    }
}
```

## 🔒 Security Model

### Basic Security (Community)
- **Tier 1 Isolation**: Docker containerized agent execution
- **Schema Verification**: Cryptographic tool validation with SchemaPin
- **Policy Engine**: Basic resource access control
- **Audit Logging**: Operation tracking and compliance

### Advanced Security (Enterprise)
- **Enhanced Sandboxing**: gVisor (Tier2) and Firecracker (Tier3) isolation **(Enterprise)**
- **AI Security Review**: Automated tool analysis and approval **(Enterprise)**
- **Encrypted Communication**: Secure inter-agent messaging **(Enterprise)**
- **Comprehensive Audits**: Cryptographic integrity guarantees **(Enterprise)**

## 🧪 Testing

```bash
# Run all tests
cargo test

# Run specific components
cd crates/dsl && cargo test          # DSL parser
cd crates/runtime && cargo test     # Runtime system

# Integration tests
cd crates/runtime && cargo test --test integration_tests
cd crates/runtime && cargo test --test rag_integration_tests
cd crates/runtime && cargo test --test mcp_client_tests
```

## 📚 Documentation

- **[Getting Started](https://docs.symbiont.dev/getting-started)** - Installation and first steps
- **[DSL Guide](https://docs.symbiont.dev/dsl-guide)** - Complete language reference
- **[Runtime Architecture](https://docs.symbiont.dev/runtime-architecture)** - System design
- **[Security Model](https://docs.symbiont.dev/security-model)** - Security implementation
- **[API Reference](https://docs.symbiont.dev/api-reference)** - Complete API documentation
- **[Contributing](https://docs.symbiont.dev/contributing)** - Development guidelines

### Technical References
- [`crates/runtime/README.md`](crates/runtime/README.md) - Runtime-specific docs
- [`crates/runtime/API_REFERENCE.md`](crates/runtime/API_REFERENCE.md) - Complete API reference
- [`crates/dsl/README.md`](crates/dsl/README.md) - DSL implementation details

## 🤝 Contributing

Contributions welcome! Please see [`docs/contributing.md`](docs/contributing.md) for guidelines.

**Development Principles:**
- Security first - all features must pass security review
- Zero trust - assume all inputs are potentially malicious
- Comprehensive testing - maintain high test coverage
- Clear documentation - document all features and APIs

## 🎯 Use Cases

### Development & Automation
- Secure code generation and refactoring
- Automated testing with policy compliance
- AI agent deployment with tool verification
- Knowledge management with semantic search

### Enterprise & Regulated Industries
- Healthcare data processing with HIPAA compliance **(Enterprise)**
- Financial services with audit requirements **(Enterprise)**
- Government systems with security clearances **(Enterprise)**
- Legal document analysis with confidentiality **(Enterprise)**

## 📄 License

**Community Edition**: MIT License  
**Enterprise Edition**: Commercial license required

Contact [ThirdKey](https://thirdkey.ai) for Enterprise licensing.

## 🔗 Links

- [ThirdKey Website](https://thirdkey.ai)
- [Runtime API Reference](crates/runtime/API_REFERENCE.md)

---

*Symbi enables secure collaboration between AI agents and humans through intelligent policy enforcement, cryptographic verification, and comprehensive audit trails.*
