# 入门指南

> 要在收容（containment）条件下运行智能体？请先阅读[收容运维指南](/containment-branch-guide)，了解执行前置条件、审批方面的变化以及当前的覆盖范围。

本指南将指导您设置 Symbi 并创建您的第一个 AI 智能体。

▶ **观看入门教程视频：**

[![Symbiont — get started](https://img.youtube.com/vi/RPyKpqKz5ik/hqdefault.jpg)](https://www.youtube.com/watch?v=RPyKpqKz5ik)

## 目录


---

## 前置要求

您需要什么取决于您如何安装和运行 Symbi。

### 运行预编译二进制文件

预编译二进制文件已经编译完成——安装或运行它们**无需** Rust、protobuf 或 Git。可以使用 Homebrew、安装脚本（`curl`）或从 GitHub Releases 手动下载进行安装。

- **Docker** 仅在*运行时*需要，前提是您在默认沙箱层级（`tier1`，基于 Docker）下执行智能体。安装 Symbi 或运行 `symbi init`、`symbi dsl` 或 `symbi --version` 时**不需要**它。

### 从源代码构建

仅当您通过 `cargo install` 安装或自行构建仓库时才需要：

- **Rust 1.82+**
- **protobuf-compiler**（在 Ubuntu 上使用 `apt install protobuf-compiler`，在 macOS 上使用 `brew install protobuf`）
- **Git**（用于克隆仓库）

### 可选

- **[symbi-claude-code](https://github.com/thirdkeyai/symbi-claude-code)**（Claude Code 治理插件）
- **[symbi-gemini-cli](https://github.com/thirdkeyai/symbi-gemini-cli)**（Gemini CLI 治理扩展）

> **注意：** 向量搜索已内置。Symbi 自带 [LanceDB](https://lancedb.com/) 作为嵌入式向量数据库——无需外部服务。

---

## 安装

### 选项 1：Docker（推荐）

获得可工作运行时的最快方法是让容器为您生成项目脚手架：

```bash
# 1. 生成 symbiont.toml、agents/、policies/、docker-compose.yml，以及
#    包含新生成 SYMBIONT_MASTER_KEY 的 .env。
docker run --rm -v $(pwd):/workspace ghcr.io/thirdkeyai/symbi:latest \
  init --profile assistant --no-interact --dir /workspace

# 2. 启动运行时。自动读取 .env。
docker compose up
```

运行时 API 现在位于 `http://localhost:8080`，HTTP Input 位于 `http://localhost:8081`。

如果您更愿意从克隆仓库工作（以便自行构建镜像或运行测试）：

```bash
git clone https://github.com/thirdkeyai/symbiont.git
cd symbiont

# 构建统一的 symbi 容器
docker build -t symbi:latest .

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

## 无需 API 密钥即可试用

在配置任何模型提供方之前，有两项功能可以离线运行。二者都展示了 Symbiont 的实际能力，建议从这里开始。

**定义一个工具并进行试运行。** 参数类型、作用域限制和注入检查都会在任何内容执行之前强制生效：

```bash
symbi tools init greet
symbi tools validate
symbi tools test greet --arg target=example
```

```
greet                                    OK

  ✓ target (string): example → OK

  Command:   greet example
  Cedar:     Tool::Greet / execute_tool

  [dry run — command not executed]
```

运行*智能体*需要模型提供方 —— 云端密钥或本地模型均可，下文均有说明。

## 项目初始化

启动新 Symbiont 项目的最快方式是 `symbi init`：

```bash
symbi init
```

这将启动一个交互式向导，引导您完成：
- **配置文件选择**：`minimal`、`assistant`、`dev-agent` 或 `multi-agent`
- **SchemaPin 模式**：`tofu`（首次使用信任）、`strict` 或 `disabled`
- **沙箱层级**：`landlock`（Linux 原生）、`tier0`（无，仅供开发使用）、`tier1`（Docker）、`tier2`（gVisor / `runsc`）或 `tier3`（Firecracker microVM）

使用 `--sandbox landlock --profile dev-agent` 时，向导还会询问源码仓库、已安装的
Claude Code 可执行文件、兼容 Messages 接口的推理 URL、模型，以及凭据环境变量名。
它会在一个独立的空控制目录中生成只读的代码审阅配置。非交互式调用方必须提供
`--source`、`--managed-executable`、`--inference-url`、`--inference-model` 和
`--inference-key-env`。参见 [Linux 开发者上手指南](/landlock-development)。

### `init` 生成的内容

每次运行都会写入：

| 文件 | 用途 |
|------|------|
| `symbiont.toml` | 运行时和策略配置 |
| `policies/default.cedar` | 默认拒绝的 Cedar 策略 |
| `agents/*.symbi` | 特定配置文件的智能体定义（同时也可识别旧后缀 `.dsl`；`minimal` 除外） |
| `AGENTS.md` | 自动生成的已声明智能体索引 |
| `.symbiont/audit/` | 防篡改审计日志目录 |
| `.gitignore` | 追加 Symbiont 特定条目，包括 `.env` |
| `.env` | 从 `/dev/urandom` 生成的 `SYMBIONT_MASTER_KEY`（0600 权限） |
| `.env.example` | 可安全提交的模板，展示所需的环境变量 |
| `docker-compose.yml` | 带有卷挂载和环境变量接线的 compose 文件；使用 Landlock 时不生成 |

传递 `--no-docker-compose` 可跳过 compose 文件，使用 `--dir <PATH>` 可写入当前目录之外的其他目录（在 Docker 容器内运行时必需 — 见下文）。

### 非交互模式

用于 CI/CD 或脚本化设置：

```bash
symbi init --profile assistant --schemapin tofu --sandbox tier1 --no-interact
```

### 在 Docker 内运行 `init`

由于镜像的 WORKDIR 为 `/var/lib/symbi`，请使用 `--dir` 写入您挂载的卷：

```bash
docker run --rm -v $(pwd):/workspace ghcr.io/thirdkeyai/symbi:latest \
  init --profile assistant --no-interact --dir /workspace
```

这将在主机的当前目录中填充完整的项目树。

### 配置文件

| 配置文件 | 创建的内容 |
|---------|-----------|
| `minimal` | `symbiont.toml` + 默认 Cedar 策略 |
| `assistant` | + 单个治理助手智能体 |
| `dev-agent` | + 托管 CLI 智能体；使用 Landlock 时还会加入已配置的读取/列目录/搜索工具、限定范围的策略和 `DEVELOPMENT.md` |
| `multi-agent` | + 协调器/工作器智能体及智能体间策略 |

### 从目录导入

在通用配置文件旁导入预构建的智能体（只读的 Landlock 开发初始化器不支持同时导入目录）：

```bash
symbi init --profile minimal --no-interact
symbi init --catalog assistant,dev
```

列出可用的目录智能体：

```bash
symbi init --catalog list
```

初始化完成后，验证并启动：

```bash
symbi dsl -f agents/assistant.symbi   # 验证您的智能体
symbi run assistant -i '{"query": "hello"}'  # 测试单个智能体
symbi up                             # 在本地启动运行时
docker compose up                    # ...或在 Docker 中启动（读取 .env）
```

### 运行单个智能体

使用 `symbi run` 执行单个智能体，无需启动完整运行时服务器：

```bash
symbi run <agent-name-or-file> --input <json>
```

该命令通过以下方式解析智能体名称：先搜索直接路径，然后搜索 `agents/` 目录。它从环境变量（`OPENROUTER_API_KEY`、`OPENAI_API_KEY` 或 `ANTHROPIC_API_KEY`）设置云端推理，运行 ORGA 推理循环后退出。

```bash
symbi run assistant -i 'Summarize this document'
symbi run agents/recon.symbi -i '{"target": "10.0.1.5"}' --max-iterations 5
```

工具命令、解析器、MCP 和 PTY 执行都需要所选的容器后端、一个包含所声明可执行文件
的已缓存镜像，以及显式的数据挂载。后端不可用时不能回退到主机执行。所选的智能体
设置和项目默认值会在推理之前进行检查。运行还需要受保护的 `.symbiont/governed/`
存储，并会打印其公开的审计引用。参见[命令配置](/toolclad-command-boundary)和
[运行审计](/run-audit)。

### 使用本地模型

提供方不必是云服务。将 `OPENAI_BASE_URL` 指向任意兼容 OpenAI 的服务器 —— [Ollama](https://ollama.com)、vLLM、LM Studio 和 llama.cpp 都提供此接口 —— 完全无需云端密钥：

```bash
export OPENAI_API_KEY=ollama
export OPENAI_BASE_URL=http://localhost:11434/v1
export CHAT_MODEL=llama3.1

symbi run assistant -i 'hello'
```

同样的三个变量也适用于 `symbi up`。当基础 URL 使用明文 `http://` 时 Symbiont 会发出警告，因为密钥会随请求一同发送。对于本机上的模型，这属于预期情况。

### 从模板开始（`symbi new`）

`symbi init` 用于搭建通用项目；`symbi new` 则围绕若干面向任务的模板来搭建项目。当你在还不确定需要哪些具体智能体之前，就已经知道你要的是什么类型的智能体时，它会非常有用。

```bash
symbi new --list                     # 显示可用的模板
symbi new <template> <project-name>  # 从模板创建一个新项目
```

内置模板：

| 模板 | 你将获得的内容 |
|----------|--------------|
| `webhook-min` | 最小化的 webhook 驱动智能体 —— HTTP Input 配置 + 一个处理程序 DSL |
| `webscraper-agent` | 带 Cedar 访问策略和 ToolClad 抓取工具的爬虫智能体 |
| `slm-first` | 路由器 + SLM 白名单 + 置信度回退模式 |
| `rag-lite` | 基于 Qdrant 的摄取脚本以及一个搜索智能体 |

`symbi new` 与 `symbi init` 是互补的：`new` 提供面向任务的起点，`init`（配合 `--catalog`）则提供面向治理的起点。你也可以组合使用 —— 先用 `new` 搭建骨架，再用 `symbi init --catalog ...` 从目录中引入额外的预置智能体。

---

## 您的第一个智能体

让我们创建一个简单的数据分析智能体来了解 Symbi 的基础知识。

### 1. 创建智能体定义

创建一个新文件 `my_agent.symbi`：

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
cargo run -- dsl parse my_agent.symbi

# 在运行时中运行智能体
cd crates/runtime && cargo run --example basic_agent -- --agent ../../my_agent.symbi
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

一行命令启动云原生智能体，支持 LLM 推理：

```bash
cargo build --features standalone-agent
# 启用：cloud-llm
```

> **Note:** Composio MCP and SymbiBot integration were removed in this version due to security concerns — see SECURITY_AUDIT.md C3 for context.

#### 高级推理原语

启用工具筛选、卡住循环检测、上下文预获取和范围约定：

```bash
cargo build --features orga-adaptive
```

请参阅 [orga-adaptive 指南](/orga-adaptive) 获取完整文档。

#### Cedar 策略引擎

使用 Cedar 策略语言进行正式授权。**自 v1.14.x 起默认启用**：已发布的 `symbi` 二进制文件（crates.io、Docker、GitHub Release tarball）均包含 Cedar，并且 `symbi up` / `symbi run` 会在启动时从 `policies/*.cedar` 文件自动接线 `CedarPolicyGate`；若不存在任何此类文件，运行时将回退到失败即关闭的 `DefaultPolicyGate`。如需在不包含 Cedar 的情况下构建（例如，你打算改为接入 `OpaPolicyGateBridge` 或自定义的 `ReasoningPolicyGate`），请使用：

```bash
cargo build --no-default-features --features "keychain,vector-lancedb"  # drop cedar
```

#### 向量数据库（内置）

Symbi 包含 LanceDB 作为零配置嵌入式向量数据库。语义搜索和 RAG 开箱即用——无需启动额外服务：

```bash
# 运行具有 RAG 功能的智能体（向量搜索直接可用）
cd crates/runtime && cargo run --example rag_example

# 使用高级搜索测试上下文管理
cd crates/runtime && cargo run --example context_example
```

> **最小化构建：** LanceDB 默认包含在内，但可以为更轻量的二进制文件排除它：`cargo build --no-default-features`。运行时会平稳回退到一个无操作的向量后端。
>
> **规模化部署：** Qdrant 作为可选后端提供。使用 `--features vector-qdrant` 构建并设置 `SYMBIONT_VECTOR_BACKEND=qdrant`。

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
| `standalone-agent` | 云端 LLM 元特性 | 否 |
| `cedar` | Cedar 策略引擎 — 启动时从 `policies/*.cedar` 自动接线 | **Yes** |
| `orga-adaptive` | 高级推理原语 | 否 |
| `cron` | 持久化 cron 调度 | 否 |
| `cli-executor` | 受治理的 AI CLI 子进程（Claude Code 等）—— 模式 B | 是 |
| `native-sandbox` | 原生进程沙箱 | 否 |
| `metrics` | OpenTelemetry 指标/追踪 | 否 |
| `mcp-client` | 基于 MCP、通过 stdio 的 ToolClad 工具执行（经 SchemaPin 验证） | No |
| `toolclad-browser` | 浏览器（CDP）ToolClad 后端 — 仅为占位接口，在 CDP 后端就绪前会明确返回错误 | No |
| `interactive` | `symbi init` 的交互式提示（dialoguer） | 默认 |
| `full` | 所有可选的运行时、向量和策略特性 | 否 |

```bash
# 使用特定特性构建
cargo build --features "cloud-llm,orga-adaptive,cedar"

# 使用所有特性构建
cargo build --features full
```

---

## AI 助手插件

Symbiont 为流行的 AI 编码助手提供第一方治理插件，包含三个渐进式保护层级：

1. **Awareness**（默认）—— 对所有修改状态的工具调用进行建议性日志记录
2. **Protection** —— 阻断式钩子强制执行本地拒绝列表（`.symbiont/local-policy.toml`）
3. **Governance** —— 当 `symbi` 位于 PATH 上时进行 Cedar 策略评估

拒绝列表配置与工具无关 —— 同一份 `.symbiont/local-policy.toml` 可同时用于两个插件：

```toml
[deny]
paths = [".env", ".ssh/", ".aws/"]
commands = ["rm -rf", "git push --force"]
branches = ["main", "master", "production"]
```

### Claude Code

```bash
# Install from marketplace
/plugin marketplace add https://github.com/thirdkeyai/symbi-claude-code

# Available skills: /symbi-init, /symbi-policy, /symbi-verify, /symbi-audit, /symbi-dsl
```

详情请参阅 [symbi-claude-code](https://github.com/thirdkeyai/symbi-claude-code)。

#### 模式 B：受治理的 Claude Code 子进程

声明了 `metadata { executor = "claude_code" }` 的智能体，会在所选的 Docker/gVisor
容器中运行其 CLI 子进程，只有临时暂存存储以及私有的运行时推理/工具通道。捆绑的
`code_reviewer` 是参考智能体。请先配置一个已缓存的 CLI/Python 镜像、显式的后端源码
挂载、已注册的 ToolClad 工具和 Cedar 策略，以及 `[managed_cli.inference]`。完整示例
参见[托管 CLI 收容](/managed-cli-containment)。

```bash
# /srv/source must map to an explicit backend mount in the control project.
symbi run code_reviewer --target /srv/source --max-turns 12 --budget-timeout 15m

# Add operator review for tools that require approval.
symbi run code_reviewer --target /srv/source --approval-terminal
```

子进程不会获得直接的源码挂载、外部网络访问、主机登录状态或提供方凭据。允许的
文件/Git 访问由已注册的工具代为中转。内置工具和自动发现均被禁用。每个动作都需要
运行时授权；对启动的一次批准并不授权后续动作。插件不会被加载，`--plugin-dir`
会被拒绝。

| 标志 / 设置 | 用途 |
|---|---|
| `--target` | 映射到显式后端挂载的源码目录 |
| `--max-turns` | 对话轮次上限；默认 12 |
| `--budget-timeout` | 包含初始化在内的挂钟时间上限；默认 `15m` |
| `--budget-tokens` | 预留的推理输出 token 额度；默认 100000，不是计费的总 token 数 |
| `--approval-terminal` | 选择启用控制终端审阅，用于强制性审批 |
| `[managed_cli.inference]` | 显式的提供方端点、模型和凭据变量；凭据保留在运行时内 |

必需的签名会话日志保存在私有的 `.symbiont/governed/` 存储中。运行时会打印公开
验证密钥。缺少审批、审计存储不安全、后端不可用或清理失败，都不会被悄悄报告为
成功。推理响应会被缓冲（包括 SSE），因此流式输出会有延迟。

### Gemini CLI

```bash
# Install extension
gemini extensions install https://github.com/thirdkeyai/symbi-gemini-cli
```

Gemini CLI 扩展通过 `excludeTools` 清单阻断以及平台级别的原生 `policies/*.toml` 强制执行，提供了额外的纵深防御。

详情请参阅 [symbi-gemini-cli](https://github.com/thirdkeyai/symbi-gemini-cli)。

---

## 配置

### 环境变量

设置您的环境以获得最佳性能：

```bash
# 必需：用于加密持久化状态的 32 字节十六进制密钥。
# 生成方式：openssl rand -hex 32
# `symbi init` 会自动将一个密钥写入 .env。
export SYMBIONT_MASTER_KEY="..."

# 基本配置
export SYMBI_LOG_LEVEL=info
export SYMBI_RUNTIME_MODE=development

# 向量搜索通过内置的 LanceDB 后端开箱即用。
# 如需改用 Qdrant（可选，启用 `vector-qdrant` 特性）：
# export SYMBIONT_VECTOR_BACKEND=qdrant
# export QDRANT_URL=http://localhost:6333

# MCP 集成（可选）
export MCP_SERVER_URLS="http://localhost:8080"
```

#### 安全相关环境变量（v1.13.0 审计之后）

| 变量 | 默认值 | 作用 |
|---|---|---|
| `SYMBI_INSECURE_ALLOW_ALL` | 未设置 | 设为 `1` 时，`symbi up` / `symbi run` 会使用宽松策略门控（所有工具调用和委派均被允许）。等同于 `--insecure-allow-all` 标志。会打印醒目的 stderr 横幅。**仅限本地开发使用。** 不设置该变量时，推理循环为失败即关闭，在未接入显式策略后端之前拒绝工具调用和委派。 |
| `SYMBI_REJECT_LEGACY_API_KEYS` | 未设置 | 设为 `1` 时，API 密钥验证器会短路掉针对无前缀密钥的已弃用 O(n) Argon2 扫描。请在以 `keyid.secret` 格式重新签发所有密钥之后立即启用。无论是否启用，旧路径都将在下一个次要版本中被移除。 |
| `SYMBI_UNSAFE_NATIVE_SANDBOX` | 未设置 | 构造 `native` 沙箱执行器时必需（且需 `SYMBI_ENV=production` 未设置）。`native-sandbox` Cargo 特性在 release 构建中也会编译失败。原生执行器不提供任何隔离，仅供本地调试使用。 |
| `SYMBI_TRUSTED_PROXIES` | 未设置 | 受信反向代理的 CIDR 白名单；仅当来自这些地址时才采信 `X-Forwarded-For`。 |

以下环境变量已被**移除**：

- `SYMBIONT_ALLOW_NO_JWT_AUDIENCE` — JWT 验证器现在始终要求 `aud`。（在 v1.13.0 审计之后移除；曾是不安全的逃生通道。）
- `COMPOSIO_API_KEY`、`COMPOSIO_MCP_URL` — Composio MCP 集成已被整体移除。参见 `SECURITY_AUDIT.md` C3。

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

### 人机协同审批

即使 Cedar 允许某次调用，清单/子命令中声明的审批要求仍然是强制性的。共享队列会
保留每个请求，直到出现有权限的决定、请求过期或被取消。某个通知器卡住不会阻塞
其他审批界面。

- **普通 / 托管 CLI：** 添加 `--approval-terminal`，可以附带
  `--approval-timeout 120`（1–3600 秒）。审阅完整转义后的 JSON，然后在控制终端上
  准确输入 `approve <request-id>`。不带该标志时，需要审批的调用会失败关闭。
- **REST：** 经过认证的 `GET /api/v1/approvals`、
  `POST /api/v1/approvals/{id}/approve` 和 `.../deny` 可处置待决请求。
- **Shell：** Ctrl+G 即使在轮次执行过程中也能打开 Gate 面板。用 ↑/↓ 选择，按
  Enter 审阅完整请求，滚动查看，然后按 `a` 或 `d`。`/gate` 同样可以打开该面板。
  仅停留在列表行上无法完成批准。
- **聊天：** 位于允许列表中的发送者使用 `/symbi gate show <id>`，然后从完整审阅
  内容中复制 `/symbi gate approve <id> <review-digest>`，或发送
  `/symbi gate deny <id>`。只带 ID 的批准会被拒绝。消息过长时需要另外接入一个
  审阅界面。

请求发生变更、过期或被移除后，都需要重新审阅。TUI 会明确报告处置错误和结果未知
的情况。批准只是允许通过门控继续执行；实际是否执行请在签名的运行审计中核实。
Slack 在所有环境中都要求非空的签名密钥和有效的回调签名。此前允许未签名回调的
覆盖开关已不再支持。限制条件和信任假设参见[审批生命周期](/approval-lifecycle)。

在 `symbiont.toml` 中配置超时和聊天审批通道：

```toml
[escalation]
timeout_seconds = 120

[[escalation.approval_channels]]
platform   = "slack"
channel_id = "C0APPROVERS"
approvers  = ["U0ALICE", "U0BOB"]   # allowlisted sender ids; empty = nobody may approve via chat
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
cargo run -- dsl parse your_agent.symbi

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

准备好构建令人惊叹的东西了吗？从我们的[示例项目](https://github.com/thirdkeyai/symbiont/tree/main/crates/runtime/examples)开始，或深入了解[完整规范](/dsl-specification)。
