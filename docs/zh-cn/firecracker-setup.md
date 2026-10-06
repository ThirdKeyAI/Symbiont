---
layout: default
title: Firecracker 设置（第 3 层）
nav_order: 8
nav_exclude: true
---

# Firecracker 设置（第 3 层）

第 3 层在全新的 Firecracker microVM 中运行一次性命令、自定义输出解析器、
MCP stdio 服务器、PTY 会话和托管 CLI 工作进程。所选的 ToolClad 边界和公开的
`FirecrackerRunner` 使用同一套来宾协议和独立的监督进程。VM 启动成功或 VMM 退出
都不能替代所请求命令的实际结果。
主机内核、VMM、客户机镜像和运维配置仍属于受信任的组件。
这种隔离并不保证所有逃逸路径都已被排除。

第 3 层是开源运行时的一部分。它不需要许可证密钥，也不需要 Enterprise 构建：`symbi-sandbox-guest` 和 `symbi-sandbox-supervisor` 这两个 crate 都包含在本仓库中，因此你可以自行构建、审计并复现来宾镜像。

最新设置步骤请参阅 [英文指南](../firecracker-setup.md)。其中说明了匹配的
`symbi-sandbox-guest`、内核、rootfs 以及受限的 vsock 协议。
主机工作目录不会自动传入 VM；工具结果也不通过串口控制台获取。
`symbi-sandbox-guest` 必须作为来宾 PID 1 运行，并与主机运行时使用同一个源码修订版
构建：握手会校验协议版本和来宾 crate 源码的指纹，过期镜像会在发送任何命令之前被拒绝。

固定的 `read_file`、`list_files` 和 `grep_files` 操作通过运行时的有界文件中转器
使用显式只读的 `source_roots`。这些根目录不会成为来宾挂载点，也不会被自动导入；
配置和限制参见 [源码查询](../source-queries.md)。已声明的命令/MCP/PTY 文件使用
有界的协议 5 字节传输，新的主机输出另有 `output_roots` 上限，参见
[文件授权](../filesystem-grants.md#firecracker-file-transfer)。Git 使用单独的
[有界快照流](../git-source-queries.md#firecracker-snapshots)，来宾文件系统为封闭状态。
从协议 1、2、3 或 4 升级后必须重建来宾镜像；旧的来宾指纹会被有意拒绝。

隔离的浏览器执行仍不可用。选择了不受支持的路径会显式失败。原生 HTTP 仍然是主机
中转操作，自定义解析器则被送入所选的 VM。参见
[命令隔离](../toolclad-command-boundary.md)和[分支覆盖范围](../containment-branch-guide.md)。

普通监督进程以用户账户运行，在 [共享池](../shared-budgets.md) 中预留客户机的
CPU 和内存。它不配置 jailer 或主机 cgroup，也不预留 VMM 的额外内存。

可选的 [托管主机服务](../firecracker-host-service.md) 提供经过批准的运行文件、
jailer、每个 VMM 的独立身份、主机 cgroup，以及包含 VMM 开销的内存预留。
systemd 监督服务和清理过程。`service_uid = 0` 要求使用该服务；服务不可用时，
不会启动本地替代进程。Docker/gVisor 需要单独分配容量。

构建、针对性测试和 Docker 回归 E2E 已通过。包含 KVM、服务故障和 watchdog 的
特权主机 E2E 仍待运行。在目标主机上通过这些测试之前，不能将此配置视为已完成部署验证。
[主机服务指南](../firecracker-host-service.md) 提供了配置与测试命令。
