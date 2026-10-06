# HTTP 输入模块

HTTP 输入模块提供了一个 webhook 服务器，允许外部系统通过 HTTP 请求调用 Symbiont 智能体。该模块通过 HTTP 端点暴露智能体，从而实现与外部服务、webhook 和 API 的集成。

## 概述

在此分支上，每个 HTTP 推理请求都会独立执行，即使其注册的智能体已经处于活跃状态
也是如此。已注册的源码和安全层级会在推理之前确定一个被冻结的工具执行器。CPU、
内存和执行时间限制会约束该次调用。受治理的工作进程还会与使用同一私有状态目录的
调度器和 CLI 启动共享监督进程所配置的 CPU、内存和工作进程池；参见
[共享预算](/shared-budgets)。成功的响应包含带有 `run_id`、`path` 和 `public_key`
的 `audit` 字段。必需的存储写入失败会阻止后续作用，被丢弃的请求仍保留清理责任。
参见[运行审计](/run-audit)和[分支指南](/containment-branch-guide)。

HTTP 输入模块包含：

- **HTTP 服务器**：基于 Axum 的 Web 服务器，监听传入的 HTTP 请求
- **身份验证**：支持 Bearer 令牌和基于 JWT 的身份验证
- **请求路由**：灵活的路由规则，将请求定向到特定智能体
- **响应控制**：可配置的响应格式和状态码
- **安全功能**：CORS 支持、请求大小限制和审计日志记录
- **并发管理**：内置请求速率限制和并发控制
- **使用 ToolClad 的 LLM 调用**：每个请求都会通过已配置的 LLM 提供商和受治理的 ORGA 工具调用循环，独立地调用已注册的智能体，即使另一次调用正在进行中也是如此

该模块通过 `http-input` 功能标志进行条件编译，并与 Symbiont 智能体运行时无缝集成。

## 配置

HTTP 输入模块使用 [`HttpInputConfig`](../crates/runtime/src/http_input/config.rs) 结构进行配置：

### 基本配置

```rust
use symbiont_runtime::http_input::HttpInputConfig;
use symbiont_runtime::types::AgentId;

let config = HttpInputConfig {
    bind_address: "127.0.0.1".to_string(),
    port: 8081,
    path: "/webhook".to_string(),
    agent: AgentId::from_str("webhook_handler")?,
    // ... other fields
    ..Default::default()
};
```

### 配置字段

| 字段 | 类型 | 默认值 | 描述 |
|-------|------|---------|-------------|
| `bind_address` | `String` | `"127.0.0.1"` | HTTP 服务器绑定的 IP 地址 |
| `port` | `u16` | `8081` | 监听的端口号 |
| `path` | `String` | `"/webhook"` | HTTP 路径端点 |
| `agent` | `AgentId` | 新 ID | 为请求调用的默认智能体 |
| `auth_header` | `Option<String>` | `None` | 用于身份验证的 Bearer 令牌 |
| `jwt_public_key_path` | `Option<String>` | `None` | JWT 公钥文件路径 |
| `max_body_bytes` | `usize` | `65536` | 最大请求体大小（64 KB） |
| `concurrency` | `usize` | `10` | 最大并发请求数 |
| `routing_rules` | `Option<Vec<AgentRoutingRule>>` | `None` | 请求路由规则 |
| `response_control` | `Option<ResponseControlConfig>` | `None` | 响应格式配置 |
| `forward_headers` | `Vec<String>` | `[]` | 转发给智能体的请求头 |
| `cors_origins` | `Vec<String>` | `[]` | 允许的 CORS 来源（空 = 禁用 CORS） |
| `audit_enabled` | `bool` | `true` | 启用请求审计日志记录 |

### 智能体路由规则

根据请求特征将请求路由到不同的智能体：

```rust
use symbiont_runtime::http_input::{AgentRoutingRule, RouteMatch};

let routing_rules = vec![
    AgentRoutingRule {
        condition: RouteMatch::PathPrefix("/api/github".to_string()),
        agent: AgentId::from_str("github_handler")?,
    },
    AgentRoutingRule {
        condition: RouteMatch::HeaderEquals("X-Source".to_string(), "slack".to_string()),
        agent: AgentId::from_str("slack_handler")?,
    },
    AgentRoutingRule {
        condition: RouteMatch::JsonFieldEquals("source".to_string(), "twilio".to_string()),
        agent: AgentId::from_str("sms_handler")?,
    },
];
```

### 响应控制

使用 [`ResponseControlConfig`](../crates/runtime/src/http_input/config.rs) 自定义 HTTP 响应：

```rust
use symbiont_runtime::http_input::ResponseControlConfig;

let response_control = ResponseControlConfig {
    default_status: 200,
    agent_output_to_json: true,
    error_status: 500,
    echo_input_on_error: false,
};
```

## 安全功能

### 身份验证

HTTP 输入模块支持多种身份验证方法：

#### Bearer 令牌身份验证

配置静态 Bearer 令牌：

```rust
let config = HttpInputConfig {
    auth_header: Some("Bearer your-secret-token".to_string()),
    ..Default::default()
};
```

#### 密钥存储集成

使用密钥引用增强安全性：

```rust
let config = HttpInputConfig {
    auth_header: Some("vault://webhook/auth_token".to_string()),
    ..Default::default()
};
```

#### JWT 身份验证 (EdDSA)

配置基于 JWT 的身份验证，使用 Ed25519 公钥：

```rust
let config = HttpInputConfig {
    jwt_public_key_path: Some("/path/to/jwt/ed25519-public.pem".to_string()),
    ..Default::default()
};
```

密钥加载器接受 Ed25519 PEM 或原始公钥字节，用于 EdDSA 验证。JWT 必须带有有效的
`exp` 和非空的 `sub`（最多 512 字节）。过期校验允许五秒的时钟偏差。若提供了
`iss`，则它必须非空且不超过 2,048 字节。使用相同的签名 subject、issuer 和已配置
的密钥续签令牌，可以保持其调用方身份不变。

此 HTTP 输入验证器**不**强制 audience 或 issuer 白名单。已配置的密钥就是其信任
权威；请为该权威使用一个专用密钥。签名中的 issuer 会参与构成重试身份，但并不
构成 issuer 白名单。即使已配置 webhook 签名验证，仍然需要 Bearer 认证；webhook
签名只是一项附加检查。

#### 健康端点

HTTP 输入模块不暴露自己的 `/health` 端点。运行 `symbi up` 时，健康检查通过主 HTTP API 的 `/api/v1/health` 提供，该命令会启动完整的运行时（包括 API 服务器）：

```bash
# 通过主 API 服务器进行健康检查（默认端口 8080）
curl http://127.0.0.1:8080/api/v1/health
# => {"status": "ok"}
```

如果您需要专门针对 HTTP 输入服务器的健康探测，请将负载均衡器路由到主 API 健康端点。

### 安全控制

- **仅回环地址默认**：`bind_address` 默认为 `127.0.0.1`——服务器仅接受本地连接，除非显式配置为其他地址
- **CORS 默认禁用**：`cors_origins` 默认为空列表，表示 CORS 已禁用；添加特定来源以启用跨域访问。`cors_origins` 中出现字面量 `"*"` 会在**启动时被拒绝** —— HTTP 输入服务器拒绝以通配来源启动。（在 v1.13.0 审计之后新增；参见 `SECURITY_AUDIT.md` M1。）
- **请求大小限制**：可配置的最大主体大小防止资源耗尽
- **并发限制**：内置信号量控制并发请求处理
- **审计日志记录**：启用时对所有传入请求进行结构化日志记录
- **密钥解析**：与 Vault 和基于文件的密钥存储集成

## 使用示例

### 启动 HTTP 输入服务器

```rust
use symbiont_runtime::http_input::{HttpInputConfig, start_http_input};
use symbiont_runtime::secrets::SecretsConfig;
use std::sync::Arc;

// 配置 HTTP 输入服务器
let config = HttpInputConfig {
    bind_address: "127.0.0.1".to_string(),
    port: 8081,
    path: "/webhook".to_string(),
    agent: AgentId::from_str("webhook_handler")?,
    auth_header: Some("Bearer secret-token".to_string()),
    audit_enabled: true,
    cors_origins: vec!["https://example.com".to_string()],
    ..Default::default()
};

// 可选：配置密钥
let secrets_config = SecretsConfig::default();

// 启动服务器
start_http_input(config, Some(runtime), Some(secrets_config)).await?;
```

### 示例智能体定义

在 [`webhook_handler.symbi`](../agents/webhook_handler.symbi) 中创建 webhook 处理程序智能体：

```dsl
agent webhook_handler(body: JSON) -> Maybe<Alert> {
    capabilities = ["http_input", "event_processing", "alerting"]
    memory = "ephemeral"
    privacy = "strict"

    policy webhook_guard {
        allow: use("llm") if body.source == "slack" || body.user.ends_with("@company.com")
        allow: publish("topic://alerts") if body.type == "security_alert"
        audit: all_operations
    }

    with context = {} {
        if body.type == "security_alert" {
            alert = {
                "summary": body.message,
                "source": body.source,
                "level": body.severity,
                "user": body.user
            }
            publish("topic://alerts", alert)
            return alert
        }

        return None
    }
}
```

### 示例 HTTP 请求

发送 webhook 请求以触发智能体：

```bash
curl -X POST http://localhost:8081/webhook \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer secret-token" \
  -H "Idempotency-Key: 72d6a833-b825-4b22-b50c-206337d77f7c" \
  -d '{
    "type": "security_alert",
    "message": "Suspicious login detected",
    "source": "slack",
    "severity": "high",
    "user": "admin@company.com"
  }'
```

请为每一项预期任务选定并保留一个全新的 UUID。每次 HTTP 提交都必须恰好带一个
`Idempotency-Key` 标头；重试时请使用相同的 ID、URI 和 JSON 负载。把本示例中的 ID
复用于不同的工作会被拒绝。webhook 发送方必须为每次投递保留一个稳定的 UUID，或者
使用一个适配器，在提交前把其投递标识映射为稳定的 UUID。服务器不会从模型输出推断
身份，也不会在标头缺失时自动生成替代值。

### 重试状态

在每个规范项目内，ID 只在一个 HTTP 域中有效。持久化的占用会绑定已验证的调用方、
请求 URI、JSON 输入以及受信任的目标源码/设置。已注册的智能体 ID 在重启后发生变化
并不会构成一个不同的请求。独立的 SDK 服务器必须在重启后保留其已配置的 `AgentId`。
在同一个 ID 下更改源码、目标、调用方或负载都会拒绝执行；不会把缓存内容或审计引用
返回给另一个调用方。

| HTTP 状态码 | 响应体 `status` | 含义 |
|---|---|---|
| 默认 200 | `completed` | 原始结果已持久化；`replayed` 标识这是一个已保存的响应。 |
| 422 | `failed` | 已持久化一次带有完整追踪证据的终态失败；重试会返回它。 |
| 409 | `in_progress` | 另一个持有者占用着该 ID；此次请求不会启动任何工作。 |
| 409 | `unresolved` | 原始运行需要核销；在可用时会附带其审计引用。 |
| 409 | `reconciled` | 返回一份独立签名的运维评定；该原始 ID 不能再次执行。 |
| 409 | `conflict` | 该 ID 已绑定到不同的调用方或请求。 |
| 400 | `invalid_invocation_id` | UUID 标头缺失、重复或无效。 |
| 503 | `unavailable` | 必需的调用存储无法授权执行。 |

调用状态响应包含 `Idempotency-Key`、`Idempotency-Replayed` 和 `Cache-Control:
no-store` 标头。已配置的成功响应格式仍然适用于已完成的结果；但它无法把未解决或
冲突的结果变成成功响应。已配置的 CORS 源会允许并公开这些调用标头。

被保留的持有者会在准备、执行、清理和保存结果的整个过程中持有该占用。客户端断开
连接会取消该持有者的工作，但不会释放该 ID 以便再次执行。在结果持久化之前进程丢失
会留下一个未解决的占用。已保存的结果在返回前会对照其原始签名审计进行校验；取回
结果不会再次运行提供方或执行器。

静态共享凭据代表一个调用方。JWT 调用方绑定已配置的密钥、签名的 issuer 和 subject；
过期/续签字段不会改变它。轮换凭据或密钥材料会让已有 ID 产生冲突，而不是悄悄创建
第二个任务。占用在整个项目范围内跨 HTTP 监听器生效，因此请使用全新的随机 UUID，
并把该存储与其审计证据一起保留。存储方面的限制参见
[持久化调用身份](/invocation-idempotency)。

### 预期响应

每个推理请求都会返回自己的结果，即使同一智能体的另一次调用正在进行中也是如此。
此路由上不再使用旧的 `execution_started`/`message_id` 交接式响应。成功的响应包含
本次运行的公开审计引用、`invocation_id`、`replayed`、`total_usage` 以及共享的
`budget` 快照。示例取值：

```json
{
  "status": "completed",
  "agent_id": "11111111-1111-4111-8111-111111111111",
  "response": "Task complete.",
  "tool_runs": [],
  "termination_reason": "Completed",
  "iterations": 1,
  "audit": {
    "run_id": "22222222-2222-4222-8222-222222222222",
    "path": "/srv/control/.symbiont/governed/11111111-1111-4111-8111-111111111111.22222222-2222-4222-8222-222222222222.jsonl",
    "public_key": "<hex-encoded-public-key>"
  },
  "model": "<configured-model>",
  "provider": "<configured-provider>",
  "latency_ms": 4821,
  "timestamp": "2024-01-15T10:30:00Z"
}
```

`tool_runs` 汇总的是相互关联的工具观测结果，其中也包括拒绝和校验失败。它的存在
并不证明某项作用确实执行了，而且 `status: completed` 也可能伴随一个策略拒绝的
响应。核实时请使用受保护日志中精确的规范化参数、决策和作用记录。必需的审计或清理
失败会返回错误，即使此前已经发生过某项作用也是如此。`audit.path` 是运行时主机上
的路径，不是下载 URL。

## 使用 ToolClad 工具的 LLM 调用

每个 HTTP 推理请求都会启动一次独立的受治理调用，即使已注册的智能体已经有另一次
活跃调用也是如此。

### 工作原理

1. 在运行时已附加的情况下，从受信任的注册表解析智能体。冻结其所选的源码、沙箱和
   资源设置；在推理之前拒绝不存在的智能体、有歧义的选择以及不匹配的安全层级。
   独立的 SDK 服务器使用其显式配置的通用智能体/执行器。
2. 仅根据所选的智能体源码构建系统提示。调用方可选提供的 `system_prompt` 仍会被
   长度限制并记录日志；它不提供策略、主体或沙箱方面的权限。用户消息则根据请求负载
   构建。
3. 在被冻结的项目中发现 ToolClad 工具，并打开一份必需的私有签名日志。ORGA 循环最多
   允许 15 轮迭代。已注册的以及智能体所选的截止时间会收紧循环和工具的限制；单个
   工具的默认上限为 120 秒。
4. 在 Cedar 之前先准备并规范化被提议的调用。强制性的精确审批、必需的审计和一次性
   授权都先于作用发生。重复或为空的调用 ID 会被拒绝；结果必须与实际准备的调用相
   对应。
5. 等待工作进程清理完成和终态日志写入。成功的响应包含最终响应、工具结果、提供商/
   模型元数据以及 `audit` 引用。取消操作仍保留清理责任；必需的存储或清理错误不会
   悄悄产生一个成功的结果。

参见[预备调用](/prepared-calls)和[运行审计](/run-audit)。按次调用的资源限制并不
构成对请求的汇总准入控制。

### 提供商自动检测

LLM 客户端在服务器启动时根据环境变量初始化。按以下顺序，第一个设置了 API 密钥的提供商生效：

| 环境变量 | 提供商 | 模型覆盖 | Base URL 覆盖 |
|---------|----------|----------------|-------------------|
| `OPENROUTER_API_KEY` | OpenRouter | `OPENROUTER_MODEL`（默认：`anthropic/claude-sonnet-4`） | `OPENROUTER_BASE_URL` |
| `OPENAI_API_KEY` | OpenAI | `CHAT_MODEL`（默认：`gpt-4o`） | `OPENAI_BASE_URL` |
| `ANTHROPIC_API_KEY` | Anthropic | `ANTHROPIC_MODEL`（默认：`claude-sonnet-4-20250514`） | `ANTHROPIC_BASE_URL` |

若未配置推理提供方，推理请求会返回错误。运维方配置的本地端点仍受支持。

### 输入字段

当采用 LLM 路径时，webhook 的 JSON 主体按如下方式解释：

- `prompt` 或 `message` — 用作用户消息。如果两者都不存在，整个负载会被美化打印并作为任务描述传入。
- `system_prompt` — 调用方可选提供的系统提示，追加到由 DSL 派生的系统提示之后。上限为 4096 字节并会被记录日志。将其视为提示注入的攻击面：当此端点暴露给不受信任的调用方时，务必强制执行身份验证。

### 规范化的工具调用格式

LLM 客户端将 OpenAI/OpenRouter 的函数调用规范化为与 Anthropic Messages API 相同的内容块形式。无论使用哪个提供商，每个响应内容块要么是 `{"type": "text", "text": "..."}`，要么是 `{"type": "tool_use", "id": "...", "name": "...", "input": {...}}`，且 `stop_reason` 为 `"end_turn"` 或 `"tool_use"`。

## 集成模式

### Webhook 端点

为不同的 webhook 源配置不同的智能体：

```rust
let routing_rules = vec![
    AgentRoutingRule {
        condition: RouteMatch::HeaderEquals("X-GitHub-Event".to_string(), "push".to_string()),
        agent: AgentId::from_str("github_push_handler")?,
    },
    AgentRoutingRule {
        condition: RouteMatch::JsonFieldEquals("source".to_string(), "stripe".to_string()),
        agent: AgentId::from_str("payment_processor")?,
    },
];
```

### API 网关集成

作为 API 网关后的后端服务使用：

```rust
let config = HttpInputConfig {
    bind_address: "0.0.0.0".to_string(),
    port: 8081,
    path: "/api/webhook".to_string(),
    cors_origins: vec!["https://example.com".to_string()],
    forward_headers: vec![
        "X-Forwarded-For".to_string(),
        "X-Request-ID".to_string(),
    ],
    ..Default::default()
};
```

### 健康检查集成

HTTP 输入模块不包含专用的健康端点。请使用主 API 健康端点（`/api/v1/health`）进行负载均衡器和监控集成。详见上方的[健康端点](#健康端点)部分。

## 错误处理

HTTP 输入模块提供全面的错误处理：

- **身份验证错误**：对于无效令牌返回 `401 Unauthorized`
- **速率限制**：当超过并发限制时返回 `429 Too Many Requests`
- **载荷错误**：对于格式错误的 JSON 返回 `400 Bad Request`
- **调用结果**：返回上文列出的明确重试状态；未解决的工作绝不会被报告为已完成。
- **服务器错误**：无法归类的运行时故障返回可配置的状态码和一条通用的公开消息。

## 监控和可观测性

### 审计日志记录

当 `audit_enabled` 为 true 时，模块记录有关所有请求的结构化信息：

```log
INFO HTTP Input: Received request with 5 headers
INFO Agent webhook_handler is running, dispatching via communication bus
INFO Runtime execution dispatched for agent webhook_handler: message_id=… latency=3ms
```

当使用 LLM 调用路径时，会有额外的日志行追踪 ORGA 循环：

```log
INFO Agent webhook_handler is not running, using LLM invocation path
INFO Invoking LLM for agent webhook_handler: provider=Anthropic model=… tools=4 …
INFO ORGA ACT: executing tool 'nmap_scan' (id=…) for agent webhook_handler
INFO Tool 'nmap_scan' executed successfully
INFO ORGA loop iteration 1 for agent webhook_handler: executed 1 tool(s), continuing
INFO LLM invocation completed for agent webhook_handler: latency=4821ms tool_runs=1 response_len=…
```

### 指标集成

该模块与 Symbiont 运行时的指标系统集成，提供：

- 请求计数和速率
- 响应时间分布
- 按类型划分的错误率
- 活动连接计数
- 并发利用率

## 最佳实践

1. **安全性**：在生产环境中始终使用身份验证
2. **速率限制**：根据您的基础设施配置适当的并发限制
3. **监控**：启用审计日志记录并与您的监控堆栈集成
4. **错误处理**：为您的用例配置适当的错误响应
5. **智能体设计**：设计智能体以处理特定于 webhook 的输入格式
6. **资源限制**：设置合理的主体大小限制以防止资源耗尽

## 参见

- [入门指南](getting-started.md)
- [DSL 指南](dsl-guide.md)
- [API 参考](api-reference.md)
- [推理循环 (ORGA)](reasoning-loop.md)
- [ToolClad 工具契约](toolclad.md)
- [智能体运行时文档](../crates/runtime/README.md)

若一个被保留的调用已有运维核销结论，它会返回 HTTP 409 和 `status: "reconciled"`、
其原始审计引用，以及一份独立签名的 `resolution` 回执。它不会返回伪造的成功结果，
也不会再次执行。参见[运维核销](/invocation-reconciliation)。
