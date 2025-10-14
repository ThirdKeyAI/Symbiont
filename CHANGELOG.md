# Changelog

All notable changes to the Symbiont project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.5.0] - 2025-10-14

### Added

#### 🛠️ Enhanced CLI Experience
- **System Health Diagnostics**: New `symbi doctor` command for comprehensive system health checks
  - [`src/commands/doctor.rs`](src/commands/doctor.rs): Validates system dependencies, configuration, and runtime environment
  - Checks for required tools, permissions, and connectivity
  - Provides actionable recommendations for fixing issues
- **Log Management**: New `symbi logs` command for viewing and filtering application logs
  - [`src/commands/logs.rs`](src/commands/logs.rs): Real-time log streaming and filtering
  - Support for log levels, time ranges, and pattern matching
  - Integration with system logging infrastructure
- **Project Scaffolding**: New `symbi new` command for creating new agent projects
  - [`src/commands/new.rs`](src/commands/new.rs): Interactive project creation with templates
  - Pre-configured project structure with best practices
  - Multiple project templates (basic, advanced, custom)
  - Automatic dependency setup and configuration
- **Status Monitoring**: New `symbi status` command for real-time system status
  - [`src/commands/status.rs`](src/commands/status.rs): Display running agents, resource usage, and system health
  - Quick overview of active components and their states
- **Quick Start**: New `symbi up` command for rapid environment initialization
  - [`src/commands/up.rs`](src/commands/up.rs): One-command setup for development and production
  - Automatic dependency installation and service startup
  - Health checks and validation after startup

#### 📦 Installation & Distribution
- **Automated Installation Script**: New [`scripts/install.sh`](scripts/install.sh) for easy setup
  - Cross-platform installation support (Linux, macOS)
  - Automatic dependency detection and installation
  - Version management and upgrade capabilities
  - Configurable installation paths and options

#### 📋 Documentation
- **Version 1.0 Planning Documents**: Comprehensive planning for next major release
  - [`docs/v1-plan.md`](docs/v1-plan.md): Detailed roadmap and feature planning
  - [`docs/v1-plan-original.md`](docs/v1-plan-original.md): Original design documents and architecture decisions

### Improved

#### User Experience
- **CLI Interface**: Enhanced command-line interface with improved help text and error messages
  - Better command organization and discoverability
  - Consistent command structure across all operations
  - Improved error messages with actionable guidance
- **README Documentation**: Streamlined and updated README files across all languages
  - Simplified getting started guide
  - Clearer feature descriptions and use cases
  - Updated installation instructions
  - Better examples and quick start guides

#### Developer Experience
- **Project Structure**: Enhanced organization for better maintainability
  - Clearer separation of concerns in command modules
  - Improved code organization in [`src/commands/mod.rs`](src/commands/mod.rs:5)
- **Main CLI Entry Point**: Updated [`src/main.rs`](src/main.rs) with new command routing
  - Better command registration and handling
  - Enhanced error handling and logging
  - Improved startup performance

### Fixed
- **CLI Command Registration**: Properly integrated new commands into main CLI interface
- **Error Handling**: Improved error messages and recovery in CLI commands
- **Documentation Links**: Fixed broken references in README files across all language versions

### Performance Improvements
- **Startup Time**: Optimized CLI initialization and command loading
- **Log Processing**: Enhanced log streaming performance for real-time monitoring
- **Status Checks**: Faster system status queries and health checks

## [0.4.0] - 2025-08-28

### Added

#### 🧠 SLM-First Architecture (New)
- **Policy-Driven Routing Engine**: Intelligent routing between Small Language Models (SLMs) and Large Language Models (LLMs)
  - [`crates/runtime/src/routing/engine.rs`](crates/runtime/src/routing/engine.rs): Core routing engine with SLM-first preference and LLM fallback
  - [`crates/runtime/src/routing/policy.rs`](crates/runtime/src/routing/policy.rs): Configurable policy evaluation with rule-based decision logic
  - [`crates/runtime/src/routing/config.rs`](crates/runtime/src/routing/config.rs): Comprehensive routing configuration management
  - [`crates/runtime/src/routing/decision.rs`](crates/runtime/src/routing/decision.rs): Route decision types and execution paths
- **Task Classification System**: Automatic categorization of requests for optimal model selection
  - Task-aware routing with capability matching
  - Pattern recognition and keyword analysis for task classification
- **Confidence-Based Quality Control**: Adaptive learning system for model performance tracking
  - [`crates/runtime/src/routing/confidence.rs`](crates/runtime/src/routing/confidence.rs): Confidence monitoring and threshold management
  - Real-time quality assessment with configurable confidence thresholds
  - Automatic fallback on low-confidence responses

#### ⚡ Performance & Reliability
- **Thread-Safe Operations**: Full async/await support with proper concurrency handling
- **Error Recovery**: Graceful fallback mechanisms with exponential backoff retry logic
- **Runtime Configuration**: Dynamic policy updates and threshold adjustments without restart
- **Comprehensive Logging**: Detailed audit trail of routing decisions and performance metrics

### Improved

#### Routing & Model Management
- **Model Catalog Integration**: Deep integration with existing model catalog for SLM selection
- **Resource Management**: Intelligent resource allocation and constraint handling
- **Load Balancing**: Multiple strategies for distributing requests across available models
- **Scheduler Integration**: Seamless integration with the existing agent scheduler

#### Developer Experience
- **Comprehensive Testing**: Complete test coverage for all routing components with mock implementations
- **Documentation**: Extensive design documents and implementation guides
  - [`docs/slm_config_design.md`](docs/slm_config_design.md): SLM configuration architecture
  - [`docs/router_design.md`](docs/router_design.md): Router design and implementation guide
  - [`docs/unit_testing_guide.md`](docs/unit_testing_guide.md): Testing methodology and coverage
- **Configuration Validation**: Enhanced validation of routing policies and model configurations

### Fixed
- **Module Exports**: Fixed routing module structure in [`crates/runtime/src/routing/mod.rs`](crates/runtime/src/routing/mod.rs:5)
  - Added missing `pub mod config;` and `pub mod policy;` declarations
  - Added corresponding `pub use` statements for proper re-exports
- **Task Type Updates**: Replaced deprecated `TaskType::TextGeneration` with `TaskType::CodeGeneration`
  - Updated routing engine references throughout codebase
  - Fixed task type usage in test modules and policy evaluation
- **Import Resolution**: Resolved compilation errors in routing components
  - Updated ModelLogger constructor calls to match current API
  - Fixed import paths in test modules for proper dependency resolution
- **Code Quality**: Applied clippy suggestions and resolved all warnings
  - Improved code patterns and removed unused imports
  - Enhanced error handling and async operation safety

### Performance Improvements
- **Routing Throughput**: Optimized routing decision performance with efficient policy evaluation
- **Memory Efficiency**: Reduced memory overhead in confidence monitoring and statistics tracking
- **Async Operations**: Enhanced async runtime efficiency for concurrent request handling
- **Configuration Loading**: Optimized configuration parsing and validation performance

### Breaking Changes
- **Routing API**: New routing engine interface with SLM-first architecture
- **Task Classification**: Updated task type enumeration with `CodeGeneration` replacing `TextGeneration`
- **Configuration Schema**: Enhanced routing configuration structure with policy-driven settings

## [0.3.1] - 2025-08-10

### Added

#### 🔒 Security Enhancements
- **Centralized Configuration Management**: New [`config.rs`](crates/runtime/src/config.rs) module for secure configuration handling
  - Environment variable abstraction layer with validation
  - Multiple secret key providers (environment, file, external services)
  - Centralized configuration access patterns
- **Enhanced CI/CD Security**: Automated security scanning in GitHub Actions
  - Daily cargo audit vulnerability scanning
  - Clippy security lints integration
  - Secret leak detection in build pipeline

#### 📋 API Documentation
- **SwaggerUI Integration**: Interactive API documentation for HTTP endpoints
  - Auto-generated OpenAPI specifications
  - Interactive API testing interface
  - Complete endpoint documentation with examples

### Security Fixes

#### 🛡️ Critical Vulnerability Resolutions
- **RUSTSEC-2022-0093**: Fixed ed25519-dalek Double Public Key Signing Oracle Attack
  - Updated from v1.0.1 → v2.2.0
- **RUSTSEC-2024-0344**: Resolved curve25519-dalek timing variability vulnerability
  - Updated from v3.2.0 → v4.1.3 (transitive dependency)
- **RUSTSEC-2025-0009**: Fixed ring AES panic vulnerability
  - Updated from v0.16 → v0.17.12
- **Timing Attack Prevention**: Implemented constant-time token comparison
  - Replaced vulnerable string comparison in authentication middleware
  - Added `subtle` crate for constant-time operations
  - Enhanced authentication logging and error handling

### Improved

#### Configuration Management
- **Environment Variable Security**: Eliminated direct `env::var` usage throughout codebase
- **Secret Handling**: Secure configuration management with validation
- **Error Handling**: Enhanced configuration error reporting and validation

#### Authentication & Security
- **Middleware Security**: Updated authentication middleware to use configuration management
- **Request Logging**: Enhanced security logging for authentication failures
- **Token Validation**: Improved bearer token validation with timing attack prevention

### Dependencies

#### Security Updates
- **Updated**: `ed25519-dalek` from v1.0.1 to v2.2.0 (critical security fix)
- **Updated**: `reqwest` from v0.11 to v0.12 (security and performance)
- **Updated**: `ring` from v0.16 to v0.17.12 (AES panic fix)
- **Added**: `subtle` v2.5 for constant-time cryptographic operations

#### Documentation & Tooling
- **Added**: `utoipa` and `utoipa-swagger-ui` for API documentation generation
- **Enhanced**: CI/CD security workflow with automated vulnerability scanning

### Verification
- ✅ **cargo audit**: All critical vulnerabilities resolved
- ✅ **cargo clippy**: No security or performance warnings
- ✅ **Timing attack tests**: Constant-time comparison verified
- ✅ **Configuration migration**: Seamless upgrade path from v0.3.0

## [0.3.0] - 2025-08-09

### Added

#### 🚀 HTTP API Server (New)
- **Complete API Server**: Full-featured HTTP server implementation using Axum framework
  - RESTful endpoints for agent management, execution, and monitoring
  - Authentication middleware with bearer token and JWT support
  - CORS support and comprehensive security headers
  - Request tracing and structured logging
  - Graceful shutdown with active request completion
- **Agent Management API**: Create, update, delete, and monitor agents via HTTP
  - Agent status tracking with real-time metrics
  - Agent execution history and performance data
  - Agent configuration updates without restart
- **System Monitoring**: Health checks, metrics collection, and system status endpoints
  - Real-time system resource utilization
  - Agent scheduler statistics and performance metrics
  - Comprehensive health check with component status

#### 🧠 Advanced Context & Knowledge Management (New)
- **Hierarchical Memory System**: Multi-layered memory architecture for agents
  - **Working Memory**: Variables, active goals, attention focus for immediate processing
  - **Short-term Memory**: Recent experiences and temporary information
  - **Long-term Memory**: Persistent knowledge and learned experiences
  - **Episodic Memory**: Structured experience episodes with events and outcomes
  - **Semantic Memory**: Concept relationships and domain knowledge graphs
- **Knowledge Base Operations**: Comprehensive knowledge management capabilities
  - **Facts**: Subject-predicate-object knowledge with confidence scoring
  - **Procedures**: Step-by-step procedural knowledge with error handling
  - **Patterns**: Learned behavioral patterns with occurrence tracking
  - **Knowledge Sharing**: Inter-agent knowledge sharing with trust scoring
- **Context Persistence**: File-based and configurable storage backend
  - Automatic context archiving and retention policies
  - Compression and encryption support for sensitive data
  - Migration utilities for legacy storage formats
- **Vector Database Integration**: Semantic search and similarity matching
  - Qdrant integration for high-performance vector operations
  - Embedding generation and storage for context items
  - Batch operations for efficient data processing
- **Context Examples**: Comprehensive [`context_example.rs`](crates/runtime/examples/context_example.rs) demonstration

#### ⚡ Production-Grade Agent Scheduler (New)
- **Priority-Based Scheduling**: Multi-level priority queue with resource-aware scheduling
  - Configurable priority levels and scheduling algorithms
  - Resource requirements tracking and allocation
  - Load balancing with multiple strategies (round-robin, resource-based)
- **Task Management**: Complete lifecycle management for agent tasks
  - Task health monitoring and failure detection
  - Automatic retry logic with exponential backoff
  - Timeout handling and graceful termination
- **System Monitoring**: Real-time scheduler metrics and health monitoring
  - Agent performance tracking (CPU, memory, execution time)
  - System capacity monitoring and utilization alerts
  - Comprehensive scheduler statistics and dashboards
- **Graceful Shutdown**: Production-ready shutdown with active task completion
  - Resource cleanup and allocation tracking
  - Metrics persistence and system state preservation
  - Configurable shutdown timeouts and force termination

#### 📊 Enhanced Documentation & Examples
- **Production Examples**: Real-world usage patterns and best practices
  - RAG engine integration with [`rag_example.rs`](crates/runtime/examples/rag_example.rs)
  - Context persistence and management workflows
  - Agent lifecycle and resource management
- **API Reference**: Complete HTTP API documentation with examples
  - OpenAPI-compatible endpoint specifications
  - Authentication and authorization guides
  - Integration examples for common use cases

### Improved

#### Runtime Stability & Performance
- **Memory Management**: Optimized memory usage with configurable limits
- **Error Handling**: Enhanced error propagation and recovery mechanisms
- **Async Performance**: Improved async runtime efficiency and task scheduling
- **Resource Utilization**: Better CPU and memory resource management

#### Configuration & Deployment
- **Feature Flags**: Granular feature control for different deployment scenarios
  - `http-api`: HTTP server and API endpoints
  - `http-input`: Webhook input processing
  - `vector-db`: Vector database integration
  - `embedding-models`: Local embedding model support
- **Directory Structure**: Standardized data directory layout
  - Separate directories for state, logs, prompts, and vector data
  - Automatic directory creation and permission management
  - Legacy migration utilities for existing deployments

#### Developer Experience
- **Examples**: Comprehensive example implementations for all major features
- **Testing**: Enhanced test coverage with integration tests
- **Logging**: Structured logging with configurable verbosity levels
- **Debugging**: Improved debugging capabilities with detailed metrics

### Fixed
- **Scheduler Deadlocks**: Resolved potential deadlock conditions in agent scheduling
- **Memory Leaks**: Fixed memory leaks in context management and vector operations
- **Graceful Shutdown**: Improved shutdown reliability under high load
- **Configuration Validation**: Enhanced validation of configuration parameters
- **Error Recovery**: Better error recovery in network and storage operations

### Dependencies
- **Added**: Axum 0.7 for HTTP server implementation
- **Added**: Tower and Tower-HTTP for middleware and CORS support
- **Added**: Governor for rate limiting capabilities
- **Added**: Qdrant-client 1.14.0 for vector database operations
- **Updated**: Tokio async runtime optimizations
- **Updated**: Enhanced serialization with serde improvements

### Breaking Changes
- **Context API**: Updated context management API with hierarchical memory model
- **Scheduler Interface**: New scheduler trait with enhanced lifecycle management
- **Configuration Format**: Updated configuration structure for directory management

### Performance Improvements
- **Scheduler Throughput**: Up to 10x improvement in agent scheduling performance
- **Memory Efficiency**: 40% reduction in memory usage for large context operations
- **Vector Search**: Optimized vector database operations with batch processing
- **HTTP Response Time**: Sub-100ms response times for standard API operations

### Security Enhancements
- **Authentication**: Multi-factor authentication support for HTTP API
- **Encryption**: Enhanced encryption for data at rest and in transit
- **Access Control**: Improved permission management for context operations
- **Data Protection**: Secure handling of sensitive agent data and configurations

## Installation

### Docker
```bash
docker pull ghcr.io/thirdkeyai/symbi:v0.3.0
```

### Cargo (with all features)
```bash
cargo install symbi-runtime --features full
```

### Cargo (minimal installation)
```bash
cargo install symbi-runtime --features minimal
```

### From Source
```bash
git clone https://github.com/thirdkeyai/symbiont.git
cd symbiont
git checkout v0.3.0
cargo build --release --features full
```

## Quick Start - HTTP API

```rust
use symbi_runtime::api::{HttpApiServer, HttpApiConfig};

let config = HttpApiConfig {
    bind_address: "0.0.0.0".to_string(),
    port: 8080,
    enable_cors: true,
    enable_tracing: true,
};

let server = HttpApiServer::new(config);
server.start().await?;
```

## Quick Start - Context Management

```rust
use symbi_runtime::context::{StandardContextManager, ContextManagerConfig};

let config = ContextManagerConfig {
    max_contexts_in_memory: 1000,
    enable_auto_archiving: true,
    enable_vector_db: true,
    ..Default::default()
};

let context_manager = StandardContextManager::new(config, "system").await?;
let session_id = context_manager.create_session(agent_id).await?;
```

---

**Full Changes**: [v0.1.2...v0.3.0](https://github.com/thirdkeyai/symbiont/compare/v0.1.2...v0.3.0)

## [0.1.1] - 2025-07-26

### Added

#### Secrets Management System
- HashiCorp Vault backend with multiple authentication methods:
  - Token-based authentication
  - Kubernetes service account authentication
  - AWS IAM role authentication (framework ready)
  - AppRole authentication
- Encrypted file backend with AES-256-GCM encryption
- OS keychain integration for master key storage
- Audit trail for all secrets operations
- Agent-scoped secret namespaces
- CLI subcommands for encrypt/decrypt/edit operations

#### Security & Compliance
- Code of Conduct and Security Policy documentation
- Cosign container image signing
- Container security scanning with Trivy

#### Infrastructure
- Tag-based Docker builds with semantic versioning
- Multi-architecture container support (linux/amd64, linux/arm64)
- GitHub Container Registry integration

### Improved

#### Runtime Components
- MCP client error handling and stability
- RAG engine async context manager API
- HTTP API reliability (optional feature)
- Tool execution and sandboxing integration
- Vector database integration with Qdrant

#### Documentation
- Security model documentation
- API reference with examples
- Clear OSS vs Enterprise feature distinction
- Development and contribution guidelines

#### Development Experience
- Environment configuration with `.env` support
- Test coverage (17/17 secrets management tests passing)
- Error messages and debugging capabilities

### Fixed
- Import naming conflicts in test modules
- RAG engine async context manager issues
- Docker registry naming for lowercase compliance
- Documentation link references
- Cargo clippy warnings and compilation errors

### Dependencies
- Added vaultrs for Vault integration
- Updated tokio for async runtime
- Added serde for configuration serialization
- Added thiserror for error handling

### Known Issues
- Windows keychain integration pending

## Installation

### Docker
```bash
docker pull ghcr.io/thirdkeyai/symbi:v0.1.1
```

### From Source
```bash
git clone https://github.com/thirdkeyai/symbiont.git
cd symbiont
git checkout v0.1.1
cargo build --release
```

For the complete list of changes, see the [commit history](https://github.com/thirdkeyai/symbiont/compare/v0.1.0...v0.1.1).