---
layout: default
title: 入门指南
description: "Symbiont 快速入门指南"
nav_exclude: true
---

# 入门指南
{: .no_toc }

## 🌐 其他语言
{: .no_toc}

[English](getting-started.md) | **中文简体** | [Español](getting-started.es.md) | [Português](getting-started.pt.md) | [日本語](getting-started.ja.md) | [Deutsch](getting-started.de.md)

---

本指南将指导您设置 Symbi 并创建您的第一个 AI 智能体。
{: .fs-6 .fw-300 }

## 目录
{: .no_toc .text-delta }

1. TOC
{:toc}

---

## 前置要求

在开始使用 Symbi 之前，请确保您已安装以下组件：

### 必需的依赖项

- **Docker**（用于容器化开发）
- **Rust 1.88+**（如果本地构建）
- **Git**（用于克隆仓库）

### 可选的依赖项

- **SchemaPin Go CLI**（用于工具验证）

> **注意：** 向量搜索已内置。Symbi 自带 [LanceDB](https://lancedb.com/) 作为嵌入式向量数据库——无需外部服务。

---

## 安装

### 选项 1：Docker（推荐）

开始使用的最快方法是使用 Docker：

```bash
# 克隆仓库
git clone https://github.com/thirdkeyai/symbiont.git
cd symbiont

# 构建统一的 symbi 容器
docker build -t symbi:latest .

# 或使用预构建的容器
docker pull ghcr.io/thirdkeyai/symbi:latest

# 运行开发环境
docker run --rm -it -v $(pwd):/workspace symbi:latest bash
```

### 选项 2：本地安装

用于本地开发：

```bash
# 克隆仓库
git clone https://github.com/thirdkeyai/symbiont.git
cd symbiont

# 安装 Rust 依赖项并构建
cargo build --release

# 运行测试以验证安装
cargo test
```

### 验证安装

测试一切是否正常工作：

```bash
# 测试 DSL 解析器
cd crates/dsl && cargo run && cargo test

# 测试运行时系统
cd ../runtime && cargo test

# 运行示例智能体
cargo run --example basic_agent
cargo run --example full_system

# 测试统一的 symbi CLI
cd ../.. && cargo run -- dsl --help
cargo run -- mcp --help

# 使用 Docker 容器进行测试
docker run --rm symbi:latest --version
docker run --rm -v $(pwd):/workspace symbi:latest dsl parse --help
docker run --rm symbi:latest mcp --help
```

---

## 您的第一个智能体

让我们创建一个简单的数据分析智能体来了解 Symbi 的基础知识。

### 1. 创建智能体定义

创建一个新文件 `my_agent.dsl`：

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

### 2. 运行智能体

```bash
# 解析并验证智能体定义
cargo run -- dsl parse my_agent.dsl

# 在运行时中运行智能体
cd crates/runtime && cargo run --example basic_agent -- --agent ../../my_agent.dsl
```

---

## 理解 DSL

Symbi DSL 有几个关键组件：

### 元数据块

```rust
metadata {
    version = "1.0.0"
    author = "developer"
    description = "Agent description"
}
```

为您的智能体提供运行时管理和文档的基本信息。

### 智能体定义

```rust
agent agent_name(parameter: Type) -> ReturnType {
    capabilities = ["capability1", "capability2"]
    // 智能体实现
}
```

定义智能体的接口、能力和行为。

### 策略定义

```rust
policy policy_name {
    allow: action_list if condition
    deny: action_list if condition
    audit: operation_type with audit_method
}
```

在运行时强制执行的声明性安全策略。

### 执行上下文

```rust
with memory = "persistent", privacy = "high" {
    // 智能体实现
}
```

指定内存管理和隐私要求的运行时配置。

---

## 下一步

### 探索示例

仓库包含几个示例智能体：

```bash
# 基本智能体示例
cd crates/runtime && cargo run --example basic_agent

# 完整系统演示
cd crates/runtime && cargo run --example full_system

# 上下文和记忆示例
cd crates/runtime && cargo run --example context_example

# RAG 增强智能体
cd crates/runtime && cargo run --example rag_example
```

### 启用高级功能

#### HTTP API（可选）

```bash
# 启用 HTTP API 功能
cd crates/runtime && cargo build --features http-api

# 使用 API 端点运行
cd crates/runtime && cargo run --features http-api --example full_system
```

**主要 API 端点：**
- `GET /api/v1/health` - 健康检查和系统状态
- `GET /api/v1/agents` - 列出所有活跃智能体及其实时执行状态
- `GET /api/v1/agents/{id}/status` - 获取智能体的详细执行指标
- `POST /api/v1/workflows/execute` - 执行工作流

**新的智能体管理功能：**
- 实时进程监控和健康检查
- 运行中智能体的优雅关闭功能
- 全面的执行指标和资源使用跟踪
- 支持多种执行模式（临时、持久、定时、事件驱动）

#### 云端 LLM 推理

通过 OpenRouter 连接到云端 LLM 提供商：

```bash
# 启用云端推理
cargo build --features cloud-llm

# 设置 API 密钥和模型
export OPENROUTER_API_KEY="sk-or-..."
export OPENROUTER_MODEL="google/gemini-2.0-flash-001"  # 可选
```

#### 独立智能体模式

一行命令启动云原生智能体，支持 LLM 推理和 Composio 工具访问：

```bash
cargo build --features standalone-agent
# 启用：cloud-llm + composio
```

#### 高级推理原语

启用工具筛选、卡住循环检测、上下文预获取和范围约定：

```bash
cargo build --features orga-adaptive
```

请参阅 [orga-adaptive 指南](/orga-adaptive) 获取完整文档。

#### Cedar 策略引擎

使用 Cedar 策略语言进行正式授权：

```bash
cargo build --features cedar
```

#### 向量数据库（内置）

Symbi 包含 LanceDB 作为零配置嵌入式向量数据库。语义搜索和 RAG 开箱即用——无需启动额外服务：

```bash
# 运行具有 RAG 功能的智能体（向量搜索直接可用）
cd crates/runtime && cargo run --example rag_example

# 使用高级搜索测试上下文管理
cd crates/runtime && cargo run --example context_example
```

> **企业选项：** 对于需要专用向量数据库的团队，Qdrant 可作为可选的特性门控后端使用。设置 `SYMBIONT_VECTOR_BACKEND=qdrant` 和 `QDRANT_URL` 即可启用。

**上下文管理功能：**
- **多模式搜索**：关键词、时间、相似度和混合搜索模式
- **重要性计算**：考虑访问模式、时效性和用户反馈的高级评分算法
- **访问控制**：集成策略引擎的智能体范围访问控制
- **自动归档**：带有压缩存储和清理的保留策略
- **知识共享**：带有信任评分的安全跨智能体知识共享

#### 特性标志参考

| 特性 | 描述 | 默认 |
|------|------|------|
| `keychain` | 操作系统钥匙串集成，用于密钥管理 | 是 |
| `vector-lancedb` | LanceDB 嵌入式向量后端 | 是 |
| `vector-qdrant` | Qdrant 分布式向量后端 | 否 |
| `embedding-models` | 通过 Candle 的本地嵌入模型 | 否 |
| `http-api` | REST API，带 Swagger UI | 否 |
| `http-input` | Webhook 服务器，带 JWT 身份验证 | 否 |
| `cloud-llm` | 云端 LLM 推理（OpenRouter） | 否 |
| `composio` | Composio MCP 工具集成 | 否 |
| `standalone-agent` | 云端 LLM + Composio 组合 | 否 |
| `cedar` | Cedar 策略引擎 | 否 |
| `orga-adaptive` | 高级推理原语 | 否 |
| `cron` | 持久化 cron 调度 | 否 |
| `native-sandbox` | 原生进程沙箱 | 否 |
| `metrics` | OpenTelemetry 指标/追踪 | 否 |
| `full` | 所有特性（企业版除外） | 否 |

```bash
# 使用特定特性构建
cargo build --features "cloud-llm,orga-adaptive,cedar"

# 使用所有特性构建
cargo build --features full
```

---

## 配置

### 环境变量

设置您的环境以获得最佳性能：

```bash
# 基本配置
export SYMBI_LOG_LEVEL=info
export SYMBI_RUNTIME_MODE=development

# 向量搜索通过内置的 LanceDB 后端开箱即用。
# 如需改用 Qdrant（可选，企业版）：
# export SYMBIONT_VECTOR_BACKEND=qdrant
# export QDRANT_URL=http://localhost:6333

# MCP 集成（可选）
export MCP_SERVER_URLS="http://localhost:8080"
```

### 运行时配置

创建一个 `symbi.toml` 配置文件：

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
backend = "lancedb"              # 默认值；也支持 "qdrant"
collection_name = "symbi_knowledge"
# url = "http://localhost:6333"  # 仅在 backend = "qdrant" 时需要
```

---

## 常见问题

### Docker 问题

**问题**：Docker 构建因权限错误而失败
```bash
# 解决方案：确保 Docker 守护进程正在运行且用户有权限
sudo systemctl start docker
sudo usermod -aG docker $USER
```

**问题**：容器立即退出
```bash
# 解决方案：检查 Docker 日志
docker logs <container_id>
```

### Rust 构建问题

**问题**：Cargo 构建因依赖项错误而失败
```bash
# 解决方案：更新 Rust 并清理构建缓存
rustup update
cargo clean
cargo build
```

**问题**：缺少系统依赖项
```bash
# Ubuntu/Debian
sudo apt-get update
sudo apt-get install build-essential pkg-config libssl-dev

# macOS
brew install pkg-config openssl
```

### 运行时问题

**问题**：智能体启动失败
```bash
# 检查智能体定义语法
cargo run -- dsl parse your_agent.dsl

# 启用调试日志记录
RUST_LOG=debug cd crates/runtime && cargo run --example basic_agent
```

---

## 获取帮助

### 文档

- **[DSL 指南](/dsl-guide)** - 完整的 DSL 参考
- **[运行时架构](/runtime-architecture)** - 系统架构详情
- **[安全模型](/security-model)** - 安全和策略文档

### 社区支持

- **问题**：[GitHub Issues](https://github.com/thirdkeyai/symbiont/issues)
- **讨论**：[GitHub Discussions](https://github.com/thirdkeyai/symbiont/discussions)
- **文档**：[完整 API 参考](https://docs.symbiont.dev/api-reference)

### 调试模式

用于故障排除，启用详细日志记录：

```bash
# 启用调试日志记录
export RUST_LOG=symbi=debug

# 使用详细输出运行
cd crates/runtime && cargo run --example basic_agent 2>&1 | tee debug.log
```

---

## 下一步是什么？

现在您已经运行了 Symbi，请探索这些高级主题：

1. **[DSL 指南](/dsl-guide)** - 学习高级 DSL 功能
2. **[推理循环指南](/reasoning-loop)** - 了解 ORGA 循环
3. **[高级推理（orga-adaptive）](/orga-adaptive)** - 工具筛选、卡住循环检测、预水化
4. **[运行时架构](/runtime-architecture)** - 了解系统内部结构
5. **[安全模型](/security-model)** - 实施安全策略
6. **[贡献](/contributing)** - 为项目做出贡献

准备好构建令人惊叹的东西了吗？从我们的[示例项目](https://github.com/thirdkeyai/symbiont/tree/main/crates/runtime/examples)开始，或深入了解[完整规范](/specification)。
