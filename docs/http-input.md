# HTTP Input Module

The HTTP Input module provides a webhook server that allows external systems to invoke Symbiont agents via HTTP requests. This module enables integration with external services, webhooks, and APIs by exposing agents through HTTP endpoints.

## Overview

On this branch, each HTTP reasoning request executes independently even if its
registered agent is already active. Registered source and security tier select a
frozen tool executor before inference. CPU, memory and execution-time limits
constrain that invocation. Governed workers also share the supervisor's configured
CPU, memory and worker pool with scheduler and CLI launches using the same private
state directory; see [shared budgets](shared-budgets.md). Successful
responses include `audit` with `run_id`, `path` and `public_key`. Required storage
failures stop further effects, and dropped requests retain cleanup ownership.
See [run audit](run-audit.md) and the [branch guide](containment-branch-guide.md).

The HTTP Input module consists of:

- **HTTP Server**: An Axum-based web server that listens for incoming HTTP requests
- **Authentication**: Support for Bearer token and JWT-based authentication
- **Request Routing**: Flexible routing rules to direct requests to specific agents
- **Response Control**: Configurable response formatting and status codes
- **Security Features**: CORS support, request size limits, and audit logging
- **Concurrency Management**: Built-in request rate limiting and concurrency control
- **LLM Invocation with ToolClad**: Each request invokes the registered agent independently through the configured LLM provider and governed ORGA tool-calling loop, including when another invocation is active

The module is conditionally compiled with the `http-input` feature flag and integrates seamlessly with the Symbiont agent runtime.

## Configuration

The HTTP Input module is configured using the [`HttpInputConfig`](../crates/runtime/src/http_input/config.rs) structure:

### Basic Configuration

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

### Configuration Fields

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `bind_address` | `String` | `"127.0.0.1"` | IP address to bind the HTTP server |
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
| `cors_origins` | `Vec<String>` | `[]` | Allowed CORS origins (empty = CORS disabled) |
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

#### JWT Authentication (EdDSA)

Configure JWT-based authentication with Ed25519 public keys:

```rust
let config = HttpInputConfig {
    jwt_public_key_path: Some("/path/to/jwt/ed25519-public.pem".to_string()),
    ..Default::default()
};
```

The key loader accepts Ed25519 PEM or raw public-key bytes for EdDSA verification.
JWTs must have a valid `exp` and a nonempty `sub` (at most 512 bytes). Expiry
validation permits five seconds of clock skew. If supplied, `iss` must be
nonempty and at most 2,048 bytes. Renewing a token with the same signed subject,
issuer and configured key preserves its caller identity.

This HTTP Input verifier does **not** enforce an audience or issuer allowlist.
The configured key is its trust authority; use a key dedicated to that authority.
A signed issuer contributes to retry identity, but does not establish an issuer
allowlist. Bearer authentication is required even when webhook signature
verification is configured; the webhook signature is an additional check.

#### Health Endpoint

The HTTP Input module does not expose its own `/health` endpoint. Health checks are available via the main HTTP API at `/api/v1/health` when running `symbi up`, which starts the full runtime including the API server:

```bash
# Health check via the main API server (default port 8080)
curl http://127.0.0.1:8080/api/v1/health
# => {"status": "ok"}
```

If you need health probes for the HTTP Input server specifically, route your load balancer to the main API health endpoint instead.

### Security Controls

- **Loopback-Only Default**: `bind_address` defaults to `127.0.0.1` — the server only accepts local connections unless explicitly configured otherwise
- **CORS Disabled by Default**: `cors_origins` defaults to an empty list, meaning CORS is disabled; add specific origins to enable cross-origin access. A literal `"*"` in `cors_origins` is **rejected at startup** — the HTTP Input server will refuse to start with a wildcard origin. (Added in the post-v1.13.0 audit; see `SECURITY_AUDIT.md` M1.)
- **Request Size Limits**: Configurable maximum body size prevents resource exhaustion
- **Concurrency Limits**: Built-in semaphore controls concurrent request processing
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
    cors_origins: vec!["https://example.com".to_string()],
    ..Default::default()
};

// Optional: Configure secrets
let secrets_config = SecretsConfig::default();

// Start the server
start_http_input(config, Some(runtime), Some(secrets_config)).await?;
```

### Example Agent Definition

Create a webhook handler agent in [`webhook_handler.symbi`](../agents/webhook_handler.symbi):

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
  -H "Idempotency-Key: 72d6a833-b825-4b22-b50c-206337d77f7c" \
  -d '{
    "type": "security_alert",
    "message": "Suspicious login detected",
    "source": "slack",
    "severity": "high",
    "user": "admin@company.com"
  }'
```

Choose and retain a fresh UUID for each intended task. Every HTTP submission
requires exactly one `Idempotency-Key` header; retry with the same ID, URI and
JSON payload. Reusing this example ID for different work will be refused.
Webhook senders must retain a stable UUID per delivery, or use an adapter that
maps their delivery identity to a stable UUID before submission. The server does
not infer identity from model output or generate a replacement when the header
is absent.

### Retry states

IDs occupy one HTTP domain per canonical project. The durable claim binds the
verified caller, request URI, JSON input and trusted target source/settings.
Registered agent IDs can change on restart without creating a different request.
A standalone SDK server must retain its configured `AgentId` across restarts.
Changing the source, target, caller or payload under an existing ID refuses work;
no cache contents or audit reference are returned to a different caller.

| HTTP status | Body `status` | Meaning |
|---|---|---|
| 200 by default | `completed` | Original result persisted; `replayed` identifies a saved response. |
| 422 | `failed` | A terminal failure with complete tracked evidence was persisted; retries return it. |
| 409 | `in_progress` | Another owner holds the ID; this request starts no work. |
| 409 | `unresolved` | The original run needs reconciliation; includes its audit reference when available. |
| 409 | `reconciled` | Returns a separate signed operator assessment; the original ID cannot execute again. |
| 409 | `conflict` | The ID is bound to a different caller or request. |
| 400 | `invalid_invocation_id` | Missing, repeated or invalid UUID header. |
| 503 | `unavailable` | Required invocation storage could not authorize execution. |

Invocation-state replies include `Idempotency-Key`, `Idempotency-Replayed` and `Cache-Control:
no-store` headers. Configured success formatting still applies to completed
results; it cannot turn unresolved/conflicting outcomes into successful responses.
Configured CORS origins permit and expose the invocation headers.

A retained owner holds the claim through setup, execution, cleanup and saving the
result. Client disconnection cancels that owner’s work but does not release the ID
for execution again. Process loss before result persistence leaves an unresolved
claim. Saved results are verified against their original signed audit before
return; retrieval does not run the provider or executor again.

Static shared credentials represent one caller. A JWT caller binds the configured
key, signed issuer and subject; expiry/renewal fields do not change it. Rotating
credentials or key material makes an existing ID conflict rather than silently
creating a second task. Claims remain project-wide across HTTP listeners, so use
fresh random UUIDs and keep the store with its audit evidence. See
[persistent invocation identities](invocation-idempotency.md) for storage limits.

### Expected Response

Each reasoning request returns its own result, including when another invocation
of the same agent is active. The old `execution_started`/`message_id` handoff
response is no longer used on this route. Successful responses include the run's
public audit reference, `invocation_id`, `replayed`, `total_usage` and the shared
`budget` snapshot. Illustrative values:

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

`tool_runs` summarizes correlated tool observations, including denials and
validation failures. Its presence does not prove that an effect executed, and
`status: completed` can accompany a policy-refusal response. Use the protected
journal's exact normalized arguments, decision and effect records for verification.
Required audit or cleanup failures return an error, even if an earlier effect
already occurred. The `audit.path` is a path on the runtime host, not a download URL.

## LLM Invocation with ToolClad Tools

Each HTTP reasoning request starts an independent governed invocation, including
when the registered agent already has another active invocation.

### How it works

1. With an attached runtime, resolve the agent from the trusted registry. Freeze
   its selected source, sandbox and resource settings; reject missing agents,
   ambiguous selections and mismatched security tiers before inference. A
   standalone SDK server uses its explicitly configured generic agent/executor.
2. Build the system prompt from only the selected agent source. An optional
   caller-supplied `system_prompt` remains length-capped and logged; it does not
   supply policy, principal or sandbox authority. Build the user message from the
   request payload.
3. Discover ToolClad tools in the frozen project and open a required private signed
   journal. The ORGA loop allows up to 15 iterations. Registered and agent-selected
   deadlines tighten the loop and tool limits; the default per-tool limit is
   120 seconds.
4. Prepare and normalize proposed calls before Cedar. Mandatory exact approvals,
   required audit and single-use authorization precede effects. Duplicate or empty
   call IDs are rejected; results must correlate to the actual prepared calls.
5. Await worker cleanup and terminal journaling. Successful responses include the
   final response, tool outcomes, provider/model metadata and `audit` reference.
   Cancellation retains cleanup ownership; required storage or cleanup errors
   cannot silently produce a successful result.

See [prepared calls](prepared-calls.md) and [run audit](run-audit.md). Per-invocation
resource limits do not establish aggregate request admission.

### Provider auto-detection

The LLM client is initialized from environment variables at server start. The first provider whose API key is set wins, in this order:

| Env var | Provider | Model override | Base URL override |
|---------|----------|----------------|-------------------|
| `OPENROUTER_API_KEY` | OpenRouter | `OPENROUTER_MODEL` (default: `anthropic/claude-sonnet-4`) | `OPENROUTER_BASE_URL` |
| `OPENAI_API_KEY` | OpenAI | `CHAT_MODEL` (default: `gpt-4o`) | `OPENAI_BASE_URL` |
| `ANTHROPIC_API_KEY` | Anthropic | `ANTHROPIC_MODEL` (default: `claude-sonnet-4-20250514`) | `ANTHROPIC_BASE_URL` |

Without a configured inference provider, reasoning requests return an error.
Operator-configured local endpoints remain supported.

### Input fields

The webhook JSON body is interpreted as follows when the LLM path is taken:

- `prompt` or `message` — used as the user message. If neither is present, the whole payload is pretty-printed and passed as the task description.
- `system_prompt` — optional caller-supplied system prompt appended to the DSL-derived system prompt. Capped at 4096 bytes and logged. Treat as a prompt-injection surface: always enforce authentication when exposing this endpoint to untrusted callers.

### Normalized tool-call format

The LLM client normalizes OpenAI/OpenRouter function calling into the same content-block shape used by the Anthropic Messages API. Regardless of provider, each response content block is either `{"type": "text", "text": "..."}` or `{"type": "tool_use", "id": "...", "name": "...", "input": {...}}`, and `stop_reason` is `"end_turn"` or `"tool_use"`.

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
    cors_origins: vec!["https://example.com".to_string()],
    forward_headers: vec![
        "X-Forwarded-For".to_string(),
        "X-Request-ID".to_string(),
    ],
    ..Default::default()
};
```

### Health Check Integration

The HTTP Input module does not include a dedicated health endpoint. Use the main API health endpoint (`/api/v1/health`) for load balancer and monitoring integration. See the [Health Endpoint](#health-endpoint) section above for details.

## Error Handling

The HTTP Input module provides comprehensive error handling:

- **Authentication Errors**: Returns `401 Unauthorized` for invalid tokens
- **Rate Limiting**: Returns `429 Too Many Requests` when concurrency limits are exceeded
- **Payload Errors**: Returns `400 Bad Request` for malformed JSON
- **Invocation Outcomes**: Returns the explicit retry states above; unresolved work is never reported as completed.
- **Server Errors**: Unclassified runtime failures return a configurable status with a generic public message.

## Monitoring and Observability

### Audit Logging

When `audit_enabled` is true, the module logs structured information about all requests:

```log
INFO HTTP Input: Received request with 5 headers
INFO Agent webhook_handler is running, dispatching via communication bus
INFO Runtime execution dispatched for agent webhook_handler: message_id=… latency=3ms
```

When the LLM invocation path is used, additional lines trace the ORGA loop:

```log
INFO Agent webhook_handler is not running, using LLM invocation path
INFO Invoking LLM for agent webhook_handler: provider=Anthropic model=… tools=4 …
INFO ORGA ACT: executing tool 'nmap_scan' (id=…) for agent webhook_handler
INFO Tool 'nmap_scan' executed successfully
INFO ORGA loop iteration 1 for agent webhook_handler: executed 1 tool(s), continuing
INFO LLM invocation completed for agent webhook_handler: latency=4821ms tool_runs=1 response_len=…
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
- [Reasoning Loop (ORGA)](reasoning-loop.md)
- [ToolClad Tool Contracts](toolclad.md)
- [Agent Runtime Documentation](../crates/runtime/README.md)

A retained invocation with an operator resolution returns HTTP 409 and
`status: "reconciled"`, its original audit reference and a separate signed
`resolution` receipt. It does not return a fabricated successful result or execute
again. See [operator reconciliation](invocation-reconciliation.md).
