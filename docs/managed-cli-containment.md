# Managed CLI containment

Agents with `metadata { executor = "claude_code" }` run Claude Code in the
project-selected Landlock, Docker, gVisor or Firecracker worker. Landlock uses a private tmpfs, private loopback network namespace and two inherited connected broker capabilities; see [Linux development](landlock-development.md). Containers receive scratch
`/workspace` and a private channel mount. Firecracker receives scratch `/tmp` and
fixed vsock capabilities for tools and inference, without host mounts or a NIC.
Configured source access belongs to governed tool backends. Policy, approval state,
provider credentials and signed journals stay in the runtime.

For Landlock, configure the installed executable and system Python 3 as described in the Linux guide. Container and VM routes require an image or matching rootfs containing Claude Code and Python 3. The reference code
reviewer also needs Git. Missing images or backends fail without host fallback.
The independent sandbox supervisor owns worker creation, lifetime and cleanup,
including detached descendants and runtime process loss. Container deployments
require a responsive Docker daemon. VM deployments require Linux/KVM, a matching
guest service, kernel and VMM; see [Firecracker setup](firecracker-setup.md).
The worker image and host deployment remain trusted components.

## Project configuration

Keep the control project separate from the source repository being reviewed.
Its `tools/`, `agents/`, `policies/`, `.symbiont/` and other control paths cannot
be exposed through backend mounts. For example:

```toml
[sandbox]
tier = "docker"

[sandbox.docker]
image = "your-provisioned-cli-image"
network_mode = "none"
volumes = ["/srv/source:/source:ro"]

[managed_cli.inference]
base_url = "https://api.anthropic.com"
model = "claude-sonnet-4-5-20250929"
api_key_env = "REVIEW_PROVIDER_KEY"
max_requests = 32
max_output_tokens_per_request = 4096
request_timeout_seconds = 60
beta_headers = ["context-management-2025-06-27"]
```

The credential variable is read by the runtime while preparing its private
inference broker. No provider request or child process starts until admission
has passed policy, required approval and durable pre-effect recording. It is not inherited by the child. The child
uses a fixed placeholder credential and a loopback adapter to the private Unix
socket (containers), inherited connection (Landlock) or runtime-issued vsock port 4052 (VMs). An explicit local HTTP endpoint is supported. URL credentials, query
parameters, fragments, redirects and ambient proxies are rejected or disabled.
Only the configured model and Messages/token-counting routes are available.
Provider-hosted tools, remote MCP servers and URL-based document/image sources
are refused. Only the thinking-history retention hint is supported for context
management; server-side compaction is refused. Optional `beta_headers` are supplied by the operator, not the
worker. Provider failures cannot expose raw response headers or error bodies.

The child profile keeps the selected isolation and resource settings. Containers
replace source mounts with the private channel and force network mode `none`.
VMs expose only guest loopback and the two issued broker capabilities, without
an external network interface. Backend tools retain their independently authorized profile. The
operator may explicitly grant backend network access; this is separate from
the CLI's private inference channel.

## Tool and policy configuration

`allowed_tools` names an exact subset of registered ToolClad tools. Built-in
CLI names and wildcard permission expressions are not accepted as registry
entries. The reference reviewer uses `read_file`, `list_files`, `grep_files`,
`git_diff`, `git_staged_diff`, `git_log` and `git_status`. The first three now use
[fixed source queries](source-queries.md) against Linux Docker/gVisor ceilings
or explicit read-only Landlock/Firecracker `source_roots`. The CLI receives authorized
results through its tool channel, without host mounts. The Git tools use
[isolated repository snapshots](git-source-queries.md) with networking disabled
and fixed Git configuration. Firecracker receives bounded Git snapshots in a
sealed guest filesystem; see [Git queries](git-source-queries.md).

```text
metadata {
    executor = "claude_code"
    allowed_tools = "read_file,list_files,grep_files"
}
```

The runtime disables built-in tools with `--tools ""`, loads only its explicit
MCP configuration, disables automatic project/plugin discovery, and uses
`dontAsk` for the child's permission prompts. Each MCP call still goes through
the runtime's prepared-call dispatcher: normalize and validate, obtain any
mandatory exact approval, evaluate Cedar, persist the pre-effect record,
execute the grant, and persist the outcome. The child cannot supply principal,
trusted context, approval, policy or sandbox overrides. Request identities are
single-use within a connection, and reconnecting does not replenish budgets.

Policies in `policies/managed-cli/` authorize the spawn and individual tools.
Shared files directly under `policies/` also apply. A minimal read policy is:

```cedar
permit(principal, action == Action::"tool_call::claude_code", resource);
permit(principal, action == Action::"tool_call::read_file", resource)
when { context.invocation.arguments.path == "src/main.rs" };
```

The complete source's supported inline allow/deny policies restrict both the
`claude_code` admission action and normalized broker calls before approval.
An inline allowlist must name `claude_code` explicitly. Unsupported selected or
file-wide rules are refused before worker or provider startup. This does not
interpret executable DSL bodies; see [inline policies](inline-policies.md).
Execution metadata remains file-wide. Inline policy selection includes global
rules and the selected agent's rules; sibling-agent policies do not apply.

Admission is a fixed, single-use prepared contract containing argv, explicit
environment, task, stdin, bounds, inference settings and selected tool scope.
Its `sandbox` describes the actual scratch child, and `tool_sandbox` describes
the broker backends. The profile is retained during review; use an immutable
image reference when deployment requires image content binding.
`claude_code` is reserved for admission and cannot be a registered managed tool.
Set metadata `human_approval = true` to require admission review; false or omitted
keeps direct operator launches subject to source and Cedar policy. Other values
are rejected. Each tool's approval requirement remains independent.

Enable `--approval-terminal` to resolve
mandatory approvals from the runtime operator's controlling terminal. Each
answer must name the exact displayed request; `--approval-timeout` bounds display
and decision waiting. Without a relay, manifests requiring approval fail closed.
SDK callers can attach an `EscalationGate` and the shared approval queue to
`GovernedToolSession`. Host SDK applications can use `ManagedCliActionExecutor`
and `dispatch_host_action` for the separate admission; wrap both executors with
the same retained source policy. Admission must precede broker tool calls, and
the host method is not exposed through MCP. See [approval lifetime](approval-lifecycle.md).

For Docker/gVisor, `--target` is a host directory mapped to an explicit backend
mount; it defaults to the control project's current directory. For Firecracker,
`--target` is an absolute guest path without parent traversal, defaulting to
`[sandbox.firecracker].working_dir`. Relative file tool arguments resolve beneath
that backend's working directory. The CLI itself always starts in its own scratch
directory. Each VM backend call has a fresh rootfs view and private `/tmp`.

A VM target does not automatically upload a host repository. Provision tool
executables in the rootfs. Backend tools may explicitly request bounded
`[filesystem]` inputs and a new output; the CLI child receives only their authorized
results. The CLI and tool VMs share the same base image, so readable files baked
into it are also readable by the CLI. Keep private backend data in scoped grants.
Fixed source read/list/search operations use the host broker. Git queries receive
private repository snapshots; Firecracker seals its imported tree read-only before
running the fixed driver. These routes refuse unsupported links and special files,
and bounded searches report incomplete results. See [file grants](filesystem-grants.md)
and [Git snapshots](git-source-queries.md).

Git helpers, hooks and network access remain disabled. See [Git source queries](git-source-queries.md)
for supported repository layouts and the metadata grant. `--plugin-dir` is
rejected; managed runs do not load plugins.

## Bounds and audit

`--max-turns` bounds the child conversation. The runtime separately caps tool
calls at eight times that value, up to 1,000. `--budget-timeout` is a wall-clock
bound including initialization. `--budget-tokens` is a **reserved output-token
allowance**: every Messages request consumes its requested `max_tokens` before
send, without refunds after cancellation or errors. Input traffic is separately
bounded by request count and bytes; this option is not a total billed-token or
currency limit. Requests are at most 1 MiB and responses at most 4 MiB. Responses,
including SSE, are buffered until complete, so downstream streaming is delayed.

The runtime creates a required per-run signed journal in
`.symbiont/governed/<session>.jsonl`. The directory is private, the journal and
persistent signing key use mode `0600`, and worker mounts cannot expose them.
Records contain a sequence, hash chain and Ed25519 signature over canonical
JSON. Signatures cover the exact embedded payload bytes; verifiers must not
reformat numbers or whitespace before checking them. Writes are synchronized to storage before authorization proceeds.
The admission pre-effect record includes the source-bound fingerprint and any
exact approval receipt. The worker's final observation records process status,
output hashes and byte lengths; it is separate from broker tool observations.
The VM CLI retains verified nonzero exit codes and stderr as failed execution;
child-reported success cannot replace the observed status.
Closing a session waits for worker cleanup acknowledgement before terminal audit.
Cancellation or a dropped dispatch cannot record successful completion. Fixed
runtime diagnostics preserve deadline and protected-credential rejection causes
without copying upstream response data into errors.
Inference requests and responses are correlated by identifiers and hashes;
credentials are excluded. This audit key is separate from AgentPin identity,
which continues to use ES256.

The runtime prints the audit public key. Retain it through a trusted channel
for later verification. `ProtectedJournal::verify` checks signatures, chain,
sequence and principal against that key. A valid prefix without a terminal
record is incomplete evidence. Interrupted dispatch, interrupted inference or
cleanup failure cannot produce a successful terminal session record. Journals
are bounded to 64 MiB; storage failure stops the run. These journals are per-run
records, not an automatic replay or recovery mechanism.

Signed durable storage also applies to ordinary CLI, HTTP reasoning and scheduler
invocations; see `run-audit.md` for invocation-bound records and verification.
Default DSL `reason()` and `tool_call()` also require protected per-run journals.
Direct DSL inference and composition have protected per-provider-call journals;
coordinator, delegated and chat-platform turns also retain required audit.
Scheduled ORGA execution and Firecracker command/stdio/PTY/managed transports are
implemented. Governed workers using one supervisor state directory share
[worker capacity admission](shared-budgets.md), including managed sessions and
their tool workers. Size the pool for this overlap; a session occupying the only
slot cannot start another worker and receives an explicit refusal. Unsupported
scheduling transports, public browser execution and additional VM input/output capabilities remain separate
work. See the [branch guide](containment-branch-guide.md) for route scope and
operator deployment assumptions.
Docker validation does not establish gVisor socket compatibility.

The CLI flags are documented in the
[official CLI reference](https://code.claude.com/docs/en/cli-reference).

## Selecting an agent boundary

Both ordinary and managed `symbi run` resolve execution settings from the selected
agent before inference. For example:

```symbi
agent reviewer(input: String) -> String {
    with sandbox = "docker", timeout = 30.seconds {
        return input;
    }
}
```

A single-agent file can be addressed by its filename alias. A file containing
several agents requires the filename selector to match one unique declared name.
Sibling definitions do not supply execution settings or enter the selected
agent's ordinary reasoning prompt. Syntax errors, duplicate names, unknown tiers,
and conflicting sandbox or timeout attributes are errors. Statements belong in
`with` blocks. A reasoning process has one sandbox and one lifetime; switching
between conflicting block settings is not supported.

An explicit agent tier overrides the project default before backend validation.
If omitted, the project default applies. Each tier retains its own configured
profile: gVisor uses `[sandbox.gvisor.docker]`, without inheriting Docker mounts.
Agent deadlines tighten existing tool and process limits and cannot extend them.
Unavailable selected backends fail without host fallback.

Registered HTTP Input agents use the registry's name and source, validate the
selected tier against the registered security tier, and build one frozen executor
before inference. A disagreement is a configuration error. Configuration changes
during a run cannot replace that snapshot. Standalone SDK servers can supply a
trusted custom executor; the embedder owns that executor's isolation guarantees.
Scheduled agent registration resolves the named agent's source, including legacy
`.dsl` files. [Scheduled ORGA execution](scheduled-execution.md) now uses this
selection with real invocation results and cleanup. Managed CLI scheduling remains
unavailable. Firecracker oneshot commands and parsers now use the matching
[guest transport](firecracker-setup.md), which also supports MCP stdio and PTY sessions. The VM
managed CLI uses these streams with runtime-issued tool and inference capabilities.
Its rootfs needs Python and the native CLI; the minimal BusyBox test image is
insufficient.
