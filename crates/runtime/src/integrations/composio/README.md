# Composio MCP Integration

*Added 2026-02-16*

Composio provides 850+ toolkits (11,000+ tools) accessible via MCP over SSE transport. This integration adds a thin "MCP Proxy" layer so Symbiont agents can use Composio-hosted tools (GitHub, Slack, Jira, etc.) through the existing `SecureMcpClient` enforcement pipeline.

OAuth for third-party services is handled entirely by Composio -- Symbiont only needs an API key and server IDs.

## Architecture

```
Agent -> Runtime -> ComposioMcpSource -> SSE Transport -> Composio MCP Server
                          |
                    SecureMcpClient (tool registry)
                          |
                    ToolInvocationEnforcer (policy)
                          |
                    Tool Execution (via SSE JSON-RPC)
```

## Feature Flag

The integration is gated behind the `composio` Cargo feature (included in `full`):

```bash
cargo build --features composio -p symbi-runtime
```

No additional dependencies are required -- the integration uses `reqwest` (already a dependency) for SSE transport and HTTP JSON-RPC.

## Configuration

Create `~/.symbiont/mcp-config.toml`:

```toml
[composio]
api_key = "env:COMPOSIO_API_KEY"       # or a literal key
# base_url = "https://backend.composio.dev"  # default

[[mcp_servers]]
type = "composio"
name = "my-tools"
server_id = "your_session_id"
user_id = "default"
# url = "https://backend.composio.dev/tool_router/<session_id>/mcp"  # optional direct URL override

[mcp_servers.policy]
allowed_tools = ["GITHUB_*", "SLACK_*"]   # glob patterns (empty = allow all)
require_approval = ["*_DELETE_*"]          # tools matching these require approval
audit_level = "full"                       # "none", "basic", "full"
max_calls_per_minute = 60
```

### Secret Resolution

The `api_key` field supports two formats:

- **Literal:** `api_key = "ak_your_key_here"`
- **Environment variable:** `api_key = "env:COMPOSIO_API_KEY"` -- reads `$COMPOSIO_API_KEY` at runtime

### Server Types

The config file supports both Composio and stdio MCP servers via a `type` discriminator:

| Type | Transport | Use case |
|------|-----------|----------|
| `composio` | SSE over HTTPS | Composio-hosted toolkits |
| `stdio` | stdin/stdout | Local MCP server processes |

## Modules

| Module | Purpose |
|--------|---------|
| `config.rs` | TOML config types, loader, secret resolution |
| `error.rs` | `ComposioError` enum |
| `transport.rs` | SSE client, JSON-RPC over HTTP, Composio SSE response parsing |
| `source.rs` | `ComposioMcpSource` -- tool discovery, policy filtering, `McpTool` conversion |

## CLI Usage

```bash
# Add a Composio server
symbi-mcp add composio:<server_id>/<user_id> --name github

# List configured servers
symbi-mcp list --detailed
```

## Policy Filtering

Per-server policies control which tools are exposed:

- `allowed_tools`: Glob patterns (e.g. `"GITHUB_*"`) -- only matching tools are discovered
- `require_approval`: Glob patterns for tools that need user approval before invocation
- `max_calls_per_minute`: Rate limit per server

Glob syntax: `*` matches any characters, `?` matches a single character.

## Verification Status

Composio-hosted tools receive `VerificationStatus::Skipped` since SchemaPin verification doesn't apply to externally-hosted tools (no public key URL). Policy enforcement via `ToolInvocationEnforcer` still applies.

## Smoke Test

```bash
COMPOSIO_API_KEY=<your_key> COMPOSIO_MCP_URL=<your_mcp_url> \
  cargo run --features composio -p symbi-runtime --example composio_smoke_test
```

## Design Decisions

- **No SSE crate dependency** -- reqwest streaming + manual `text/event-stream` parsing keeps the dependency footprint minimal
- **Feature-gated** -- zero cost when `composio` is not enabled
- **Handles Composio's SSE-wrapped responses** -- POST requests return SSE format (`event: message\ndata: {...}`) even though it's a request/response flow; the transport parses both plain JSON-RPC and SSE-wrapped payloads
- **Glob-to-regex policy matching** -- uses the existing `regex` crate dependency
