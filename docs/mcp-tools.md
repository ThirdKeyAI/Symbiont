# MCP-Backed Tool Execution

Symbiont agents can call tools exposed by external [Model Context
Protocol](https://modelcontextprotocol.io/) (MCP) servers. Tools are declared
as ToolClad contracts, connected over stdio, and — by default —
**SchemaPin-verified before every invocation** (fail-closed).

> **Build feature:** MCP execution is gated behind the `mcp-client` cargo
> feature. The published `symbi` binary enables it by default; a
> `--no-default-features` or library build without `mcp-client` compiles
> without any MCP client transport, and tool calls fall back to an honest
> "no tool backend" error (never a fabricated success).

## The two layers

Declaring an MCP-backed tool takes two pieces:

1. **A ToolClad manifest** (`tools/<name>.clad.toml`) — the tool contract
   (arguments, validation, evidence envelope) plus an `[mcp]` block that names
   an upstream server + tool and maps argument names.
2. **A server registry** (`mcp-config.toml`) — maps each server *name* to how
   to launch it over stdio (`command`, `args`, `env`) and, for verification,
   its SchemaPin public-key URL.

### 1. ToolClad manifest — `tools/weather.clad.toml`

```toml
[tool]
name = "weather"
version = "1.0.0"
description = "Current weather for a city"
mode = "oneshot"
risk_tier = "low"

[args.city]
position = 1
required = true
type = "string"
description = "City name"

[output]
format = "json"
envelope = true
schema = { type = "object" }

# Route this tool to an MCP server instead of a local command.
[mcp]
server = "weather-mcp"          # name resolved in mcp-config.toml
tool = "get_current_weather"    # upstream tool name on that server

# Optional: map local arg names -> the upstream tool's arg names.
[mcp.field_map]
city = "location"
```

### 2. Server registry — `mcp-config.toml`

Loaded from `./mcp-config.toml` (per-project, takes precedence) or
`~/.symbiont/mcp-config.toml` (user default):

```toml
[servers.weather-mcp]
command = "mcp-server-weather"
args = ["--units", "metric"]
env = { WEATHER_API_KEY = "..." }
# SchemaPin public key for this server's tools. Required for tools to pass
# verification under enforcement; omit only if you disable verification.
public_key_url = "https://example.com/.well-known/schemapin.pem"
```

## How a call flows

When an agent (via `symbi run` or the DSL `reason()`/`tool_call()` builtins)
proposes a tool call:

1. The reasoning loop's policy gate authorizes the call (fail-closed by
   default; see below).
2. `ToolCladExecutor` validates + field-maps the arguments per the manifest.
3. The `[mcp].server` name is resolved in the registry to a stdio launch spec.
4. The server subprocess is spawned; the MCP handshake runs; the tool schema is
   fetched.
5. **Verification (enforced by default):** the tool must be SchemaPin-verified
   (its schema carries a signature validated against the server's
   `public_key_url`) or already TOFU-pinned. First contact pins the provider
   key; a later key change for the same server is rejected. An unverified or
   unsigned tool is **blocked** — the call returns an error, it never executes.
6. The tool is invoked with the mapped arguments; the real result is wrapped in
   ToolClad's evidence envelope and returned to the loop.

Any failure at steps 3–5 (server not in registry, spawn failure, verification
failure, unknown tool) surfaces as an error observation — never a fabricated
success.

## Verification and local development

Verification is **enforced by default** (`enforce_mcp_verification = true`).
For local development against unsigned MCP servers you can disable it in code
via `ToolCladExecutor::with_mcp_verification(false)`. With enforcement on and no
`public_key_url` configured (or an unsigned tool), invocation is blocked
fail-closed — this is intentional: an unverifiable tool does not run.

## Policy gate

MCP tool calls go through the same reasoning-loop policy gate as any other
action. `symbi run` defaults to a fail-closed gate that denies tool calls
unless a Cedar policy (`policies/*.cedar`) allows them, or you opt into
permissive local mode with `SYMBI_INSECURE_ALLOW_ALL=1`. ToolClad manifests can
generate Cedar policy stubs (see `toolclad::cedar_gen`).

## Fallback (no tools configured)

If there is no `tools/` directory with manifests, the runner uses an honest
no-backend executor: it advertises no tools, and any proposed tool call returns
a clear error rather than a fabricated success.

## Deferred (later phases)

- HTTP/SSE MCP transport (v1 is stdio only).
- Typed upstream arguments (v1 sends all mapped arguments as JSON strings; an
  upstream tool whose schema expects a number/boolean/array/object should accept
  the string form or validate leniently).
- MCP tools in `symbi up`/shell (uses its own orchestrator toolset).
- A `.symbi` grammar construct for declaring servers inline.
- The `symbi-mcp` management CLI (`add`/`list`/`status`).
- Connection pooling (v1 spawns a fresh subprocess per invocation).
