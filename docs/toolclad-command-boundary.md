# ToolClad command isolation

ToolClad oneshot commands and custom output parsers use Docker by default, including SDK construction with `ToolCladExecutor::new`. The production executor builder reads `[sandbox]` in the project's `symbiont.toml`. Malformed, unreadable and dangling configuration fails closed. No unavailable backend falls back to host execution.

```toml
[sandbox]
tier = "docker" # also tier1; gvisor/tier2 requires registered runsc

[sandbox.docker]
image = "python:3.12-slim"
network_mode = "none"
volumes = ["/absolute/project/output:/workspace:rw"]
max_execution_time = { secs = 30, nanos = 0 }
```

For one-shot Docker/gVisor tools, these mounts are ceilings. Declare individual
inputs and a new output in the manifest's `[filesystem]` table; omission gives
the worker no host mounts. The runtime stages input copies and publishes a new
output after successful cleanup. See [file grants](filesystem-grants.md) for
bounds and migration. Executables and dependencies must be present in the selected
cached image. Images are never pulled during invocation. Docker's mandatory
memory, CPU, PID, file-size, output and lifetime limits still apply. Project
configuration, tools, policies, scope, agent definitions, Git metadata and audit
storage cannot be exposed by a project-configured mount.

The optional Linux `landlock` backend currently supports MCP stdio without
declared files. One-shot commands, custom parsers, declared-file staging and
PTYs still require Docker/gVisor or Firecracker. Landlock prepares its rules
before fork, retains the originally granted filesystem objects and refuses
missing declared roots. Governed launches now require shared admission and
[delegated cgroup supervision](landlock-supervision.md). See
[multi-tier sandboxing](security-model.md#landlock-daemon-free) for its
coverage, network limits and fail-closed kernel check.

Production Docker and gVisor profiles require `network_mode = "none"`. An explicit
`bridge` setting fails configuration validation before a worker starts; `symbi run`
refuses it before inference. Use governed native HTTP tools for outbound requests.
Non-production bridge remains available as an explicit unrestricted capability.
Changing deployment environment does not preserve an earlier bridge grant: the
configuration is checked again before Docker create arguments are built.

Preparation includes the selected command profile's digest, resource bounds and
effective file grants in the immutable contract and Cedar invocation context.
`execution_transport` identifies native HTTP, contained MCP and PTY transports,
and the unavailable browser backend. Changing the executor profile invalidates
its previously issued grants. Custom output parsers, including those after HTTP
requests, receive no host mounts. MCP retains a live worker through discovery,
signature verification and invocation. [Interactive PTYs](interactive-terminal-boundary.md)
retain a bounded worker for their run. MCP invocations use the same file broker
as one-shot commands; standalone discovery receives no host mounts. PTYs retain
a fixed, bounded file set across their commands and require a manifest-declared
finalizing command to publish output after cleanup.

Argument values remain literal argv elements. Command stdout and stderr retain
leading/trailing whitespace; custom parsers receive the complete captured stdout.
Text consumers that need trimmed display output should trim it explicitly. The container shell receives a quoted trusted `exec` wrapper. Custom parsers still require `custom:/absolute/container/path` and the trusted `SYMBIONT_TOOLCLAD_ALLOWED_PARSERS` allowlist. Their filename argument refers to container scratch storage: bounded raw output travels over stdin, without implicitly mounting host files or embedding multi-megabyte input in process arguments. Parser execution receives only the remaining tool deadline. Cancellation removes the container, including descendants which created new sessions.

Explicit development host execution is available through `with_development_host_execution()`. Project configuration with `tier = "none"` additionally requires `SYMBIONT_ALLOW_UNISOLATED=1`. Command execution in this mode is refused when `SYMBIONT_ENV=production`, even with that flag. It provides process lifecycle controls, not containment.

Firecracker oneshot commands and custom parsers use a fresh independently owned
microVM with exact argv, explicit environment and a correlated guest result. See
[Firecracker setup](firecracker-setup.md) for matching guest artifacts and limits.
Firecracker MCP stdio, PTY sessions and managed CLI workers use the same guest
transport. Hosted commands and isolated browser execution currently report unavailable. Ordinary/managed CLI and registered HTTP/scheduled execution
bind the selected agent's sandbox; canonical DSL/REPL binding remains open.
[Managed CLI brokerage](managed-cli-containment.md) and an
[independent cleanup supervisor](docker-containment.md) are implemented. Full
egress brokerage, race-resistant host mount resolution and broader failure
recovery remain outstanding. Native HTTP retains its DNS-aware broker controls;
selecting a command container does not put the HTTP client inside that container.
See the [branch guide](containment-branch-guide.md) for current coverage.

## Verification

`scripts/test-container-network.py --binary target/debug/symbi --report /tmp/container-network.json`
checks the shipping CLI with a cached Docker image and local synthetic inference
and TCP receivers. It covers production bridge refusal, production aliases with
gVisor selection, invalid environment text/bytes, host-network refusal, useful isolated
work and explicit development bridge connectivity. It verifies tool results,
host-observed connections and worker cleanup. Refusal cases do not require runsc;
they establish configuration rejection, not successful gVisor execution. Run
without Python optimization. The driver uses the local Docker Unix socket with
an empty client configuration for both execution and observation; it ignores
ambient Docker contexts and does not contact a registry.

`crates/runtime/tests/toolclad_sandbox.rs` has three explicitly ignored Docker
integration tests covering literal arguments and host-file denial, large parser
input without inherited host mounts, and cancellation of detached parser
descendants. Run them deliberately with Docker and cached `python:3.12-slim`.
Ordinary workspace tests do not count them as passing containment evidence.
The [file-grant E2E](filesystem-grants.md#verification) checks the shipping CLI,
actual mounts and file-size limits, useful outputs and signed authorization.
