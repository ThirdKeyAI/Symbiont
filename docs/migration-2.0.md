# Migrating to Symbiont 2.0

Version 2.0 makes ToolClad contracts strict and removes ungoverned execution
from the normal Rust API. It also gates publication on the tested source commit
and releases invocation locks explicitly. Existing optional features, including
governed improvements, remain opt-in.

## Before upgrading

Protected run journals and the execution routes that require them currently need
Unix. Windows builds support portable CLI functionality; they do not establish
equivalent support for protected scheduled execution, durable chat admission or
governed improvements. Those routes refuse unsupported operation. Landlock adds
Linux-specific kernel and systemd requirements. A successful platform build is
not a containment qualification for that platform.

Keep a recoverable copy of the project configuration, tool manifests and runtime
state. Do not delete invocation claims or journals to make an upgrade pass:
that discards retry history. Stop new admissions and settle or inspect active
work before replacing the runtime and its matching supervisor.

On a disposable copy of the project, run the new binary's `symbi tools validate`
and `symbi doctor`. Correct every manifest error, then exercise a representative
approved action and a denied action through the normal agent route. Validation
does not execute tools; a valid manifest is not evidence that its backend works.

## Manifests

An unknown field or unsupported control now fails the whole configured tool set.
Previously, some fields were ignored and some invalid tools disappeared with a
warning. Correct spelling and migrate the contract; do not remove a required
security control merely to pass validation. Custom types in `toolclad.toml`
follow the same rule.

Prefer explicit argv arrays. A placeholder inside quoted template text is now
refused. For example, replace this legacy command:

```toml
[command]
template = "printf '%s' '{message}'"
```

with:

```toml
[tool]
name = "print_message"
version = "2"
binary = "printf"
description = "Print one bounded message"

[args.message]
type = "string"
required = true
allow_whitespace = true
pattern = ".{1,256}"

[command]
exec = ["printf", "%s", "{message}"]

[output]
format = "text"
```

String-derived arguments reject control characters, quotes, backslashes and
leading `-`/`+`; whitespace needs an explicit allowance with `exec`. Values are
no longer trimmed and patterns match the entire value. Review leading-option
requirements individually; `allow_leading_dash` and `end_of_options = "--"`
are explicit choices, not universal compatibility switches.

Use `{{` and `}}` for literal braces in trusted command text. Mapping placeholders
use `{_<argument>_flags}`; migrate `{_scan_flags}` to the actual argument name.
Remove unsupported `{_output_file}` and `{_evidence_dir}` command placeholders;
emit captured results to stdout or declare specific files under `[filesystem]`.
An interpreter such as Python or a shell requires `allow_interpreter = true`
under **`[tool]`**, and the command executable must match `tool.binary`.

High and critical risk tiers now require an exact-call approval even when
`human_approval` was omitted. Supply a supported approval relay or retain the
denial. Session/browser command overrides also participate in this check. Policy
rules should use `resource.base_tool` when governing all subcommands of a tool.

Output schemas fail closed: a violation produces `schema_error`, not a successful
envelope with `schema_warnings`. Consumers must handle this status, and a tool
error does not establish that no external effect occurred. `[output] max_bytes`
bounds capture; unsupported resource limits refuse execution.

## Network and registries

URL arguments default to HTTP(S). Noncanonical addresses, userinfo, unsafe
schemes, metadata/link-local destinations, and internal-name spellings are
refused. Egress tools must declare `block_internal = true` on network-target
arguments; this includes DNS resolution and overlapping CIDR ranges. Do not use
these tools as a substitute for an explicitly designed private-service broker.

HTTP URL placeholders cannot change the scheme or host. URL values are encoded,
JSON body values are escaped, and header line breaks are refused. Remove manual
encoding/escaping that would double-encode values. A wildcard scope rule covers
strict subdomains; add the apex explicitly if it is intended to be allowed.

Put `mcp-config.toml` in the selected project. Production no longer consults
`~/.symbiont/mcp-config.toml`; an invalid project registry never falls back.
Landlock `require_network = false` is also refused in production. Managed CLI
continues to use private loopback and inherited broker connections.

## Rust integrations

`load_manifests_from_dir` and `load_custom_types` return `Result`. Propagate
failures rather than replacing them with an empty tool set:

```rust,ignore
let manifests = symbi_runtime::toolclad::load_manifests_from_dir(tools_dir)?;
let executor = symbi_runtime::toolclad::ToolCladExecutor::new(manifests);
```

Use the configured reasoning loop and its prepared-action dispatcher for actual
execution. Calling `ActionExecutor::execute_actions` directly does not obtain an
authorization grant. Direct ToolClad/MCP/session/browser execution helpers are
available only under unit tests or the `toolclad-direct` fixture feature and are
refused in production even with that feature. Never enable it in deployed builds.

## Release qualification

The release pipeline requires a successful **Test** workflow and **Linux Landlock
release qualification** for the exact source SHA on `main`. Run the latter
manually on the intended release commit before tagging. Newer failures and
unfinished reruns override older successes. Missing evidence blocks publication.
Binary and crate jobs check out the qualified SHA; manually naming a tag no
longer labels a build from a different branch. Development Docker pushes require
Test success; versioned Docker tags require both workflows.

The Linux suite uses real restricted workers and synthetic providers. It covers
onboarding, file grants, source queries, Git snapshots/capacity/crash cleanup,
managed CLI transport, exact approvals, durable retry refusal and reconciliation.
It does not establish model quality or independent human review. See
[release validation](release-2.0-validation.md) for the branch's measured results
and remaining qualification boundaries.

## Authentication, source ownership and evidence limits

HTTP Input JWTs require configured issuer and audience. Reissue legacy unprefixed
API keys. WebSocket clients exchange authenticated credentials for a single-use
60-second ticket instead of sending API keys in URLs; update clients together.
Assign an explicit tenant/workspace owner to every Enterprise runtime, audit
source and ORGA source. Unassigned sources are unavailable, including instruction
review routes. Consult the Console security model in Enterprise deployments
for configuration. Trusted platform-admin provisioning records verified status
atomically; self-service signup still requires email verification.

Rate limiting uses the peer address unless its CIDR is a configured trusted proxy.
Runtime HTTP clients refuse redirects and need explicit CIDRs for private
addresses in addition to host egress permission. Remote-bus construction now
returns an error for invalid TLS endpoints; callers must propagate that result.

Re-sign skill bundles with `symbi skills sign DIR --key KEY.pem --domain DOMAIN`.
The `symbi-skill-v2` signature binds the full manifest, domain, name and timestamp;
old skill signatures are refused. Consumers must support the new format. Tool
schema signatures are unchanged. See [the security model](security-model.md).

RAG ingestion rejects documents over 2 MiB, more than 1,024 chunks, or more than
8 MiB of combined chunk text before embedding or storage. Audit and ORGA source
files reject more than 32 MiB, 256 KiB per line, or 50,000 records in full, even
when a request selects fewer records. Rotate oversized logs before upgrading.

## Optional prompt caching

Existing calls keep their defaults. SDK integrations can explicitly select prefix
or rolling caching and TTL using `PromptCacheConfig`. Usage observations expose
cache counts when provided; admission, audit and recovery retain raw token
accounting. See [prompt caching](prompt-caching.md) for supported providers,
wrapper forwarding requirements and the downstream integration handoff.
