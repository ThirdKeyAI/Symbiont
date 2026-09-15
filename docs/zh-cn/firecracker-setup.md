---
layout: default
title: Firecracker 设置（第 3 层）
nav_order: 8
nav_exclude: true
---

# Firecracker 设置（第 3 层）

Firecracker 在具有独立客户机内核的 microVM 中执行工作负载。
主机内核、VMM、客户机镜像和运维配置仍属于受信任的组件。
这种隔离并不保证所有逃逸路径都已被排除。

第 3 层是开源运行时的一部分。它不需要许可证密钥，也不需要 Enterprise 构建：`symbi-sandbox-guest` 和 `symbi-sandbox-supervisor` 这两个 crate 都包含在本仓库中，因此你可以自行构建、审计并复现来宾镜像。

最新设置步骤请参阅 [英文指南](../firecracker-setup.md)。其中说明了匹配的
`symbi-sandbox-guest`、内核、rootfs 以及受限的 vsock 协议。
主机工作目录不会自动传入 VM；工具结果也不通过串口控制台获取。

普通监督进程以用户账户运行，在 [共享池](../shared-budgets.md) 中预留客户机的
CPU 和内存。它不配置 jailer 或主机 cgroup，也不预留 VMM 的额外内存。

可选的 [托管主机服务](../firecracker-host-service.md) 提供经过批准的运行文件、
jailer、每个 VMM 的独立身份、主机 cgroup，以及包含 VMM 开销的内存预留。
systemd 监督服务和清理过程。`service_uid = 0` 要求使用该服务；服务不可用时，
不会启动本地替代进程。Docker/gVisor 需要单独分配容量。

构建、针对性测试和 Docker 回归 E2E 已通过。包含 KVM、服务故障和 watchdog 的
特权主机 E2E 仍待运行。在目标主机上通过这些测试之前，不能将此配置视为已完成部署验证。
[主机服务指南](../firecracker-host-service.md) 提供了配置与测试命令。
