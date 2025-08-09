# Symbiont DSL Specification

## Overview

The Symbiont DSL (Domain-Specific Language) provides a declarative syntax for defining agent behaviors, configurations, and interactions within the Symbiont runtime environment. The DSL is designed to be expressive, safe, and easily parseable while enabling sophisticated agent automation scenarios.

## Core Language Constructs

### 1. Agent Definition

```symbiont
agent MyAgent {
    name: "My Test Agent"
    version: "1.0.0"
    author: "Developer"
    description: "A sample agent for demonstration"
    
    // Resource requirements
    resources {
        memory: 512MB
        cpu: 1000ms
        network: allow
        storage: 100MB
    }
    
    // Security configuration
    security {
        tier: Tier1
        capabilities: [FileSystem.Read, Network.Http]
        sandbox: strict
    }
    
    // Policies
    policies {
        execution_timeout: 30s
        retry_count: 3
        failure_action: terminate
    }
}
```

### 2. Behavior Definitions

```symbiont
behavior ProcessFiles {
    input {
        directory: string
        pattern: string = "*.txt"
    }
    
    output {
        processed_count: number
        results: list<string>
    }
    
    steps {
        // Variable binding
        let files = fs.list(directory, pattern)
        
        // Conditional logic
        if files.length == 0 {
            log.warn("No files found in directory: {directory}")
            return { processed_count: 0, results: [] }
        }
        
        // Iteration
        let results = []
        for file in files {
            // Capability-gated operation
            require capability FileSystem.Read
            let content = fs.read(file)
            
            // Function call
            let processed = process_content(content)
            results.push(processed)
            
            // Progress reporting
            emit progress { file: file, status: "completed" }
        }
        
        return { 
            processed_count: files.length, 
            results: results 
        }
    }
}
```

### 3. Function Definitions

```symbiont
function process_content(content: string) -> string {
    // String operations
    let lines = content.split("\n")
    let filtered = lines.filter(line -> line.trim().length > 0)
    let processed = filtered.map(line -> line.upper())
    return processed.join("\n")
}
```

### 4. Event Handling

```symbiont
on file_changed(path: string) {
    log.info("File changed: {path}")
    
    // Trigger behavior
    let result = invoke ProcessFiles {
        directory: path.parent()
        pattern: path.basename()
    }
    
    // Emit results
    emit file_processed { path: path, result: result }
}

on timer(interval: 5m) {
    log.debug("Periodic health check")
    emit heartbeat { timestamp: now() }
}
```

### 5. Variable and Data Types

```symbiont
// Primitive types
let name: string = "example"
let count: number = 42
let active: boolean = true
let timestamp: datetime = now()

// Collections
let items: list<string> = ["a", "b", "c"]
let config: map<string, any> = {
    "host": "localhost",
    "port": 8080,
    "enabled": true
}

// Optional types
let optional_value: string? = null

// Custom types (structs)
struct FileInfo {
    path: string
    size: number
    modified: datetime
}
```

### 6. Control Flow

```symbiont
// Conditional statements
if condition {
    // if block
} else if other_condition {
    // else if block
} else {
    // else block
}

// Pattern matching
match file_type {
    "txt" -> process_text_file(file)
    "json" -> parse_json_file(file)
    "csv" -> process_csv_file(file)
    _ -> log.warn("Unknown file type: {file_type}")
}

// Loops
for item in collection {
    process(item)
}

while condition {
    // loop body
}

// Error handling
try {
    let result = risky_operation()
    log.info("Success: {result}")
} catch error {
    log.error("Failed: {error}")
    return null
}
```

### 7. Built-in Libraries

#### File System (fs)
```symbiont
fs.read(path: string) -> string
fs.write(path: string, content: string) -> void
fs.exists(path: string) -> boolean
fs.list(directory: string, pattern: string = "*") -> list<string>
fs.delete(path: string) -> void
fs.create_directory(path: string) -> void
```

#### HTTP Client (http)
```symbiont
http.get(url: string, headers: map<string, string> = {}) -> HttpResponse
http.post(url: string, body: string, headers: map<string, string> = {}) -> HttpResponse
http.put(url: string, body: string, headers: map<string, string> = {}) -> HttpResponse
http.delete(url: string, headers: map<string, string> = {}) -> HttpResponse
```

#### Logging (log)
```symbiont
log.debug(message: string, context: map<string, any> = {}) -> void
log.info(message: string, context: map<string, any> = {}) -> void
log.warn(message: string, context: map<string, any> = {}) -> void
log.error(message: string, context: map<string, any> = {}) -> void
```

#### Time (time)
```symbiont
time.now() -> datetime
time.sleep(duration: duration) -> void
time.format(dt: datetime, format: string) -> string
time.parse(input: string, format: string) -> datetime
```

### 8. Agent Lifecycle Commands

```symbiont
// Agent management
agent.start(config: AgentConfig) -> AgentId
agent.stop(id: AgentId) -> void
agent.restart(id: AgentId) -> void
agent.status(id: AgentId) -> AgentStatus

// Behavior invocation
let result = invoke BehaviorName {
    param1: value1
    param2: value2
}

// Event emission
emit event_name { data: value }

// Policy checks
require capability CapabilityName
check policy PolicyName
```

### 9. REPL Commands

```symbiont
// Agent lifecycle
:create agent MyAgent
:start agent MyAgent
:stop agent <agent_id>
:list agents
:status agent <agent_id>

// Behavior execution
:run behavior ProcessFiles { directory: "/tmp", pattern: "*.log" }
:invoke MyAgent.ProcessFiles { directory: "/home" }

// Debugging
:debug on
:trace agent <agent_id>
:breakpoint set ProcessFiles:10
:step
:continue

// Inspection
:inspect agent <agent_id>
:memory agent <agent_id>
:events agent <agent_id>
:logs agent <agent_id>

// Snapshots and sessions
:snapshot save session1
:snapshot restore session1
:snapshot list
```

## Syntax Rules

### Comments
```symbiont
// Single-line comment
/* Multi-line
   comment */
```

### String Interpolation
```symbiont
let name = "World"
let greeting = "Hello, {name}!"  // Result: "Hello, World!"
```

### Duration Literals
```symbiont
let timeout = 30s      // 30 seconds
let interval = 5m      // 5 minutes
let deadline = 2h      // 2 hours
let period = 1d        // 1 day
```

### Size Literals
```symbiont
let small_file = 1KB
let medium_file = 10MB
let large_file = 1GB
```

## Error Handling

The DSL provides structured error handling with try-catch blocks and automatic error propagation:

```symbiont
function safe_operation() -> Result<string, Error> {
    try {
        let result = risky_call()
        return Ok(result)
    } catch error {
        return Err(error)
    }
}

// Automatic error propagation with ?
function chained_operations() -> Result<string, Error> {
    let step1 = operation1()?
    let step2 = operation2(step1)?
    let final_result = operation3(step2)?
    return Ok(final_result)
}
```

## Security Model

The DSL enforces capability-based security at the language level:

```symbiont
behavior SecureFileOperation {
    steps {
        // This will fail at runtime if FileSystem.Read capability is not granted
        require capability FileSystem.Read
        let content = fs.read("/sensitive/file.txt")
        
        // Multiple capabilities can be required
        require capabilities [FileSystem.Write, Network.Http]
        
        // Conditional capability checking
        if has_capability(FileSystem.Write) {
            fs.write("/output/result.txt", content)
        } else {
            log.warn("Write capability not available")
        }
    }
}
```

## Execution Model

- **Deterministic**: All operations use seeded randomness and controlled time
- **Sandboxed**: Agents run in isolated environments with resource limits
- **Policy-Enforced**: All operations are subject to policy evaluation
- **Observable**: All agent actions can be monitored and traced
- **Recoverable**: Agent state can be snapshotted and restored

This DSL specification provides a foundation for implementing sophisticated agent behaviors while maintaining security, observability, and determinism within the Symbiont runtime environment.