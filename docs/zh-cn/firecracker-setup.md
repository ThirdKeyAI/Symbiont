---
layout: default
title: Firecracker 设置（第 3 层）
nav_order: 8
nav_exclude: true
---

# Firecracker 设置（第 3 层）

Symbiont 的第 3 层沙箱将每个智能体运行在一个独立的 Firecracker microVM 中，使用真正的 Linux 内核和由运维方提供的根文件系统。这是运行时附带的最强隔离层级 —— 智能体代码永远不会触及主机内核，不会与其他租户共享任何内核表面，并且在每次执行结束后都会被销毁。

本指南介绍在用 `symbi init --sandbox tier3` 指向之前你需要准备的内容。

---

## 为什么由运维方提供？

Firecracker 与 Docker 或 gVisor 在本质上不同。它针对你提供的磁盘映像引导一个真正的内核，而不是在共享的内核命名空间中运行一个进程。这意味着 **由你**控制用户空间契约 —— 安装了什么、init 如何运行、工作负载如何投递。Symbiont 无法内置一个一刀切的 rootfs，因为信任假设、性能预算和运行时依赖会因部署而异。

运行时端是完全实现的：`FirecrackerRunner` 写入逐次执行的 VM 配置，将代码放入主机端的工作目录，执行 `firecracker --no-api --config-file <path>`，并从串口控制台捕获输出。你需要提供的是知道如何消费该工作目录的内核 + rootfs。

---

## 前置条件

主机上需要三个工件：

1. **`firecracker` 二进制** —— 从 [firecracker-microvm/firecracker releases](https://github.com/firecracker-microvm/firecracker/releases) 安装。`symbi doctor` 会报告它是否在 `$PATH` 上可用。
2. **引导内核映像（`vmlinux`）** —— 未压缩的 ELF，而非 bzImage。可以下载预构建的 CI 工件（最快），或基于上游 Linux 使用 Firecracker 推荐的配置自行构建（在笔记本上约 10 分钟）。
3. **根文件系统映像（`rootfs.ext4`）** —— 一个小型 Linux 用户空间（Alpine、BusyBox 或 Debian），加上一个实现 Symbiont VM 内启动契约的 init 脚本（见下文）。

主机上还必须有可用的 KVM（运行 `symbi up` 的用户对 `/dev/kvm` 可读）。

---

## 快速上手配方

下面的步骤会生成一个可工作的约 30 MB `rootfs.ext4`，并从 Firecracker 的 CI 工件中拉取一个预构建的 `vmlinux`。它特意保持最小化 —— 没有网络、没有 Python、没有额外运行时。请根据你的工作负载进行调整。

### 1. 下载预构建内核

```bash
ARCH="$(uname -m)"
curl -fsSL "https://s3.amazonaws.com/spec.ccfc.min/firecracker-ci/v1.7/${ARCH}/vmlinux-5.10.210" \
  -o /var/lib/firecracker/vmlinux
```

这是 Firecracker CI 测试所用的内核 —— 对运行时已知良好。

### 2. 构建一个最小化的 Alpine rootfs

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

### 3. 安装 Symbiont VM 内 init 契约

作为 microVM 内 PID 1 运行的 init 脚本必须实现下述协议。将以下脚本保存为 `/mnt/rootfs/sbin/symbi-init`：

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

### 4. 卸载并校验

```bash
sudo umount /mnt/rootfs
e2fsck -f /var/lib/firecracker/rootfs.ext4
```

### 5. 接入 Symbiont 项目

```bash
symbi init --profile assistant --sandbox tier3 \
  --firecracker-kernel /var/lib/firecracker/vmlinux \
  --firecracker-rootfs /var/lib/firecracker/rootfs.ext4
```

`symbi init` 会在脚手架之前校验两个路径是否存在。生成的 `symbiont.toml` 最终会包含：

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

之后 `symbi up` 会让每个智能体在自己的 microVM 中运行。

---

## VM 内 init 契约

这是你的 rootfs 必须实现的内容。该契约的运行时端是固定的；用户空间端由你负责。

| 主机写入的内容 | 位置 | 格式 |
|----------------|------|------|
| 智能体代码 | 主机工作目录 → 在 VM 内挂载为 `/work/code` | 可执行文件 |
| 环境变量 | 主机工作目录 → 挂载为 `/work/env.json` | JSON 对象 |

| VM 必须执行的操作 |
|-------------------|
| 挂载 `/proc`、`/sys`、`/dev`、`/tmp` |
| 将 `ttyS0` 设为控制台（Firecracker 串口输出 → 主机 stdout） |
| 定位 `/work/code` 并执行它 |
| 捕获退出码并写入控制台 |
| 关停 VM（`reboot -f` + `panic=1` 引导参数 = 关机） |

VM 退出后，运行时会清理主机工作目录。

### 为 `/work` 选择传输方式

上面的示例把 `/work` 留作占位挂载，因为有两种合理的接线方式，正确的选择取决于你的内核配置：

- **vsock** —— 主机在 vsock CID 上监听，init 连接后接收一个 tarball 并解压到 `/work`。更整洁，无需第二个驱动，但需要 `CONFIG_VSOCKETS=y` 以及 rootfs 中的用户态 vsock 客户端。
- **第二个块设备** —— 把主机工作目录作为附加为 `/dev/vdb` 的 ext4 映像传入，init 将其挂载到 `/work`。内核配置更简单，但在主机上增加了逐次执行的镜像构建步骤。

Symbiont 的 `FirecrackerRunner` 当前会把工作目录写到主机文件系统的 `$TMPDIR/symbi-fc-<uuid>/` 下，并期望你的 init 知道如何在 VM 内呈现它。请选择适合你内核的传输方式，并相应更新 `symbi-init`。

---

## 加固清单

在为生产流量启用之前：

- [ ] **内核配置** —— 剥离不需要的内容。Firecracker 推荐使用其 `microvm-kernel-x86_64.config` 构建 guest 内核。
- [ ] **rootfs 只读** —— 在 `[sandbox.firecracker]` 中保留 `rootfs_read_only = true`。需要临时空间的智能体写入 `/tmp`（tmpfs）。
- [ ] **不要 SSH，不要 getty** —— 唯一的入口应当是 init 脚本。
- [ ] **降权** —— 通过 `jailer` 让 `firecracker` 本身以专用的非特权用户身份运行。如果你将 `firecracker_binary` 设置为一个包装脚本，运行时会自动接入。
- [ ] **内存预算** —— 把 `mem_mib` 设为工作负载能容忍的最小值。越小 = 启动越快。
- [ ] **网络** —— 不需要出站网络的智能体不应分配 TAP 设备。Symbiont 不会自动创建。

---

## 故障排查

**`Firecracker is not available at 'firecracker'`** —— 安装该二进制，或将 `[sandbox.firecracker]` 中的 `firecracker_binary` 设为它的路径。

**`Firecracker kernel image not found at ...`** —— `kernel_image_path` 中的路径不存在。`symbi init` 会在脚手架阶段校验，因此 init 之后不会出现，除非你移动了文件。

**VM 启动但智能体卡死** —— 你的 init 脚本很可能没找到 `/work/code`。连接到串口控制台并查看 VM 内的日志。参考 init 实现使用 1 秒等待循环；如果你的工作目录传输较慢，请加长等待时间。

**`/dev/kvm` permission denied** —— 运行 `symbi up` 的用户需要读权限。把它加入 `kvm` 用户组：`sudo usermod -aG kvm $USER`，然后重新登录。

---

## Symbiont 不附带的内容

按设计：

- **不附带 rootfs 构建器。** 这是运维方的领域 —— rootfs 构建因部署不同差异太大，无法用一个一刀切的脚本。
- **不附带内核构建脚本。** Firecracker 项目已经发布预构建内核和推荐配置；我们链接到它们，而不是把它们 vendoring。
- **不附带网络管道。** TAP 配置、NAT、出口策略 —— 全部按部署而定。如果需要，请通过 VM JSON 配置中的 `network-interfaces` 接入。
- **不附带把秘密注入 rootfs 的路径。** 通过 `env.json`（仅在执行期间存在于主机工作目录中）传递秘密，而不是把它们烤进映像。

对大多数工作负载，第 1 层（Docker）或第 2 层（gVisor）才是正确选择。当你确实需要内核边界时再选择第 3 层 —— 多租户不受信任代码、受监管数据，或 gVisor 的系统调用过滤粒度不够的工作负载。
