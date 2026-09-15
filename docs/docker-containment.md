# Docker execution boundary

`DockerRunner` creates a stopped container with a unique name, inspects its
mounts, then starts and attaches to that container. Every completed invocation
removes the container and its anonymous volumes. Timeout, stream overflow,
transport error, and caller cancellation also remove the container. Removal
stops container processes even when they create their own process sessions.
An independent supervisor process owns creation and cleanup. The shipping
`symbi` executable embeds this service; SDK applications install the matching
`symbi-sandbox-supervisor` binary or configure its absolute path. Docker/gVisor
commands, parsers, MCP workers and PTYs use the same service.

Before invoking Docker, the supervisor fsyncs a private lease record. The
runtime may start only the inspected immutable container ID carrying the
reserved `ai.symbiont.lease` label. Caller EOF, process loss and the service's
own lifetime deadline trigger removal. Creation continues under independent
ownership after a caller timeout; the cancelled payload never starts. A failed
or interrupted creation request leaves an uncertain record. A missing name
alone cannot resolve it: recovery must observe and remove the owned container.
Cleanup failures remain errors and retain their records for retry.

The default configuration uses a non-root UID:GID (`65534:65534`), a read-only
root filesystem, private IPC and cgroup namespaces, no capabilities,
`no-new-privileges`, no healthcheck, no restart policy, no daemon log storage,
and no network. CPU, memory, process count, output, and elapsed-time limits
are mandatory. A per-file hard size limit defaults to 64 MiB; one-shot file
grants tighten it further. Scratch storage uses bounded tmpfs. Images must already exist
locally; execution does not pull images. Image-declared anonymous volumes are
rejected before the payload starts because they bypass the configured writable
storage limits.

Bind mounts must name existing absolute host paths. Named volumes, special
files, traversal, protected host paths and their ancestors, shared propagation,
and mounts over container system directories are refused. Paths are resolved
before being passed to Docker. Omitted mount mode means `ro`; use an explicit
`:rw` for permitted output directories. Host permissions must allow the
configured container UID:GID to access those directories. Host source paths
and their parents must remain controlled by the operator while Docker mounts
them; lexical checks and canonicalization alone do not prevent concurrent
host-side path replacement.

One-shot ToolClad calls use the [file broker](filesystem-grants.md) instead of
passing those configured paths directly to Docker: inputs are private snapshots,
outputs are staged separately, and configured mounts are only access ceilings.
The direct-mount deployment assumptions above still apply to other runner users.

`extra_flags` accepts only `--label=value` metadata; the lease label is reserved.
Resource, privilege,
network, mount, entrypoint, user, and runtime settings cannot be overridden
through extra flags. The gVisor wrapper selects its runtime separately.
`auto_remove = false`, absent resource limits, root users, and host/shared
network namespaces are rejected. Production also rejects `network_mode = "bridge"`
for Docker and gVisor, including shell, MCP, PTY and parser workers. Use
`network_mode = "none"` and governed native HTTP tools for outbound requests.
The strict `SYMBIONT_ENV` parser recognizes production aliases and refuses
unknown or non-UTF-8 values when bridge is requested. Explicit non-production bridge remains
unrestricted egress; it is not destination authorization.

The Docker daemon, OCI runtime, image, operator configuration, and host kernel
are trusted components. Removing a container requires a responsive daemon.
Docker [live restore](https://docs.docker.com/engine/daemon/live-restore/) can
keep workers running while the daemon is unavailable. Lease recovery does not
establish a hard deadline under that condition. An automatically launched
supervisor survives its caller's death, but a shared service manager or outer
container can terminate both processes together. Deploy the persistent helper
in a separately supervised service for recovery after helper failure and boot.
These runner controls do not yet establish mandatory sandbox routing for every
Symbiont entry point or per-action network brokerage.

## Installation and recovery

Build/install the helper from the same source revision as the runtime:

```sh
cargo install --path crates/sandbox-supervisor --locked
symbi-sandbox-supervisor --state-dir /absolute/private/sandbox-leases --persistent
```

SDKs look for `symbi-sandbox-supervisor` on PATH. The main CLI starts its embedded
helper automatically. `SYMBIONT_SANDBOX_SUPERVISOR` overrides the executable;
`SYMBIONT_SANDBOX_STATE_DIR` overrides the default
`~/.symbiont/sandbox-leases`. A command profile can bind both explicitly:

```toml
[sandbox.docker.supervisor]
binary = "/usr/local/bin/symbi-sandbox-supervisor"
state_dir = "/absolute/private/sandbox-leases"
```

Keep the state directory on durable local storage, mode 0700, owned by the
runtime account. The socket and records are mode 0600. The runner rejects bind
grants overlapping this directory in either direction, including aliases.
Workers must not share the host runtime account or receive this control socket
through another transport. The helper has the runtime account's Docker access;
it is not a privilege boundary against another trusted host process using that
account. Worker argv and environment values are excluded from lease records.
Private environment files exist only during creation and stale files are
removed when the exclusive service lock is reacquired after a crash.

For continuous recovery, run the command above under a service manager with
`Restart=always`, using the same account and state directory as the runtime.
`--persistent` keeps it available while idle; automatic instances exit when
idle and free of records. Each startup validates durable state and reconciles
orphan leases. Version and source-implementation handshakes reject incompatible
helpers; drain existing leases before replacing the helper during an upgrade.

Uncertain records intentionally persist when no container has appeared. They
can represent a rejected Docker request or a request still arriving at the
daemon. Do not delete them based only on an empty `docker ps` result. Investigate
the recorded daemon and establish that outstanding creation cannot complete
before retiring a tombstone. Malformed or unsafe storage blocks new execution.
Records and connection counts are bounded; exhausted capacity fails explicitly.
After a helper crash, the running caller reports lost cleanup acknowledgement;
restart recovery must complete before treating that operation as cleaned up.

## Real-backend regression tests

Use a Linux Docker host with cgroup v2 and a cached `python:3.12-slim` image:

```sh
cargo build -p symbi-sandbox-supervisor
export SYMBIONT_SANDBOX_SUPERVISOR="$PWD/target/debug/symbi-sandbox-supervisor"
export SYMBIONT_SANDBOX_STATE_DIR=/tmp/symbi-test-private-leases
cargo test -p symbi-sandbox-supervisor --test lifecycle -- --ignored --test-threads=2
SYMBI_DOCKER_AMBIENT_CANARY=synthetic-ambient-value \
  cargo test -p symbi-runtime --lib sandbox::docker::tests::e2e:: \
  -- --ignored --test-threads=2
```

These tests execute the shipping runner against the Docker daemon. They check
allowed writes, read-only inputs, a synthetic host file and credential canary,
a local network receiver, exact/excess output boundaries, failure exit codes,
process/CPU/memory ceilings, timeout, cancellation, and descendants that call
`setsid()`. A temporary derived image verifies image-volume rejection before
execution. Test containers and that image are removed. Missing Docker or a
missing image fails the explicit run; there is no silent successful skip.
The CI test workflow runs this suite explicitly.

The supervisor suite adds useful work, detached descendants, open-connection
deadline expiry, caller disconnection, delayed submitted creation, failed
creation acknowledgement, helper SIGKILL/restart during creation and execution,
temporary removal failure, and refusal to delete a differently labelled
container. Docker client wrappers inject bounded local faults; the Docker
daemon itself is not stopped or modified. Late-container tests require evidence
that a real container materialized after interruption before checking removal.
