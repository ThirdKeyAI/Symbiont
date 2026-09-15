# MCP server

`symbi mcp` serves tools and project resources over stdio. Standard output contains
MCP messages; diagnostics and policy-loading notices go to standard error. Start
the process in the project whose agents and files the client should use.

## Registered text responses

`invoke_agent` accepts `agent`, `prompt` and optional `system_prompt`. The agent
name identifies exactly one `.symbi` or legacy `.dsl` filename stem in the startup
registry. Unknown or ambiguous names return an MCP tool error. Single-definition
filename aliases use the same declared response principal. Multiple definitions
require the filename stem to select one declared agent.

The selected source must be a supported canonical conversational definition.
Executable DSL statements, unsupported policies and alternate executors are refused
before inference. This endpoint produces one text response; it does not execute
ToolClad tools, commands, managed CLI agents or delegated work. A model-proposed tool
call cannot enable those capabilities.

The legacy `system_prompt` field is additional caller input, sent as a user message.
It cannot replace the registered definition or runtime policy context. The generated
section of `AGENTS.md`, when available, is also caller guidance. Input and additional
instructions each have a 64 KiB limit. The response request permits at most 4,096
output tokens; the selected source timeout can tighten the 120-second deadline.

Cedar policies load from shared `policies/*.cedar` and `policies/mcp-server/*.cedar`.
They evaluate `Action::"respond"` against the exact prepared response and registered
source context before text is returned. Missing policy files retain the framework's
default text-response permission. An unreadable, malformed or empty configured
policy set denies all actions; it cannot restore that default permission.

## Audit and cancellation

Inference requires a private per-invocation journal under `.symbiont/governed/`.
The response's `structuredContent` contains `agent_id` and
`audit: { run_id, path, public_key }`. Errors after initialization retain that
reference; preflight or storage-initialization refusals can have no reference.
The ordinary text content stays in the MCP result's `content` array.

The signed startup record binds the declared agent, complete source and selected
declaration hashes, execution mode and surface. `DirectInferenceRequested` must
persist before the provider call. Typed response hashes must persist before policy
evaluation, and the policy decision and terminal record must persist before a
successful MCP result is produced. Required write failures withhold model text.
Typed requests are bounded to 1 MiB and typed responses to 4 MiB.

MCP cancellation notifications cancel active inference. A retained owner finishes
the cancellation audit even when the calling future is dropped. Abrupt process
loss can leave an incomplete signed prefix. Completion establishes preparation of
an authorized response for the MCP caller; it does not establish remote receipt or
downstream actions. Preserve the public key independently and verify the exact
invocation as described in [protected run audit](run-audit.md).

## Project files

Source discovery, `parse_dsl`, `get_agent_dsl`, `get_agents_md` and the `AGENTS.md`
resource use a directory descriptor pinned to the startup project. Relative path
components resolve without following symlinks. Absolute paths, parent traversal,
hidden components, the `policies` directory and hardlinked files are unavailable.
Only regular UTF-8 files can be read. Project file confinement requires Unix.

DSL files and inline DSL are limited to 1 MiB. `AGENTS.md` is limited to 64 KiB;
its inference guidance is additionally capped at 2,000 characters. Source discovery
inspects at most 1,024 directory entries and retains at most 16 MiB. Files outside
that startup snapshot cannot be invoked until restart. Schema verification input
is limited to 1 MiB with an 8 KiB key-URL limit.

The focused shipping tests use stdio MCP and a local synthetic inference endpoint:

```bash
python3 scripts/test-mcp-response.py --binary target/debug/symbi --report /tmp/mcp-response.json
python3 scripts/test-mcp-response.py --invalid-policy --binary target/debug/symbi --report /tmp/mcp-invalid-policy.json
```
