<img src="logo-hz.png" alt="Symbi">

[English](README.md) | **中文简体** | [Español](README.es.md) | [Português](README.pt.md) | [日本語](README.ja.md) | [Deutsch](README.de.md)

[![Build](https://img.shields.io/github/actions/workflow/status/thirdkeyai/symbiont/docker-build.yml?branch=main)](https://github.com/thirdkeyai/symbiont/actions)
[![Crates.io](https://img.shields.io/crates/v/symbi)](https://crates.io/crates/symbi)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Docs](https://img.shields.io/badge/docs-online-brightgreen)](https://docs.symbiont.dev)

---

## 🚀 什么是 Symbiont？

**Symbi** 是一个 **Rust 原生、零信任智能体框架**，用于构建自主的、策略感知的 AI 智能体。
它通过专注于以下方面解决了 LangChain 和 AutoGPT 等现有框架的最大缺陷：

* **安全优先**：密码学审计追踪、强制策略和沙箱。
* **零信任**：默认情况下所有输入都被视为不可信。
* **企业级合规**：专为受监管行业（HIPAA、SOC2、金融）设计。

Symbiont 智能体与人类、工具和 LLM 安全协作 — 不牺牲安全性或性能。

---

## ⚡ 为什么选择 Symbiont？

| 特性         | Symbiont                      | LangChain    | AutoGPT   |
| ------------ | ----------------------------- | ------------ | --------- |
| 语言         | Rust（安全、性能）            | Python       | Python    |
| 安全性       | 零信任、密码学审计            | 最少         | 无        |
| 策略引擎     | 内置 DSL                      | 有限         | 无        |
| 部署         | REPL、Docker、HTTP API        | Python 脚本  | CLI 技巧  |
| 审计追踪     | 密码学日志                    | 否           | 否        |

---

## 🏁 快速开始

### 前提条件

* Docker（推荐）或 Rust 1.88+
* 无需外部向量数据库（LanceDB 内嵌；Qdrant 可作为大规模部署的可选后端）

### 使用预构建容器运行

```bash
# 解析智能体 DSL 文件
docker run --rm -v $(pwd):/workspace ghcr.io/thirdkeyai/symbi:latest dsl parse /workspace/agent.dsl

# 运行 MCP 服务器
docker run --rm -p 8080:8080 ghcr.io/thirdkeyai/symbi:latest mcp

# 交互式开发 shell
docker run --rm -it -v $(pwd):/workspace ghcr.io/thirdkeyai/symbi:latest bash
```

### 从源代码构建

```bash
# 构建开发环境
docker build -t symbi:latest .
docker run --rm -it -v $(pwd):/workspace symbi:latest bash

# 构建统一二进制文件
cargo build --release

# 运行 REPL
cargo run -- repl

# 解析 DSL 并运行 MCP
cargo run -- dsl parse my_agent.dsl
cargo run -- mcp --port 8080
```

---

## 🔧 主要特性

* ✅ **DSL 语法** – 使用内置安全策略声明式定义智能体。
* ✅ **智能体运行时** – 任务调度、资源管理和生命周期控制。
* 🔒 **沙箱隔离** – 用于智能体执行的 Tier-1 Docker 隔离。
* 🔒 **SchemaPin 安全** – 工具和模式的密码学验证。
* 🔒 **密钥管理** – HashiCorp Vault / OpenBao 集成，AES-256-GCM 加密存储。
* 📊 **RAG 引擎** – 向量搜索（LanceDB 内嵌）与混合语义 + 关键词检索。可选 Qdrant 后端用于大规模部署。
* 🧩 **MCP 集成** – 对模型上下文协议工具的原生支持。
* 📡 **可选 HTTP API** – 用于外部集成的功能门控 REST 接口。

---

## 📐 Symbiont DSL 示例

```symbiont
metadata {
    version = "1.0.0"
    author = "Your Name"
    description = "Data analysis agent"
}

agent analyze_data(input: DataSet) -> Result {
    capabilities = ["data_analysis", "visualization"]
    
    policy data_privacy {
        allow: read(input) if input.anonymized == true
        deny: store(input) if input.contains_pii == true
        audit: all_operations
    }
    
    with memory = "persistent", requires = "approval" {
        if (llm_check_safety(input)) {
            result = analyze(input);
            return result;
        } else {
            return reject("Safety check failed");
        }
    }
}
```

---

## 🔒 安全模型

* **零信任** – 默认情况下所有智能体输入都不可信。
* **沙箱执行** – 基于 Docker 的进程隔离。
* **审计日志** – 密码学防篡改日志。
* **密钥控制** – Vault/OpenBao 后端，加密本地存储，智能体命名空间。

---

## 📚 文档

* [入门指南](https://docs.symbiont.dev/getting-started)
* [DSL 指南](https://docs.symbiont.dev/dsl-guide)
* [运行时架构](https://docs.symbiont.dev/runtime-architecture)
* [安全模型](https://docs.symbiont.dev/security-model)
* [API 参考](https://docs.symbiont.dev/api-reference)

---

## 🎯 使用场景

* **开发与自动化**

  * 安全代码生成和重构。
  * 带有强制策略的 AI 智能体部署。
  * 带有语义搜索的知识管理。

* **企业与受监管行业**

  * 医疗保健（HIPAA 合规处理）。
  * 金融（审计就绪工作流）。
  * 政府（机密上下文处理）。
  * 法律（机密文档分析）。

---

## 📄 许可证

* **社区版**：MIT 许可证
* **企业版**：需要商业许可证

联系 [ThirdKey](https://thirdkey.ai) 获取企业许可。

---

*Symbiont 通过智能策略执行、密码学验证和全面审计追踪，实现 AI 智能体与人类之间的安全协作。*


<div align="right">
  <img src="symbi-trans.png" alt="Symbi 标志" width="120">
</div>