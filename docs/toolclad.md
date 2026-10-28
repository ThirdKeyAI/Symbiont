# ToolClad — Declarative Tool Contracts

ToolClad provides declarative tool contracts for the Symbiont runtime. Define what a tool does, what arguments it accepts, and how it executes — in a single `.clad.toml` manifest. The runtime handles argument validation, scope enforcement, secret injection, Cedar policy generation, and evidence capture.

Upgrading from 1.x? Read the [2.0 migration guide](migration-2.0.md) before
replacing manifests or Rust integrations.

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

A `.clad.toml` file has up to 9 sections. The format follows the ToolClad
reference specification, plus Symbiont extensions (`[source]`, `[filesystem]`,
`feeds_decision`, `finalize`, `[browser.network]`, `[session] sentinel_command`,
and the `literal_text` / `agent_summary` types).

**Loading is strict.** A manifest is a security contract, so an unknown key
anywhere (`tiemout_seconds`, `block_internal_typo`) is a load error, as is any
control Symbiont cannot enforce. When one manifest in `tools/` fails to load,
the whole tool set fails: `symbi up` and `symbi tools list` exit with the
error, and a reasoning run refuses to start, instead of silently running
without that tool. `toolclad.toml` custom types are strict in the same way.

### [tool] — Metadata

```toml
[tool]
name = "nmap_scan"
version = "1.0.0"
binary = "nmap"           # argv[0]'s basename must match
description = "Network port scanner"
mode = "oneshot"          # oneshot | session | browser
timeout_seconds = 60      # must be greater than zero (default 30)
risk_tier = "medium"      # info | low | medium | high | critical
human_approval = false    # require operator approval before execution
network_egress = true     # the tool dials destinations from its arguments
allow_interpreter = false # permit sh/python/env/... as argv[0]

[tool.cedar]
resource = "Tool::NmapScan"
action = "execute_tool"

[tool.evidence]
output_dir = "{evidence_dir}/{scan_id}-nmap"
capture = true
hash = "sha256"

[tool.limits]             # applied to the selected sandbox boundary
nproc = 64
nofile = 256
fsize_bytes = 10485760
address_space_bytes = 536870912
cpu_seconds = 120
```

- `risk_tier = "high"` or `"critical"` (on the manifest or on a session or
  browser command) requires an exact-call human approval, like
  `human_approval = true`.
- An interpreter as `argv[0]` (`sh`, `bash`, `python`, `node`, `env`, `xargs`,
  ...) turns an argument into code and is refused unless
  `allow_interpreter = true`.
- `[tool.limits]` tightens the selected tier (Docker/gVisor: PIDs, open
  files, file size, memory, lifetime; Landlock: PIDs, memory, lifetime;
  Firecracker: memory, lifetime). A limit the tier cannot express refuses the
  call rather than running without it.
- `dispatch` is accepted only as `"exec"`.

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
| `string` | Non-empty, string hygiene (below), shell-metacharacter blocklist, optional anchored `pattern` |
| `integer` | Plain decimal i64 with optional `min`/`max`/`clamp`; a negative result needs `allow_leading_dash` |
| `number` | Decimal float with optional `min_float`/`max_float`/`clamp`; NaN and infinity refused |
| `port` | 1–65535 |
| `boolean` | "true" or "false" |
| `enum` | Must match one of the `allowed` values |
| `scope_target` | Canonical IP, CIDR, or ASCII hostname (no wildcards, punycode, traversal, non-canonical IP encodings such as `2130706433` or `127.1`); egress policy applies; always checked against the project scope |
| `url` | Scheme allowlist (default `http`/`https`; `file`, `gopher`, `data`, `javascript`, ... are always refused), no userinfo, no backslash or whitespace, host through the egress policy, and the WHATWG parse must agree with the lexical host and re-serialize to the same string. The canonical URL is what reaches argv |
| `path` | Relative path; rejects absolute/rooted paths and `..` components; preserves the validated spelling |
| `ip_address` | Canonical IPv4 or IPv6; egress policy applies |
| `cidr` | Canonical CIDR; the whole range is tested against the egress policy |
| `credential_file` | Relative path that must name an existing regular file on the controller |
| `msf_options` | One `set KEY VALUE` (Symbiont refuses `;` chaining) |
| `duration` | Seconds, or `h`/`m`/`s`/`ms` components (`1h30m`), normalized to whole seconds |
| `literal_text` | Explicit bounded UTF-8 content (32 KiB), preserving whitespace and metacharacters; rejects NUL. Use with a pre-tokenized argv contract. An interpreter such as `sh -c` deliberately treats it as code and requires `allow_interpreter`, the appropriate policy, sandbox and approval. |
| `regex_match` | Must match the anchored `pattern` |
| `agent_summary` | Free text bound for a downstream agent's prompt — strips invisible Unicode and renderer-hidden markup, rejects known injection markers. Best-effort defense-in-depth, **not** a load-bearing control (see [Typed + grounded decisions](#typed-grounded-decisions)) |

**String hygiene.** "Valid according to the type" means "safe as exactly one
argv entry". Every string-derived type (`string`, `path`, `credential_file`,
`regex_match`, `url`, `scope_target`, `duration`, `msf_options`) refuses,
before any type-specific parsing and with nothing trimmed away:

| Rule | Refused | Opt-out |
|---|---|---|
| NUL | `\0`, for every type | none |
| Control characters | U+0000–U+001F, U+007F–U+009F | none |
| Quotes and backslash | `'` `"` `\` | none |
| Whitespace | any whitespace | `allow_whitespace = true` admits U+0020 only, and only with the `exec` command form (never for `url`) |
| Leading dash | a value starting with `-` or `+` | `allow_leading_dash = true` |

The shell-metacharacter blocklist (`;` `|` `&` `$` `` ` `` `(` `)` `{` `}`
`[` `]` `<` `>` `!`) stays on top as defense in depth. `pattern` is anchored:
`[a-z]+` is compiled as `^(?:[a-z]+)$`, a whole-value match. The explicit
`literal_text` type is exempt and preserves its content as data.

**Egress policy.** For `scope_target`, `url`, `ip_address` and `cidr`:

- *always refused*: link-local and cloud metadata (`169.254.0.0/16`,
  `fe80::/10`, their IPv4-mapped forms) and the internal-name set
  (`localhost`, `*.localhost`, `*.local`, `*.internal`, `*.home.arpa`,
  `*.lan`, `*.intranet`, `*.corp`, `*.private`, `metadata.google.internal`);
- *refused with `block_internal = true`*: loopback, RFC 1918, CGNAT,
  unspecified, broadcast, documentation, benchmarking and reserved ranges,
  IPv6 equivalents, any address Symbiont's HTTP guard treats as non-public,
  IP literals embedded in a hostname (`127.0.0.1.nip.io`, `10-0-0-1.nip.io`),
  any CIDR that *overlaps* a blocked range (`8.0.0.0/5` contains `10.0.0.0/8`),
  and a hostname unless every A/AAAA answer is public (resolution failure is
  a refusal).

A tool is an *egress tool* when it sets `network_egress = true`, uses `[http]`
or `[browser]`, or runs a known network binary (`curl`, `wget`, `nmap`, `nc`,
`masscan`, `ssh`, `nikto`, `hydra`, `ffuf`, ..., plus `dig` and `whois`). In an
egress tool every network-target argument must set `block_internal = true`,
and a custom type cannot stand in for one; otherwise the manifest does not load.

Path validation is lexical and portable. It does not canonicalize against the
controller filesystem: a relative path remains relative to the selected
worker's working directory. Ordinary dotted filenames such as `report..csv`
remain valid. For intentional absolute argv data, declare `literal_text`
explicitly and apply the appropriate policy and sandbox restrictions.

`credential_file` adds a controller-side regular-file preflight and preserves the
relative argument. It follows filesystem links and does not prove that a file
exists or is confined in the worker. Mounts, working directory and the selected
sandbox define the worker's filesystem authority; path syntax alone does not
prevent a tool from following symlinks within that authority.

**Optional argument flags:**

| Flag | Meaning |
|------|---------|
| `scope_check = true` | Validate the value against the project scope (`scope_target`, `url`, `ip_address`, `cidr` only; `scope_target` is always scope-checked) |
| `block_internal = true` | Refuse non-public targets (see the egress policy) |
| `allow_leading_dash = true` | Permit a value starting with `-` or `+` |
| `allow_whitespace = true` | Permit U+0020 inside the value (`exec` form only) |
| `schemes = [...]` | `url` scheme allowlist |
| `sanitize = ["injection"]` | Accepted for compatibility; the blocklist always applies to string-derived types |
| `feeds_decision = true` | This argument's value feeds a privileged downstream decision (routing, escalation, authorization). Free-text types (`string`, `literal_text`, `agent_summary`, `regex_match`) marked this way are flagged as an anti-pattern by ToolClad manifest validation — use an `enum` plus Cedar grounding instead (see [Typed + grounded decisions](#typed-grounded-decisions)) |

`allowed` is honored only on `enum`; on any other type it is a load error.

**Custom types** can be defined in a project-level `toolclad.toml`. Accepted
keys are `base`, `allowed`, `pattern`, `min`, `max`, `min_float`, `max_float`,
`clamp`, `schemes`, `scope_check` and `description`; a custom type may not
shadow a built-in type:

```toml
[types.service_protocol]
base = "enum"
allowed = ["ssh", "ftp", "http"]
description = "Network service protocol"
```

### [command] — Command construction

The argv array is the only form that reaches a process. Declare it with `exec`:

```toml
[command]
exec = [
    "nmap",
    "{_scan_type_flags}",
    "--max-rate", "{max_rate}",
    "-oX", "-",
    "{target}",
]
end_of_options = "--"     # optional; inserted before the first agent value

[command.defaults]
max_rate = 1000

[command.mappings.scan_type]
quick = "-sV --top-ports 100"
full = "-sV -sC -p-"
stealth = "-sS -T2"
ping = "-sn -PE"

[command.conditionals.verbose_flags]
when = "verbose != ''"
template = "-v {verbose}"
```

- Each `exec` element is one argv entry. An argument placeholder
  (`{target}`) resolves to exactly one entry and is never re-split, so a
  value with spaces or quotes cannot add arguments.
- `{_<arg>_flags}` (from `[command.mappings.<arg>]`) and `{_<cond>}` (from
  `[command.conditionals.<cond>]`) are manifest text and may expand to zero or
  more entries; they must stand alone as a whole element. No other spelling
  (such as `{_scan_flags}`) is recognized. A mapping over an `enum` must cover
  every allowed value (map a value to `""` for no flags).
- `{{` and `}}` are literal braces (`"-w", "%{{http_code}}"`). Any other brace,
  and any placeholder that names nothing the manifest declares, is a load
  error.
- `{_scan_id}` is injected (128 bits of entropy). ToolClad's host-path
  variables `{_output_file}` and `{_evidence_dir}` are refused: tools run inside
  an isolated boundary, so write results to stdout, which is captured.
- `end_of_options = "--"` is the only accepted value.
- `template = "..."` is deprecated. It is tokenized at load, before any value
  exists; a placeholder inside quotes (`-H '{headers}'`) is refused, and
  `allow_whitespace` is not honored for it.
- Divergence from the reference, kept because it is stricter: an empty
  optional argument keeps its (empty) argv slot instead of being dropped, so
  `-H {headers} {url}` can never turn into `-H <url>`. Omit optional syntax
  with a conditional instead.

`symbi tools test <name> --arg k=v` validates exactly as the executor does and
prints the argv that will run, element by element.

### [output] — Output configuration

```toml
[output]
format = "xml"            # json | xml | csv | jsonl | text
parser = "builtin:xml"    # builtin:* or custom:<absolute path> (operator allowlist)
envelope = true           # wrap in evidence envelope
max_bytes = 8388608       # per-stream capture cap (default 8 MiB, at most 1 GiB)

[output.schema]
type = "object"
properties.hosts.type = "array"
```

Schema validation is **fail-closed**: a violation replaces `results` with
`{"schema_error": "<pointer>: <message>", "raw_output": ...}` and sets the
envelope status to `schema_error`. The bundled validator supports `type`,
`properties`, `required`, `items`, `additionalProperties` (boolean), `enum`,
`minimum`, `maximum`, `minLength`, `maxLength` and `pattern` (anchored).
Output beyond `max_bytes` is truncated and flagged with
`"output_truncated": true`. XML containing `<!DOCTYPE` or `<!ENTITY` is
refused. A parser that is neither `builtin:*` nor `custom:<absolute path>` is
a load error; custom parsers run only when listed in
`SYMBIONT_TOOLCLAD_ALLOWED_PARSERS` and inside the command boundary.

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
exec = ["nmap", "{target}"]
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

The scheme and host of `http.url` are literal manifest text; a placeholder in
either is a load error. Argument values are percent-encoded into the path and
query, JSON-escaped into `body_template`, and a header value containing CR, LF
or NUL after interpolation is refused. With a project scope, the resolved URL
must be in scope. HTTP requests also include SSRF protection — private IP
ranges, localhost, and cloud metadata endpoints are blocked.

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

Spawns a real pseudo-terminal inside the selected Docker/gVisor container and maintains state across commands in one governed run. Requires `toolclad-session`, enabled in the default CLI. See [terminal boundaries and SDK lifecycle](interactive-terminal-boundary.md) for image requirements, limits and cleanup.

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
pattern = "help"
description = "Show command help"
risk_tier = "low"
human_approval = false
```

Each declared command becomes a separate MCP tool definition (e.g., `msfconsole.run`).

Session input rules (ToolClad S1–S3), applied identically on every path:

- The input is one line: C0 controls (tab, CR and LF included), DEL, C1
  controls and U+2028/U+2029 are refused; the executor appends the only line
  terminator. The command `pattern` must match the whole input.
- Sink policies, inferred from the startup binary and extendable with
  `[session.interaction] input_sanitize`: `psql_meta` (psql sessions; no
  backslash, so `\gexec`, `\c`, `\copy`, `\lo_import`, `\o` and `\!` cannot be
  expressed), `msf_meta` (msfconsole sessions; no `;` chaining and no `irb`,
  `pry`, `load`, `resource`, `makerc`, `edit`, `spool`, `save`, `cd`, shell
  verbs or `!`), and `injection` (the shell-metacharacter blocklist, opt-in
  only, so SQL such as `count(*)` or `a != b` is not refused by default).
  msfconsole passes unknown verbs to the system shell, so pin the verb in each
  command's `pattern`.
- In a session whose binary can name a network destination (`msfconsole` and
  the egress binaries), every command must set `extract_target = true` with a
  `(?P<target>...)` group, which is checked against the project scope, or
  require human approval.
- The session sandbox has no network unless `[tool] network_egress = true`.
- The prompt is detected with a fresh 128-bit sentinel per interaction: the
  executor sends `sentinel_command` (psql defaults to `\echo {sentinel}`) after
  the input, and accepts `ready_pattern` only at the start of the final line
  after the sentinel line, so prompt-shaped tool output cannot frame a
  response. Without a sentinel command the prompt must be the final,
  unterminated line (`prompt_anchor: "end_of_output"`).
- Terminal output is stripped of CSI, OSC, DCS/SOS/PM/APC strings and every
  other C0/C1 control except newline and tab.
- `[session.child_detection]` is refused: a child session is never registered
  from tool output (S7).

### Browser (CDP)

> **Status: not yet executable.** Browser (`mode = "browser"`) tools are parsed,
> argument-validated, and scope-checked, but Chrome DevTools Protocol execution
> is not implemented yet. It is gated behind the `toolclad-browser` cargo feature,
> whose real backend is still pending — until then a browser command returns an
> honest error, never a fabricated result. Private transport fixtures can drive
> contained Chromium, but they do not enable the public browser executor.

The manifest describes the intended Chrome DevTools Protocol interface.
Neither `connect = "launch"` nor live attachment starts a browser today.

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

[browser.network]
allowed_methods = ["GET", "HEAD"]
private_origins = []

[browser.commands.navigate]
description = "Navigate to URL"
```

Planned browser commands: `navigate`, `snapshot`, `click`, `type_text`, `submit_form`, `extract`, `screenshot`, `execute_js`, `wait_for`, `go_back`, `list_tabs`, `network_timing`.

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

Every `scope_target` argument, and every argument with `scope_check = true`,
is validated against this scope; with no scope file the call is refused. The
whole requested range must fit an allowed rule and avoid every exclusion. An
allow rule `*.example.com` matches strict subdomains only — not the apex
`example.com` and not `evil-example.com`; an exclusion `*.example.com` also
covers the apex. URLs are scope-checked with the same strict parser the `url`
type uses, so a backslash or userinfo cannot give the scope check one host and
the tool another.

The ToolClad reference keys are accepted too and merge into the rules above:

```toml
[scope]
allowed_domains = ["example.com", "*.example.com"]
allowed_cidrs = ["10.0.1.0/24"]
blocked_domains = ["admin.example.com"]
blocked_cidrs = ["10.0.1.1/32"]
```

### URL scope (browser mode)

Navigation requires `[browser.scope]` and an HTTP(S) URL without embedded
credentials. Domain comparisons use canonical hostnames, including case,
internationalized names, encoded host characters and a final DNS dot. Blocked
rules take precedence; `*.example.com` matches the base domain and its
subdomains. Malformed rules fail closed, including when `allow_external = true`.
At most 256 domain rules and an 8 KiB URL are accepted. Control characters,
backslashes and ambiguous URL forms are rejected.

Prepared navigation binds the normalized URL before policy, approval and audit.
Its prepared context also includes the effective `[browser.network]` settings;
the manifest digest binds these capabilities through authorization. GET and HEAD
are the default methods. Other supported methods (POST, PUT, PATCH, DELETE and
OPTIONS) require explicit inclusion; CONNECT and TRACE are refused.

Domain scope does not grant access to private addresses. An operator can grant an
exact literal origin, such as `private_origins = ["http://127.0.0.1:8080"]`, and
also include its address in `allowed_domains`. The scheme, address and effective
port must match. The exception accepts loopback, RFC1918 and IPv6 unique-local
addresses, excluding the AWS IPv6 metadata endpoint. DNS names, link-local,
multicast, unspecified and mapped IPv6 addresses cannot use it. A redirect to
another port or origin needs its own grant. Blocked domain rules still win.

The reusable `browser_network::BrowserNetworkPolicy` builds scoped requests from
CDP interception events. It removes authority/framing and hop-by-hop headers,
requires complete bounded request bodies, and refuses multipart/file uploads.
Requests are limited to 2 MiB of application data, responses to 4 MiB, and headers
to 128 entries / 64 KiB. The broker client uses DNS filtering, no ambient proxy,
no redirect following and no automatic content decompression. Responses preserve
duplicate cookies and binary header values for CDP fulfillment. Chromium receives
redirect responses and each resulting request must be checked again.

These request-construction helpers do not issue execution authority. A trusted
driver must retain the worker, mediate every target and use the call-bound journal
before sending each unchanged request. The public browser route stays unavailable
while that production driver and browser lifetime ownership are integrated.
The [contained Chromium transport fixtures](https://github.com/ThirdKeyAI/Symbiont/blob/main/crates/runtime/tests/fixtures/browser/README.md)
include a real local HTTP case with pre-connection signed audit, an exact POST
effect, denied method/subresource/redirect destinations and worker removal.
This is component integration evidence, not a shipping browser-command E2E.

### SSRF protection (HTTP backend)

HTTP backend requests automatically block:
- Localhost (`127.0.0.1`, `::1`, `localhost`)
- Cloud metadata (`169.254.169.254`, `metadata.google.internal`)
- Shared address space (`100.64.0.0/10`), including `100.100.100.200`, in both URL checks and DNS answers
- Private IP ranges (RFC 1918, link-local, broadcast)
- Non-HTTP/HTTPS schemes

HTTP exchange uses asynchronous I/O, a bounded request and response body, and the
remaining call deadline. Automatic redirects and ambient proxies stay disabled.
Preparation resolves the method and canonical destination before policy and
approval, including path segments that the HTTP client would normalize. URL
userinfo credentials are rejected; use explicit headers. URLs are limited to
8 KiB without control characters.
Governed calls record the built request before connecting and record the actual
response or incomplete outcome through their existing protected journal. See
[request audit records](run-audit.md). No new approval or UI control is required.

---

## Secret injection

Legacy HTTP manifests can inject secrets into trusted headers and body templates
using `{_secret:NAME}`. Secret placeholders in URLs are rejected:

```toml
[http]
url = "https://cases.example.com/records/{record}"
[http.headers]
authorization = "Bearer {_secret:case_service}"
```

The runtime resolves `{_secret:case_service}` from `TOOLCLAD_SECRET_CASE_SERVICE`.
Missing secrets produce an error before execution. The optional
[outbound capability profile](outbound-capabilities.md) replaces these templates
with an explicit credential source and a named operation bound to the current
execution. It does not change legacy manifests until the operator opts in.

---

## Cedar policy generation

ToolClad auto-generates Cedar policies from manifest metadata:

```bash
symbi tools schema my_tool  # includes Cedar policy suggestion
```

Each generated policy permits the manifest's action when
`resource.base_tool` names the manifest, which every session and browser
sub-command shares (their per-call `tool_name` is `<manifest>.<command>`). The
prepared resource also carries the call's effective `risk_tier` (the manifest
tier raised to a sub-command's own tier). Forbids carry the approval rules:

```cedar
permit (principal, action == Tool::Exploit::Action::"execute_tool", resource)
when { resource has base_tool && resource.base_tool == "exploit" };

forbid (principal, action == Tool::Exploit::Action::"execute_tool", resource)
when { resource has base_tool && resource.base_tool == "exploit" &&
       resource has risk_tier && (resource.risk_tier == "high" || resource.risk_tier == "critical") }
unless { context has has_human_approval && context.has_human_approval == true };
```

A manifest with `human_approval = true` gets an additional forbid without the
tier condition. Independently of Cedar, a `high` or `critical` call requires an
exact-call approval before dispatch.

---

## Evidence envelope

All tool executions are wrapped in a structured evidence envelope:

```json
{
  "status": "success",
  "scan_id": "1712073600-9f1c2e7a4b6d8e0f1a2b3c4d5e6f7a8b",
  "tool": "nmap_scan",
  "command": "nmap -sV --top-ports 100 --max-rate 1000 -oX - 10.0.1.5",
  "duration_ms": 4523,
  "timestamp": "2026-04-02T18:00:00Z",
  "output_hash": "sha256:a1b2c3...",
  "output_truncated": false,
  "exit_code": 0,
  "stderr": "",
  "output_file": "/var/lib/symbiont/evidence/1712073600-9f1c...-nmap/output",
  "results": { "hosts": [...] }
}
```

`status` is `success`, `error`, or `schema_error`. With `[tool.evidence]
capture = true`, the captured output is also written to `output_file` under the
evidence root (`TOOLCLAD_EVIDENCE_DIR`, or `symbiont-evidence-<uid>` in the
system temp directory). The root and each per-call directory are created mode
0700 and the root is refused if it is a symlink, foreign-owned, or group/other
accessible; the file is created with `O_CREAT | O_EXCL | O_NOFOLLOW`, mode 0600.
`output_dir` may use only `{evidence_dir}` and `{scan_id}` and must stay below
the root.

HTTP backend envelopes include `http_method`, `http_url`, `http_status`. MCP proxy envelopes include `mcp_server`, `mcp_tool`. Session envelopes include `session_id`, `interaction_count`, `prompt_anchor`, and a transcript with timestamps.

---

## Hot-reload (development)

In development mode, ToolClad watches the `tools/` directory for changes:

- New `.clad.toml` files are discovered automatically
- Modified manifests are reloaded on save
- Deleted manifests are removed from the registry
- Parse errors produce warnings without crashing the runtime; the previous
  version of the manifest stays in effect

The executor also checks manifest version at execution time — if a manifest was reloaded between planning and execution, the call is rejected to prevent plan drift.

---

## Built-in manifests

Symbiont ships these manifests in `tools/`:

| Manifest | Description | Mode | Loads in ToolClad's reference CLI |
|----------|-------------|------|------|
| `whois_lookup.clad.toml` | WHOIS domain/IP lookup | Oneshot (shell) | Yes |
| `dig_lookup.clad.toml` | DNS record lookup | Oneshot (shell) | Yes |
| `curl_fetch.clad.toml` | HTTP request with scope and egress policy | Oneshot (shell) | Yes |
| `nmap_scan.clad.toml` | Network port scanner with evidence capture | Oneshot (shell) | Yes |
| `submit_triage.clad.toml` | Typed + grounded triage decision (reference) | Reference (typed decision) | No (`feeds_decision`) |
| `read_file`, `list_files`, `grep_files`, `git_*` | Fixed source-broker queries | Oneshot (source) | No (`[source]`) |

These serve as reference implementations for writing your own manifests.

---

## Typed + grounded decisions

When one agent's output drives a **privileged downstream decision** — routing, escalation, or tool authorization — that decision must not be made over the upstream agent's free text. A free-text "summary" spliced into a downstream prompt is an injection surface: a held-out red-team evaluation showed the `agent_summary` marker fence reduces orchestrator-injection escape only from ~28% to ~26% (it does not generalize to novel paraphrases), while moving the decision onto typed fields grounded in trusted context reaches 0%.

The pattern that holds:

1. **Type the decision.** The upstream agent emits an `enum`-constrained decision (e.g. `category`, `severity`), never a free-text instruction. Enum validation makes paraphrase injection structurally inert — there is no instruction channel.
2. **Ground it in trusted context.** Derive the authoritative facts from trusted input (e.g. severity inferred from the ticket the system received, not from the agent's self-report), and make the decision a [Cedar policy](/security-model) over that trusted context. The lower-trust agent's output is advisory, not authoritative.
3. **Keep `agent_summary` as defense-in-depth only.** It still strips invisible Unicode / hidden markup and logs injection-shaped attempts, but it is never the load-bearing control for a privileged decision.

Mark any argument that feeds such a decision with `feeds_decision = true`. ToolClad manifest validation flags free-text decision inputs (`string`, `literal_text`, `agent_summary`, `regex_match`) as an anti-pattern, steering authors to an `enum` plus Cedar grounding.

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
7. **Execution** — the runtime executes via the appropriate backend (shell argv, HTTP, MCP proxy, session, browser). Only a call carrying an authorization grant from the governed dispatcher executes; a raw proposal handed to `ToolCladExecutor::execute_actions` is refused
8. **Evidence** — output is parsed, schema-validated, and wrapped in an evidence envelope
9. **Observation** — the result is returned to the reasoning loop as an Observation

The policy gate and argument validation happen before execution. A failed policy check or invalid argument blocks the tool call entirely.

## Which surfaces execute tools

Not every entry point runs ToolClad tools. Knowing which do is the difference between a policy that governs real execution and one that governs nothing:

| Surface | Executes ToolClad/MCP tools | Notes |
|---|---|---|
| `symbi run <agent>` | Yes | Loads `tools/` at startup |
| DSL reasoning builtins | Yes | Same loader, invoked from `.symbi` programs |
| `symbi tools test` | No | Dry run: validates arguments and prints the argv |
| HTTP input server | Yes, when `tools/` exists and holds at least one manifest | Otherwise starts with no executor and every tool call is refused |
| `symbi up` chat coordinator | **No** | Has no ToolClad executor at all |
| `symbi shell` (ORCH) | No | Ships its own fixed tool set — `read_file`, `search`, `edit_file`, `save_artifact` — which are not ToolClad manifests |

The chat coordinator is deliberately monitoring-only: it answers questions about a running fleet and cannot act on it. A Cedar `permit` for `tool_call::<name>` has no effect there, because no tool call is ever proposed.

This matters when writing policy. Policies sitting flat in `policies/*.cedar` are loaded by **every** surface's gate. A permit written for `symbi run` reaches the coordinator's gate too — harmless while the coordinator executes nothing, but it becomes a real grant the moment a surface gains an executor. Put surface-specific grants in `policies/<surface>/` instead, which only the named surface reads. The surface names are `run`, `coordinator`, `http-input`, `managed-cli`, `eval`, and `shell`.

If you want the coordinator to execute tools later, scope it per tool rather than switching it on globally — otherwise every permit already written for another surface applies to chat on day one.
