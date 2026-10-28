# 可选的出站访问控制

此功能在 3.0 中引入，需要显式启用。由运维人员管理的 `[outbound]` 配置将每个受管的
ToolClad HTTP 工具限制为命名操作，并固定目标源站、HTTP 方法、路径边界和请求次数。
未配置此功能时，保留原有行为。

凭据仅从显式选择的环境变量或密钥存储中读取。密钥存储失败时，不会回退到环境变量。
运行时在发送请求时注入凭据，并阻止包含该凭据原始值的响应。这不等于全面的数据
泄露防护，也不能检测所有编码或转换后的凭据。

每次执行都有独立且可撤销的权限。取消执行会阻止后续请求获准，但无法撤销目标服务
已经收到的请求所产生的效果。结果不确定的操作需要检查和核对，不能自动重试。
写入操作可以要求使用现有的终端精确审批流程。

默认拒绝路径参数。`allow_path_parameters = true` 允许普通参数，但仍拒绝
`..%3B` 等点路径段。可选的 `allowed_query_keys` 限制解码后的查询键，并拒绝重复键。
同一次运行中的并发调用共享成功获取的凭据；失败可重试，权限撤销时清空缓存。
日志哈希不包含 Vault 令牌或 AppRole Secret ID。响应头名称或值中原样反射的凭据
也会在计算哈希前被拒绝。通过声明的 `version` 标识有意进行的凭据轮换。

Vault 在读取前使用配置的方式认证，Vault 令牌或 TLS 环境变量不会覆盖这些设置。反射检查也考虑 HTTP 对响应头名称的小写化以及对响应头值前后空格的移除。

`symbi tools validate` 和 `symbi doctor` 检查配置时不会获取应用凭据。所有控制均属于
OSS；现有 Linux Landlock 部署无需额外代理、容器或虚拟机。其他通信路径和浏览器执行
不在此功能范围内。保存的 CLI 测试结果并不能证明沙箱隔离有效。

完整 TOML 示例、限制及 Rust 调用方的变更见[英文配置与迁移指南](/outbound-capabilities)。
[控制目录](/security/README)分别记录实现状态和验证状态。

Rust API 迁移：请在结构体字面量中添加 `HttpDef.capability: None`、`InferenceBrokerConfig.credential: None` 和 `ToolEffect::NetworkRequestStarted.outbound: None`。`SecurityTier` 不再实现全序（`Ord`）。请使用 `satisfies` 判断最低要求；Landlock 不满足容器或虚拟机的隔离要求，反之亦然。
