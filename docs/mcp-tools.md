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
   its SchemaPin public-key URL or provisioned public key.

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
# verification under enforcement, unless public_key_pem is provisioned instead.
public_key_url = "https://example.com/.well-known/schemapin.pem"
```

## How a call flows

When an agent (via `symbi run` or the DSL `reason()`/`tool_call()` builtins)
proposes a tool call:

1. ToolClad validates and normalizes the arguments against the registered
   manifest and freezes the contract, server registry and sandbox profile.
   Declared input files become bounded immutable snapshots; their paths, hashes
   and optional new output path are included in the prepared call.
   The reasoning loop's policy gate authorizes that prepared call fail-closed.
2. `ToolCladExecutor` field-maps the authorized arguments to the upstream names
   and JSON types. An `integer`/`number`/`boolean`/`array`/`object` argument is
   sent as the matching JSON type, not a quoted string.
3. The `[mcp].server` name is resolved in the registry to a stdio launch spec.
4. The server starts inside the selected sandbox with only its declared file
   snapshots and optional staged output; the MCP handshake runs and
   the tool schema is fetched over the live session.
5. **Verification (enforced by default):** the tool must be SchemaPin-verified
   (its schema carries a signature validated against the server's
   `public_key_url` or provisioned key) using the exact key checked against the
   persistent provider pin. A pin
   alone never substitutes for a valid schema signature. First contact pins the provider
   key; a later key change for the same server is rejected. An unverified or
   unsigned tool is **blocked** — the call returns an error, it never executes.
6. The tool is invoked on the same live connection with the mapped arguments.
   After success and confirmed worker cleanup, the runtime atomically publishes
   the declared new output without overwriting an existing file. The real result
   and `created_files` receipts are returned in ToolClad's evidence envelope.

Any failure at steps 3–5 (server not in registry, spawn failure, verification
failure, unknown tool) surfaces as an error observation — never a fabricated
success.

Container workers receive the selected image's environment and the explicit
`env` map in the server registry; parent credentials are not inherited. The
selected profile bounds stdout and stderr before decoding (10 MiB by default)
and caps the worker lifetime. The invocation deadline covers startup,
discovery, verification and execution. An exhausted deadline prevents spawn.
Completion and cancellation remove the container, including detached
processes. See the lifecycle limits below.

## File access

Configured Docker/gVisor mounts are ceilings. A tool with no `[filesystem]`
declaration receives no host mounts. For a tool taking local `input` and `output`
arguments, add:

```toml
[filesystem]
read = ["{input}"]
create = ["{output}"]
max_file_bytes = 8388608
```

These paths are relative to the worker working directory. Inputs are read-only
snapshots; the output must be a new file. A server can use private scratch files
without publishing them. Startup writes, failed signature verification, MCP
errors and cancellation never publish the output. Host-mounted executables and
directory-wide file browsing must migrate to image-provisioned dependencies and
explicit individual files. Standalone SDK discovery also receives no host mounts;
SDK callers use `verified_invoke_with_files` for explicit invocation grants.
See [filesystem grants](filesystem-grants.md) for bounds, SDK behavior and the
remaining Firecracker and development-host limitations.

## Verification and local development

Verification is **enforced by default** (`enforce_mcp_verification = true`),
including the SDK's `call_tool`. Local development against unsigned servers
can explicitly use `ToolCladExecutor::with_mcp_verification(false)`; production
rejects that opt-out. With enforcement enabled, a missing key or unsigned
schema blocks invocation. Host execution requires an explicit development
boundary and is also refused in production.

## Policy gate

MCP tool calls go through the same reasoning-loop policy gate as any other
action. `symbi run` defaults to a fail-closed gate that denies tool calls
unless a Cedar policy allows them, or you opt into permissive local mode with
`SYMBI_INSECURE_ALLOW_ALL=1`. ToolClad manifests can generate Cedar policy
stubs (see `toolclad::cedar_gen`).

Policies in `policies/*.cedar` are loaded by every surface's gate; policies in
`policies/<surface>/` are read only by the surface they name. Prefer the
scoped form for anything tool-specific — see
[Which surfaces execute tools](/toolclad#which-surfaces-execute-tools) for the
full list of which entry points actually run MCP-backed tools. The `symbi up`
chat coordinator is not one of them.

## Fallback (no tools configured)

If there is no `tools/` directory with manifests, the runner uses an honest
no-backend executor: it advertises no tools, and any proposed tool call returns
a clear error rather than a fabricated success.

## Deferred (later phases)

- HTTP/SSE MCP transport (v1 is stdio only).
- MCP tools in `symbi up`/shell (uses its own orchestrator toolset).
- A `.symbi` grammar construct for declaring servers inline.
- The `symbi-mcp` management CLI (`add`/`list`/`status`).
- Connection pooling (v1 spawns a fresh subprocess per invocation).

## Contained stdio sessions

ToolClad and the static `RmcpStdioClient` SDK entrypoints use Docker by default.
Project `[sandbox]` configuration selects the ToolClad boundary; the SDK's
`*_with_boundary` methods accept an explicit `CommandBoundary`. gVisor uses the
same transport with its configured OCI runtime. Provision executables and
dependencies in the selected image. Explicit per-operation input files may
include scripts where the invocation grants them, but standalone discovery has
no such grant. Host binaries and credentials are not implicitly exposed.

Each Docker/gVisor invocation owns one container from discovery through SchemaPin verification
and the tool call. Initialization creates and inspects the stopped container
before attaching stdin/stdout. If cancellation arrives during creation, the cleanup owner waits for creation
to finish and removes the container without starting the payload. An independent
owner enforces the container lifetime, including when the MCP operation has a
longer timeout. Output is bounded before JSON decoding, and cleanup removes the
container and detached descendants. Required cleanup failures are returned as
errors. The independent sandbox supervisor retains ownership after host runtime loss.

`call_tool` now requires SchemaPin verification. An unverified call must explicitly
use the verification option, and disabling verification is refused in production.
Explicit development host execution also remains unavailable in production.
The ToolClad registry-injection helper checks the registered manifest and validates
arguments; supplying a replacement manifest cannot bypass its approval flags.

## Provisioned public trust anchors

A server may use an operator-provisioned public key instead of fetching a key over
HTTPS. This supports offline deployments and deterministic signed-tool fixtures:

```toml
[servers.example]
command = "/usr/local/bin/example-mcp"
args = []
public_key_pem = """-----BEGIN PUBLIC KEY-----
...operator-provisioned public key...
-----END PUBLIC KEY-----"""
```

The key comes only from trusted operator configuration. It is included in the
prepared contract and is not supplied by the MCP server or inherited by its
process. Schema signatures are verified with that exact key. The existing
persistent provider pin must also match before invocation; changing a key cannot
silently rotate an existing pin. Invalid signatures never establish a pin.
Without a provisioned key, guarded HTTPS discovery and the persistent pin still
apply. This option configures SchemaPin trust and does not replace AgentPin's
principal identity or Cedar authorization.

The escape harness's `verify_runtime_mcp.py` drives the actual `symbi run` command
with enforced signature verification, a provisioned public key, scoped Docker
mounts, scripted inference, and synthetic external observers. Its private signing
keys are generated temporarily and removed before the worker runs. It does not
certify containment of unrelated execution surfaces.

### Firecracker stdio

Selecting `sandbox.tier = "firecracker"` starts the MCP server in a fresh guest
using the same command/args/env registry entry and SchemaPin verification flow.
The executable and dependencies must be present in the operator image. Discovery,
verification and invocation share one process; the guest has no host mount or
network device. Bounded duplex streams deliver live output before stdin closes.
The independent supervisor retains the VM through cancellation and waits for
removal before the call completes. Invalid or oversized frames, unexpected guest
outcomes and failed cleanup cannot produce a successful tool result. See
[Firecracker setup](firecracker-setup.md) for the matching guest protocol and SDK
stream lifetime contract. VM PTY and managed CLI workers also use this transport;
isolated browser execution remains unavailable.

Native signature verification recursively orders schema object keys using the
SchemaPin canonical representation. Formatting or object-key order can change
in transit; array order and values remain signed. Native signing writes a complete
signed JSON file and reports the fingerprint derived from the actual signing key.
Existing output files are refused. Re-sign schemas that previously signed raw,
noncanonical JSON bytes before using them with this verifier.
