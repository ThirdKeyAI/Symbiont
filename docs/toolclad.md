# ToolClad — Declarative Tool Contracts

ToolClad provides declarative tool contracts for the Symbiont runtime. Define what a tool does, what arguments it accepts, and how it executes — in a single `.clad.toml` manifest. The runtime handles argument validation, scope enforcement, secret injection, Cedar policy generation, and evidence capture.

## Why ToolClad

External tools (nmap, curl, whois, custom scripts) are useful but dangerous. Passing raw arguments to shell commands creates injection risk. Trusting tool output without validation creates integrity risk. ToolClad solves this by putting a contract between the agent and the tool.

The contract defines:
- **What arguments the tool accepts** — typed, validated, injection-checked
- **How the tool executes** — shell command, HTTP API, MCP proxy, interactive session, or browser
- **What scope the tool operates in** — target restrictions, domain allowlists
- **What policies govern it** — Cedar authorization, human approval gates
- **What output the tool produces** — format, schema validation, evidence envelope

## Quick start

### Create a tool manifest

```bash
symbi tools init my_tool
```

This creates `tools/my_tool.clad.toml` with a starter template.

### Validate and test

```bash
# Validate all manifests
symbi tools validate

# Validate a specific manifest
symbi tools validate tools/my_tool.clad.toml

# Dry-run with arguments (validates without executing)
symbi tools test my_tool --arg target=10.0.1.5

# View the MCP schema generated for a tool
symbi tools schema my_tool

# List all discovered tools
symbi tools list
```

### Use in the runtime

Tools in the `tools/` directory are automatically discovered and exposed to the reasoning loop as MCP-compatible tool definitions. Agents can call them like any other tool — the runtime handles validation, execution, and evidence capture.

---

## Manifest format

A `.clad.toml` file has up to 9 sections:

### [tool] — Metadata

```toml
[tool]
name = "nmap_scan"
version = "1.0.0"
binary = "nmap"
description = "Network port scanner"
mode = "oneshot"          # oneshot | session | browser
timeout_seconds = 60
risk_tier = "medium"      # low | medium | high
human_approval = false    # require operator approval before execution

[tool.cedar]
resource = "Tool::NmapScan"
action = "execute_tool"

[tool.evidence]
output_dir = "evidence/{_scan_id}"
capture = true
hash = "sha256"
```

### [args.*] — Argument definitions

Each argument has a type, validation rules, and metadata:

```toml
[args.target]
position = 1
type = "scope_target"     # validated against project scope
required = true
description = "Target IP, CIDR, or hostname"
scope_check = true        # enforce scope boundaries

[args.scan_type]
position = 2
type = "enum"
allowed = ["quick", "full", "stealth", "ping"]
default = "quick"
description = "Scan profile"

[args.max_rate]
position = 3
type = "integer"
min = 1
max = 10000
default = "1000"
description = "Maximum packets per second"
```

**Built-in argument types:**

| Type | Validation |
|------|-----------|
| `string` | Non-empty, no shell metacharacters, optional regex pattern |
| `integer` | i64 with optional min/max/clamp |
| `port` | 1–65535 |
| `boolean` | "true" or "false" |
| `enum` | Must match one of the `allowed` values |
| `scope_target` | IP, CIDR, or hostname — validated against project scope |
| `url` | Must contain "://", optional scheme whitelist |
| `path` | No `..` traversal, symlinks canonicalized |
| `ip_address` | Valid IPv4 or IPv6 |
| `cidr` | Valid CIDR notation with prefix validation |
| `credential_file` | File path that must exist on disk |
| `duration` | Integer with suffix (s/m/h), converted to seconds |
| `regex_match` | Custom regex from the `pattern` field |
| `agent_summary` | Free text bound for a downstream agent's prompt — strips invisible Unicode and renderer-hidden markup, rejects known injection markers. Best-effort defense-in-depth, **not** a load-bearing control (see [Typed + grounded decisions](#typed--grounded-decisions)) |

All types reject shell metacharacters: `;` `|` `&` `$` `` ` `` `(` `)` `{` `}` `[` `]` `<` `>` `!` `\n` `\r` `\0`

**Optional argument flags:**

| Flag | Meaning |
|------|---------|
| `scope_check = true` | Validate the value against the project scope (IP/CIDR/hostname args) |
| `feeds_decision = true` | This argument's value feeds a privileged downstream decision (routing, escalation, authorization). Free-text types (`string`, `agent_summary`, `regex_match`) marked this way are flagged as an anti-pattern by ToolClad manifest validation — use an `enum` plus Cedar grounding instead (see [Typed + grounded decisions](#typed--grounded-decisions)) |

**Custom types** can be defined in a project-level `toolclad.toml`:

```toml
[types.service_protocol]
base = "enum"
allowed = ["ssh", "ftp", "http"]
description = "Network service protocol"
```

### [command] — Command construction

The command section defines how to build the shell command from validated arguments:

```toml
[command]
template = "nmap {_scan_flags} --max-rate {max_rate} -oX - {target}"

[command.defaults]
max_rate = 1000

[command.mappings.scan_type]
quick = "-sV --top-ports 100"
full = "-sV -sC -p-"
stealth = "-sS -T2"
ping = "-sn -PE"

[command.conditionals.verbose_flags]
when = "verbose != ''"
template = "-v{verbose}"
```

Templates use `{arg_name}` placeholders. The runtime interpolates validated arguments and executes via direct argv (no shell invocation) to prevent injection.

### [output] — Output configuration

```toml
[output]
format = "xml"            # json | xml | csv | jsonl | text
parser = "builtin:xml"    # or path to custom parser binary
envelope = true           # wrap in evidence envelope

[output.schema]
type = "object"
properties.hosts.type = "array"
```

Schema validation produces warnings (non-blocking) when output doesn't match the expected structure.

---

## Execution modes

### Oneshot (default)

Single execution with a fixed timeout. Three backend options:

**Shell backend** — build and execute a command:
```toml
[tool]
mode = "oneshot"
binary = "nmap"

[command]
template = "nmap {target}"
```

**HTTP backend** — make an HTTP request:
```toml
[tool]
mode = "oneshot"

[http]
method = "GET"
url = "https://api.example.com/lookup?q={target}"
headers = { Authorization = "Bearer {_secret:api_key}" }
success_status = [200]
```

HTTP requests include SSRF protection — private IP ranges, localhost, and cloud metadata endpoints are blocked.

**MCP proxy backend** — governed passthrough to an upstream MCP tool:
```toml
[tool]
mode = "oneshot"

[mcp]
server = "upstream_server"
tool = "raw_tool_name"

[mcp.field_map]
target = "host"
scan_type = "mode"
```

### Session (interactive CLI)

Spawns a tool in a pseudo-terminal and maintains conversation state across multiple commands. Requires the `toolclad-session` feature.

```toml
[tool]
mode = "session"
binary = "msfconsole"

[session]
startup_command = "msfconsole -q"
ready_pattern = "msf6\\s*>"
startup_timeout_seconds = 30
idle_timeout_seconds = 300
max_interactions = 50

[session.commands.run]
pattern = "use {module}; set RHOSTS {target}; run"
description = "Run a Metasploit module"
risk_tier = "high"
human_approval = true
```

Each declared command becomes a separate MCP tool definition (e.g., `msfconsole.run`).

### Browser (CDP)

> **Status: not yet executable.** Browser (`mode = "browser"`) tools are parsed,
> argument-validated, and scope-checked, but Chrome DevTools Protocol execution
> is not implemented yet. It is gated behind the `toolclad-browser` cargo feature,
> whose real backend is still pending — until then a browser command returns an
> honest error, never a fabricated result. Declare browser tools now if you like;
> they will start executing once the CDP backend lands.

Headless or live Chrome DevTools Protocol for web interaction. Requires the `toolclad-browser` feature.

```toml
[tool]
mode = "browser"

[browser]
engine = "cdp"
headless = true
connect = "launch"
extract_mode = "accessibility_tree"

[browser.scope]
allowed_domains = ["example.com", "*.test.example.com"]
blocked_domains = ["admin.example.com"]
allow_external = false

[browser.commands.navigate]
description = "Navigate to URL"
```

Built-in browser commands: `navigate`, `snapshot`, `click`, `type_text`, `submit_form`, `extract`, `screenshot`, `execute_js`, `wait_for`, `go_back`, `list_tabs`, `network_timing`.

---

## Scope enforcement

ToolClad enforces target scope to prevent agents from operating outside authorized boundaries.

### Project scope

Define allowed targets in `scope/scope.toml`:

```toml
[scope]
targets = ["10.0.1.0/24", "192.168.1.0/24"]
domains = ["example.com", "*.test.example.com"]
exclude = ["10.0.1.1"]
```

Arguments with `scope_check = true` are validated against this scope. IPs are checked against CIDR ranges, hostnames against domain patterns (with wildcard suffix matching).

### URL scope (browser mode)

Browser tools enforce domain-level scope via `[browser.scope]`. Navigation to disallowed domains is blocked.

### SSRF protection (HTTP backend)

HTTP backend requests automatically block:
- Localhost (`127.0.0.1`, `::1`, `localhost`)
- Cloud metadata (`169.254.169.254`, `metadata.google.internal`)
- Private IP ranges (RFC 1918, link-local, broadcast)
- Non-HTTP/HTTPS schemes

---

## Secret injection

Secrets are injected into HTTP URLs, headers, and body templates using the `{_secret:NAME}` syntax:

```toml
[http]
url = "https://api.shodan.io/shodan/host/{target}?key={_secret:shodan_key}"
```

The runtime resolves `{_secret:shodan_key}` from the environment variable `TOOLCLAD_SECRET_SHODAN_KEY`. Missing secrets produce an error before execution.

---

## Cedar policy generation

ToolClad auto-generates Cedar policies from manifest metadata:

```bash
symbi tools schema my_tool  # includes Cedar policy suggestion
```

Generated policies vary by risk tier:

**Low-risk tool:**
```cedar
permit (principal, action == Tool::WhoisLookup::Action::"execute_tool", resource)
when { resource.tool_name == "whois_lookup" };
```

**High-risk tool with human approval:**
```cedar
permit (principal, action == Tool::Exploit::Action::"execute_tool", resource)
when { resource.tool_name == "exploit" && context.has_human_approval == true };
```

---

## Evidence envelope

All tool executions are wrapped in a structured evidence envelope:

```json
{
  "status": "success",
  "scan_id": "1712073600-a1b2c3d4",
  "tool": "nmap_scan",
  "command": "nmap -sV --top-ports 100 --max-rate 1000 -oX - 10.0.1.5",
  "duration_ms": 4523,
  "timestamp": "2026-04-02T18:00:00Z",
  "output_hash": "sha256:a1b2c3...",
  "exit_code": 0,
  "stderr": "",
  "results": { "hosts": [...] },
  "schema_warnings": []
}
```

HTTP backend envelopes include `http_method`, `http_url`, `http_status`. MCP proxy envelopes include `mcp_server`, `mcp_tool`, `status: "delegated"`. Session and browser envelopes include `session_id`, `interaction_count`, and a transcript with timestamps.

---

## Hot-reload (development)

In development mode, ToolClad watches the `tools/` directory for changes:

- New `.clad.toml` files are discovered automatically
- Modified manifests are reloaded on save
- Deleted manifests are removed from the registry
- Parse errors produce warnings without crashing the runtime

The executor also checks manifest version at execution time — if a manifest was reloaded between planning and execution, the call is rejected to prevent plan drift.

---

## Built-in manifests

Symbiont ships with four example manifests in `tools/`:

| Manifest | Description | Mode |
|----------|-------------|------|
| `whois_lookup.clad.toml` | WHOIS domain/IP lookup | Oneshot (shell) |
| `dig_lookup.clad.toml` | DNS record lookup | Oneshot (shell) |
| `curl_fetch.clad.toml` | HTTP request with scope enforcement | Oneshot (shell) |
| `nmap_scan.clad.toml` | Network port scanner with evidence capture | Oneshot (shell) |
| `submit_triage.clad.toml` | Typed + grounded triage decision (reference) | Reference (typed decision) |

These serve as reference implementations for writing your own manifests.

---

## Typed + grounded decisions

When one agent's output drives a **privileged downstream decision** — routing, escalation, or tool authorization — that decision must not be made over the upstream agent's free text. A free-text "summary" spliced into a downstream prompt is an injection surface: a held-out red-team evaluation showed the `agent_summary` marker fence reduces orchestrator-injection escape only from ~28% to ~26% (it does not generalize to novel paraphrases), while moving the decision onto typed fields grounded in trusted context reaches 0%.

The pattern that holds:

1. **Type the decision.** The upstream agent emits an `enum`-constrained decision (e.g. `category`, `severity`), never a free-text instruction. Enum validation makes paraphrase injection structurally inert — there is no instruction channel.
2. **Ground it in trusted context.** Derive the authoritative facts from trusted input (e.g. severity inferred from the ticket the system received, not from the agent's self-report), and make the decision a [Cedar policy](/security-model) over that trusted context. The lower-trust agent's output is advisory, not authoritative.
3. **Keep `agent_summary` as defense-in-depth only.** It still strips invisible Unicode / hidden markup and logs injection-shaped attempts, but it is never the load-bearing control for a privileged decision.

Mark any argument that feeds such a decision with `feeds_decision = true`. ToolClad manifest validation flags free-text decision inputs (`string`, `agent_summary`, `regex_match`) as an anti-pattern, steering authors to an `enum` plus Cedar grounding.

Reference implementation:

- `tools/submit_triage.clad.toml` — typed `category`/`severity` enums (`feeds_decision = true`), advisory-only `rationale`
- `examples/policies/triage_routing.cedar` — grounded escalation policy (`permit … when { context.ticket_severity == "critical" }`)
- `crates/runtime/src/toolclad/decision.rs` — the Rust derives-facts / Cedar-decides reference (`route_grounded`, `decide_route`)

---

## Integration with the reasoning loop

ToolClad tools are exposed to the [reasoning loop](/reasoning-loop) as MCP-compatible tool definitions. The flow:

1. **Discovery** — manifests in `tools/` are loaded at startup
2. **Schema generation** — each tool (or session/browser command) becomes a ToolDefinition with JSON Schema
3. **Tool profile filtering** — the ORGA loop's tool curator filters available tools based on glob patterns, max count, and `require_verified` flags
4. **Proposal** — the LLM proposes a tool call with arguments
5. **Policy gate** — [Cedar policy](/security-model) evaluates whether the call is allowed
6. **Validation** — ToolClad validates arguments against the manifest (types, ranges, scope, injection checks)
7. **Execution** — the runtime executes via the appropriate backend (shell argv, HTTP, MCP proxy, session, browser)
8. **Evidence** — output is parsed, schema-validated, and wrapped in an evidence envelope
9. **Observation** — the result is returned to the reasoning loop as an Observation

The policy gate and argument validation happen before execution. A failed policy check or invalid argument blocks the tool call entirely.
