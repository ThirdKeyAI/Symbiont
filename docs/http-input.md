---
layout: default
title: HTTP Input Module
nav_order: 7
description: "HTTP Input module for webhook integration with Symbiont agents"
---

# HTTP Input Module

## 🌐 Other Languages
{: .no_toc}

**English** | [中文简体](http-input.zh-cn.md) | [Español](http-input.es.md) | [Português](http-input.pt.md) | [日本語](http-input.ja.md) | [Deutsch](http-input.de.md)

---

The HTTP Input module provides a webhook server that allows external systems to invoke Symbiont agents via HTTP requests. This module enables integration with external services, webhooks, and APIs by exposing agents through HTTP endpoints.

## Overview

The HTTP Input module consists of:

- **HTTP Server**: An Axum-based web server that listens for incoming HTTP requests
- **Authentication**: Support for Bearer token and JWT-based authentication
- **Request Routing**: Flexible routing rules to direct requests to specific agents
- **Response Control**: Configurable response formatting and status codes
- **Security Features**: CORS support, request size limits, and audit logging
- **Concurrency Management**: Built-in request rate limiting and concurrency control

The module is conditionally compiled with the `http-input` feature flag and integrates seamlessly with the Symbiont agent runtime.

## Configuration

The HTTP Input module is configured using the [`HttpInputConfig`](../crates/runtime/src/http_input/config.rs) structure:

### Basic Configuration

```rust
use symbiont_runtime::http_input::HttpInputConfig;
use symbiont_runtime::types::AgentId;

let config = HttpInputConfig {
    bind_address: "0.0.0.0".to_string(),
    port: 8081,
    path: "/webhook".to_string(),
    agent: AgentId::from_str("webhook_handler")?,
    // ... other fields
    ..Default::default()
};
```

### Configuration Fields

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `bind_address` | `String` | `"0.0.0.0"` | IP address to bind the HTTP server |
| `port` | `u16` | `8081` | Port number to listen on |
| `path` | `String` | `"/webhook"` | HTTP path endpoint |
| `agent` | `AgentId` | New ID | Default agent to invoke for requests |
| `auth_header` | `Option<String>` | `None` | Bearer token for authentication |
| `jwt_public_key_path` | `Option<String>` | `None` | Path to JWT public key file |
| `max_body_bytes` | `usize` | `65536` | Maximum request body size (64 KB) |
| `concurrency` | `usize` | `10` | Maximum concurrent requests |
| `routing_rules` | `Option<Vec<AgentRoutingRule>>` | `None` | Request routing rules |
| `response_control` | `Option<ResponseControlConfig>` | `None` | Response formatting config |
| `forward_headers` | `Vec<String>` | `[]` | Headers to forward to agents |
| `cors_enabled` | `bool` | `false` | Enable CORS support |
| `audit_enabled` | `bool` | `true` | Enable request audit logging |

### Agent Routing Rules

Route requests to different agents based on request characteristics:

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

### Response Control

Customize HTTP responses with [`ResponseControlConfig`](../crates/runtime/src/http_input/config.rs):

```rust
use symbiont_runtime::http_input::ResponseControlConfig;

let response_control = ResponseControlConfig {
    default_status: 200,
    agent_output_to_json: true,
    error_status: 500,
    echo_input_on_error: false,
};
```

## Security Features

### Authentication

The HTTP Input module supports multiple authentication methods:

#### Bearer Token Authentication

Configure a static bearer token:

```rust
let config = HttpInputConfig {
    auth_header: Some("Bearer your-secret-token".to_string()),
    ..Default::default()
};
```

#### Secret Store Integration

Use secret references for enhanced security:

```rust
let config = HttpInputConfig {
    auth_header: Some("vault://webhook/auth_token".to_string()),
    ..Default::default()
};
```

#### JWT Authentication

Configure JWT-based authentication:

```rust
let config = HttpInputConfig {
    jwt_public_key_path: Some("/path/to/jwt/public.key".to_string()),
    ..Default::default()
};
```

### Security Controls

- **Request Size Limits**: Configurable maximum body size prevents resource exhaustion
- **Concurrency Limits**: Built-in semaphore controls concurrent request processing
- **CORS Support**: Optional CORS headers for browser-based applications
- **Audit Logging**: Structured logging of all incoming requests when enabled
- **Secret Resolution**: Integration with Vault and file-based secret stores

## Usage Example

### Starting the HTTP Input Server

```rust
use symbiont_runtime::http_input::{HttpInputConfig, start_http_input};
use symbiont_runtime::secrets::SecretsConfig;
use std::sync::Arc;

// Configure the HTTP input server
let config = HttpInputConfig {
    bind_address: "127.0.0.1".to_string(),
    port: 8081,
    path: "/webhook".to_string(),
    agent: AgentId::from_str("webhook_handler")?,
    auth_header: Some("Bearer secret-token".to_string()),
    audit_enabled: true,
    cors_enabled: true,
    ..Default::default()
};

// Optional: Configure secrets
let secrets_config = SecretsConfig::default();

// Start the server
start_http_input(config, Some(runtime), Some(secrets_config)).await?;
```

### Example Agent Definition

Create a webhook handler agent in [`webhook_handler.dsl`](../agents/webhook_handler.dsl):

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

### Example HTTP Request

Send a webhook request to trigger the agent:

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

### Expected Response

The server returns a JSON response with the agent's output:

```json
{
  "status": "invoked",
  "agent_id": "webhook_handler",
  "timestamp": "2024-01-15T10:30:00Z"
}
```

## Integration Patterns

### Webhook Endpoints

Configure different agents for different webhook sources:

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

### API Gateway Integration

Use as a backend service behind an API gateway:

```rust
let config = HttpInputConfig {
    bind_address: "0.0.0.0".to_string(),
    port: 8081,
    path: "/api/webhook".to_string(),
    cors_enabled: true,
    forward_headers: vec![
        "X-Forwarded-For".to_string(),
        "X-Request-ID".to_string(),
    ],
    ..Default::default()
};
```

### Health Check Endpoint

The server automatically provides health check capabilities for load balancers and monitoring systems.

## Error Handling

The HTTP Input module provides comprehensive error handling:

- **Authentication Errors**: Returns `401 Unauthorized` for invalid tokens
- **Rate Limiting**: Returns `429 Too Many Requests` when concurrency limits are exceeded
- **Payload Errors**: Returns `400 Bad Request` for malformed JSON
- **Agent Errors**: Returns configurable error status with error details
- **Server Errors**: Returns `500 Internal Server Error` for runtime failures

## Monitoring and Observability

### Audit Logging

When `audit_enabled` is true, the module logs structured information about all requests:

```log
INFO HTTP Input: Received request with 5 headers
INFO Would invoke agent webhook_handler with input data
```

### Metrics Integration

The module integrates with the Symbiont runtime's metrics system to provide:

- Request count and rate
- Response time distributions
- Error rates by type
- Active connection counts
- Concurrency utilization

## Best Practices

1. **Security**: Always use authentication in production environments
2. **Rate Limiting**: Configure appropriate concurrency limits based on your infrastructure
3. **Monitoring**: Enable audit logging and integrate with your monitoring stack
4. **Error Handling**: Configure appropriate error responses for your use case
5. **Agent Design**: Design agents to handle webhook-specific input formats
6. **Resource Limits**: Set reasonable body size limits to prevent resource exhaustion

## See Also

- [Getting Started Guide](getting-started.md)
- [DSL Guide](dsl-guide.md)
- [API Reference](api-reference.md)
- [Agent Runtime Documentation](../crates/runtime/README.md)