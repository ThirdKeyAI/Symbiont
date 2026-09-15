# ToolClad Manifests

This directory contains `.clad.toml` manifests that define tool interfaces for the Symbiont runtime. Each manifest declares a CLI tool's typed parameters, command template, output format, and Cedar policy metadata.

The runtime auto-discovers these at startup — no Rust code needed to add a new tool.

## How It Works

1. Agent DSL references a tool: `capabilities = ["tool.nmap_scan"]`
2. Runtime finds `tools/nmap_scan.clad.toml`
3. MCP schema auto-generated from manifest parameters
4. The calling runtime surface applies its configured policy gate
5. Arguments validated against manifest types
6. Command constructed from template (agent never generates shell commands)
7. Output parsed and wrapped in evidence envelope

## Included Tools

| Tool | Binary | Risk | Description |
|------|--------|------|-------------|
| `whois_lookup` | `whois` | low | WHOIS domain/IP registration lookup |
| `nmap_scan` | `nmap` | low | Network port scanning and service detection |
| `dig_lookup` | `dig` | low | DNS record lookup |
| `curl_fetch` | `curl` | low | HTTP request with response capture |

## Adding a New Tool

Create a `.clad.toml` file:

```toml
[tool]
name = "my_tool"
version = "1.0.0"
binary = "my-binary"
description = "What this tool does"
timeout_seconds = 30
risk_tier = "low"

[tool.cedar]
resource = "Tool::MyTool"
action = "execute_tool"

[args.target]
position = 1
required = true
type = "string"
description = "The target"

[command]
template = "my-binary {target}"

[output]
format = "text"
envelope = true

[output.schema]
type = "object"

[output.schema.properties.raw_output]
type = "string"
```

See [TOOLCLAD_DESIGN_SPEC.md](https://github.com/ThirdKeyAI/ToolClad) for the full specification.

## Command argument boundaries

Command templates, mappings, and conditional fragments are trusted operator
configuration. They are split into argv before validated argument values are
inserted. A value containing spaces, quotes, or backslashes remains one
argument; it cannot add command-line options by breaking out of template quotes.
Empty values remain empty argv entries so later operands keep their positions.
Use conditionals to omit optional flags and their values together, and mappings
for a fixed set of flags instead of accepting a free-form flag list.
The executable must be fixed by the manifest. Use the target program's `--`
separator before positional operands where supported. Scope targets reject a
leading dash. The shipped nmap manifest restricts `extra_flags` to an empty
value, `-n`, or `-Pn`; additional behavior requires a reviewed manifest change. The curl manifest
accepts a single `Key: Value` header and rejects file references and embedded
line breaks.

Oneshot commands and custom output parsers receive a cleared environment containing only PATH, locale,
timezone, and Windows system-directory configuration. Host credentials and
other ambient variables are not forwarded. These tools have a 10 MiB limit per
output stream. Commands and custom output parsers share the tool deadline,
including process execution and pipe draining.
Unix process groups are terminated on timeout, output failure, and normal
oneshot completion. These lifecycle controls do not supply filesystem or
network isolation; an external sandbox is still required for untrusted code.

## Required destination scope

Arguments declared as `scope_target`, or with `scope_check = true`, require a
project `scope/scope.toml`. The runtime builder loads it relative to the tools
directory. Direct executor integrations must call `with_project_scope` or
`with_scope`. Missing scope denies scoped arguments; unreadable or malformed
scope denies dispatch rather than loading a partial allowlist.

```toml
[scope]
targets = ["192.0.2.0/24"]
domains = ["example.com", "*.test.example.com"]
exclude = ["192.0.2.128/25", "private.test.example.com"]
```

The entire requested CIDR must fit an allowed range and must not overlap an
exclusion. Hostnames and URL hosts are canonicalized before matching, and
wildcards match complete DNS labels. This is a destination authorization check;
the worker still needs network enforcement for actual DNS results and traffic.
Arguments not declared in the manifest are rejected.

## HTTP and MCP lifecycle

HTTP backends use the shared DNS filter, disable redirects and ambient proxies,
and cap response bodies at 10 MiB. HTTP templates expand only placeholders in
the trusted manifest; an argument cannot introduce a secret lookup or another
argument expansion. Response decoding and parser failures produce errors.
Async callers use `execute_actions` so blocking HTTP work runs off the loop's
runtime thread and uses the smaller of the tool and loop tool deadlines.

Stdio MCP discovery, schema verification, and invocation share one connection.
Signature verification uses the exact key checked against the provider pin.
MCP workers receive only a minimal environment and explicit registry variables;
stdout and stderr are capped at 10 MiB each. Deadlines cover the handshake,
discovery, verification, and tool call. Completion, errors, and cancellation
terminate the ordinary worker process group; this does not replace a sandbox.
