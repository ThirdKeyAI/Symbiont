# repl-core

Core REPL engine for the Symbiont agent framework. Provides DSL evaluation, agent lifecycle management, and execution monitoring capabilities.

## Features

- **DSL Evaluation**: Execute Symbiont DSL programs with runtime integration
- **Agent Management**: Create, start, stop, pause, resume, and destroy agents
- **Execution Monitoring**: Real-time monitoring with statistics and tracing
- **Policy Enforcement**: Capability checking and security policy integration
- **Session Management**: Snapshot and restore functionality
- **Built-in Functions**: Standard library of DSL functions

## Architecture

```
repl-core/
├── src/
│   ├── dsl/                    # DSL implementation
│   │   ├── ast.rs             # Abstract syntax tree definitions
│   │   ├── lexer.rs           # Lexical analysis
│   │   ├── parser.rs          # Parser implementation
│   │   ├── evaluator.rs       # DSL evaluator with runtime integration
│   │   └── mod.rs             # DSL module exports
│   ├── execution_monitor.rs   # Execution monitoring and tracing
│   ├── eval.rs                # REPL evaluation engine
│   ├── error.rs               # Error handling
│   ├── runtime_bridge.rs      # Runtime system integration
│   ├── session.rs             # Session management
│   └── lib.rs                 # Library exports
└── tests/
    └── golden.rs              # Golden tests for parser
```

## Usage

```rust
use repl_core::{ReplEngine, RuntimeBridge};
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create runtime bridge
    let runtime_bridge = Arc::new(RuntimeBridge::new());
    
    // Create REPL engine
    let engine = ReplEngine::new(runtime_bridge);
    
    // Evaluate DSL code
    let result = engine.evaluate(r#"
        agent TestAgent {
            name: "Test Agent"
            version: "1.0.0"
        }
    "#).await?;
    
    println!("Result: {}", result);
    Ok(())
}
```

## DSL Components

### AST (Abstract Syntax Tree)

The AST module defines the structure of parsed DSL programs:

- **Agents**: Agent definitions with metadata and security requirements
- **Behaviors**: Executable behavior definitions with input/output types
- **Functions**: User-defined functions with parameters and bodies
- **Expressions**: Literals, identifiers, function calls, operations
- **Statements**: Control flow, variable assignments, requirements

### Lexer

The lexer converts source text into tokens:

- Keywords (`agent`, `behavior`, `function`, `let`, `if`, etc.)
- Identifiers and literals (strings, numbers, booleans)
- Operators and punctuation
- Duration and size literals (`30s`, `100MB`)
- Comments (single-line and multi-line)

### Parser

The parser builds an AST from tokens using recursive descent parsing:

- Agent definitions with nested blocks
- Function and behavior definitions
- Expression parsing with operator precedence
- Error recovery and reporting

### Evaluator

The evaluator executes parsed DSL programs:

- **Agent Lifecycle**: Create, start, stop, pause, resume, destroy
- **Policy Enforcement**: Capability checking via runtime bridge
- **Built-in Functions**: `print`, `len`, `upper`, `lower`, `format`
- **Execution Context**: Variable scoping and function definitions
- **Monitoring Integration**: Execution tracing and statistics

## Built-in Functions

| Function | Description | Example |
|----------|-------------|---------|
| `print(...)` | Print values to output | `print("Hello", name)` |
| `len(value)` | Get length of string, list, or map | `len("hello")` → `5` |
| `upper(string)` | Convert string to uppercase | `upper("hello")` → `"HELLO"` |
| `lower(string)` | Convert string to lowercase | `lower("HELLO")` → `"hello"` |
| `format(template, ...)` | Format string with arguments | `format("Hello, {}!", name)` |

## Execution Monitoring

The execution monitor provides comprehensive tracking:

```rust
use repl_core::ExecutionMonitor;

let monitor = ExecutionMonitor::new();

// Get execution statistics
let stats = monitor.get_stats();
println!("Total executions: {}", stats.total_executions);
println!("Success rate: {:.1}%", 
    (stats.successful_executions as f64 / stats.total_executions as f64) * 100.0);

// Get recent traces
let traces = monitor.get_traces(Some(10));
for trace in traces {
    println!("{} - {:?}", trace.timestamp, trace.event_type);
}
```

### Trace Events

- `AgentCreated` - Agent instance created
- `AgentStarted` - Agent started execution
- `AgentPaused` - Agent paused
- `AgentResumed` - Agent resumed from pause
- `AgentDestroyed` - Agent destroyed
- `BehaviorExecuted` - Agent behavior executed
- `ExecutionStarted` - Function execution started
- `ExecutionCompleted` - Function execution completed

## Security & Policy

The REPL core integrates with the Symbiont runtime for security:

```rust
// Capability checking
if !evaluator.check_capability("filesystem").await? {
    return Err(ReplError::Security("Missing filesystem capability".to_string()));
}

// Policy enforcement
let decision = runtime_bridge.check_capability(agent_id, &capability).await?;
match decision {
    PolicyDecision::Allow => {
        // Proceed with operation
    }
    PolicyDecision::Deny => {
        return Err(ReplError::Security("Access denied".to_string()));
    }
}
```

## Error Handling

The `ReplError` enum covers all error conditions:

```rust
pub enum ReplError {
    Parsing(String),           // Parser errors
    Lexing(String),           // Lexer errors
    Evaluation(String),       // Evaluation errors
    Execution(String),        // Execution errors
    Security(String),         // Security violations
    Runtime(String),          // Runtime bridge errors
    Io(std::io::Error),      // I/O errors
    Serde(serde_json::Error), // Serialization errors
}
```

## Testing

Run the test suite:

```bash
# Run all tests
cargo test

# Run specific test categories
cargo test dsl::lexer::tests    # Lexer tests
cargo test dsl::parser::tests   # Parser tests
cargo test dsl::evaluator::tests # Evaluator tests
cargo test execution_monitor::tests # Monitor tests

# Run golden tests
cargo test --test golden
```

## Dependencies

- `tokio` - Async runtime
- `serde` - Serialization framework
- `serde_json` - JSON support
- `uuid` - UUID generation
- `chrono` - Date/time handling
- `tracing` - Structured logging
- `symbi-runtime` - Runtime system integration

## See Also

- [`repl-cli`](../repl-cli/README.md) - CLI interface and JSON-RPC server
- [`repl-proto`](../repl-proto/README.md) - Protocol definitions
- [`repl-lsp`](../repl-lsp/README.md) - Language Server Protocol implementation
- [REPL Guide](../../docs/repl-guide.md) - Complete user guide