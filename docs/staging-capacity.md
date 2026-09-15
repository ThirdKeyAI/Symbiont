# Shared snapshot capacity and cleanup

Linux Docker and gVisor Git queries and declared command, MCP and terminal file
grants reserve private host staging capacity in the selected supervisor state
directory. Runtime processes sharing that directory share one staging pool.
Reservation happens before copying payload bytes, under a cross-process file
lock. Exhaustion returns `shared staging capacity exhausted` without creating a
new snapshot or worker. Existing route and worker limits still apply.

The operator-owned `staging.conf` file is JSON, with these defaults:

```json
{
  "max_snapshots": 16,
  "reserved_bytes": 1073741824
}
```

Both limits must be positive; at most 512 snapshots are supported. The file must
be private, mode 0600, owned by the runtime user, and neither a symbolic nor a
hard link. Missing configuration is created with the defaults. Each admission
reads it under the pool lock. Drain work before changing limits; reducing them
below retained charges refuses new copies until cleanup frees capacity.

A Git query reserves its full 128 MiB payload ceiling plus 4 KiB for each bounded
entry and eight bookkeeping entries: 216,170,496 bytes per query. A declared file
grant reserves actual input bytes, its maximum new-output bytes, and 4 KiB per
input plus four bookkeeping entries. These are conservative application
reservations, not a filesystem block quota or hardware discovery. Other host
writers and filesystem overhead can still exhaust the volume. Leave free space
for journals, durable publication candidates and other services. In-memory source
broker results and worker scratch files have separate bounds.

Snapshots live at `staging/UUID/data` beneath the private supervisor directory.
Only the required files or Git subdirectories are mounted. The accounting files
and containing directories remain inaccessible to workers. Project configuration
cannot supply the runtime's staging references or authorize mounts of supervisor
state. Trusted SDK integrations remain responsible for their runtime authority.

A live caller holds a shared lock on its reservation. Before worker registration,
the supervisor pins those same reservations; its durable worker record then
retains their IDs. Creation uncertainty, failed worker cleanup and supervisor
restart preserve those references. Cancellation or caller death alone therefore
does not return space while a worker might still use it.

Cleanup requires both no live caller lock and no retained worker reference. It
removes the private data and syncs that removal before deleting the charge. A
failed cleanup retains state for another attempt. Admission, normal caller
release and the running supervisor's periodic sweep perform reconciliation. If a
process dies during preparation before any supervisor is running, the next
admission or supervisor startup performs cleanup. A malformed or unsafe staging
entry refuses further staging admission; supervisor worker cleanup continues.
Preserve the entry and resolve the underlying problem rather than deleting its
reservation to manufacture available capacity.

This recovery removes unreferenced private copies. It does not resume execution,
replay tools, settle provider charges or publish interrupted outputs. Existing
publication candidates and signed recovery receipts retain their separate
lifecycle. Legacy temporary copies created before this protocol are not adopted
or automatically deleted. Staging across
separate service identities remains outstanding.

Deploy matching runtime and supervisor binaries, drain old workers and preserve
durable state during upgrade. The changed worker protocol refuses an older
helper instead of admitting a worker without its staging references.

## Validation

Focused supervisor tests cover competing reservations, live ownership, durable
uncertain worker references across reopening, interrupted preparation, unsafe
paths and invalid configuration. The shipping Git fixture runs concurrent CLI
processes against a one-snapshot pool, verifies the second receives a capacity
refusal while the first retains its charge, and verifies useful completion after
release. SIGKILL cases interrupt both preparation and worker creation; they
require automatic staging cleanup and confirmed worker removal where applicable,
with no test-side deletion of private copies.

```bash
python3 scripts/test-git-source.py --binary target/debug/symbi \
  --report /tmp/git-staging-e2e.json
```

See [shared budgets](shared-budgets.md), [Git source queries](git-source-queries.md)
and [filesystem grants](filesystem-grants.md).

Firecracker Git snapshots and declared command/MCP/PTY files reserve from this pool before their
bounded byte transfer. The detached runtime owner holds the lease through
transfer and VM cleanup. Because the VM cannot access those host files, a killed
runtime leaves copies safe to reap while independent VM cleanup continues. This
does not enable cross-identity access to a root-managed state directory.
