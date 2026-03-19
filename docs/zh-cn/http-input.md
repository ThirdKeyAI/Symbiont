# HTTP 输入模块

HTTP 输入模块提供了一个 webhook 服务器，允许外部系统通过 HTTP 请求调用 Symbiont 智能体。该模块通过 HTTP 端点暴露智能体，从而实现与外部服务、webhook 和 API 的集成。

## 概述

HTTP 输入模块包含：

- **HTTP 服务器**：基于 Axum 的 Web 服务器，监听传入的 HTTP 请求
- **身份验证**：支持 Bearer 令牌和基于 JWT 的身份验证
- **请求路由**：灵活的路由规则，将请求定向到特定智能体
- **响应控制**：可配置的响应格式和状态码
- **安全功能**：CORS 支持、请求大小限制和审计日志记录
- **并发管理**：内置请求速率限制和并发控制

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

JWT 验证器从指定的 PEM 文件加载 Ed25519 公钥，并验证传入的 `Authorization: Bearer <jwt>` 令牌。仅接受 **EdDSA** 算法——HS256、RS256 及其他算法会被拒绝。

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
- **CORS 默认禁用**：`cors_origins` 默认为空列表，表示 CORS 已禁用；添加特定来源以启用跨域访问
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

在 [`webhook_handler.dsl`](../agents/webhook_handler.dsl) 中创建 webhook 处理程序智能体：

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
  -d '{
    "type": "security_alert",
    "message": "Suspicious login detected",
    "source": "slack",
    "severity": "high",
    "user": "admin@company.com"
  }'
```

### 预期响应

服务器返回包含智能体输出的 JSON 响应：

```json
{
  "status": "execution_started",
  "agent_id": "webhook_handler",
  "timestamp": "2024-01-15T10:30:00Z"
}
```

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
- **智能体错误**：返回可配置的错误状态和错误详情
- **服务器错误**：对于运行时故障返回 `500 Internal Server Error`

## 监控和可观测性

### 审计日志记录

当 `audit_enabled` 为 true 时，模块记录有关所有请求的结构化信息：

```log
INFO HTTP Input: Received request with 5 headers
INFO Would invoke agent webhook_handler with input data
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
- [智能体运行时文档](../crates/runtime/README.md)
