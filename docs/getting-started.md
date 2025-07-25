---
layout: default
title: Getting Started
nav_order: 2
description: "Quick start guide for Symbiont"
---

# Getting Started
{: .no_toc }

This guide will walk you through setting up Symbi and creating your first AI agent.
{: .fs-6 .fw-300 }

## Table of contents
{: .no_toc .text-delta }

1. TOC
{:toc}

---

## Prerequisites

Before getting started with Symbi, ensure you have the following installed:

### Required Dependencies

- **Docker** (for containerized development)
- **Rust 1.88+** (if building locally)
- **Git** (for cloning the repository)

### Optional Dependencies

- **Qdrant** vector database (for semantic search capabilities)
- **SchemaPin Go CLI** (for tool verification)

---

## Installation

### Option 1: Docker (Recommended)

The fastest way to get started is using Docker:

```bash
# Clone the repository
git clone https://github.com/thirdkeyai/symbiont.git
cd symbiont

# Build the unified symbi container
docker build -t symbi:latest .

# Or use pre-built container
docker pull ghcr.io/thirdkeyai/symbi:latest

# Run the development environment
docker run --rm -it -v $(pwd):/workspace symbi:latest bash
```

### Option 2: Local Installation

For local development:

```bash
# Clone the repository
git clone https://github.com/thirdkeyai/symbiont.git
cd symbiont

# Install Rust dependencies and build
cargo build --release

# Run tests to verify installation
cargo test
```

### Verify Installation

Test that everything is working correctly:

```bash
# Test the DSL parser
cd crates/dsl && cargo run && cargo test

# Test the runtime system
cd ../runtime && cargo test

# Run example agents
cargo run --example basic_agent
cargo run --example full_system

# Test the unified symbi CLI
cd ../.. && cargo run -- dsl --help
cargo run -- mcp --help

# Test with Docker container
docker run --rm symbi:latest --version
docker run --rm -v $(pwd):/workspace symbi:latest dsl parse --help
docker run --rm symbi:latest mcp --help
```

---

## Your First Agent

Let's create a simple data analysis agent to understand the basics of Symbi.

### 1. Create Agent Definition

Create a new file `my_agent.symbi`:

```rust
metadata {
    version = "1.0.0"
    author = "your-name"
    description = "My first Symbi agent"
}

agent greet_user(name: String) -> String {
    capabilities = ["greeting", "text_processing"]
    
    policy safe_greeting {
        allow: read(name) if name.length <= 100
        deny: store(name) if name.contains_sensitive_data
        audit: all_operations with signature
    }
    
    with memory = "ephemeral", privacy = "low" {
        if (validate_name(name)) {
            greeting = format_greeting(name);
            audit_log("greeting_generated", greeting.metadata);
            return greeting;
        } else {
            return "Hello, anonymous user!";
        }
    }
}
```

### 2. Run the Agent

```bash
# Parse and validate the agent definition
cargo run -- dsl parse my_agent.symbi

# Run the agent in the runtime
cd crates/runtime && cargo run --example basic_agent -- --agent ../../my_agent.symbi
```

---

## Understanding the DSL

The Symbi DSL has several key components:

### Metadata Block

```rust
metadata {
    version = "1.0.0"
    author = "developer"
    description = "Agent description"
}
```

Provides essential information about your agent for documentation and runtime management.

### Agent Definition

```rust
agent agent_name(parameter: Type) -> ReturnType {
    capabilities = ["capability1", "capability2"]
    // agent implementation
}
```

Defines the agent's interface, capabilities, and behavior.

### Policy Definitions

```rust
policy policy_name {
    allow: action_list if condition
    deny: action_list if condition
    audit: operation_type with audit_method
}
```

Declarative security policies that are enforced at runtime.

### Execution Context

```rust
with memory = "persistent", privacy = "high" {
    // agent implementation
}
```

Specifies runtime configuration for memory management and privacy requirements.

---

## Next Steps

### Explore Examples

The repository includes several example agents:

```bash
# Basic agent example
cd crates/runtime && cargo run --example basic_agent

# Full system demonstration
cd crates/runtime && cargo run --example full_system

# Context and memory example
cd crates/runtime && cargo run --example context_example

# RAG-powered agent
cd crates/runtime && cargo run --example rag_example
```

### Enable Advanced Features

#### HTTP API (Optional)

```bash
# Enable the HTTP API feature
cd crates/runtime && cargo build --features http-api

# Run with API endpoints
cd crates/runtime && cargo run --features http-api --example full_system
```

**Key API Endpoints:**
- `GET /api/v1/health` - Health check and system status
- `GET /api/v1/agents` - List all active agents
- `POST /api/v1/workflows/execute` - Execute workflows

#### Vector Database Integration

For semantic search capabilities:

```bash
# Start Qdrant vector database
docker run -p 6333:6333 qdrant/qdrant

# Run agent with RAG capabilities
cd crates/runtime && cargo run --example rag_example
```

---

## Configuration

### Environment Variables

Set up your environment for optimal performance:

```bash
# Basic configuration
export SYMBI_LOG_LEVEL=info
export SYMBI_RUNTIME_MODE=development

# Vector database (optional)
export QDRANT_URL=http://localhost:6333

# MCP integration (optional)
export MCP_SERVER_URLS="http://localhost:8080"
```

### Runtime Configuration

Create a `symbi.toml` configuration file:

```toml
[runtime]
max_agents = 1000
memory_limit_mb = 512
execution_timeout_seconds = 300

[security]
default_sandbox_tier = "docker"
audit_enabled = true
policy_enforcement = "strict"

[vector_db]
enabled = true
url = "http://localhost:6333"
collection_name = "symbi_knowledge"
```

---

## Common Issues

### Docker Issues

**Problem**: Docker build fails with permission errors
```bash
# Solution: Ensure Docker daemon is running and user has permissions
sudo systemctl start docker
sudo usermod -aG docker $USER
```

**Problem**: Container exits immediately
```bash
# Solution: Check Docker logs
docker logs <container_id>
```

### Rust Build Issues

**Problem**: Cargo build fails with dependency errors
```bash
# Solution: Update Rust and clean build cache
rustup update
cargo clean
cargo build
```

**Problem**: Missing system dependencies
```bash
# Ubuntu/Debian
sudo apt-get update
sudo apt-get install build-essential pkg-config libssl-dev

# macOS
brew install pkg-config openssl
```

### Runtime Issues

**Problem**: Agent fails to start
```bash
# Check agent definition syntax
cargo run -- dsl parse your_agent.symbi

# Enable debug logging
RUST_LOG=debug cd crates/runtime && cargo run --example basic_agent
```

---

## Getting Help

### Documentation

- **[DSL Guide](/dsl-guide)** - Complete DSL reference
- **[Runtime Architecture](/runtime-architecture)** - System architecture details
- **[Security Model](/security-model)** - Security and policy documentation

### Community Support

- **Issues**: [GitHub Issues](https://github.com/thirdkeyai/symbiont/issues)
- **Discussions**: [GitHub Discussions](https://github.com/thirdkeyai/symbiont/discussions)
- **Documentation**: [Complete API Reference](https://docs.symbiont.platform)

### Debug Mode

For troubleshooting, enable verbose logging:

```bash
# Enable debug logging
export RUST_LOG=symbi=debug

# Run with detailed output
cd crates/runtime && cargo run --example basic_agent 2>&1 | tee debug.log
```

---

## What's Next?

Now that you have Symbi running, explore these advanced topics:

1. **[DSL Guide](/dsl-guide)** - Learn advanced DSL features
2. **[Runtime Architecture](/runtime-architecture)** - Understand the system internals
3. **[Security Model](/security-model)** - Implement security policies
4. **[Contributing](/contributing)** - Contribute to the project

Ready to build something amazing? Start with our [example projects](https://github.com/thirdkeyai/symbiont/tree/main/crates/runtime/examples) or dive into the [complete specification](/specification).