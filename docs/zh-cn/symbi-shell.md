# Symbi Shell —— 交互式智能体编排

> **Status: Beta.** `symbi shell` 可用于日常开发，但命令接口、按键绑定和持久化格式仍可能在次要版本之间变化。请在 [thirdkeyai/symbiont](https://github.com/thirdkeyai/symbiont/issues) 以 `shell` 标签提交问题。

`symbi shell` 是一个基于 [ratatui](https://ratatui.rs) 的终端 UI，用于构建、编排和运维 Symbiont 智能体。它位于与 `symbi up` 和 `symbi run` 相同的运行时之上，但将其作为具有对话式编写、实时编排和远程附加能力的交互式会话暴露出来。

## 何时使用该 shell

| 用例 | 命令 |
|------|------|
| 构建项目脚手架并在 LLM 辅助下迭代智能体、工具和策略 | `symbi shell` |
| 运行单个智能体直至完成，不使用交互式循环 | `symbi run <agent> -i <json>` |
| 启动完整运行时以处理 webhooks、cron 和聊天适配器 | `symbi up` |

shell 是编写工作的默认入口点。非交互式命令更适合 CI、cron 任务和部署流水线。

## 启动

```bash
symbi shell                    # 启动一个新会话
symbi shell --list-sessions    # 显示已保存会话并退出
symbi shell --resume <id>      # 按 UUID 重新打开会话
```

`--resume` 接受 UUID 或先前通过 `/snapshot` 保存的快照名称。

## 布局

shell 使用与现有滚动缓冲区共享终端的内联视口。从上到下，您会看到：

- **项目结构侧栏**（可切换）—— 当前项目的文件树，突出显示智能体、策略和工具。
- **跟踪时间线** —— 使用 ORGA 阶段着色的卡片，包含 Observe、Reason、Gate 和 Act，在 LLM 调用期间实时流式显示。
- **智能体卡片** —— 当前选中智能体的元数据、策略以及最近的调用记录。
- **输入行** —— 输入 `/command` 或自由文本。`@mention` 通过模糊补全引入路径和智能体。

通过 tree-sitter 语法文件，语法高亮覆盖 Symbiont DSL、Cedar 和 ToolClad 清单。

### 按键绑定

| 绑定 | 动作 |
|------|------|
| `Enter` | 提交输入（即使补全弹窗可见也生效） |
| `/` 或 `@` | 自动打开补全弹窗 |
| `↑` / `↓` | 浏览输入历史或弹窗条目 |
| `Ctrl+R` | 反向历史搜索 |
| `Tab` | 接受高亮的补全项 |
| `Esc` | 关闭弹窗 / 取消正在进行的 LLM 调用 |
| `Ctrl+L` | 清除可见输出缓冲区 |
| `Ctrl+D` | 退出 shell |

在 Zellij 下，shell 会检测到多路复用器并打印一条内联视口兼容性警告；如果您希望改为在备用屏幕缓冲区中运行，请使用 `--full-screen`。

## 命令目录

命令按用途分组。每个命令都接受 `help` / `--help` / `-h`，以打印简短的用法说明而不调度到编排器。

### 编写

| 命令 | 作用 |
|------|------|
| `/init [profile\|description]` | 构建 Symbiont 项目脚手架。已知的 profile 名称（`minimal`、`assistant`、`dev-agent`、`multi-agent`）会运行确定性脚手架；任何其他字符串会被视为自由文本描述，编排器据此选择 profile。 |
| `/spawn <description>` | 从散文生成一个 DSL 智能体。结果在写入 `agents/` 之前会针对项目约束进行验证。 |
| `/policy <requirement>` | 为所描述的需求生成 Cedar 策略并对其进行验证。 |
| `/tool <description>` | 生成一个 ToolClad `.clad.toml` 清单并对其进行验证。 |
| `/behavior <description>` | 生成一个可复用的 DSL behavior 块并对其进行验证。 |

编写命令仅在验证通过后才会写入磁盘。约束冲突会在跟踪时间线中以精确到行的错误说明。

### 编排

| 命令 | 模式 |
|------|------|
| `/run <agent> [input]` | 启动或重新运行一个智能体。 |
| `/ask <agent> <message>` | 向智能体发送消息并等待回复。 |
| `/send <agent> <message>` | 向智能体发送消息，不等待回复。 |
| `/chain <a,b,c> <input>` | 将每个智能体的输出管道送入下一个。 |
| `/parallel <a,b,c> <input>` | 使用相同输入并行运行多个智能体；聚合结果。 |
| `/race <a,b,c> <input>` | 并行运行，第一个成功回复获胜，其余被取消。 |
| `/debate <a,b,c> <topic>` | 围绕某个话题进行结构化的多智能体辩论。 |
| `/exec <command>` | 在沙箱化的开发智能体中执行一条 shell 命令。 |

### 运维

| 命令 | 作用 |
|------|------|
| `/agents` | 列出活动的智能体。 |
| `/monitor [agent]` | 流式显示给定智能体（或所有智能体）的实时状态。 |
| `/logs [agent]` | 显示最近的日志。 |
| `/audit [filter]` | 显示最近的审计轨迹条目；可按智能体、决策或时间范围过滤。 |
| `/doctor` | 诊断本地运行时环境。 |
| `/memory <agent> [query]` | 查询智能体的记忆。 |
| `/debug <agent>` | 检查智能体的内部状态。 |
| `/pause`、`/resume-agent`、`/stop`、`/destroy` | 智能体生命周期控制。 |

### 工具、技能与校验

| 命令 | 作用 |
|------|------|
| `/tools [list\|add\|remove]` | 管理可供智能体使用的 ToolClad 工具。 |
| `/skills [list\|install\|remove]` | 管理可供智能体使用的技能。 |
| `/verify <artifact>` | 针对 SchemaPin 签名校验一个已签名的工件（工具清单、技能）。 |

### 调度

| 命令 | 作用 |
|------|------|
| `/cron list` | 列出计划中的智能体任务。 |
| `/cron add` / `/cron remove` | 创建或删除计划任务。 |
| `/cron history` | 显示最近的运行记录。 |

`/cron` 在本地和通过远程附加（见下文）均可使用。完整的 cron 引擎参见[调度指南](/scheduling)。

### 通道

| 命令 | 作用 |
|------|------|
| `/channels` | 列出已注册的通道适配器（Slack、Teams、Mattermost）。 |
| `/connect <channel>` | 注册一个新的通道适配器。 |
| `/disconnect <channel>` | 移除一个适配器。 |

当目标为已部署的运行时时，通道管理需要远程附加。

### 密钥

| 命令 | 作用 |
|------|------|
| `/secrets list\|set\|get\|remove` | 管理运行时加密本地存储中的密钥。 |

密钥在静态存储时以 `SYMBIONT_MASTER_KEY` 加密，并按智能体命名空间隔离。

### 部署（Beta）

> **Status: Beta.** 在 OSS 版本中，部署栈为单智能体。多智能体和托管部署在路线图中。

| 命令 | 目标 |
|------|------|
| `/deploy local` | 使用加固的沙箱运行器部署到本地 Docker 守护进程。 |
| `/deploy cloudrun` | Google Cloud Run —— 构建镜像、推送并部署服务。 |
| `/deploy aws` | AWS App Runner。 |

`/deploy` 读取当前活动的智能体和项目配置，并生成可复现的部署工件。对于多智能体拓扑，请分别部署协调器和每个工作者，并通过跨实例消息传递将它们连接起来（参见[运行时架构](/runtime-architecture#cross-instance-agent-messaging)）。

### 远程附加

| 命令 | 作用 |
|------|------|
| `/attach <url>` | 通过 HTTP 或 HTTPS 将此 shell 附加到远程运行时。 |
| `/detach` | 与当前附加的运行时分离。 |

对于任何远程或生产环境目标，请使用 `https://` —— 附加通道承载认证令牌和运维流量，明文 HTTP 仅适合本地开发。`local` 快捷方式默认使用 `http://localhost:8080`，未显式指定协议的 URL 会被自动加上 `http://` 前缀，以保留回环开发的便利性；其他场景请传入完整的 `https://...` URL。

一旦附加，`/cron`、`/channels`、`/agents`、`/audit` 和大多数运维命令会作用于远程运行时而非本地运行时。`/secrets` 仍为本地 —— 远程密钥保留在远程运行时的存储中。

### 会话管理

| 命令 | 作用 |
|------|------|
| `/snapshot [name]` | 保存当前会话。 |
| `/resume <snapshot>` | 恢复已保存的快照。 |
| `/export <path>` | 将对话记录导出到磁盘。 |
| `/new` | 启动新会话，丢弃当前会话。 |
| `/compact [limit]` | 压缩对话历史以适配 token 预算。 |
| `/context` | 显示当前上下文窗口和 token 使用情况。 |

会话存储在 `.symbi/sessions/<uuid>/` 下。当上下文增长超过配置的预算时，shell 会自动触发压缩。

### 会话控制

| 命令 | 作用 |
|------|------|
| `/model [name]` | 显示或切换当前使用的推理模型。 |
| `/cost` | 显示会话的 token 数量和 API 成本总计。 |
| `/status` | 显示运行时和会话状态。 |
| `/dsl` | 在 DSL 模式和编排器输入模式之间切换 —— DSL 模式在进程内求值。 |
| `/clear` | 清除可见输出缓冲区（保留历史）。 |
| `/quit` / `/exit` | 退出 shell。 |
| `/help` | 显示命令目录。 |

## DSL 模式

按 `/dsl` 可将输入行切换到 DSL 模式。在 DSL 模式下，shell 针对进程内的 DSL 解释器解析并求值输入，并通过基于 tree-sitter 的补全和错误提示，而不经过编排器路由。再按一次 `/dsl` 切换回来。

## 约束与验证

编写命令会强制执行本地验证流水线：

1. 生成的工件会根据场景分别以 Symbiont DSL 语法、Cedar 或 ToolClad 进行解析。
2. 约束加载器针对项目级约束（例如禁止的能力、必需的策略）检查结果。
3. 只有在两个步骤都成功后，工件才会写入磁盘。

编排器 LLM 可以通过验证错误看到约束文件的效果，但无法修改该文件本身 —— 这与 `symbi tools validate` 流水线使用的信任模型相同。

## Beta 注意事项

shell 的以下部分仍处于活跃开发中，可能在没有弃用窗口的情况下发生变化：

- `/branch` 和 `/copy`（会话分支）是保留命令，目前会打印 "planned for a future release" 的占位说明。
- `/deploy cloudrun` 和 `/deploy aws` 仅支持单智能体。
- 快照格式和 `.symbi/sessions/` 布局可能在次要版本之间变化；如果需要持久的对话记录，请使用 `/export`。
- 模糊补全启发式和跟踪时间线布局会根据反馈进行调整，可能会发生变化。

如果您今天就需要一个稳定的接口，请优先使用 `symbi up`、`symbi run` 和 [HTTP API](/api-reference) —— 这些由 `SECURITY.md` 中的兼容性保证覆盖。

## 另请参阅

- [入门指南](/getting-started) —— 安装与 `symbi init`
- [DSL 指南](/dsl-guide) —— 智能体定义语言参考
- [ToolClad](/toolclad) —— 声明式工具契约
- [调度](/scheduling) —— cron 引擎与投递路由
- [安全模型](/security-model) —— 信任边界与策略执行
