---
layout: default
title: Firecracker Setup (Tier 3)
nav_order: 8
---

# Firecracker Setup (Tier 3)

Symbiont's Tier 3 sandbox runs each agent inside a dedicated Firecracker microVM with a real Linux kernel and an operator-supplied root filesystem. This is the strongest isolation tier the runtime ships — agent code never touches the host kernel, never shares a kernel surface with other tenants, and is destroyed at the end of every execution.

This guide covers what you need to produce before pointing `symbi init --sandbox tier3` at it.

---

## Why operator-supplied?

Firecracker is fundamentally different from Docker or gVisor. It boots a real kernel against a disk image you provide, rather than running a process inside a shared kernel namespace. That means **you** control the userspace contract — what's installed, how init runs, how the workload is delivered. Symbiont can't bake in a one-size-fits-all rootfs because the trust assumptions, performance budget, and runtime dependencies vary by deployment.

The runtime side is fully implemented: `FirecrackerRunner` writes a per-execution VM config, drops your code into a host-side work directory, execs `firecracker --no-api --config-file <path>`, and captures output from the serial console. What you bring is the kernel + rootfs that knows how to consume the work directory.

---

## Prerequisites

You need three artifacts on the host:

1. **`firecracker` binary** — install from [firecracker-microvm/firecracker releases](https://github.com/firecracker-microvm/firecracker/releases). `symbi doctor` reports whether it's reachable on `$PATH`.
2. **A boot kernel image (`vmlinux`)** — uncompressed ELF, not bzImage. Either download a prebuilt CI artifact (fastest) or build from upstream Linux with Firecracker's recommended config (~10 minutes on a laptop).
3. **A root filesystem image (`rootfs.ext4`)** — a small Linux userland (Alpine, BusyBox, or Debian) plus an init script that implements the Symbiont in-VM contract (see below).

KVM must also be available on the host (`/dev/kvm` readable by the user running `symbi up`).

---

## Quickstart recipe

This produces a working ~30 MB `rootfs.ext4` and pulls a prebuilt `vmlinux` from Firecracker's CI artifacts. It's intentionally minimal — no network, no Python, no extra runtimes. Adapt for your workload.

### 1. Download a prebuilt kernel

```bash
ARCH="$(uname -m)"
curl -fsSL "https://s3.amazonaws.com/spec.ccfc.min/firecracker-ci/v1.7/${ARCH}/vmlinux-5.10.210" \
  -o /var/lib/firecracker/vmlinux
```

This is the kernel Firecracker's CI tests against — known-good for the runtime.

### 2. Build a minimal Alpine rootfs

```bash
sudo apt-get install -y debootstrap   # or use apk on Alpine hosts

# 256 MB image — size to your workload's working set + scratch space
dd if=/dev/zero of=/var/lib/firecracker/rootfs.ext4 bs=1M count=256
mkfs.ext4 -F /var/lib/firecracker/rootfs.ext4

mkdir -p /mnt/rootfs
sudo mount -o loop /var/lib/firecracker/rootfs.ext4 /mnt/rootfs

# Bootstrap Alpine into the image
sudo apk -X http://dl-cdn.alpinelinux.org/alpine/latest-stable/main \
  -U --allow-untrusted --root /mnt/rootfs --initdb \
  add alpine-base busybox openrc
```

### 3. Install the Symbiont in-VM init contract

The init script that runs as PID 1 inside the microVM must implement the protocol below. Save this as `/mnt/rootfs/sbin/symbi-init`:

```sh
#!/bin/sh
# Symbiont in-VM init — minimal example.
#
# Contract:
#   /work/code      — the agent payload to execute (mode 0755).
#   /work/env.json  — JSON object of env vars the host wants exposed.
#   stdout/stderr   — written to the serial console (ttyS0).
#   exit            — `reboot -f` triggers Firecracker shutdown via panic=1.

mount -t proc  proc  /proc
mount -t sysfs sys   /sys
mount -t devtmpfs dev /dev
mount -t tmpfs tmp /tmp

# Bring up the serial console for stdout capture.
exec </dev/console >/dev/console 2>&1

# /work is mounted by the host (e.g. via a vsock-backed share or a second
# block device — both wiring patterns are valid). Wait briefly for it to
# appear, then run the payload.
for _ in 1 2 3 4 5; do
  [ -x /work/code ] && break
  sleep 0.2
done

if [ ! -x /work/code ]; then
  echo "symbi-init: /work/code not found — host failed to deliver payload" >&2
  reboot -f
fi

# Export env from /work/env.json (one var per line: KEY=VALUE).
if [ -r /work/env.json ]; then
  # Tiny JSON-to-shell-env extractor; replace with `jq` if present.
  awk 'BEGIN{RS=","; FS=":"} /"/{
    gsub(/[ \t"{}\n]/,""); print $1"="$2
  }' /work/env.json | while IFS= read -r line; do
    [ -n "$line" ] && export "$line"
  done
fi

/work/code
RC=$?
echo "symbi-init: exit=$RC"
reboot -f
```

```bash
sudo cp symbi-init /mnt/rootfs/sbin/symbi-init
sudo chmod +x /mnt/rootfs/sbin/symbi-init

# Tell the kernel to run it as init.
sudo ln -sf /sbin/symbi-init /mnt/rootfs/init
```

### 4. Unmount and verify

```bash
sudo umount /mnt/rootfs
e2fsck -f /var/lib/firecracker/rootfs.ext4
```

### 5. Wire it into a Symbiont project

```bash
symbi init --profile assistant --sandbox tier3 \
  --firecracker-kernel /var/lib/firecracker/vmlinux \
  --firecracker-rootfs /var/lib/firecracker/rootfs.ext4
```

`symbi init` validates both paths exist before scaffolding. The generated `symbiont.toml` ends up with:

```toml
[sandbox]
tier = "tier3"

[sandbox.firecracker]
kernel_image_path = "/var/lib/firecracker/vmlinux"
rootfs_path       = "/var/lib/firecracker/rootfs.ext4"
vcpus             = 1
mem_mib           = 512
rootfs_read_only  = true
```

Then `symbi up` runs each agent inside its own microVM.

---

## The in-VM init contract

This is what your rootfs must implement. The runtime side of this contract is fixed; the userspace side is yours.

| What the host writes | Where | Format |
|----------------------|-------|--------|
| Agent code | host work dir → mount as `/work/code` inside the VM | executable file |
| Env vars | host work dir → mount as `/work/env.json` | JSON object |

| What the VM must do |
|---------------------|
| Mount `/proc`, `/sys`, `/dev`, `/tmp` |
| Bring up `ttyS0` as the console (Firecracker's serial output → host stdout) |
| Locate `/work/code` and execute it |
| Capture exit code, write to console |
| Halt the VM (`reboot -f` + `panic=1` boot arg = shutdown) |

The runtime cleans up the host work dir after the VM exits.

### Choosing a transport for `/work`

The example above leaves `/work` as a placeholder mount because there are two reasonable wiring patterns and the right choice depends on your kernel config:

- **vsock** — the host listens on a vsock CID, init connects, receives a tarball, extracts to `/work`. Cleaner, no second drive, but requires `CONFIG_VSOCKETS=y` and a userspace vsock client in the rootfs.
- **Second block device** — pass the host work dir as an ext4 image attached as `/dev/vdb`, init mounts it at `/work`. Simpler kernel config, but adds a per-execution image step on the host.

Symbiont's `FirecrackerRunner` currently writes the work dir to the host filesystem under `$TMPDIR/symbi-fc-<uuid>/` and expects your init to know how to surface it inside the VM. Pick the transport that fits your kernel and update `symbi-init` accordingly.

---

## Hardening checklist

Before turning this on for production traffic:

- [ ] **Kernel config** — strip what you don't need. Firecracker recommends a guest kernel built with their `microvm-kernel-x86_64.config`.
- [ ] **Rootfs read-only** — keep `rootfs_read_only = true` in `[sandbox.firecracker]`. Agents that need scratch space write to `/tmp` (tmpfs).
- [ ] **No SSH, no getty** — the only entry point should be the init script.
- [ ] **Drop privileges** — run `firecracker` itself under a dedicated unprivileged user with `jailer`. The runtime will pick this up if you set `firecracker_binary` to a wrapper script.
- [ ] **Memory budget** — set `mem_mib` to the smallest value your workload tolerates. Lower = faster boot.
- [ ] **Network** — agents that don't need outbound network shouldn't get a TAP device. Symbiont doesn't auto-create one.

---

## Troubleshooting

**`Firecracker is not available at 'firecracker'`** — install the binary or set `firecracker_binary` in `[sandbox.firecracker]` to its path.

**`Firecracker kernel image not found at ...`** — the path in `kernel_image_path` doesn't exist. `symbi init` validates this at scaffold time so this shouldn't happen post-init unless you moved files.

**VM boots but agents hang** — your init script likely isn't finding `/work/code`. Connect to the serial console and check the in-VM logs. The reference init has a 1-second wait loop; lengthen it if your work-dir transport is slow to attach.

**`/dev/kvm` permission denied** — the user running `symbi up` needs read access. Add them to the `kvm` group: `sudo usermod -aG kvm $USER`, then re-login.

---

## What Symbiont doesn't ship

By design:

- **No rootfs builder.** Operator territory — rootfs construction varies too much by deployment for a one-size-fits-all script.
- **No kernel build script.** The Firecracker project already publishes a prebuilt kernel and the recommended config; we link to those rather than vendoring.
- **No network plumbing.** TAP setup, NAT, egress policy — all per-deployment. Wire up via `network-interfaces` in the VM JSON config if you need it.
- **No secret-into-rootfs path.** Pass secrets via `env.json` (which lives only on the host work dir for the duration of the execution), not by baking them into the image.

For most workloads, Tier 1 (Docker) or Tier 2 (gVisor) is the right choice. Pick Tier 3 when you specifically need a kernel boundary — multi-tenant untrusted code, regulated data, or workloads where the syscall-filter granularity of gVisor isn't enough.
