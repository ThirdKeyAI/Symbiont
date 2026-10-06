# Supervised native workers

Landlock commands, Git snapshots, declared-file MCP workers and managed CLI sessions reserve from the same
worker, CPU and memory pool as Docker, gVisor and ordinary Firecracker workers
using the same supervisor state directory. A delegated cgroup owns the native
worker and its descendants. A new cgroup is limited and durably recorded before
the runtime joins it and executes the worker. No ordinary helper or unrestricted
host execution is used when the delegated service is unavailable.

## Desktop setup

Initialize a Linux project and check the actual isolation path:

```sh
symbi init --sandbox landlock --profile assistant --dir my-agent
cd my-agent
symbi doctor
```

The generated agents inherit the project's Landlock selection and no Docker
Compose file is created. `doctor` checks the configured boundary, supervised
launch, native workspace namespaces and private loopback with inherited diagnostic
connections. Each successful check confirms delegated worker cleanup. When
`[managed_cli]` is configured, it also runs the selected CLI's bounded `--version`
inside the managed boundary. The checks require Python 3 at `/usr/bin/python3`;
see [native development](landlock-development.md) for diagnostic scope and limits.
It does not require Docker, Qdrant or unused HTTP ports for this CLI workflow.
Provider credentials and explicit tool/file policies are separate setup steps.

The first native worker automatically starts a transient systemd user service;
`doctor` exercises this same path. No unit file or root privileges are needed.
One deterministic `symbi-workers-<pool hash>.service` name identifies each
canonical supervisor state directory, so concurrent launches use the same pool.
The service survives the invoking command, restarts on failure, and remains
available until stopped or the user manager exits. `doctor` prints its name and
the stop command. Drain active work before stopping or upgrading it: stopping
the service kills its workers, and an older incompatible service is refused.

## Requirements and externally managed services

Use an unprivileged systemd user service with cgroup v2 CPU, memory and PID
controllers, `cgroup.kill`, and `DelegateSubgroup` support. The service and runtime
must run as the same non-root user. The runtime also requires Landlock ABI 6 and
native little-endian x86_64 or aarch64 seccomp support.

Automatic setup requires `/usr/bin/systemd-run` with `--expand-environment`
support as well as the delegation features above. Unsupported systems fail
closed with the unit name and diagnostic commands; there is no host fallback.
An existing ordinary helper must be drained and stopped before this pool can
be served by a delegated service.

Operators who prefer an installed service can save the following unit as
`~/.config/systemd/user/symbi-workers.service`, replacing the
binary path with the installed build. Stop and drain any existing helper that
owns this state directory before starting it; retain its lease records.

```ini
[Unit]
Description=Symbiont delegated worker supervisor

[Service]
Type=notify
ExecStart=/usr/local/bin/symbi __sandbox_supervisor --state-dir %h/.symbiont/sandbox-leases --delegated-workers
Delegate=cpu memory pids
DelegateSubgroup=manager
WatchdogSec=5s
TimeoutStopSec=5s
KillMode=control-group
SendSIGKILL=yes
Restart=on-failure
RestartSec=1s
NoNewPrivileges=yes

[Install]
WantedBy=default.target
```

The supervisor checks the live unit's notify type, kill mode, forced kill,
watchdog and stop timeout settings. It also checks its manager subgroup and
controller delegation. Weaker or unavailable settings refuse service startup.

```sh
systemctl --user daemon-reload
systemctl --user enable --now symbi-workers.service
export SYMBIONT_SANDBOX_STATE_DIR="$HOME/.symbiont/sandbox-leases"
```

To require that external service and disable automatic startup, set
`[sandbox.landlock.supervisor] service_uid` to the operator's numeric UID. A
missing external service is then an error, including during `doctor`.

Use the same absolute state directory in every runtime sharing this allocation.
Configure its private `admission.conf` as described in [shared budgets](shared-budgets.md).
This service can also handle ordinary Docker/gVisor/Firecracker leases. The
root-managed Firecracker jailer service remains a separate ownership domain;
partition its capacity deliberately. A user manager that stops at logout also
stops its workers; unattended deployments need an operator-managed user-manager
lifetime policy.

## Worker configuration

```toml
[sandbox]
tier = "landlock"

[sandbox.landlock]
abi_floor = 6
require_network = true
memory_mib = 512
cpu_millis = 1000
pids_limit = 128
max_output_bytes = 10485760
max_execution_time = { secs = 300, nanos = 0 }
```

These are defaults. `cpu_millis = 1000` represents one CPU; the minimum is 10.
The effective lifetime is the smaller of the caller's deadline and this profile.
Supported source/agent limits tighten the profile; they cannot raise it. Startup
has at most ten seconds. Memory swap is disabled for the leaf, and an OOM kills
the whole worker group. Native workspaces use bounded private tmpfs and per-file limits; staged host files reserve shared capacity. Raw SDK workers with direct writable roots still need separately managed disk quotas.

The runtime opens the admitted cgroup before fork, writes only its child into it,
installs the prepared Landlock and syscall restrictions, then closes extra
inherited descriptors at exec. Configured roots cannot expose `/proc`, `/sys`,
or supervisor state, including resolved aliases and parent directories. Roots
remain trusted operator selections and authorize their hierarchy for the worker
lifetime on the low-level raw worker route. Governed development commands instead receive per-operation snapshots in private workspaces. See [Linux development](landlock-development.md) for the file ceilings, namespace requirements and managed broker setup.

## Cleanup, recovery and inspection

Caller disconnect, cancellation and deadlines all trigger whole-cgroup cleanup,
including descendants that create new sessions. A failed supervisor or stalled
watchdog causes the service manager to stop the service cgroup and its workers.
The durable lease binds host boot and cgroup directory identity. Recovery never
kills a replacement directory belonging to another identity.

Capacity is released only after the original cgroup is removed. Failed kill,
inspection or directory removal retains the charge and retries reconciliation.
Removal also prevents a delayed spawn through an already-open `cgroup.procs`
descriptor. A lost acknowledgement remains an uncertain operation even when the
service manager has stopped its worker; cleanup does not prove whether effects
occurred, authorize replay or complete the run journal.

**Worker capacity** now includes `landlock` rows, originating run links where
available, and separately sampled `landlock_cgroup` CPU/memory counters including
descendants. An empty, removed or changing worker is unavailable rather than
zero. See [worker capacity](worker-capacity.md). Supervisor protocol 7 and matching
implementation fingerprints require draining/restarting an older service during
upgrade. Boundary audit version 4 records the resource and supervision contract.

## Validation and scope

`scripts/test-landlock-onboarding.py` runs the shipping `init` and `doctor`
commands in fresh disposable projects. It checks concurrent automatic startup,
real restricted execution and cleanup, explicit external-service refusal,
unsupported ABI refusal, and restart after the automatic unit is stopped.
Missing kernel or service-manager capabilities fail the test.

`scripts/test-landlock-supervision.py` tests real delegated cgroups: shared
Landlock/Docker admission, PID limits, measurements, detached descendants, delayed
launch refusal, cleanup failure, deadlines, supervisor death and watchdog expiry.
`scripts/test-landlock-boundary.py` tests signed shipping MCP work, communication
and filesystem restrictions, and runtime SIGKILL with a detached MCP descendant.
Both use disposable local fixtures and require a built binary via `--binary` and
an evidence destination via `--report`. They do not estimate adaptive escape rates.
Pass `--automatic-supervisor` to the boundary test to bootstrap through `doctor`
and run the signed MCP, access-denial and runtime-crash checks on the automatically
created service.

The raw `PreparedDomain` SDK primitive installs kernel access restrictions only;
its caller owns resource supervision. Governed MCP and `CliExecutor` launches use
the delegated lease. Public one-shot commands, custom parsers, declared-file
staging, PTYs and the shipping managed CLI setup remain unsupported for Landlock.
Registered HTTP/scheduled agents still cannot select it. Existing supervised
backends remain available for those routes.
