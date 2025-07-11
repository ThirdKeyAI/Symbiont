# Symbiont Agent Runtime System

A high-performance, secure runtime system for managing autonomous agents in the Symbiont platform. Built in Rust with comprehensive security, resource management, and error handling capabilities.

## Overview

The Symbiont Agent Runtime System provides a complete infrastructure for executing and managing autonomous agents with:

- **Multi-tier Security**: Tier1 (Docker), Tier2 (gVisor), Tier3 (Firecracker) sandboxing
- **Resource Management**: Memory, CPU, disk I/O, and network bandwidth allocation and monitoring
- **Priority-based Scheduling**: Efficient task scheduling with load balancing
- **Encrypted Communication**: Secure inter-agent messaging with Ed25519 signatures and AES-256-GCM encryption
- **Error Recovery**: Circuit breakers, retry strategies, and automatic recovery mechanisms
- **Audit Trail**: Cryptographic audit logging for compliance and debugging

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                    Agent Runtime System                     │
├─────────────────┬─────────────────┬─────────────────────────┤
│   Scheduler     │   Lifecycle     │    Resource Manager     │
│                 │   Controller    │                         │
├─────────────────┼─────────────────┼─────────────────────────┤
│ Communication   │ Error Handler   │   External Integrations │
│     Bus         │                 │                         │
└─────────────────┴─────────────────┴─────────────────────────┘
```

## Quick Start

### Installation

Add to your `Cargo.toml`:

```toml
[dependencies]
symbiont-runtime = "0.1.0"
tokio = { version = "1.0", features = ["full"] }
```

### Basic Usage

```rust
use symbiont_runtime::*;
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
use symbiont_runtime::lifecycle::*;

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

Tracks and enforces resource limits:

```rust
use symbiont_runtime::resource::*;

let config = ResourceManagerConfig {
    total_memory: 8192 * 1024 * 1024, // 8GB
    total_cpu_cores: 8,
    enforcement_enabled: true,
    monitoring_interval: Duration::from_secs(30),
    // ... other config
};

let resource_manager = DefaultResourceManager::new(config).await?;

// Allocate resources
let allocation = resource_manager.allocate_resources(agent_id, limits).await?;
```

### 3. Task Scheduling

Priority-based scheduling with load balancing:

```rust
use symbiont_runtime::scheduler::*;

let config = SchedulerConfig {
    max_concurrent_tasks: 100,
    task_timeout: Duration::from_secs(300),
    load_balancing_strategy: LoadBalancingStrategy::ResourceBased,
    // ... other config
};

let scheduler = DefaultScheduler::new(config).await?;

// Schedule a task
let task = ScheduledTask {
    id: TaskId::new(),
    agent_id,
    priority: Priority::High,
    scheduled_time: SystemTime::now(),
    timeout: Duration::from_secs(60),
    retry_count: 0,
};

scheduler.schedule_task(task).await?;
```

### 4. Communication Bus

Secure inter-agent messaging:

```rust
use symbiont_runtime::communication::*;

let config = CommunicationConfig {
    enable_encryption: true,
    max_message_size: 1024 * 1024, // 1MB
    delivery_timeout: Duration::from_secs(30),
    // ... other config
};

let comm_bus = DefaultCommunicationBus::new(config).await?;

// Register agent
comm_bus.register_agent(agent_id, capabilities).await?;

// Send message
let message = Message {
    id: MessageId::new(),
    from: sender_id,
    to: recipient_id,
    topic: "example_topic".to_string(),
    payload: b"Hello, agent!".to_vec(),
    priority: Priority::Normal,
    ttl: Duration::from_secs(300),
};

comm_bus.send_message(message).await?;
```

### 5. Error Handling

Comprehensive error recovery:

```rust
use symbiont_runtime::error_handler::*;

let config = ErrorHandlerConfig {
    enable_auto_recovery: true,
    max_recovery_attempts: 3,
    circuit_breaker_threshold: 10,
    // ... other config
};

let error_handler = DefaultErrorHandler::new(config).await?;

// Handle an error
let action = error_handler.handle_error(agent_id, error).await?;

match action {
    ErrorAction::Retry { max_attempts, backoff } => {
        // Implement retry logic
    }
    ErrorAction::Restart => {
        // Restart the agent
    }
    ErrorAction::Terminate => {
        // Terminate the agent
    }
    // ... other actions
}
```

## Security Features

### Multi-tier Sandboxing

- **Tier1**: Docker containers with resource limits
- **Tier2**: gVisor for enhanced isolation
- **Tier3**: Firecracker microVMs for maximum security

### Encryption

- **Message Encryption**: AES-256-GCM for message payloads
- **Digital Signatures**: Ed25519 for message authentication
- **Key Management**: Secure key generation and rotation

### Audit Trail

All operations are logged with cryptographic integrity:

```rust
use symbiont_runtime::integrations::audit_trail::*;

let audit_trail = MockAuditTrail::new().await?;

// Record an event
let event = AuditEvent {
    id: AuditId::new(),
    timestamp: SystemTime::now(),
    event_type: AuditEventType::AgentCreated,
    agent_id: Some(agent_id),
    details: "Agent created successfully".to_string(),
    metadata: HashMap::new(),
};

audit_trail.record_event(event).await?;
```

## Configuration

### Environment Variables

- `SYMBIONT_LOG_LEVEL`: Set logging level (debug, info, warn, error)
- `SYMBIONT_MAX_AGENTS`: Maximum number of concurrent agents
- `SYMBIONT_RESOURCE_ENFORCEMENT`: Enable/disable resource enforcement

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

[scheduler]
max_concurrent_tasks = 100
load_balancing_strategy = "ResourceBased"

[communication]
enable_encryption = true
max_message_size = 1048576  # 1MB

[error_handler]
enable_auto_recovery = true
max_recovery_attempts = 3
```

## Testing

Run all tests:

```bash
cd runtime
cargo test
```

Run specific test suites:

```bash
# Unit tests only
cargo test --lib

# Integration tests only
cargo test --test integration_tests

# Specific module tests
cargo test scheduler::tests
```

## Performance

### Benchmarks

- **Agent Creation**: ~1ms per agent
- **Message Throughput**: 10,000+ messages/second
- **Resource Allocation**: ~100μs per allocation
- **State Transitions**: ~50μs per transition

### Memory Usage

- **Base Runtime**: ~10MB
- **Per Agent**: ~1-5MB (depending on configuration)
- **Message Buffers**: Configurable (default 1MB per agent)

## Error Codes

| Code | Description |
|------|-------------|
| `AGENT_001` | Agent not found |
| `RESOURCE_001` | Insufficient resources |
| `COMM_001` | Message delivery failed |
| `SCHED_001` | Task scheduling failed |
| `SEC_001` | Security violation |

## Contributing

1. Fork the repository
2. Create a feature branch
3. Add tests for new functionality
4. Ensure all tests pass: `cargo test`
5. Submit a pull request

## License

Licensed under the MIT License. See `LICENSE` file for details.

## Support

For issues and questions:
- GitHub Issues: [symbiont-runtime/issues](https://github.com/symbiont/runtime/issues)
- Documentation: [docs.symbiont.dev](https://docs.symbiont.dev)
- Community: [discord.gg/symbiont](https://discord.gg/symbiont)