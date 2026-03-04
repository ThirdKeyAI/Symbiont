---
layout: default
title: 高级推理原语 (symbi-dev)
description: "高级推理循环原语：工具筛选、卡住循环检测、上下文预获取和范围约定"
nav_exclude: true
---

# 高级推理原语
{: .no_toc }

## 其他语言
{: .no_toc}

[English](symbi-dev.md) | **中文简体** | [Español](symbi-dev.es.md) | [Português](symbi-dev.pt.md) | [日本語](symbi-dev.ja.md) | [Deutsch](symbi-dev.de.md)

---

特性门控的运行时原语，通过工具筛选、卡住循环检测、确定性上下文预获取和目录范围约定检索来增强推理循环。
{: .fs-6 .fw-300 }

## 目录
{: .no_toc .text-delta }

1. TOC
{:toc}

---

## 概述

`symbi-dev` 特性门控为推理循环添加了四项高级功能：

| 原语 | 解决的问题 | 模块 |
|------|-----------|------|
| **Tool Profile** | LLM 看到太多工具，在无关工具上浪费 token | `tool_profile.rs` |
| **Progress Tracker** | 循环卡在重复尝试同一个失败步骤 | `progress_tracker.rs` |
| **Pre-Hydration** | 冷启动上下文空白——智能体必须自行发现引用 | `pre_hydrate.rs` |
| **Scoped Conventions** | 约定检索是语言级别的，不是目录特定的 | `knowledge_bridge.rs` |

### 启用方式

```toml
# 在您的 Cargo.toml 中
[dependencies]
symbi-runtime = { version = "1.6", features = ["symbi-dev"] }
```

或从源码构建：

```bash
cargo build --features symbi-dev
cargo test --features symbi-dev
```

所有原语都是增量和向后兼容的——没有特性门控的现有代码编译和运行完全相同。

---

## 工具配置文件过滤

在 LLM 看到工具定义之前进行过滤。减少 token 浪费，防止模型选择无关工具。

### 配置

```rust
use symbi_runtime::reasoning::ToolProfile;

// 仅包含文件相关工具
let profile = ToolProfile::include_only(&["file_*", "code_*"]);

// 排除调试工具
let profile = ToolProfile::exclude_only(&["debug_*", "internal_*"]);

// 组合：包含 web 工具，排除实验性工具，上限 10 个
let profile = ToolProfile {
    include: vec!["web_*".into(), "search_*".into()],
    exclude: vec!["web_experimental_*".into()],
    max_tools: Some(10),
    require_verified: false,
};
```

### 过滤管线

管线按顺序应用：

1. **Include** — 如果非空，只有匹配任一 include glob 的工具通过
2. **Exclude** — 匹配任一 exclude glob 的工具被移除
3. **Verified** — 如果 `require_verified` 为 true，只有描述中包含 `[verified]` 的工具通过
4. **Max cap** — 如果设置了 `max_tools`，则截断至该数量

### Glob 语法

| 模式 | 匹配 |
|------|------|
| `web_*` | `web_search`、`web_fetch`、`web_scrape` |
| `tool_?` | `tool_a`、`tool_1`（单个字符） |
| `exact_name` | 仅 `exact_name` |

### 与 LoopConfig 集成

```rust
let config = LoopConfig {
    tool_profile: Some(ToolProfile::include_only(&["search_*", "file_*"])),
    ..Default::default()
};
```

配置文件在 `ReasoningLoopRunner::run()` 中自动应用，在工具定义从执行器和知识桥接填充之后。

---

## 进度追踪器

按步骤跟踪重试次数，并通过比较连续错误输出的归一化 Levenshtein 相似度来检测卡住循环。

### 配置

```rust
use symbi_runtime::reasoning::{ProgressTracker, StepIterationConfig, LimitAction};

let config = StepIterationConfig {
    max_reattempts_per_step: 2,    // 2 次失败尝试后停止
    similarity_threshold: 0.85,    // 85%+ 相似度的错误 = 卡住
    on_limit_reached: LimitAction::SkipStep,
};

let mut tracker = ProgressTracker::new(config);
```

### 使用方法（协调器级别）

进度追踪器**不直接集成到推理循环中**——它是用于编排多步骤任务的协调器的高阶关注点。

```rust
// 开始跟踪一个步骤
tracker.begin_step("extract_data");

// 每次尝试后，记录错误并检查
let decision = tracker.record_and_check("extract_data", &error_output);

match decision {
    StepDecision::Continue => { /* 重试 */ }
    StepDecision::Stop { reason } => {
        // 发出 LoopEvent::StepLimitReached 并继续
        match tracker.limit_action() {
            LimitAction::SkipStep => { /* 跳到下一步 */ }
            LimitAction::AbortTask => { /* 中止整个任务 */ }
            LimitAction::Escalate => { /* 移交给人类 */ }
        }
    }
}
```

### 卡住检测

追踪器计算连续错误输出之间的归一化 Levenshtein 距离。如果相似度超过阈值（默认 85%），则认为步骤已卡住——即使尚未达到最大重试次数。

这可以捕获智能体不断遇到相同错误但措辞略有不同的场景。

---

## 预水化引擎

从任务输入中提取引用（URL、文件路径、GitHub issues/PR），并在推理循环开始前并行解析。这消除了冷启动延迟，否则智能体需要自行发现和获取这些引用。

### 配置

```rust
use symbi_runtime::reasoning::PreHydrationConfig;
use std::time::Duration;

let config = PreHydrationConfig {
    custom_patterns: vec![],
    resolution_tools: [
        ("url".into(), "web_fetch".into()),
        ("file".into(), "file_read".into()),
    ].into(),
    timeout: Duration::from_secs(15),
    max_references: 10,
    max_context_tokens: 4000,  // 1 token ~ 4 chars
};
```

### 内置模式

| 模式 | 类型 | 匹配示例 |
|------|------|----------|
| URL | `url` | `https://example.com/api`、`http://localhost:3000` |
| 文件路径 | `file` | `./src/main.rs`、`~/config.toml` |
| Issues | `issue` | `#42`、`#100` |
| Pull requests | `pr` | `PR #55`、`pr #12` |

### 自定义模式

```rust
use symbi_runtime::reasoning::pre_hydrate::ReferencePattern;

let config = PreHydrationConfig {
    custom_patterns: vec![
        ReferencePattern {
            ref_type: "jira".into(),
            pattern: r"[A-Z]+-\d+".into(),  // PROJ-123
        },
    ],
    ..Default::default()
};
```

### 解析流程

1. **提取** — 正则模式扫描任务输入，去重匹配结果
2. **解析** — 每个引用通过配置的工具解析（例如，URL 使用 `web_fetch`）
3. **预算** — 结果被裁剪以适应 `max_context_tokens`
4. **注入** — 格式化为 `[PRE_HYDRATED_CONTEXT]` 系统消息（与知识桥接的 `[KNOWLEDGE_CONTEXT]` 槽分离）

### 与 LoopConfig 集成

```rust
let config = LoopConfig {
    pre_hydration: Some(PreHydrationConfig {
        resolution_tools: [("url".into(), "web_fetch".into())].into(),
        ..Default::default()
    }),
    ..Default::default()
};
```

预水化在 `run_inner()` 开始时自动运行，在主推理循环开始之前。会发出带有提取和解析统计的 `LoopEvent::PreHydrationComplete` 日志事件。

---

## 目录范围约定

扩展 `recall_knowledge` 工具，添加 `directory` 和 `scope` 参数，用于检索特定目录范围的编码约定。

### 工作原理

当使用 `scope: "conventions"` 和 `directory` 调用时，知识桥接：

1. 搜索与目录路径匹配的约定
2. 向上遍历父目录（例如，`src/api/` -> `src/` -> 项目根目录）
3. 回退到语言级别的约定
4. 跨所有层级按内容去重
5. 截断到请求的限制数量

### LLM 工具调用

```json
{
  "name": "recall_knowledge",
  "arguments": {
    "query": "rust",
    "directory": "src/api/handlers",
    "scope": "conventions"
  }
}
```

### 向后兼容性

`directory` 和 `scope` 参数是可选的。没有它们时，`recall_knowledge` 的行为与标准版本完全相同——使用 `query` 和 `limit` 进行普通知识搜索。

---

## LoopConfig 字段

当启用 `symbi-dev` 特性时，`LoopConfig` 新增三个可选字段：

```rust
pub struct LoopConfig {
    // ... 现有字段 ...

    /// 用于过滤 LLM 可见工具的工具配置文件。
    pub tool_profile: Option<ToolProfile>,
    /// 用于卡住循环检测的逐步迭代限制。
    pub step_iteration: Option<StepIterationConfig>,
    /// 用于确定性上下文预获取的预水化配置。
    pub pre_hydration: Option<PreHydrationConfig>,
}
```

所有字段默认为 `None`，并使用 `#[serde(default, skip_serializing_if = "Option::is_none")]` 序列化以确保向后兼容性。

## 日志事件

提供两个新的 `LoopEvent` 变体：

```rust
pub enum LoopEvent {
    // ... 现有变体 ...

    /// 步骤达到重试限制（由协调器发出）。
    StepLimitReached {
        step_id: String,
        attempts: u32,
        reason: String,
    },
    /// 预水化阶段完成。
    PreHydrationComplete {
        references_found: usize,
        references_resolved: usize,
        references_failed: usize,
        total_tokens: usize,
    },
}
```

---

## 测试

```bash
# 不使用特性（无回归）
cargo clippy --workspace -j2
cargo test --workspace -j2

# 使用特性
cargo clippy --workspace -j2 --features symbi-dev
cargo test --workspace -j2 --features symbi-dev
```

所有测试都是内联 `#[cfg(test)]` 模块——无需外部测试 fixture。

---

## 模块映射

| 模块 | 公共类型 | 描述 |
|------|---------|------|
| `tool_profile` | `ToolProfile` | 基于 glob 的工具过滤，支持验证标志和最大数量上限 |
| `progress_tracker` | `ProgressTracker`、`StepIterationConfig`、`StepDecision`、`LimitAction` | 逐步迭代跟踪，支持 Levenshtein 卡住检测 |
| `pre_hydrate` | `PreHydrationEngine`、`PreHydrationConfig`、`HydratedContext` | 引用提取、并行解析、token 预算裁剪 |
| `knowledge_bridge` | （扩展） | `retrieve_scoped_conventions()`、扩展的 `recall_knowledge` 工具 |

---

## 下一步

- **[推理循环指南](reasoning-loop.md)** — 核心 ORGA 循环文档
- **[运行时架构](runtime-architecture.md)** — 完整系统架构概览
- **[API 参考](api-reference.md)** — 完整的 API 文档
