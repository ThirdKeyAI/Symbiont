# Worker capacity and usage

Open **Worker capacity** in the operations console with an administrative bearer
token. The page reads the running supervisor selected by the runtime project's
default sandbox profile. It shows one pool shared by all governed worker routes
using that supervisor directory. Separate directories and hosts are separate
pools; this is not an account-wide or cluster-wide dashboard.

**Refresh capacity** obtains a new timestamped snapshot and clears earlier
measurements. The three cards distinguish reserved worker slots, memory and CPU
from remaining capacity and configured pool limits. Retained worker rows show
their lease ID, backend, creation state and resource reservation. A retained
record is not proof that its worker is currently running: cleanup may have failed.

**Measure usage** requests one sample for a created worker. It does not refresh
the capacity snapshot. Each sample has its own observation time. Neither the
page nor the API polls automatically, releases a reservation, starts a missing
supervisor, reconciles a lease or retries work.

## Interpreting the figures

Reservations charge the worker's configured allowance, including retained charges
after interruption or failed cleanup. Low measured use does not increase available
capacity. Only confirmed removal releases the reservation. The admission warning
means an entire capacity axis is exhausted or accounting metadata is unknown;
even without that warning, a particular request can exceed the remaining CPU or
memory allowance or an additional route limit.

Docker and gVisor samples use the retained worker's Docker client and exact
container identity. CPU percentage and memory/limit retain Docker CLI display
precision. On Linux, Docker's memory display subtracts inactive file cache; it
is not raw RSS or the admission reservation. See the
[Docker stats reference](https://docs.docker.com/reference/cli/docker/container/stats/).

Ordinary Firecracker samples report the VMM process's resident memory in bytes
and cumulative CPU time in microseconds. The UI converts those units to MiB and
seconds. The managed host service instead uses its VMM cgroup's `memory.current`
and `cpu.stat` counters. These describe the VMM process or cgroup, not individual
applications inside the guest. The reader checks the recorded host boot identity,
PID and process start time, and refuses exited processes or changed lease records.
The managed host cgroup path still requires actual privileged deployment validation.

Unknown values are explicit. A legacy lease without resource metadata makes the
aggregate memory and CPU balances unknown and blocks admission. An unavailable
sample replaces an earlier sample with **Usage unavailable**. A failed capacity
refresh clears the previous snapshot and displays **Capacity unavailable**.
Neither error is displayed as an empty pool or zero usage.

The [Run Inspector](run-inspector.md) separately verifies signed historical
budgets, permissions, parent/child references and outcomes. Capacity responses
are live administrative observations, not signed audit evidence or per-run
attribution. A worker lease cannot yet be navigated to its originating run.

## API and deployment

Both endpoints require administrative authority:

```text
GET /api/v1/sandbox/capacity
GET /api/v1/sandbox/workers/{lease_uuid}/usage
```

The snapshot contains `observed_at_unix_ms`, `state_dir`, `limits`, `reserved`,
`available`, `unknown_resource_leases`, `admission_blocked` and `workers`. Each
worker contains `lease`, `backend`, `phase` and nullable `resources`. CPU
reservations use `cpu_nanos`: 1,000,000,000 represents one CPU. Memory reservations
use bytes. Unknown aggregate resource values are JSON `null`.

A measurement contains its `lease`, `observed_at_unix_ms` and `source`:

| Source | Populated measurement fields |
| --- | --- |
| `docker_cli` | `cpu_percent`, `memory_usage` strings |
| `vmm_process` | `cpu_time_micros`, `memory_bytes` numbers |
| `vmm_cgroup` | `cpu_time_micros`, `memory_bytes` numbers |

Fields not supplied by that measurement source are `null`. Missing authentication
receives 401 and scoped keys receive 403. An unavailable, busy, mismatched or
unsupported supervisor, missing lease, uncertain creation or failed measurement
receives 503 with code `CAPACITY_UNAVAILABLE`. Successful and handler-generated
error responses use `Cache-Control: no-store`.

The supervisor allows two concurrent capacity readers and two concurrent usage
samplers. Docker sampling has a four-second timeout and 64 KiB output bound; the
runtime supervisor request has a six-second timeout. These requests also share
the supervisor's connection limit with worker controllers, so saturation can make
inspection unavailable. Do not interpret such a refusal as available capacity.

Runtime and supervisor must have matching implementations. Drain active work
before upgrading, retain the durable state directory, and restart the helper with
the matching build. See [shared admission](shared-budgets.md) for pool configuration.

## Focused validation

The Docker shipping fixture checks real reservations and measured usage, useful
signed output, failed sampling, cleanup failure, supervisor restart and unknown
legacy metadata. It uses local scripted inference and synthetic credentials:

```bash
python3 scripts/test-worker-capacity.py --binary target/debug/symbi --report /tmp/capacity.json
```

Add `--serve --api-port 18086` for browser checks. Build the console, then run
`SYMBI_RUNTIME_URL=http://127.0.0.1:18086 npm run preview` from `crates/symbi-a2ui`.
The fixture prints its synthetic token and private `observer/control` path. Write
`start-worker` to that file once the browser is ready for a 20-second worker; use `stop-supervisor` and
`start-supervisor` to check outage display, and `finish` to stop the fixture. It
expires after 15 minutes and only controls its own workers and services.

The separate KVM fixture accepts existing, read-only Firecracker artifacts. It
checks actual process counters, useful signed completion and killed-worker
measurement refusal. It does not validate privileged host provisioning:

```bash
python3 scripts/test-worker-capacity-vm.py --binary target/debug/symbi \
  --firecracker /absolute/path/firecracker --kernel /absolute/path/vmlinux \
  --rootfs /absolute/path/rootfs.ext4 --report /tmp/capacity-vm.json
```
