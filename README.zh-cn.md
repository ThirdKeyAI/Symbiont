<img src="logo-hz.png" alt="Symbi">

[English](README.md) | **中文简体** | [Español](README.es.md) | [Português](README.pt.md) | [日本語](README.ja.md) | [Deutsch](README.de.md)

[![Build](https://img.shields.io/github/actions/workflow/status/thirdkeyai/symbiont/docker-build.yml?branch=main)](https://github.com/thirdkeyai/symbiont/actions)
[![Crates.io](https://img.shields.io/crates/v/symbi)](https://crates.io/crates/symbi)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)
[![Docs](https://img.shields.io/badge/docs-online-brightgreen)](https://docs.symbiont.dev)

---

**面向生产环境的策略治理智能体运行时。**

Symbiont 是一个 Rust 原生运行时，用于在显式策略、身份和审计控制下执行 AI 智能体、工具和工作流。

大多数智能体框架侧重于编排。Symbiont 侧重于智能体在真实环境中运行时面临的真实风险：不可信工具、敏感数据、审批边界、审计要求和可重复的执行控制。

---

## 为什么选择 Symbiont

AI 智能体易于演示，却难以信任。

一旦智能体可以调用工具、访问文件、发送消息或调用外部服务，你需要的不仅仅是提示词和胶水代码。你需要：

* **策略执行** 控制智能体可以做什么 — 内置 DSL 和 [Cedar](https://www.cedarpolicy.com/) 授权
* **工具验证** 使执行不再是盲目信任 — [SchemaPin](https://github.com/ThirdKeyAI/SchemaPin) 对 MCP 工具的密码学验证
* **智能体身份** 使你了解谁在执行操作 — [AgentPin](https://github.com/ThirdKeyAI/AgentPin) 域锚定 ES256 身份
* **沙箱隔离** 用于高风险工作负载 — 带资源限制的 Docker 隔离
* **审计追踪** 记录发生了什么以及原因 — 密码学防篡改日志
* **审批工作流** 用于需要批准的操作 — 推理循环中的人机协作审批门

Symbiont 正是为这一层而构建的。

---

## 快速开始

### 前提条件

* Docker（推荐）或 Rust 1.82+
* 无需外部向量数据库（LanceDB 内嵌；Qdrant 可作为大规模部署的可选后端）

### 使用 Docker 运行

```bash
# 启动运行时（API 端口 :8080，HTTP 输入端口 :8081）
docker run --rm -p 8080:8080 -p 8081:8081 ghcr.io/thirdkeyai/symbi:latest up

# 仅运行 MCP 服务器
docker run --rm -p 8080:8080 ghcr.io/thirdkeyai/symbi:latest mcp

# 解析智能体 DSL 文件
docker run --rm -v $(pwd):/workspace ghcr.io/thirdkeyai/symbi:latest dsl parse /workspace/agent.dsl
```

### 从源代码构建

```bash
cargo build --release
./target/release/symbi --help

# 运行运行时
cargo run -- up

# 交互式 REPL
cargo run -- repl
```

> 对于生产部署，请在启用不可信工具执行之前查阅 `SECURITY.md` 和[部署指南](https://docs.symbiont.dev/getting-started)。

---

## 工作原理

Symbiont 将智能体意图与执行权限分离：

1. **智能体提出**操作请求，通过 ORGA 推理循环（Observe-Reason-Gate-Act）
2. **运行时评估**每个操作的策略、身份和信任检查
3. **策略决定** — 允许的操作被执行；拒绝的操作被阻止或路由到审批流程
4. **一切皆被记录** — 每个决策都有防篡改审计追踪

这意味着模型输出永远不会被视为执行权限。运行时控制实际发生的操作。

### 示例：不可信工具被策略阻止

智能体尝试调用一个未验证的 MCP 工具。运行时：

1. 检查 SchemaPin 验证状态 — 工具签名缺失或无效
2. 评估 Cedar 策略 — `forbid(action == Action::"tool_call") when { !resource.verified }`
3. 阻止执行并记录拒绝详情的完整上下文
4. 可选地路由给操作员进行人工审批

无需更改代码。策略治理执行。

---

## DSL 示例

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

## 核心能力

| 能力 | 说明 |
|-----------|-------------|
| **Cedar 策略引擎** | 对智能体操作、工具调用和资源访问的细粒度授权 |
| **SchemaPin 验证** | 执行前对 MCP 工具 schema 的密码学验证 |
| **AgentPin 身份** | 面向智能体和计划任务的域锚定 ES256 身份 |
| **ORGA 推理循环** | 类型状态强制的 Observe-Reason-Gate-Act 循环，带策略门和熔断器 |
| **沙箱隔离** | 基于 Docker 的隔离，带资源限制，用于不可信工作负载 |
| **审计日志** | 防篡改日志，为每个策略决策提供结构化记录 |
| **ClawHavoc 扫描** | 跨 10 个攻击类别的 40 条规则，用于 skill/tool 内容分析 |
| **密钥管理** | Vault/OpenBao 集成，AES-256-GCM 加密存储，按智能体命名空间隔离 |
| **Cron 调度** | 基于 SQLite 的调度器，支持抖动、并发保护和死信队列 |
| **持久化记忆** | 基于 Markdown 的智能体记忆，支持事实提取、流程记录和压缩 |
| **RAG 引擎** | 通过 LanceDB（内嵌）或 Qdrant（大规模部署）实现混合语义 + 关键词搜索 |
| **MCP 集成** | 原生 Model Context Protocol 支持，带治理工具访问 |
| **Webhook 验证** | HMAC-SHA256 和 JWT 验证，预置 GitHub、Stripe 和 Slack 支持 |
| **交付路由** | 将智能体输出路由到 webhook、Slack、邮件或自定义通道 |
| **指标与遥测** | 通过 OpenTelemetry 追踪 span 进行 OTLP 导出，覆盖推理循环 |
| **HTTP 安全** | 仅本地回环绑定、CORS 白名单、JWT EdDSA 验证、按智能体 API 密钥 |
| **AI 助手插件** | 面向 [Claude Code](https://github.com/thirdkeyai/symbi-claude-code) 和 [Gemini CLI](https://github.com/thirdkeyai/symbi-gemini-cli) 的治理插件 |

性能：策略评估 <1ms，ECDSA P-256 验证 <5ms，10k 智能体调度 CPU 开销 <2%。参见 [benchmarks](crates/runtime/benches/performance_claims.rs) 和[阈值测试](crates/runtime/tests/performance_claims.rs)。

---

## 安全模型

Symbiont 围绕一个简单原则设计：**模型输出永远不应被信任为执行权限。**

操作通过运行时控制流转：

* **零信任** — 所有智能体输入默认不可信
* **策略检查** — 每次工具调用和资源访问前进行 Cedar 授权
* **工具验证** — SchemaPin 对工具 schema 的密码学验证
* **沙箱边界** — 基于 Docker 的不可信执行隔离
* **操作员审批** — 敏感操作的人机协作审批门
* **密钥控制** — Vault/OpenBao 后端、加密本地存储、智能体命名空间
* **审计日志** — 每个决策的密码学防篡改记录

如果你正在执行不可信代码或高风险工具，请不要仅依赖弱本地执行模型作为唯一屏障。参见 [`SECURITY.md`](SECURITY.md) 和[安全模型文档](https://docs.symbiont.dev/security-model)。

---

## 工作区

| Crate | 说明 |
|-------|-------------|
| `symbi` | 统一 CLI 二进制文件 |
| `symbi-runtime` | 核心智能体运行时与执行引擎 |
| `symbi-dsl` | DSL 解析器与求值器 |
| `symbi-channel-adapter` | Slack/Teams/Mattermost 适配器 |
| `repl-core` / `repl-proto` / `repl-cli` | 交互式 REPL 和 JSON-RPC 服务器 |
| `repl-lsp` | Language Server Protocol 支持 |
| `symbi-a2ui` | 管理仪表板（Lit/TypeScript，alpha 阶段） |

治理插件：[`symbi-claude-code`](https://github.com/thirdkeyai/symbi-claude-code) | [`symbi-gemini-cli`](https://github.com/thirdkeyai/symbi-gemini-cli)

---

## 文档

* [入门指南](https://docs.symbiont.dev/getting-started)
* [安全模型](https://docs.symbiont.dev/security-model)
* [运行时架构](https://docs.symbiont.dev/runtime-architecture)
* [推理循环指南](https://docs.symbiont.dev/reasoning-loop)
* [DSL 指南](https://docs.symbiont.dev/dsl-guide)
* [API 参考](https://docs.symbiont.dev/api-reference)
* [高级推理原语](https://docs.symbiont.dev/orga-adaptive)

如果你正在评估 Symbiont 用于生产环境，请从安全模型和入门指南文档开始。

---

## 许可证

* **社区版**（Apache 2.0）：核心运行时、DSL、ORGA 推理循环、Cedar 策略引擎、SchemaPin/AgentPin 验证、Docker 沙箱、持久化记忆、Cron 调度、MCP 集成、RAG（LanceDB）、审计日志、Webhook 验证、ClawHavoc skill 扫描，以及所有 CLI/REPL 工具。
* **企业版**（商业许可证）：多层沙箱（gVisor、Firecracker、E2B）、带合规导出的密码学审计追踪（HIPAA、SOX、PCI-DSS）、AI 驱动的工具审查与威胁检测、加密多智能体协作、实时监控仪表板和专属支持。

联系 [ThirdKey](https://thirdkey.ai) 获取企业许可。

---

*同一个智能体。安全的运行时。*

<div align="right">
  <img src="symbi-trans.png" alt="Symbi 标志" width="120">
</div>
