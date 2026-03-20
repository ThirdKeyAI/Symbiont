# ToolClad Manifests

This directory contains `.clad.toml` manifests that define tool interfaces for the Symbiont runtime. Each manifest declares a CLI tool's typed parameters, command template, output format, and Cedar policy metadata.

The runtime auto-discovers these at startup — no Rust code needed to add a new tool.

## How It Works

1. Agent DSL references a tool: `capabilities = ["tool.nmap_scan"]`
2. Runtime finds `tools/nmap_scan.clad.toml`
3. MCP schema auto-generated from manifest parameters
4. Cedar policy evaluated using manifest-declared resource/action
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
