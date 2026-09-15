---
layout: default
title: Firecracker Setup (Tier 3)
nav_order: 8
---

# Firecracker Setup (Tier 3)

Tier 3 runs oneshot commands, custom output parsers, MCP stdio servers, PTY sessions
and managed CLI workers in a
fresh Firecracker microVM. The selected ToolClad boundary and public `FirecrackerRunner` use the
same guest protocol and independent supervisor. A VM boot or VMM exit cannot
stand in for the requested command's result.

Tier 3 is part of the open-source runtime. It needs no license key and no Enterprise
build: the `symbi-sandbox-guest` and `symbi-sandbox-supervisor` crates ship in this
repository, so you can build, audit and reproduce the guest image yourself.

Fixed `read_file`, `list_files` and `grep_files` operations use explicit read-only
`source_roots` through the runtime's bounded file broker. These roots never become
guest mounts or automatic imports. See [source queries](source-queries.md) for
configuration and limits. Declared command/MCP/PTY files use bounded protocol-5
byte transfers and a separate `output_roots` ceiling for new host outputs. See
[file grants](filesystem-grants.md#firecracker-file-transfer). Git uses a separate [bounded snapshot stream](git-source-queries.md#firecracker-snapshots)
with a sealed guest filesystem.

Isolated browser execution remains unavailable. Selected unsupported paths fail explicitly. Native
HTTP remains a host broker operation, with custom parsers sent to the selected VM.
See [command isolation](toolclad-command-boundary.md) and
[branch coverage](containment-branch-guide.md).

## Provision the artifacts

The host requires Linux with accessible `/dev/kvm`, a compatible
[Firecracker executable](https://github.com/firecracker-microvm/firecracker/releases),
a kernel with virtio block/vsock support, a read-only ext4 root image and a matching
supervisor helper. Follow the
[versioned Firecracker setup instructions](https://github.com/firecracker-microvm/firecracker/blob/v1.16.1/docs/getting-started.md)
for kernel and VMM compatibility. Verify artifact provenance and keep artifacts
outside workload-writable storage. The profile binds paths and configuration;
it does not pin operator artifact contents against later replacement.

Install `symbi-sandbox-guest` at `/sbin/symbi-sandbox-guest` inside the image. It
must run as guest PID 1 without other services. Build it and the host runtime from
the same source revision: the handshake checks protocol version and a fingerprint
of the guest crate sources and build script. Stale images are refused before a
command is sent. The kernel and image remain trusted artifacts.

For Linux x86-64 with a static GNU toolchain:

```bash
cargo build --locked -p symbi-sandbox-supervisor
cargo rustc --locked -p symbi-sandbox-guest --bin symbi-sandbox-guest \
  --target x86_64-unknown-linux-gnu -- -C target-feature=+crt-static
```

A suitable musl target is another option. Include all loaders, libraries and tools
needed by an operator-built image. The following rootless test-image builder
requires a static guest executable and static BusyBox:

```bash
python3 scripts/build_firecracker_rootfs.py \
  --guest-binary target/x86_64-unknown-linux-gnu/debug/symbi-sandbox-guest \
  --busybox /absolute/path/to/static-busybox \
  --output /absolute/path/to/test-rootfs.ext4
```

The builder uses `mkfs.ext4 -d`, creates a 64 MiB image, records binary/image
SHA-256 hashes beside it and refuses overwrite. It installs selected BusyBox
commands and a synthetic root-only canary. It downloads nothing and mounts no
host filesystem. Additional local static test programs can be installed with
`--program NAME=/absolute/path/to/binary`; their hashes are included in the record.
This is a local test fixture, not a maintained production image.

## Configure a project

The existing scaffold accepts kernel and rootfs paths:

```bash
symbi init --profile assistant --sandbox tier3 \
  --firecracker-kernel /var/lib/symbi/vmlinux \
  --firecracker-rootfs /var/lib/symbi/rootfs.ext4
```

Review the generated configuration. Use a private supervisor state directory with
a short absolute path, because Unix socket names have a kernel length limit:

```toml
[sandbox]
tier = "firecracker" # tier3 is also accepted

[sandbox.firecracker]
kernel_image_path = "/var/lib/symbi/vmlinux"
rootfs_path = "/var/lib/symbi/rootfs.ext4"
firecracker_binary = "/usr/local/bin/firecracker"
rootfs_read_only = true
vcpus = 1
mem_mib = 512
working_dir = "/tmp"
max_execution_time = { secs = 30, nanos = 0 }
startup_timeout = { secs = 5, nanos = 0 }
max_output_bytes = 1048576

[sandbox.firecracker.supervisor]
binary = "/usr/local/bin/symbi-sandbox-supervisor"
state_dir = "/var/lib/symbi/vm-leases"
```

Shipping CLI executables can serve the embedded supervisor protocol. SDKs need
the matching standalone helper or an explicitly configured helper path.

Default boot arguments select `init=/sbin/symbi-sandbox-guest` and a read-only root.
Writable rootfs and the old `work_dir` option are refused. Use the supervisor state
directory for host lifecycle data and `/tmp` for guest scratch. Limits permit
1–32 vCPUs, 64–16,384 MiB memory and at most 24 hours per invocation. Registered
agent CPU, memory and time budgets only tighten the profile. CPU budgets below
one whole vCPU and memory below 64 MiB are rejected.

`init` and `doctor` check prerequisites. An actual command trial is needed to
establish that the kernel, image, helper and host can execute together.

## Guest execution and results

Guest PID 1 mounts `/proc`, `/sys`, device nodes and a 128 MiB `/tmp` tmpfs, and
enables guest loopback for local adapters. Before
invocation it drops supplementary groups, GID and UID to `65534`, sets
`no_new_privs`, disables core dumps and caps processes and open descriptors at
256. No external network interface, host source mount, ambient provider credential,
approval socket or audit directory is supplied by the runtime to the VM. Tools must exist in the image; scratch
storage disappears with each command.

The supervisor durably registers a lease, starts the VMM and records its PID,
start time and boot identity before reporting readiness. The runtime connects
to the actual vsock device and requests guest port 4050. Each VM accepts one
versioned request with an ID, exact argv, explicit environment, working directory
and limits. Defaults are `PATH=/usr/bin:/bin`, `HOME=/tmp` and `LANG=C.UTF-8`.
Host ambient variables are not inherited; values are never converted through a
JSON-to-shell environment parser.

A four-byte big-endian length precedes each JSON header; raw stream bytes follow
separately. Custom parsers receive `/tmp/symbi-parser-input`, created by PID 1
from supplied input inside the guest. No host temporary file is mounted. Headers
are limited to 1 MiB, input and each output stream to 10 MiB, with a configurable
smaller output limit. Runtime string results replace invalid UTF-8 bytes.

The response binds the same ID to exit code, byte lengths, timeout/truncation
flags and service error. Malformed, stale, mismatched, oversized, incomplete or
extra data fails the invocation. Success requires a valid correlated response,
exit zero, no error/timeout/truncation and confirmed VM cleanup. Kernel panic,
missing init, VMM exit and failed cleanup cannot fabricate success. Guest console
and VMM logging are discarded; command output uses only the bounded protocol.

## MCP and live byte streams

The current protocol is version 5, with explicit oneshot, stdio and PTY modes. The guest acknowledges the exact
request ID after spawning the command. A frame contains a one-byte type, a
four-byte big-endian length and at most 64 KiB of raw data. Stdin, stdin EOF,
stdout, stderr, startup, file finalization and terminal outcome have distinct types. Total stdin is
limited to 10 MiB; each output stream uses the configured output limit. Partial
frames and slow readers cannot allocate unbounded queues or bypass the lifetime.
Raw SDK streams preserve binary bytes, including invalid UTF-8.

`FirecrackerRunner::spawn_stdio` returns stdin, stdout, stderr and a lifetime
guard. Keep the guard through the complete interaction and await `finish()` to
confirm cleanup. Dropping it cancels the retained VM owner. An unexpected exit,
invalid frame, excess output or lifetime expiry remains an error when finishing.
`wait_for_exit()` passively awaits the verified exit code and removal; a nonzero
code is returned to CLI callers, while `finish()` still reports it as failure.
`finish_cleanup()` separately confirms removal after a failed operation.
Cancellation or a missing acknowledgement cannot invent an exit status.
Closing stdin does not discard pending output. Blocked readers cannot prevent
VM removal at the deadline.

The selected MCP adapter uses these streams for initialization, discovery,
SchemaPin verification and invocation in one guest process. Configure the server
executable and its dependencies inside the image; `mcp-config.toml` keeps the same
command/args/env and public-key fields. The runtime does not pass its provider credentials, signing keys or host project
files to the VM automatically; only declared file snapshots and explicitly
configured server environment values enter. A valid MCP response and confirmed
worker removal are required; the server can remain alive until the adapter closes
the session. Unsigned or changed schemas and pinned-key substitutions are refused.

The static `mcp_fixture` example is only for deterministic local tests. Build it
for the same static target and install it with
`--program mcp_fixture=/absolute/path/to/mcp_fixture` to run the signed MCP case in
`firecracker_sandbox.rs`. Rebuild the guest image after upgrading from protocol
versions 1, 2, 3 or 4; old guest fingerprints are deliberately refused.

## Interactive PTY sessions

[Session tools](interactive-terminal-boundary.md) now use a real controlling PTY
inside the selected VM. The guest kernel must support Unix PTYs and devpts. The
runtime retains the session across authorized calls, and terminal stderr merges
into stdout. Startup, interaction, idle, total lifetime and output limits remain
enforced; a matched prompt reports `prompt_observed` with no invented exit code.
The guest starts at 80×24 with echo and canonical input disabled. Ordinary stdio
keeps its separate binary streams. There is no host terminal or mount fallback.

For local tests, build the static `pty_fixture` example and install it with
`--program pty_fixture=/absolute/path/to/pty_fixture`. This complements the
`mcp_fixture` example; neither is a production tool server. The same bounded
protocol and independent VMM ownership carry all three execution modes.

## Managed CLI workers

[Managed CLI execution](managed-cli-containment.md) uses the same selected VM
profile and supervised streams. Provision Python 3, the native CLI and its
libraries in the read-only rootfs; the static BusyBox fixture alone cannot run
this route. Rebuild the guest service from this branch even if an older image
already uses an earlier implementation of protocol 5: the implementation fingerprint includes loopback setup.

The runtime issues only two guest-to-host capabilities: CID 2, port 4051 reaches
this run's governed tool broker, and port 4052 reaches its protected inference
broker. A tools-only SDK broker receives only 4051. Other ports have no endpoint.
Project configuration cannot deserialize or select these socket capabilities.
Private lease-directory links are removed by the supervisor with the VM.

The generated Python bridges are part of the exact admission argv. The inference
adapter listens on guest loopback, while provider credentials remain in the host
broker. The MCP bridge sends a private close notification after complete stdin
frames, allowing the broker to finish pending responses before closing its
connection. Closing this connection grants no authority and closes no other
connection or run.

`--target` selects an absolute guest backend path; omission uses the configured
working directory. The CLI starts in private `/tmp`. No host repository is
transferred, and scratch files do not persist across backend calls. Rootfs files
readable by the workload are available to both CLI and backend VMs, so the base
image must not contain private control material.

## Lifetime and deployment limits

PID 1 kills remaining workload processes, including descendants that created new
sessions, before returning output. The supervisor kills and waits for the entire
VMM on release, runtime disconnect or lifetime expiry. A dropped execution future
signals its retained owner; governed runs retain cleanup acknowledgement. Linux
parent-death signaling kills the directly launched VMM if the per-user supervisor
dies. Recovery verifies
boot/start identity and uses pidfds, so a stale numeric PID alone cannot authorize
cleanup.

The kernel, VMM, image, service account and host storage remain trusted. The
ordinary per-user supervisor does not provision a jailer or host cgroups. Its
worker admission shares configured guest CPU and memory reservations with
Docker/gVisor workers using the same state directory, without charging VMM
overhead. See [shared budgets](shared-budgets.md).

The opt-in [managed host service](firecracker-host-service.md) provisions approved
artifacts, the jailer, distinct VMM identities, host cgroups and guest-plus-overhead
admission. It requires a separately managed root service and uses systemd watchdog
and recovery commands for cleanup across supervisor failure; UID changes mean the
direct-launch parent-death contract alone is insufficient. Its endpoint accepts
only approved VM launches, so any Docker/gVisor allocation must be partitioned
separately. Privileged host and outage E2E is still pending; the opt-in profile is
not yet deployment-validated. No network destination broker is provided.

Provision the host for the deployment and review Firecracker's
[production host guidance](https://github.com/firecracker-microvm/firecracker/blob/v1.16.1/docs/prod-host-setup.md).
The test image and regression cases do not establish complete containment.

## Verification

`crates/runtime/tests/firecracker_sandbox.rs` exercises real KVM through the public
runner, selected command boundary and ToolClad. Set `SYMBI_FIRECRACKER_BINARY`,
`SYMBI_FIRECRACKER_KERNEL`, `SYMBI_FIRECRACKER_ROOTFS` and
`SYMBIONT_SANDBOX_SUPERVISOR` to local artifacts. The stale-image test also requires
`SYMBI_FIRECRACKER_STALE_ROOTFS`, a bootable earlier guest implementation.

```bash
cargo test -p symbi-runtime --features cli-executor,mcp-client,toolclad-session,cedar \
  --test firecracker_sandbox cli:: -- --ignored --test-threads=1
```

This command selects the managed CLI cases and requires Python in the rootfs.
Select the relevant other tests with their required fixture programs when changing
command, MCP or PTY behavior. Run ignored tests explicitly on a provisioned host. Ordinary ignored-test
counts are not VM evidence. Retain source/artifact identities, positive outputs,
expected failures and cleanup observations from actual trials.
