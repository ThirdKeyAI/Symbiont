# repl-cli

Command-line interface and JSON-RPC server for the Symbiont REPL. Provides both interactive and programmatic access to the Symbiont DSL evaluation engine.

## Features

- **Interactive REPL**: Command-line interface for DSL development
- **JSON-RPC Server**: Programmatic access via JSON-RPC 2.0 over stdio
- **Agent Management**: Complete agent lifecycle control
- **Execution Monitoring**: Real-time statistics and trace visualization
- **Session Management**: Snapshot and restore capabilities
- **IDE Integration**: Foundation for Language Server Protocol support

## Installation

```bash
# Build from source
cargo build --release

# Install from crates.io
cargo install repl-cli

# Run via symbi CLI
symbi repl
```

## Usage

### Interactive Mode

```bash
# Start interactive REPL
symbi repl

# With custom configuration
symbi repl --config custom-config.toml

# Enable debug logging
RUST_LOG=debug symbi repl
```

### JSON-RPC Server Mode

```bash
# Start JSON-RPC server on stdio
symbi repl --json-rpc

# Specify alternative communication method
symbi repl --json-rpc --port 9257
```

## Interactive Commands

### Basic Usage

```symbiont
# Define an agent
agent Calculator {
  name: "Basic Calculator"
  version: "1.0.0"
}

# Define a behavior
behavior Add {
  input { a: number, b: number }
  output { result: number }
  steps {
    return a + b
  }
}

# Execute expressions
let result = Add(5, 3)
print("5 + 3 =", result)
```

### REPL Commands

| Command | Description |
|---------|-------------|
| `:help` or `:h` | Show help message |
| `:agents` | List all agents |
| `:agent start <id>` | Start an agent |
| `:agent stop <id>` | Stop an agent |
| `:agent execute <id> <behavior> [args]` | Execute agent behavior |
| `:monitor stats` | Show execution statistics |
| `:monitor traces [limit]` | Show execution traces |
| `:snapshot` | Create session snapshot |
| `:clear` | Clear session |
| `:version` | Show version information |

## JSON-RPC Protocol

The CLI supports JSON-RPC 2.0 for programmatic access:

### Evaluate DSL Code

**Request:**
```json
{
  "jsonrpc": "2.0",
  "method": "evaluate",
  "params": {
    "input": "let x = 42\nprint(x)"
  },
  "id": 1
}
```

**Response:**
```json
{
  "jsonrpc": "2.0",
  "result": {
    "output": "42\n",
    "value": "null",
    "type": "null"
  },
  "id": 1
}
```

### List Agents

**Request:**
```json
{
  "jsonrpc": "2.0",
  "method": "list_agents",
  "params": {},
  "id": 2
}
```

**Response:**
```json
{
  "jsonrpc": "2.0",
  "result": {
    "agents": [
      {
        "id": "abc-123-def-456",
        "name": "Calculator",
        "state": "Running",
        "created_at": "2024-01-15T14:30:00Z"
      }
    ]
  },
  "id": 2
}
```

### Execute Agent Behavior

**Request:**
```json
{
  "jsonrpc": "2.0",
  "method": "execute_agent_behavior",
  "params": {
    "agent_id": "abc-123-def-456",
    "behavior_name": "Add",
    "arguments": "5 3"
  },
  "id": 3
}
```

**Response:**
```json
{
  "jsonrpc": "2.0",
  "result": {
    "value": "8",
    "type": "integer",
    "execution_time_ms": 12
  },
  "id": 3
}
```

### Error Responses

```json
{
  "jsonrpc": "2.0",
  "error": {
    "code": -32600,
    "message": "Invalid Request",
    "data": "Missing required parameter: input"
  },
  "id": null
}
```

## Configuration

### Configuration File

```toml
# repl-config.toml

[repl]
max_history = 1000
auto_save_snapshots = true
snapshot_interval = 300  # seconds

[runtime]
timeout = 30000  # milliseconds
max_agents = 100
enable_sandbox = true

[monitoring]
enable_traces = true
max_trace_entries = 10000
trace_buffer_size = 1000

[security]
enable_policy_enforcement = true
require_capabilities = true
audit_all_operations = true

[logging]
level = "info"
format = "json"
output = "stdout"
```

### Environment Variables

| Variable | Description | Default |
|----------|-------------|---------|
| `RUST_LOG` | Logging level | `info` |
| `REPL_CONFIG` | Configuration file path | `repl-config.toml` |
| `REPL_HISTORY` | History file path | `.repl_history` |
| `REPL_SNAPSHOTS_DIR` | Snapshots directory | `./snapshots` |

## Architecture

```
repl-cli/
├── src/
│   ├── main.rs          # CLI entry point
│   ├── repl.rs          # Interactive REPL implementation
│   ├── server.rs        # JSON-RPC server
│   ├── config.rs        # Configuration management
│   ├── history.rs       # Command history
│   └── output.rs        # Output formatting
└── Cargo.toml
```

### Components

- **CLI Interface**: Interactive command-line interface with history and completion
- **JSON-RPC Server**: Protocol implementation for programmatic access
- **Configuration Manager**: Settings and environment handling
- **Output Formatter**: Pretty-printing and syntax highlighting
- **History Manager**: Command history persistence

## Integration Examples

### Python Client

```python
import json
import subprocess

class SymbiREPL:
    def __init__(self):
        self.process = subprocess.Popen(
            ['symbi', 'repl', '--json-rpc'],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True
        )
    
    def evaluate(self, code):
        request = {
            "jsonrpc": "2.0",
            "method": "evaluate",
            "params": {"input": code},
            "id": 1
        }
        
        self.process.stdin.write(json.dumps(request) + '\n')
        self.process.stdin.flush()
        
        response = self.process.stdout.readline()
        return json.loads(response)
    
    def close(self):
        self.process.terminate()

# Usage
repl = SymbiREPL()
result = repl.evaluate("let x = 42")
print(result)
repl.close()
```

### Node.js Client

```javascript
const { spawn } = require('child_process');

class SymbiREPL {
    constructor() {
        this.process = spawn('symbi', ['repl', '--json-rpc']);
        this.requestId = 0;
    }
    
    async evaluate(code) {
        const request = {
            jsonrpc: "2.0",
            method: "evaluate",
            params: { input: code },
            id: ++this.requestId
        };
        
        return new Promise((resolve, reject) => {
            this.process.stdout.once('data', (data) => {
                try {
                    const response = JSON.parse(data.toString());
                    resolve(response);
                } catch (error) {
                    reject(error);
                }
            });
            
            this.process.stdin.write(JSON.stringify(request) + '\n');
        });
    }
    
    close() {
        this.process.kill();
    }
}

// Usage
const repl = new SymbiREPL();
repl.evaluate("let x = 42").then(result => {
    console.log(result);
    repl.close();
});
```

## Testing

```bash
# Run all tests
cargo test

# Run integration tests
cargo test --test integration

# Test JSON-RPC protocol
cargo test rpc_protocol

# Test interactive features
cargo test interactive
```

## Dependencies

- `tokio` - Async runtime
- `serde` - Serialization framework
- `serde_json` - JSON support
- `clap` - Command-line argument parsing
- `rustyline` - Interactive line editing
- `tracing` - Structured logging
- `repl-core` - Core REPL functionality
- `repl-proto` - Protocol definitions

## See Also

- [`repl-core`](../repl-core/README.md) - Core REPL engine
- [`repl-proto`](../repl-proto/README.md) - Protocol definitions
- [`repl-lsp`](../repl-lsp/README.md) - Language Server Protocol implementation
- [REPL Guide](../../docs/repl-guide.md) - Complete user guide