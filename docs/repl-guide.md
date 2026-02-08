---
layout: default
title: REPL Guide
nav_order: 9
---

# Symbiont REPL Guide

The Symbiont REPL (Read-Eval-Print Loop) provides an interactive environment for developing, testing, and debugging Symbiont agents and DSL code.

## Features

- **Interactive DSL Evaluation**: Execute Symbiont DSL code in real-time
- **Agent Lifecycle Management**: Create, start, stop, pause, resume, and destroy agents
- **Execution Monitoring**: Real-time monitoring of agent execution with statistics and traces
- **Policy Enforcement**: Built-in policy checking and capability gating
- **Session Management**: Snapshot and restore REPL sessions
- **JSON-RPC Protocol**: Programmatic access via JSON-RPC over stdio
- **LSP Support**: Language Server Protocol for IDE integration

## Getting Started

### Starting the REPL

```bash
# Interactive REPL mode
symbi repl

# JSON-RPC server mode (for IDE integration)
symbi repl --json-rpc

# With custom configuration
symbi repl --config custom-config.toml
```

### Basic Usage

```rust
# Define an agent
agent GreetingAgent {
  name: "Greeting Agent"
  version: "1.0.0"
  description: "A simple greeting agent"
}

# Define a behavior
behavior Greet {
  input { name: string }
  output { greeting: string }
  steps {
    let greeting = format("Hello, {}!", name)
    return greeting
  }
}

# Execute expressions
let message = "Welcome to Symbiont"
print(message)
```

## REPL Commands

### Agent Management

| Command | Description |
|---------|-------------|
| `:agents` | List all agents |
| `:agent list` | List all agents |
| `:agent start <id>` | Start an agent |
| `:agent stop <id>` | Stop an agent |
| `:agent pause <id>` | Pause an agent |
| `:agent resume <id>` | Resume a paused agent |
| `:agent destroy <id>` | Destroy an agent |
| `:agent execute <id> <behavior> [args]` | Execute agent behavior |
| `:agent debug <id>` | Show debug info for an agent |

### Monitoring Commands

| Command | Description |
|---------|-------------|
| `:monitor stats` | Show execution statistics |
| `:monitor traces [limit]` | Show execution traces |
| `:monitor report` | Show detailed execution report |
| `:monitor clear` | Clear monitoring data |

### Session Commands

| Command | Description |
|---------|-------------|
| `:snapshot` | Create a session snapshot |
| `:clear` | Clear the session |
| `:help` or `:h` | Show help message |
| `:version` | Show version information |

## DSL Features

### Agent Definitions

```rust
agent DataAnalyzer {
  name: "Data Analysis Agent"
  version: "2.1.0"
  description: "Analyzes datasets with privacy protection"
  
  security {
    capabilities: ["data_read", "analysis"]
    sandbox: true
  }
  
  resources {
    memory: 512MB
    cpu: 2
    storage: 1GB
  }
}
```

### Behavior Definitions

```rust
behavior AnalyzeData {
  input { 
    data: DataSet
    options: AnalysisOptions 
  }
  output { 
    results: AnalysisResults 
  }
  
  steps {
    # Check data privacy requirements
    require capability("data_read")
    
    if (data.contains_pii) {
      return error("Cannot process data with PII")
    }
    
    # Perform analysis
    let results = analyze(data, options)
    emit analysis_completed { results: results }
    
    return results
  }
}
```

### Built-in Functions

| Function | Description | Example |
|----------|-------------|---------|
| `print(...)` | Print values to output | `print("Hello", name)` |
| `len(value)` | Get length of string, list, or map | `len("hello")` → `5` |
| `upper(string)` | Convert string to uppercase | `upper("hello")` → `"HELLO"` |
| `lower(string)` | Convert string to lowercase | `lower("HELLO")` → `"hello"` |
| `format(template, ...)` | Format string with arguments | `format("Hello, {}!", name)` |

### Data Types

```rust
# Basic types
let name = "Alice"          # String
let age = 30               # Integer
let height = 5.8           # Number
let active = true          # Boolean
let empty = null           # Null

# Collections
let items = [1, 2, 3]      # List
let config = {             # Map
  "host": "localhost",
  "port": 8080
}

# Time and size units
let timeout = 30s          # Duration
let max_size = 100MB       # Size
```

## Architecture

### Components

```
symbi repl
├── repl-cli/          # CLI interface and JSON-RPC server
├── repl-core/         # Core REPL engine and evaluator  
├── repl-proto/        # JSON-RPC protocol definitions
└── repl-lsp/          # Language Server Protocol implementation
```

### Core Components

- **DslEvaluator**: Executes DSL programs with runtime integration
- **ReplEngine**: Coordinates evaluation and command handling
- **ExecutionMonitor**: Tracks execution statistics and traces
- **RuntimeBridge**: Integrates with Symbiont runtime for policy enforcement
- **SessionManager**: Handles snapshots and session state

### JSON-RPC Protocol

The REPL supports JSON-RPC 2.0 for programmatic access:

```json
// Evaluate DSL code
{
  "jsonrpc": "2.0",
  "method": "evaluate",
  "params": {"input": "let x = 42"},
  "id": 1
}

// Response
{
  "jsonrpc": "2.0",
  "result": {"value": "42", "type": "integer"},
  "id": 1
}
```

## Security & Policy Enforcement

### Capability Checking

The REPL enforces capability requirements defined in agent security blocks:

```rust
agent SecureAgent {
  name: "Secure Agent"
  security {
    capabilities: ["filesystem", "network"]
    sandbox: true
  }
}

behavior ReadFile {
  input { path: string }
  output { content: string }
  steps {
    # This will check if agent has "filesystem" capability
    require capability("filesystem")
    let content = read_file(path)
    return content
  }
}
```

### Policy Integration

The REPL integrates with the Symbiont policy engine to enforce access controls and audit requirements.

## Debugging & Monitoring

### Execution Traces

```
:monitor traces 10

Recent Execution Traces:
  14:32:15.123 - AgentCreated [Agent: abc-123] (2ms)
  14:32:15.125 - AgentStarted [Agent: abc-123] (1ms)  
  14:32:15.130 - BehaviorExecuted [Agent: abc-123] (5ms)
  14:32:15.135 - AgentPaused [Agent: abc-123]
```

### Statistics

```
:monitor stats

Execution Monitor Statistics:
  Total Executions: 42
  Successful: 38
  Failed: 4
  Success Rate: 90.5%
  Average Duration: 12.3ms
  Total Duration: 516ms
  Active Executions: 2
```

### Agent Debugging

```
:agent debug abc-123

Agent Debug Information:
  ID: abc-123-def-456
  Name: Data Analyzer
  Version: 2.1.0
  State: Running
  Created: 2024-01-15 14:30:00 UTC
  Description: Analyzes datasets with privacy protection
  Author: data-team@company.com
  Available Functions/Behaviors: 5
  Required Capabilities: 2
    - data_read
    - analysis
  Resource Configuration:
    Memory: 512MB
    CPU: 2
    Storage: 1GB
```

## IDE Integration

### Language Server Protocol

The REPL provides LSP support for IDE integration:

```bash
# Start LSP server
symbi repl --lsp --port 9257
```

### Supported Features

- Syntax highlighting
- Code completion  
- Error diagnostics
- Hover information
- Go to definition
- Symbol search

## Best Practices

### Development Workflow

1. **Start with Simple Expressions**: Test basic DSL constructs
2. **Define Agents Incrementally**: Start with minimal agent definitions
3. **Test Behaviors Separately**: Define and test behaviors before integration
4. **Use Monitoring**: Leverage execution monitoring for debugging
5. **Create Snapshots**: Save important session states

### Performance Tips

- Use `:monitor clear` periodically to reset monitoring data
- Limit trace history with `:monitor traces <limit>`
- Destroy unused agents to free resources
- Use snapshots for complex session states

### Security Considerations

- Always define appropriate capabilities for agents
- Test policy enforcement in development
- Use sandbox mode for untrusted code
- Monitor execution traces for security events

## Troubleshooting

### Common Issues

**Agent Creation Fails**
```
Error: Missing capability: filesystem
```
*Solution*: Add required capabilities to agent security block

**Execution Timeout**
```
Error: Maximum execution depth exceeded
```
*Solution*: Check for infinite recursion in behavior logic

**Policy Violation**
```
Error: Policy violation: data access denied
```
*Solution*: Verify agent has appropriate permissions

### Debug Commands

```rust
# Check agent state
:agent debug <agent-id>

# View execution traces
:monitor traces 50

# Check system statistics  
:monitor stats

# Create debug snapshot
:snapshot
```

## Examples

### Simple Agent

```rust
agent Calculator {
  name: "Basic Calculator"
  version: "1.0.0"
}

behavior Add {
  input { a: number, b: number }
  output { result: number }
  steps {
    return a + b
  }
}

# Test the behavior
let result = Add(5, 3)
print("5 + 3 =", result)
```

### Data Processing Agent

```rust
agent DataProcessor {
  name: "Data Processing Agent"
  version: "1.0.0"
  
  security {
    capabilities: ["data_read", "data_write"]
    sandbox: true
  }
  
  resources {
    memory: 256MB
    cpu: 1
  }
}

behavior ProcessCsv {
  input { file_path: string }
  output { summary: ProcessingSummary }
  
  steps {
    require capability("data_read")
    
    let data = read_csv(file_path)
    let processed = transform_data(data)
    
    require capability("data_write") 
    write_results(processed)
    
    return {
      "rows_processed": len(data),
      "status": "completed"
    }
  }
}
```

## See Also

- [DSL Guide](dsl-guide.md) - Complete DSL language reference
- [Runtime Architecture](runtime-architecture.md) - System architecture overview
- [Security Model](security-model.md) - Security implementation details
- [API Reference](api-reference.md) - Complete API documentation