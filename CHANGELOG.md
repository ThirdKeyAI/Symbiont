# Changelog

All notable changes to the Symbiont project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [1.4.0] - 2026-02-15

### Added

#### Persistent Memory (`MarkdownMemoryStore`)
- **Markdown-backed agent memory** implementing `ContextPersistence` trait
  - Facts, Procedures, and Learned Patterns sections in `memory.md`
  - Daily log append with timestamped entries
  - Retention-based compaction with configurable max age
- **DSL `memory` block**: Declarative memory configuration in agent definitions
  - `store`: Storage format (`markdown`)
  - `path`: File path for memory persistence
  - `retention`: Duration-based retention (`90d`, `6months`, etc.) via `humantime`
  - `compact_after`: Compaction threshold
- **REPL `:memory` command**: Inspect, compact, and purge agent memory at runtime

#### Webhook DX (`SignatureVerifier`)
- **`SignatureVerifier` trait** with two implementations:
  - `HmacVerifier`: HMAC-SHA256 with constant-time comparison via `subtle` crate
  - `JwtVerifier`: HS256 JWT token verification
- **`WebhookProvider` presets**: GitHub, Stripe, Slack, Custom — each maps provider name to correct header and signing scheme
- **DSL `webhook` block**: Declarative webhook endpoint configuration
  - `provider`: Provider preset name
  - `secret`: Secret key or environment variable reference (`$ENV_VAR`)
  - `path`: HTTP endpoint path
  - `filter`: Event type filtering
- **Wired into `HttpInputServer`**: Pre-handler signature verification on raw `Bytes` before JSON parsing. Returns 401 on failure, 400 on bad JSON.
- **REPL `:webhook` command**: List configured webhook endpoints

#### DSL Parser Fixes
- **Bare identifier in `value` rule**: `store markdown`, `provider github` now parse correctly
- **Short-form duration literals**: `90d`, `6m`, `1y` alongside existing `90.seconds` form
- **Conflict resolution**: `conflicts` declaration for `expression`/`value` ambiguity

### Crate Versions
| Crate | Version |
|-------|---------|
| `symbi` | 1.4.0 |
| `symbi-dsl` | 1.4.0 |
| `symbi-runtime` | 1.4.0 |
| `repl-core` | 1.4.0 |

## [1.1.0] - 2026-02-12

### Added

#### Security Hardening v2 (symbi-runtime)
- **Per-agent API key authentication** with Argon2 hashing and file-backed key store
- **Per-IP rate limiting middleware** wired into HTTP router (governor, 100 req/min)
- **Schema-driven argument redaction** via `sensitive_params` on MCP tools
- **File locking for secret store** reads (fd-lock shared read locks)
- **Safe sandbox defaults**: empty `allowed_executables`, shell warnings

#### DSL Improvements (symbi-dsl)
- **Structured `DslDiagnostic` type** replacing println-based error reporting
- **Humantime-based timeout parsing** with backward-compatible `.seconds` suffix

#### symbi-a2ui (experimental/alpha)
- New Lit-based admin UI for fleet management, compliance dashboards, and audit trail viewing
- Not published to npm — private, experimental

### Fixed
- **Teams Auth** (symbi-channel-adapter): Migrated to jsonwebtoken v10 API with proper claim validation

### Crate Versions
| Crate | Version |
|-------|---------|
| `symbi` | 1.1.0 |
| `symbi-dsl` | 1.1.0 |
| `symbi-runtime` | 1.1.0 |
| `symbi-channel-adapter` | 0.1.1 |
| `repl-core` | 1.0.1 |

## [1.0.1] - 2026-02-11

### Added

#### AgentPin Integration
- **DiscoveryMode Resolver Dispatch**: Multi-strategy agent identity resolution
  - `StaticDocument`: Use a pre-loaded discovery document (offline/testing)
  - `WellKnown`: Fetch `.well-known/agent-identity.json` over HTTPS (default)
  - `DnsRecord`: DNS TXT record lookup (future)
  - Automatic fallback chain: Static → WellKnown → DnsRecord
- **agentpin 0.2.0**: Switched from local path dependency to crates.io release
  - Trust bundle support for fully offline verification
  - Enhanced discovery document validation

#### MCP Server
- **Real MCP Server**: Replaced stub with full MCP server over stdio using `rmcp` SDK
  - `symbi mcp` command now serves a proper MCP protocol endpoint
  - Tool registration, invocation, and result marshalling

#### Channel Adapters
- **Bidirectional Slack Adapter**: Real-time Slack integration (Phase 1)
  - Socket Mode for event streaming
  - Message sending and channel management
- **Teams & Mattermost Adapters**: Additional chat platform support
  - Microsoft Teams webhook and Bot Framework integration
  - Mattermost WebSocket and REST API integration
- **Channel Management REST API**: CRUD endpoints for channel configurations
- **Declarative `channel {}` Block**: DSL grammar support for channel definitions
- **Enterprise Channel Governance**: Policy enforcement for channel operations

#### Infrastructure
- **`.claude/` Release Documentation**: Added CLAUDE.md development guidelines and RELEASE_RUNBOOK.md
- **ROADMAP.md**: v1.1.0+ release planning document

### Fixed
- **Docker Build Cache**: Fixed cleanup glob to include `libsymbi*` and `.fingerprint/symbi*` cached artifacts
- **Clippy**: Use `derive(Default)` for DiscoveryMode enum instead of manual impl
- **CI Tests**: Fixed all failing CI tests and runtime bugs
- **Compilation Warnings**: Resolved warnings, made Qdrant optional
- **Runtime Init**: Fixed auth header and model defaults for HTTP-only mode

### Changed
- **Docker Base Image**: Bumped Rust image to 1.88 for dependency compatibility
- **OSS Sync**: Hardened sync script with dry-run mode, interactive prompts, expanded safety checks

### Crate Versions
| Crate | Version |
|-------|---------|
| `symbi` | 1.0.1 |
| `symbi-dsl` | 1.0.1 |
| `symbi-runtime` | 1.0.2 |
| `repl-core` | 1.0.1 |

## [1.0.0] - 2026-02-07

### Added

#### 🎯 Production-Ready Scheduling (v1.0.0)
- **Session Isolation**: Per-run AgentContext with HeartbeatContextMode control
  - `EphemeralWithSummary`: Fresh context per iteration with summary carryover (default)
  - `SharedPersistent`: Persistent context across all iterations
  - `FullyEphemeral`: Stateless execution with no context carryover
  - Prevents unbounded memory growth in long-running heartbeat agents
- **Jitter Support**: Random 0-N second delay to prevent thundering herd
  - Configurable `max_jitter_seconds` in CronSchedulerConfig
  - Spreads job starts across time window when multiple jobs share a schedule
- **Per-Job Concurrency Guards**: Limit concurrent runs per job
  - `max_concurrent` field on CronJobDefinition
  - Prevents resource exhaustion from overlapping executions
  - Scheduler skips tick when job at max concurrency
- **Dead-Letter Queue**: Jobs exceeding max_retries move to DeadLetter status
  - Manual review and recovery workflow via CLI
  - Audit trail of failure patterns
  - `symbi cron reset <job-id>` to reactivate after fixing
- **CronMetrics Observability**: Comprehensive metrics collection
  - `runs_total`, `runs_succeeded`, `runs_failed` counters
  - `execution_duration_seconds` histogram
  - `in_flight_jobs` gauge
  - `dead_letter_total` counter
  - Prometheus-compatible export
- **CronSchedulerHealth Endpoint**: `/api/v1/health/scheduler` for monitoring
  - Active/paused/in-flight job counts
  - Aggregated metrics and performance data
  - Integration with ops monitoring systems
- **AgentPin JWT Field**: Cryptographic identity verification on CronJobDefinition
  - ES256 (ECDSA P-256) signature verification
  - Domain-anchored agent identity
  - `require_agent_pin` policy enforcement
  - Prevents unauthorized agent execution

#### 🔒 Security Enhancements
- **New SecurityEventType Variants**:
  - `CronJobDeadLettered`: Job moved to dead-letter queue after max retries
  - `AgentPinVerificationFailed`: AgentPin JWT validation failure
- **Policy Enforcement**: Enhanced security checks before scheduled execution
  - Time window restrictions
  - Capability requirements
  - Human approval workflows
  - AgentPin cryptographic verification

#### 📋 Documentation
- **Comprehensive Scheduling Guide**: [`docs/scheduling.md`](docs/scheduling.md)
  - Complete architecture overview
  - DSL syntax reference (cron and at/one-shot)
  - CLI command reference
  - Heartbeat pattern guide
  - Session isolation strategies
  - Delivery routing configuration
  - Policy enforcement examples
  - Production hardening best practices
  - HTTP API endpoint reference
  - SDK examples (JS + Python)
  - Configuration reference

### Improved

#### Reliability & Stability
- **Graceful Shutdown**: Enhanced in-flight job tracking during scheduler shutdown
- **Error Recovery**: Better retry logic with exponential backoff
- **State Management**: More robust job state transitions with ACID guarantees
- **Audit Trail**: Complete lifecycle tracking for all scheduled jobs

#### Performance
- **Optimized Tick Loop**: Reduced scheduler overhead with efficient job selection
- **Concurrent Execution**: Improved throughput with configurable global concurrency
- **Resource Management**: Better CPU and memory utilization tracking

### Fixed
- **Scheduler Stability**: Resolved edge cases in job state management
- **Heartbeat Memory Leaks**: Fixed unbounded context growth in long-running agents
- **Concurrency Deadlocks**: Eliminated potential deadlocks under high load
- **Metric Collection**: Fixed race conditions in metrics aggregation

### Breaking Changes
- **CronJobDefinition Schema**: Added new fields (`max_concurrent`, `agent_pin_jwt`)
  - Migration: Existing jobs work without changes (optional fields default to safe values)
- **HeartbeatContextMode**: New enum for session isolation control
  - Migration: Defaults to `EphemeralWithSummary` (previous behavior)

### Migration from v0.9.0
No breaking API changes. All v0.9.0 scheduled jobs continue to work.

Optional enhancements:
1. Add `max_concurrent` limits to high-frequency jobs
2. Enable `max_jitter_seconds` to spread job starts
3. Configure `agent_pin_jwt` for identity verification
4. Set `default_max_retries` to enable dead-letter queue

## [0.9.0] - 2026-01-15

### Added

#### 🚀 Delivery Routing & Policy Enforcement
- **DeliveryRouter Trait**: Pluggable output routing system
  - [`crates/runtime/src/scheduler/delivery.rs`](crates/runtime/src/scheduler/delivery.rs): Core delivery abstractions
  - DefaultDeliveryRouter implementation with multiple channels
- **Delivery Channels**: Six built-in output destinations
  - **Webhook**: HTTP POST with configurable headers and authentication
  - **Slack**: Slack webhook and API integration
  - **Email**: SMTP email delivery with templates
  - **Custom**: User-defined delivery handlers
  - **Stdout**: Console output for development
  - **LogFile**: Job-specific log file persistence
- **PolicyGate**: Schedule policy enforcement before execution
  - [`crates/runtime/src/scheduler/policy.rs`](crates/runtime/src/scheduler/policy.rs): Policy evaluation engine
  - Integration with RealPolicyParser (replacing stub in repl-core)
  - Support for time windows, capabilities, approvals, and AgentPin requirements
- **Real Policy Parser**: Production-grade policy evaluation
  - Replaced stub implementation with full policy DSL support
  - Condition evaluation with complex boolean logic
  - Integration with security audit trail

#### 🌐 HTTP API Schedule Endpoints
- **Complete Schedule Management API**: 10 RESTful endpoints with OpenAPI annotations
  - `POST /api/v1/schedule`: Create new scheduled job
  - `GET /api/v1/schedule`: List all jobs (filterable by status, agent ID)
  - `GET /api/v1/schedule/{job_id}`: Get job details
  - `PUT /api/v1/schedule/{job_id}`: Update job configuration
  - `DELETE /api/v1/schedule/{job_id}`: Delete job
  - `POST /api/v1/schedule/{job_id}/pause`: Pause job
  - `POST /api/v1/schedule/{job_id}/resume`: Resume paused job
  - `POST /api/v1/schedule/{job_id}/run`: Trigger immediate execution
  - `GET /api/v1/schedule/{job_id}/history`: Get run history
  - `GET /api/v1/schedule/{job_id}/next_run`: Get next scheduled run time
- **OpenAPI Integration**: Full Swagger/OpenAPI 3.0 documentation
  - Interactive API explorer with Swagger UI
  - Request/response schema definitions
  - Authentication examples

#### 📦 SDK Integration
- **JavaScript SDK ScheduleClient**: Complete TypeScript SDK for schedule management
  - [`symbiont-sdk-js/src/schedule.ts`](../symbiont-sdk-js/src/schedule.ts): Schedule client implementation
  - Full CRUD operations for jobs
  - Run history and status queries
  - Webhook and Slack delivery configuration
- **Python SDK ScheduleClient**: Full Python SDK with async support
  - [`symbiont-sdk-python/src/symbiont/schedule.py`](../symbiont-sdk-python/src/symbiont/schedule.py): Schedule client implementation
  - Type hints and dataclass models
  - Idiomatic Python API design

### Improved

#### Developer Experience
- **SDK Examples**: Comprehensive examples in both JavaScript and Python
  - Job creation and lifecycle management
  - Delivery routing configuration
  - Policy enforcement patterns
- **API Documentation**: Enhanced endpoint documentation with usage examples
- **Error Handling**: Better error messages for delivery failures and policy violations

#### Operational Excellence
- **Delivery Retry Logic**: Configurable retry with exponential backoff
- **Webhook Timeout**: Configurable timeout for webhook delivery
- **Channel Fallback**: Graceful degradation when delivery channels fail

### Fixed
- **Policy Parser Integration**: Fixed integration issues between scheduler and policy engine
- **Delivery Error Handling**: Improved error propagation for failed deliveries
- **SDK Type Safety**: Enhanced type definitions in both JS and Python SDKs

## [0.8.0] - 2025-12-10

### Added

#### 💓 Heartbeat Pattern & DSL
- **Heartbeat Agent Pattern**: Continuous monitoring with assessment-action-sleep cycles
  - [`crates/runtime/src/scheduler/heartbeat.rs`](crates/runtime/src/scheduler/heartbeat.rs): Heartbeat execution engine
  - HeartbeatConfig for iteration limits and context management
  - HeartbeatContextMode enum for session isolation strategies
  - HeartbeatAssessment tracking for agent decisions
- **DSL Grammar**: Schedule definition blocks in Symbiont DSL
  - [`crates/dsl/src/grammar/schedule.pest`](crates/dsl/src/grammar/schedule.pest): Pest grammar for schedule blocks
  - Cron expression syntax with validation
  - At/one-shot timestamp syntax (ISO 8601)
  - Nested policy and heartbeat configuration blocks
- **DSL Schedule Extraction**: Parse and validate schedule definitions from DSL files
  - [`crates/dsl/src/schedule.rs`](crates/dsl/src/schedule.rs): Schedule AST and validation
  - Integration with existing DSL parser infrastructure
  - Semantic validation (cron syntax, timestamp format, policy rules)

#### ⌨️ CLI Subcommands
- **`symbi cron` Command Group**: Complete CLI for schedule management
  - [`src/commands/cron/mod.rs`](src/commands/cron/mod.rs): Command router and shared utilities
  - **`symbi cron list`**: List jobs with filtering (status, agent ID)
  - **`symbi cron add`**: Create job from DSL file or JSON
  - **`symbi cron remove`**: Delete job by ID or name
  - **`symbi cron pause`**: Pause job scheduling
  - **`symbi cron resume`**: Resume paused job
  - **`symbi cron status`**: Job details with next run time
  - **`symbi cron run`**: Trigger immediate execution
  - **`symbi cron history`**: View run history with filtering
- **Interactive CLI**: Rich terminal output with colors and formatting
  - Table views for job lists and history
  - Human-readable timestamps and durations
  - JSON output mode for scripting

### Improved

#### DSL Integration
- **Unified Configuration**: Schedule definitions colocated with agent definitions
- **Validation**: Comprehensive validation at parse time vs runtime
- **Error Messages**: Clear error reporting for invalid schedules

#### Developer Experience
- **CLI Discoverability**: Intuitive command structure with helpful error messages
- **Documentation**: Inline help for all CLI commands
- **Testing**: Integration tests for CLI workflows

### Fixed
- **DSL Parser**: Fixed parsing of nested schedule blocks
- **Cron Validation**: Improved cron expression validation with better error messages
- **CLI Error Handling**: Better error propagation from runtime to CLI

## [0.7.0] - 2025-11-20

### Added

#### ⏰ Cron Foundation
- **CronScheduler**: Background tick loop for scheduled execution
  - [`crates/runtime/src/scheduler/cron.rs`](crates/runtime/src/scheduler/cron.rs): Core scheduler implementation
  - 1-second tick interval with job selection by next run time
  - Concurrent execution with configurable limits
  - Graceful shutdown with in-flight job tracking
- **SQLite Persistent Job Store**: Durable job storage with ACID guarantees
  - [`crates/runtime/src/scheduler/store.rs`](crates/runtime/src/scheduler/store.rs): SqliteJobStore implementation
  - Transaction support for atomic state updates
  - Query capabilities (filter by status, agent ID, name)
  - Job run history with audit trail
- **CronJobDefinition**: Complete job lifecycle management
  - Cron expression parsing and validation
  - Job states: Active, Paused, Completed, Failed
  - One-shot job support with `at` timestamp field
  - Delivery configuration (channels, webhooks, Slack, email)
- **CronScheduled ExecutionMode**: New variant in AgentExecutionMode enum
  - Integration with existing scheduler infrastructure
  - Session management for scheduled agents
  - Context lifecycle hooks for pre/post execution
- **One-Shot Job Support**: Jobs that run once at a specific time
  - ISO 8601 timestamp parsing
  - Automatic job completion after successful execution
  - Failure handling with retry logic
- **Audit-Aware Run Records**: JobRunRecord with security event integration
  - Execution metadata (start time, duration, status)
  - Output capture and storage
  - Error message tracking
  - Integration with SecurityEventType for audit trail

#### 📊 Monitoring & Observability
- **Job Status Tracking**: Real-time job state monitoring
  - Next run time calculation
  - Last run status and duration
  - Failure count and retry tracking
- **Run History**: Persistent execution history per job
  - Queryable by status, time range
  - Success/failure statistics
  - Performance metrics (execution time)

### Improved

#### Scheduler Architecture
- **Separation of Concerns**: Clean separation between scheduler, store, and execution engine
- **Testability**: Mockable components for unit testing
- **Configuration**: Flexible CronSchedulerConfig for operational tuning

#### Runtime Integration
- **AgentContext Integration**: Seamless integration with existing context management
- **Policy Enforcement**: Placeholder for policy gates (implemented in v0.9.0)
- **Delivery Routing**: Framework for output delivery (implemented in v0.9.0)

### Fixed
- **Cron Expression Parsing**: Robust parsing with validation
- **Timezone Handling**: UTC-based scheduling with clear timezone semantics
- **Concurrency Safety**: Thread-safe job state management

### Dependencies
- **Added**: `cron` crate for expression parsing and scheduling
- **Added**: `rusqlite` for persistent job storage

## [0.6.1] - 2025-11-16

### Fixed
- **Compilation Issues**: Resolved crates.io publishing compilation errors
  - Fixed SecureMessage API usage with correct field names and types
  - Added missing SystemTime import for timestamp handling
  - Fixed ModelLogger API compatibility
  - Added missing dependencies (tokio, serde) to REPL crates
  - Fixed HttpInputConfig struct with required fields
  - Resolved match arm type compatibility issues in JSON-RPC server

### Dependencies
- **Version Specifications**: Added proper version specifications to all workspace dependencies for crates.io publishing

## [0.6.0] - 2025-11-15

### Added

#### 🧠 Complete REPL System (New)
- **Interactive Development Environment**: Full REPL (Read-Eval-Print Loop) system for Symbiont DSL
  - [`crates/repl-core`](crates/repl-core): Core REPL engine with DSL evaluation, session management, and policy enforcement
  - [`crates/repl-cli`](crates/repl-cli): Interactive CLI interface and JSON-RPC server for programmatic access
  - [`crates/repl-proto`](crates/repl-proto): JSON-RPC protocol definitions for client-server communication
  - [`crates/repl-lsp`](crates/repl-lsp): Language Server Protocol implementation for IDE integration
- **Agent Lifecycle Management**: Create, start, stop, pause, resume, and destroy agents through REPL
- **Real-time Monitoring**: Execution monitoring with statistics, traces, and performance metrics
- **Session Management**: Snapshot and restore REPL sessions with persistent state
- **Policy Integration**: Built-in policy checking and capability gating for security

#### 🏢 Enterprise Features (New)
- **Suspended Agent Tracking**: Enterprise scheduler with advanced agent state management
  - [`enterprise/src/scheduler.rs`](enterprise/src/scheduler.rs): Enhanced scheduler with suspension tracking
  - Configurable suspension criteria and automatic resume capabilities
  - Integration with base runtime scheduler maintaining full compatibility
- **Retention Policy Scheduler**: Automated data lifecycle management
  - [`enterprise/docs/RETENTION_POLICY_SCHEDULER.md`](enterprise/docs/RETENTION_POLICY_SCHEDULER.md): Comprehensive retention policy system
  - Automatic cleanup of expired context items and memories
  - Configurable retention policies with compliance support
  - Background task execution with monitoring and metrics

#### 🛡️ AI-Driven Tool Review System (New)
- **Automated Security Analysis**: Complete workflow for MCP tool review and signing
  - [`enterprise/src/tool_review/`](enterprise/src/tool_review/): Tool review orchestrator and components
  - AI-powered security analysis with RAG (Retrieval-Augmented Generation)
  - Human oversight integration with streamlined review interface
  - Digital signing and verification of approved tools
- **Security Assessment**: Risk-based analysis with configurable severity levels
  - Vulnerability detection and impact assessment
  - Automated recommendations with confidence scoring
  - Audit trail and compliance reporting

#### ☁️ E2B Sandbox Integration (New)
- **Cloud Sandbox Support**: E2B.dev integration for secure code execution
  - [`crates/runtime/src/sandbox/e2b.rs`](crates/runtime/src/sandbox/e2b.rs): E2B sandbox implementation
  - Multi-tier sandbox architecture (Docker, gVisor, Firecracker, E2B)
  - Automatic tier selection based on risk assessment
  - Remote execution capabilities with enhanced isolation

#### 📊 Enhanced Scheduler Features
- **Real Task Execution**: Production-grade task processing capabilities
  - Process spawning with secure execution environments
  - Resource monitoring (CPU, memory) with 5-second intervals
  - Health checks and automatic failure detection
  - Support for ephemeral, persistent, scheduled, and event-driven execution modes
- **Graceful Shutdown**: Enhanced termination handling
  - 30-second graceful termination period with force termination fallback
  - Resource cleanup and metrics persistence
  - Queue cleanup and state synchronization

#### 📋 Documentation & Architecture
- **Data Directory Design**: Comprehensive directory structure specification
  - [`data_directory_structure_design.md`](data_directory_structure_design.md): Enhanced data persistence architecture
  - Unified management of agent contexts, logs, prompts, and vector database storage
  - Migration utilities and backward compatibility support
- **Tool Review Documentation**: Complete workflow documentation
  - [`docs/tool_review_workflow.md`](docs/tool_review_workflow.md): AI-driven tool review process
  - Security analysis procedures and human oversight protocols
- **REPL Guide**: Comprehensive user and developer documentation
  - [`docs/repl-guide.md`](docs/repl-guide.md): Complete REPL usage guide
  - Interactive examples and integration patterns

#### 🔧 Release Management
- **Version Bump**: Updated to 0.6.0 across all workspace crates
- **Documentation Updates**: Updated version references in documentation and examples

### Improved

#### Developer Experience
- **Unified Workspace**: Enhanced project organization with REPL crates
  - Consistent versioning across all workspace members
  - Improved dependency management between crates
- **IDE Integration**: Language Server Protocol support for enhanced development
  - Syntax highlighting and completion for Symbiont DSL
  - Real-time error checking and validation
  - Integrated debugging capabilities

#### Enterprise Scheduler
- **Advanced State Management**: Enhanced agent lifecycle tracking
  - Suspension and resume capabilities with configurable criteria
  - Resource optimization during agent suspension periods
  - Seamless integration with existing scheduler infrastructure
- **Compliance & Monitoring**: Enterprise-grade operational capabilities
  - Comprehensive audit trails and compliance reporting
  - Advanced metrics collection and performance monitoring
  - Retention policy enforcement with automated cleanup

#### Security & Compliance
- **Enhanced Tool Security**: AI-driven security analysis and verification
  - Automated vulnerability detection with high confidence scoring
  - Human-in-the-loop verification for critical security decisions
  - Digital signing and integrity verification for tool distribution
- **Multi-tier Sandboxing**: Advanced isolation capabilities
  - Automatic risk assessment and tier selection
  - Enhanced security boundaries with cloud sandbox options
  - Improved resource management and monitoring

### Fixed
- **Scheduler Integration**: Resolved enterprise scheduler compatibility issues
- **REPL Session Management**: Fixed session persistence and restoration
- **Tool Review Workflow**: Enhanced error handling and timeout management
- **E2B Integration**: Improved authentication and endpoint configuration
- **Version References**: Updated all version references from 0.5.0 to 0.6.0 in documentation

### Breaking Changes
- **Workspace Structure**: New REPL crates require updated import statements
- **Enterprise Scheduler**: Enhanced scheduler interface with additional methods
- **Sandbox Architecture**: Updated sandbox tier enumeration with E2B support

### Dependencies
- **Added**: REPL system dependencies for interactive development
- **Updated**: Enterprise features with enhanced scheduling capabilities
- **Enhanced**: Tool review system with AI-powered analysis

### Performance Improvements
- **REPL Performance**: Optimized DSL evaluation and session management
- **Scheduler Throughput**: Enhanced task processing with real execution support
- **Tool Review Efficiency**: Streamlined security analysis workflow

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