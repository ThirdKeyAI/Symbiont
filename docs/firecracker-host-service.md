# Managed Firecracker host service

The managed host profile adds the Firecracker jailer, separate network and mount
namespaces, dedicated VMM identities and cgroup v2 limits to the existing guest
boundary. It is Linux-specific and requires an independently supervised root
service. The ordinary per-user supervisor remains available for development;
selecting `service_uid = 0` requires the managed service and disables automatic
helper startup or fallback.

## Current validation status

Workspace build, formatting, Clippy, focused supervisor tests and the existing
HTTP/scheduler/CLI Docker admission E2E pass. The privileged managed-host E2E must
still pass on the deployment host before this profile is considered validated.
The test command below covers actual KVM work, observed host restrictions,
supervisor death, a stopped supervisor, watchdog recovery and refusal while the
service is unavailable. Compilation and unit tests do not establish those host
properties.

## Provisioning

Build `symbi` and `symbi-sandbox-supervisor` from the same source. Supply matching
Firecracker and jailer releases and a guest image containing the matching
`symbi-sandbox-guest`. Review and pin those artifacts before installation. The
provisioner copies them into a new root-owned installation, verifies the copies,
records their hashes and refuses existing installation or service names. It does
not download binaries, update an existing installation or enable a service unless
requested.

```sh
sudo python3 scripts/provision-firecracker-host.py \
  --supervisor /path/to/symbi-sandbox-supervisor \
  --jailer /path/to/jailer --firecracker /path/to/firecracker \
  --kernel /path/to/vmlinux --rootfs /path/to/rootfs.ext4 \
  --client-user symbiont --name symbivmm \
  --uid-start 200000 --workers 16 --memory-mib 8192 --cpus 8 \
  --overhead-mib 128 --lifetime-seconds 300 --start --enable
```

The client account must already exist and be unprivileged. The VMM UID/GID range
must be unused; the installer creates non-login accounts for it. Each live or
unreconciled VM holds a distinct identity until cleanup completes. The service
refuses an identity range whose accounts or empty groups do not match the
installation. Choose the memory overhead and host allocation for the workload;
the defaults are configuration values, not automatic host sizing.

The generated `/var/lib/symbivmm/runtime.toml` contains the runtime configuration.
Merge it into the trusted project's `symbiont.toml`. Set
`SYMBIONT_SANDBOX_STATE_DIR=/var/lib/symbivmm/state` consistently for processes
sharing this allocation; conflicting backend state paths are refused. The
managed service accepts only Firecracker requests using its approved artifacts.
Docker/gVisor need their own deliberately partitioned allocation if deployed
alongside it; they cannot use this privileged endpoint for Docker commands.

## Enforced boundaries

The control socket authenticates the configured runtime UID. Lease records and
host policy remain root-owned; the client can traverse to its socket but cannot
list or rewrite the protected state. The runtime verifies the server UID and its
advertised jail support before submitting work.

Every launch checks the requested artifacts and lifetime against the host policy.
Guest memory plus the configured VMM overhead is charged to shared admission.
The jailer receives per-VM memory, CPU and process limits, disabled swap and
bounded file descriptors and file sizes. Host image links are read-only and
root-owned. VMMs cannot access the runtime's provider environment, audit keys or
host project directories through the jail.

Before returning a worker, the service checks its UID/GID, supplementary groups,
capabilities, no-new-privileges flag, seccomp, jail root, mount/network namespaces
and actual cgroup configuration. It exposes only that VM's private guest socket
to the runtime. A failed check triggers cleanup and returns an error.

The systemd slice bounds the combined VMM allocation. The supervisor has separate
CPU, memory and process limits, restarts on failure and must maintain its watchdog.
An `ExecStopPost` recovery command kills retained VM cgroups before releasing
their durable leases. Recovery does not depend on an artifact still matching its
launch hash. A damaged image therefore cannot prevent cleanup of an earlier VM.
The process manager, host kernel, approved artifacts and root-owned policy remain
trusted. This profile is based on Firecracker's
[jailer](https://github.com/firecracker-microvm/firecracker/blob/v1.16.1/docs/jailer.md)
and [production host guidance](https://github.com/firecracker-microvm/firecracker/blob/v1.16.1/docs/prod-host-setup.md).

## Deployment E2E

```sh
sudo python3 scripts/test-firecracker-host.py \
  --binary /path/to/symbi --supervisor /path/to/symbi-sandbox-supervisor \
  --jailer /path/to/jailer --firecracker /path/to/firecracker \
  --kernel /path/to/vmlinux --rootfs /path/to/rootfs.ext4 \
  --client-user symbiont --report /path/to/host-e2e.json
```

The fixture creates two temporary non-login accounts and ephemeral systemd units,
runs useful work through the shipping CLI and a synthetic loopback provider,
observes host restrictions, injects supervisor outages and verifies signed
journals. Its final cleanup removes the temporary allocation, units and accounts.
Failed cleanup remains a failed result even if emergency cleanup succeeds. The
host Docker daemon is not stopped or reconfigured. Keep the report, artifact hashes
and observer evidence. This fixed regression suite does not measure adaptive
escape probability.
