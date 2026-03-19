# 原生执行模式（无 Docker/隔离）

## 其他语言

[English](native-execution-guide.md) | ## 概述

Symbiont 支持在没有 Docker 或容器隔离的情况下运行智能体，适用于开发环境或需要最大性能和最小依赖的受信任部署。

## 安全警告

**重要**：原生执行模式绕过了所有基于容器的安全控制：

- 无进程隔离
- 无文件系统隔离
- 无网络隔离
- 无资源限制执行
- 直接访问主机系统

**仅在以下情况使用**：
- 使用受信任代码的本地开发
- 使用受信任智能体的受控环境
- 测试和调试
- Docker 不可用的环境

**不要用于**：
- 运行不受信任代码的生产环境
- 多租户部署
- 面向公众的服务
- 处理不受信任的用户输入

## 架构

### 沙箱层级

```
┌─────────────────────────────────────────┐
│ SecurityTier::None (Native Execution)   │ ← No isolation
├─────────────────────────────────────────┤
│ SecurityTier::Tier1 (Docker)            │ ← Container isolation
├─────────────────────────────────────────┤
│ SecurityTier::Tier2 (gVisor)            │ ← Enhanced isolation
├─────────────────────────────────────────┤
│ SecurityTier::Tier3 (Firecracker)       │ ← Maximum isolation
└─────────────────────────────────────────┘
```

### 原生执行流程

```mermaid
graph LR
    A[Agent Request] --> B{Security Tier?}
    B -->|None| C[Native Process Runner]
    B -->|Tier1+| D[Sandbox Orchestrator]

    C --> E[Direct Process Execution]
    E --> F[Host System]

    D --> G[Docker/gVisor/Firecracker]
    G --> H[Isolated Environment]
```

## 配置

### 选项 1：TOML 配置

```toml
# config.toml

[security]
# Allow native execution (default: false)
allow_native_execution = true
# Default sandbox tier
default_sandbox_tier = "None"  # or "Tier1", "Tier2", "Tier3"

[security.native_execution]
# Apply resource limits even in native mode
enforce_resource_limits = true
# Maximum memory in MB
max_memory_mb = 2048
# Maximum CPU cores
max_cpu_cores = 4.0
# Maximum execution time in seconds
max_execution_time_seconds = 300
# Working directory for native execution
working_directory = "/tmp/symbiont-native"
# Allowed commands/executables
allowed_executables = ["python3", "node", "bash"]
```

### 完整配置示例

包含原生执行及其他系统设置的完整 `config.toml`：

```toml
# config.toml
[api]
port = 8080
host = "127.0.0.1"
timeout_seconds = 30
max_body_size = 10485760

[database]
# Default: LanceDB embedded (zero-config, no external services needed)
vector_backend = "lancedb"
vector_data_path = "./data/vector_db"
vector_dimension = 384

# Optional: Qdrant (uncomment to use Qdrant instead of LanceDB)
# vector_backend = "qdrant"
# qdrant_url = "http://localhost:6333"
# qdrant_collection = "symbiont"

[logging]
level = "info"
format = "Pretty"
structured = true

[security]
key_provider = { Environment = { var_name = "SYMBIONT_KEY" } }
enable_compression = true
enable_backups = true
enable_safety_checks = true

[storage]
context_path = "./data/context"
git_clone_path = "./data/git"
backup_path = "./data/backups"
max_context_size_mb = 1024

[native_execution]
enabled = true
default_executable = "python3"
working_directory = "/tmp/symbiont-native"
enforce_resource_limits = true
max_memory_mb = 2048
max_cpu_seconds = 300
max_execution_time_seconds = 300
allowed_executables = ["python3", "python", "node", "bash", "sh"]
```

### NativeExecutionConfig 字段

| 字段 | 类型 | 默认值 | 描述 |
|------|------|--------|------|
| `enabled` | bool | `false` | 启用原生执行模式 |
| `default_executable` | string | `"bash"` | 默认解释器/Shell |
| `working_directory` | path | `/tmp/symbiont-native` | 执行目录 |
| `enforce_resource_limits` | bool | `true` | 应用操作系统级别限制 |
| `max_memory_mb` | Option<u64> | `Some(2048)` | 内存限制（MB） |
| `max_cpu_seconds` | Option<u64> | `Some(300)` | CPU 时间限制 |
| `max_execution_time_seconds` | u64 | `300` | 挂钟超时 |
| `allowed_executables` | Vec<String> | `[bash, python3, etc.]` | 可执行文件白名单 |

### 选项 2：环境变量

```bash
export SYMBIONT_ALLOW_NATIVE_EXECUTION=true
export SYMBIONT_DEFAULT_SANDBOX_TIER=None
export SYMBIONT_NATIVE_MAX_MEMORY_MB=2048
export SYMBIONT_NATIVE_MAX_CPU_CORES=4.0
export SYMBIONT_NATIVE_WORKING_DIR=/tmp/symbiont-native
```

### 选项 3：智能体级别配置

```symbi
agent NativeWorker {
  metadata {
    name: "Local Development Agent"
    version: "1.0.0"
  }

  security {
    tier: None
    sandbox: Permissive
    capabilities: ["local_filesystem", "network"]
  }

  on trigger "local_processing" {
    // Executes directly on host
    execute_native("python3 process.py")
  }
}
```

## 使用示例

### 示例 1：开发模式

```rust
use symbi_runtime::{Config, SecurityTier, SandboxOrchestrator};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Enable native execution for development
    let mut config = Config::default();
    config.security.allow_native_execution = true;
    config.security.default_sandbox_tier = SecurityTier::None;

    let orchestrator = SandboxOrchestrator::new(config)?;

    // Execute code natively
    let result = orchestrator.execute_code(
        SecurityTier::None,
        "print('Hello from native execution!')",
        HashMap::new()
    ).await?;

    println!("Output: {}", result.stdout);
    Ok(())
}
```

### 示例 2：CLI 标志

```bash
# Run with native execution
symbiont run agent.dsl --native

# Or with explicit tier
symbiont run agent.dsl --sandbox-tier=none

# With resource limits
symbiont run agent.dsl --native \
  --max-memory=1024 \
  --max-cpu=2.0 \
  --timeout=300
```

### 示例 3：混合执行

```rust
// Use native execution for trusted local operations
let local_result = orchestrator.execute_code(
    SecurityTier::None,
    local_code,
    env_vars
).await?;

// Use Docker for external/untrusted operations
let isolated_result = orchestrator.execute_code(
    SecurityTier::Tier1,
    untrusted_code,
    env_vars
).await?;
```

## 实现细节

### 原生进程运行器

原生运行器使用 `std::process::Command` 并带有可选的资源限制：

```rust
pub struct NativeRunner {
    config: NativeConfig,
}

impl NativeRunner {
    pub async fn execute(&self, code: &str, env: HashMap<String, String>)
        -> Result<ExecutionResult> {
        // Direct process execution
        let mut command = Command::new(&self.config.executable);
        command.current_dir(&self.config.working_dir);
        command.envs(env);

        // Optional: Apply resource limits via rlimit (Unix)
        #[cfg(unix)]
        if self.config.enforce_limits {
            self.apply_resource_limits(&mut command)?;
        }

        let output = command.output().await?;

        Ok(ExecutionResult {
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            exit_code: output.status.code().unwrap_or(-1),
            success: output.status.success(),
        })
    }
}
```

### 资源限制（Unix）

在 Unix 系统上，原生执行仍然可以执行一些限制：

- **内存**：使用 `setrlimit(RLIMIT_AS)`
- **CPU 时间**：使用 `setrlimit(RLIMIT_CPU)`
- **进程数**：使用 `setrlimit(RLIMIT_NPROC)`
- **文件大小**：使用 `setrlimit(RLIMIT_FSIZE)`

### 平台支持

| 平台 | 原生执行 | 资源限制 |
|------|----------|----------|
| Linux | 完全支持 | rlimit |
| macOS | 完全支持 | 部分支持 |
| Windows | 完全支持 | 有限 |

## 从 Docker 迁移

### 步骤 1：更新配置

```diff
# config.toml
[security]
- default_sandbox_tier = "Tier1"
+ default_sandbox_tier = "None"
+ allow_native_execution = true
```

### 步骤 2：移除 Docker 依赖

```bash
# No longer required
# docker build -t symbi:latest .
# docker run ...

# Direct execution
cargo build --release
./target/release/symbiont run agent.dsl
```

### 混合方案

策略性地使用两种执行模式——原生用于受信任的本地操作，Docker 用于不受信任的代码：

```rust
// Trusted local operations
let local_result = orchestrator.execute_code(
    SecurityTier::None,  // Native
    trusted_code,
    env
).await?;

// External/untrusted operations
let isolated_result = orchestrator.execute_code(
    SecurityTier::Tier1,  // Docker
    external_code,
    env
).await?;
```

### 步骤 3：处理环境变量

Docker 自动隔离环境变量。使用原生执行时，需要显式设置它们：

```bash
export AGENT_API_KEY="xxx"
export AGENT_DB_URL="postgresql://..."
symbiont run agent.dsl --native
```

## 性能对比

| 模式 | 启动时间 | 吞吐量 | 内存 | 隔离性 |
|------|----------|--------|------|--------|
| 原生 | ~10ms | 100% | 最小 | 无 |
| Docker | ~500ms | ~95% | +128MB | 良好 |
| gVisor | ~800ms | ~70% | +256MB | 更好 |
| Firecracker | ~125ms | ~90% | +64MB | 最佳 |

## 故障排除

### 问题：权限被拒绝

```bash
# Solution: Ensure working directory is writable
mkdir -p /tmp/symbiont-native
chmod 755 /tmp/symbiont-native
```

### 问题：命令未找到

```bash
# Solution: Ensure executable is in PATH or use absolute path
export PATH=$PATH:/usr/local/bin
# Or configure absolute path
allowed_executables = ["/usr/bin/python3", "/usr/bin/node"]
```

### 问题：资源限制未应用

Windows 上的原生执行资源限制支持有限。考虑：
- 使用 Job Objects（Windows 特有）
- 监控和终止失控进程
- 升级到基于容器的执行

## 最佳实践

1. **仅用于开发**：主要在开发时使用原生执行
2. **渐进迁移**：先使用容器，稳定后再切换到原生
3. **监控**：即使没有隔离，也要监控资源使用
4. **白名单**：限制允许的可执行文件和路径
5. **日志记录**：启用全面的审计日志
6. **测试**：在部署原生模式之前先用容器测试

## 安全检查清单

在任何环境中启用原生执行之前：

- [ ] 所有智能体代码来自受信任的来源
- [ ] 环境与生产环境隔离
- [ ] 不处理外部用户输入
- [ ] 监控和日志已启用
- [ ] 资源限制已配置
- [ ] 可执行文件白名单限制严格
- [ ] 文件系统访问受限
- [ ] 团队了解安全影响

## 相关文档

- [安全模型](security-model.md) - 完整的安全架构
- [沙箱架构](runtime-architecture.md#sandbox-architecture) - 容器层级
- [配置指南](getting-started.md#configuration) - 设置选项
- [DSL 安全指令](dsl-guide.md#security) - 智能体级别的安全

---

**请记住**：原生执行以安全换取便利。始终了解风险，并为您的部署环境应用适当的控制措施。
