---
layout: default
title: 入门指南
nav_order: 2
description: "Symbiont 快速入门指南"
---

# 入门指南
{: .no_toc }

## 🌐 其他语言

[English](getting-started.md) | **中文简体** | [Español](getting-started.es.md) | [Português](getting-started.pt.md) | [日本語](getting-started.ja.md) | [Deutsch](getting-started.de.md)

---

本指南将指导您设置 Symbi 并创建您的第一个 AI 代理。
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

- **Qdrant** 向量数据库（用于语义搜索功能）
- **SchemaPin Go CLI**（用于工具验证）

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

# 运行示例代理
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

## 您的第一个代理

让我们创建一个简单的数据分析代理来了解 Symbi 的基础知识。

### 1. 创建代理定义

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

### 2. 运行代理

```bash
# 解析并验证代理定义
cargo run -- dsl parse my_agent.dsl

# 在运行时中运行代理
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

为您的代理提供运行时管理和文档的基本信息。

### 代理定义

```rust
agent agent_name(parameter: Type) -> ReturnType {
    capabilities = ["capability1", "capability2"]
    // 代理实现
}
```

定义代理的接口、功能和行为。

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
    // 代理实现
}
```

指定内存管理和隐私要求的运行时配置。

---

## 下一步

### 探索示例

仓库包含几个示例代理：

```bash
# 基本代理示例
cd crates/runtime && cargo run --example basic_agent

# 完整系统演示
cd crates/runtime && cargo run --example full_system

# 上下文和内存示例
cd crates/runtime && cargo run --example context_example

# RAG 增强代理
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
- `GET /api/v1/agents` - 列出所有活跃代理
- `POST /api/v1/workflows/execute` - 执行工作流

#### 向量数据库集成

用于语义搜索功能：

```bash
# 启动 Qdrant 向量数据库
docker run -p 6333:6333 qdrant/qdrant

# 运行具有 RAG 功能的代理
cd crates/runtime && cargo run --example rag_example
```

---

## 配置

### 环境变量

设置您的环境以获得最佳性能：

```bash
# 基本配置
export SYMBI_LOG_LEVEL=info
export SYMBI_RUNTIME_MODE=development

# 向量数据库（可选）
export QDRANT_URL=http://localhost:6333

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
url = "http://localhost:6333"
collection_name = "symbi_knowledge"
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

**问题**：代理启动失败
```bash
# 检查代理定义语法
cargo run -- dsl parse your_agent.dsl

# 启用调试日志记录
RUST_LOG=debug cd crates/runtime && cargo run --example basic_agent
```

---

## 获取帮助

### 文档

- **[DSL 指南](/dsl-guide)** - 完整的 DSL 参考
- **[运行时架构](/runtime-architecture)** - 系统架构详细信息
- **[安全模型](/security-model)** - 安全和策略文档

### 社区支持

- **问题**：[GitHub Issues](https://github.com/thirdkeyai/symbiont/issues)
- **讨论**：[GitHub Discussions](https://github.com/thirdkeyai/symbiont/discussions)
- **文档**：[完整 API 参考](https://docs.symbiont.platform)

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
2. **[运行时架构](/runtime-architecture)** - 了解系统内部结构
3. **[安全模型](/security-model)** - 实施安全策略
4. **[贡献](/contributing)** - 为项目做出贡献

准备好构建令人惊叹的东西了吗？从我们的[示例项目](https://github.com/thirdkeyai/symbiont/tree/main/crates/runtime/examples)开始，或深入了解[完整规范](/specification)。