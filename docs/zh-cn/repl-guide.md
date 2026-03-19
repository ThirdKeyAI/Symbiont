# Symbiont REPL 指南

## 其他语言


## 功能特性

- **交互式 DSL 求值**：实时执行 Symbiont DSL 代码
- **智能体生命周期管理**：创建、启动、停止、暂停、恢复和销毁智能体
- **执行监控**：通过统计信息和追踪进行实时执行监控
- **策略执行**：内置策略检查和能力门控
- **会话管理**：快照和恢复 REPL 会话
- **JSON-RPC 协议**：通过 stdio 上的 JSON-RPC 进行程序化访问
- **LSP 支持**：用于 IDE 集成的语言服务器协议

## 入门指南

### 启动 REPL

```bash
# Interactive REPL mode
symbi repl

# JSON-RPC server mode over stdio (for IDE integration)
symbi repl --stdio
```

> **注意：** `--config` 标志尚不支持。配置从默认的 `symbiont.toml` 位置读取。自定义配置支持计划在未来版本中提供。

### 基本使用

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

## REPL 命令

### 智能体管理

| 命令 | 描述 |
|------|------|
| `:agents` | 列出所有智能体 |
| `:agent list` | 列出所有智能体 |
| `:agent start <id>` | 启动智能体 |
| `:agent stop <id>` | 停止智能体 |
| `:agent pause <id>` | 暂停智能体 |
| `:agent resume <id>` | 恢复已暂停的智能体 |
| `:agent destroy <id>` | 销毁智能体 |
| `:agent execute <id> <behavior> [args]` | 执行智能体行为 |
| `:agent debug <id>` | 显示智能体的调试信息 |

### 监控命令

| 命令 | 描述 |
|------|------|
| `:monitor stats` | 显示执行统计信息 |
| `:monitor traces [limit]` | 显示执行追踪 |
| `:monitor report` | 显示详细执行报告 |
| `:monitor clear` | 清除监控数据 |

### 内存命令

| 命令 | 描述 |
|------|------|
| `:memory inspect <agent-id>` | 查看智能体的内存状态 |
| `:memory compact <agent-id>` | 压缩智能体的内存存储 |
| `:memory purge <agent-id>` | 清除智能体的所有内存 |

### Webhook 命令

| 命令 | 描述 |
|------|------|
| `:webhook list` | 列出已配置的 webhook |
| `:webhook add` | 添加新的 webhook |
| `:webhook remove` | 移除 webhook |
| `:webhook test` | 测试 webhook |
| `:webhook logs` | 显示 webhook 日志 |

### 录制命令

| 命令 | 描述 |
|------|------|
| `:record on <file>` | 开始将会话录制到文件 |
| `:record off` | 停止录制会话 |

### 会话命令

| 命令 | 描述 |
|------|------|
| `:snapshot` | 创建会话快照 |
| `:clear` | 清除会话 |
| `:help` 或 `:h` | 显示帮助信息 |
| `:version` | 显示版本信息 |

## DSL 功能

### 智能体定义

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

### 行为定义

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
    # NOTE: analyze() is a planned built-in function (not yet implemented).
    # This example illustrates the intended behavior definition pattern.
    let results = analyze(data, options)
    emit analysis_completed { results: results }

    return results
  }
}
```

### 内置函数

| 函数 | 描述 | 示例 |
|------|------|------|
| `print(...)` | 将值输出到控制台 | `print("Hello", name)` |
| `len(value)` | 获取字符串、列表或映射的长度 | `len("hello")` -> `5` |
| `upper(string)` | 将字符串转换为大写 | `upper("hello")` -> `"HELLO"` |
| `lower(string)` | 将字符串转换为小写 | `lower("HELLO")` -> `"hello"` |
| `format(template, ...)` | 使用参数格式化字符串 | `format("Hello, {}!", name)` |

> **计划的内置函数：** 高级 I/O 函数如 `read_file()`、`read_csv()`、`write_results()`、`analyze()` 和 `transform_data()` 尚未实现。这些计划在未来版本中提供。

### 数据类型

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

## 架构

### 组件

```
symbi repl
├── repl-cli/          # CLI interface and JSON-RPC server
├── repl-core/         # Core REPL engine and evaluator
├── repl-proto/        # JSON-RPC protocol definitions
└── repl-lsp/          # Language Server Protocol implementation
```

### 核心组件

- **DslEvaluator**：通过运行时集成执行 DSL 程序
- **ReplEngine**：协调求值和命令处理
- **ExecutionMonitor**：跟踪执行统计信息和追踪
- **RuntimeBridge**：与 Symbiont 运行时集成进行策略执行
- **SessionManager**：处理快照和会话状态

### JSON-RPC 协议

REPL 支持 JSON-RPC 2.0 进行程序化访问：

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

## 安全与策略执行

### 能力检查

REPL 执行智能体安全块中定义的能力要求：

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
    # NOTE: read_file() is a planned built-in function (not yet implemented).
    # This example illustrates how capability checking works.
    let content = read_file(path)
    return content
  }
}
```

### 策略集成

REPL 与 Symbiont 策略引擎集成，以执行访问控制和审计要求。

## 调试与监控

### 执行追踪

```
:monitor traces 10

Recent Execution Traces:
  14:32:15.123 - AgentCreated [Agent: abc-123] (2ms)
  14:32:15.125 - AgentStarted [Agent: abc-123] (1ms)
  14:32:15.130 - BehaviorExecuted [Agent: abc-123] (5ms)
  14:32:15.135 - AgentPaused [Agent: abc-123]
```

### 统计信息

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

### 智能体调试

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

## IDE 集成

### 语言服务器协议

REPL 通过 `repl-lsp` crate 为 IDE 集成提供 LSP 支持。LSP 服务器与 REPL 本身是分开启动的：

```bash
# The LSP server is provided by the repl-lsp crate and launched
# by your editor's LSP client configuration (not via symbi repl flags).
```

> **注意：** `symbi repl` 不支持 `--lsp` 标志。LSP 在 `repl-lsp` crate 中实现，应通过编辑器的 LSP 设置进行配置。

### 支持的功能

- 语法高亮
- 错误诊断
- 文本同步

**计划功能**（尚未实现）：
- 代码补全
- 悬停信息
- 转到定义
- 符号搜索

## 最佳实践

### 开发工作流

1. **从简单表达式开始**：测试基本的 DSL 构造
2. **增量定义智能体**：从最小的智能体定义开始
3. **单独测试行为**：在集成之前定义和测试行为
4. **使用监控**：利用执行监控进行调试
5. **创建快照**：保存重要的会话状态

### 性能建议

- 定期使用 `:monitor clear` 重置监控数据
- 使用 `:monitor traces <limit>` 限制追踪历史
- 销毁未使用的智能体以释放资源
- 对复杂会话状态使用快照

### 安全注意事项

- 始终为智能体定义适当的能力
- 在开发中测试策略执行
- 对不受信任的代码使用沙箱模式
- 监控执行追踪以发现安全事件

## 故障排除

### 常见问题

**智能体创建失败**
```
Error: Missing capability: filesystem
```
*解决方案*：在智能体安全块中添加所需的能力

**执行超时**
```
Error: Maximum execution depth exceeded
```
*解决方案*：检查行为逻辑中的无限递归

**策略违规**
```
Error: Policy violation: data access denied
```
*解决方案*：验证智能体是否具有适当的权限

### 调试命令

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

## 示例

### 简单智能体

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

### 数据处理智能体

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

    # NOTE: read_csv(), transform_data(), and write_results() are planned
    # built-in functions (not yet implemented). This example illustrates
    # the intended pattern for data processing behaviors.
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

## 另请参阅

- [DSL 指南](dsl-guide.md) - 完整的 DSL 语言参考
- [运行时架构](runtime-architecture.md) - 系统架构概览
- [安全模型](security-model.md) - 安全实现细节
- [API 参考](api-reference.md) - 完整的 API 文档
