# 调度指南

## 其他语言

[English](scheduling.md) | ## 概述

Symbiont 的调度系统为 AI 智能体提供生产级的 cron 定时任务执行能力。系统支持：

- **Cron 调度**：使用传统 cron 语法定义周期性任务
- **一次性任务**：在指定时间运行一次
- **心跳模式**：用于监控智能体的持续"评估-执行-休眠"循环
- **会话隔离**：临时性、共享式或完全隔离的智能体上下文
- **交付路由**：多种输出通道（Stdout、LogFile、Webhook、Slack、Email、Custom）
- **策略执行**：执行前进行安全与合规性检查
- **生产加固**：抖动、并发限制、死信队列以及 AgentPin 验证

## 架构

调度系统基于三个核心组件构建：

```
┌─────────────────────┐
│   CronScheduler     │  后台定时循环（1 秒间隔）
│   (Tick Loop)       │  任务选择与执行编排
└──────────┬──────────┘
           │
           ▼
┌─────────────────────┐
│   SqliteJobStore    │  持久化任务存储
│   (Job Storage)     │  事务支持、状态管理
└──────────┬──────────┘
           │
           ▼
┌─────────────────────┐
│DefaultAgentScheduler│  智能体执行运行时
│ (Execution Engine)  │  AgentContext 生命周期管理
└─────────────────────┘
```

### CronScheduler

`CronScheduler` 是主要入口点，负责管理：

- 以 1 秒间隔运行的后台定时循环
- 基于下次运行时间的任务选择
- 并发控制和抖动注入
- 指标收集和健康监控
- 优雅关闭以及运行中任务的跟踪

### SqliteJobStore

`SqliteJobStore` 提供持久化的任务存储，具备以下特性：

- 任务状态更新的 ACID 事务
- 任务生命周期跟踪（Active、Paused、Completed、Failed、DeadLetter）
- 带审计追踪的运行历史
- 按状态、智能体 ID 等条件进行过滤的查询能力

### DefaultAgentScheduler

`DefaultAgentScheduler` 负责执行已调度的智能体：

- 创建隔离或共享的 `AgentContext` 实例
- 管理会话生命周期（创建、执行、销毁）
- 将交付内容路由至配置的通道
- 在执行前执行策略门控

## DSL 语法

### 调度块结构

调度块在 Symbiont DSL 文件中定义：

```symbiont
schedule {
  name: "daily-report"
  agent: "reporter-agent"
  cron: "0 0 9 * * *"

  session_mode: "ephemeral_with_summary"
  delivery: ["stdout", "log_file"]

  policy {
    require_approval: false
    max_runtime: "5m"
  }
}
```

### Cron 语法

扩展 cron 语法包含六个字段（秒在最前，可选的第七个字段为年份）：

```
┌─────────────── 秒 (0-59)
│ ┌───────────── 分钟 (0-59)
│ │ ┌─────────── 小时 (0-23)
│ │ │ ┌───────── 月份中的日期 (1-31)
│ │ │ │ ┌─────── 月份 (1-12)
│ │ │ │ │ ┌───── 星期几 (0-6, 星期日 = 0)
│ │ │ │ │ │
* * * * * *
```

**示例：**

```symbiont
# 每天上午 9 点
cron: "0 0 9 * * *"

# 每周一下午 6 点
cron: "0 0 18 * * 1"

# 每 15 分钟
cron: "0 */15 * * * *"

# 每月第一天午夜
cron: "0 0 0 1 * *"
```

### 一次性任务（At 语法）

用于在指定时间只运行一次的任务：

```symbiont
schedule {
  name: "deployment-check"
  agent: "health-checker"
  at: "2026-02-15T14:30:00Z"  # ISO 8601 时间戳

  delivery: ["webhook"]
  webhook_url: "https://ops.example.com/hooks/deployment"
}
```

### 心跳模式

用于持续监控的智能体，遵循"评估 -> 执行 -> 休眠"循环：

```symbiont
schedule {
  name: "system-monitor"
  agent: "heartbeat-agent"
  cron: "0 */5 * * * *"  # 每 5 分钟唤醒一次

  heartbeat: {
    enabled: true
    context_mode: "ephemeral_with_summary"
    max_iterations: 100  # 安全限制
  }
}
```

心跳智能体遵循以下循环：

1. **评估**：评估系统状态（例如检查指标、日志）
2. **执行**：在需要时采取纠正措施（例如重启服务、通知运维团队）
3. **休眠**：等待下一个调度周期

## CLI 命令

`symbi cron` 命令提供完整的生命周期管理：

### 列出任务

```bash
# 列出所有任务
symbi cron list

# 按状态过滤
symbi cron list --status active
symbi cron list --status paused

# 按智能体过滤
symbi cron list --agent "reporter-agent"

# JSON 输出
symbi cron list --format json
```

### 添加任务

```bash
# 从 DSL 文件添加
symbi cron add --file agent.symbi --schedule "daily-report"

# 内联定义（JSON）
symbi cron add --json '{
  "name": "quick-task",
  "agent_id": "agent-123",
  "cron_expr": "0 0 * * * *"
}'
```

### 删除任务

```bash
# 按任务 ID 删除
symbi cron remove <job-id>

# 按名称删除
symbi cron remove --name "daily-report"

# 强制删除（跳过确认）
symbi cron remove <job-id> --force
```

### 暂停/恢复

```bash
# 暂停任务（停止调度，保留状态）
symbi cron pause <job-id>

# 恢复已暂停的任务
symbi cron resume <job-id>
```

### 状态

```bash
# 查看任务详情和下次运行时间
symbi cron status <job-id>

# 包含最近 10 条运行记录
symbi cron status <job-id> --history 10

# 监视模式（每 5 秒自动刷新）
symbi cron status <job-id> --watch
```

### 立即运行

```bash
# 触发立即执行（绕过调度计划）
symbi cron run <job-id>

# 使用自定义输入
symbi cron run <job-id> --input "Check production database"
```

### 历史记录

```bash
# 查看任务的运行历史
symbi cron history <job-id>

# 最近 20 次运行
symbi cron history <job-id> --limit 20

# 按状态过滤
symbi cron history <job-id> --status failed

# 导出为 CSV
symbi cron history <job-id> --format csv > runs.csv
```

## 心跳模式

### HeartbeatContextMode

控制上下文在心跳迭代之间如何持久化：

```rust
pub enum HeartbeatContextMode {
    /// Fresh context each iteration, append summary to run history
    EphemeralWithSummary,

    /// Shared context across all iterations (memory accumulates)
    SharedPersistent,

    /// Fresh context each iteration, no summary (stateless)
    FullyEphemeral,
}
```

**EphemeralWithSummary（默认）**：
- 每次迭代创建新的 `AgentContext`
- 前一次迭代的摘要会追加到上下文中
- 防止内存无限增长
- 为相关操作保持连续性

**SharedPersistent**：
- 所有迭代复用单个 `AgentContext`
- 保留完整的对话历史
- 内存使用较高
- 最适合需要深层上下文的智能体（例如调试会话）

**FullyEphemeral**：
- 每次迭代创建新的 `AgentContext`，无状态延续
- 最低内存占用
- 最适合独立检查（例如 API 健康探测）

### 心跳智能体示例

```symbiont
agent heartbeat_monitor {
  model: "claude-sonnet-4.5"
  system_prompt: """
  You are a system monitoring agent. On each heartbeat:
  1. Check system metrics (CPU, memory, disk)
  2. Review recent error logs
  3. If issues detected, take action:
     - Restart services if safe
     - Alert ops team via Slack
     - Log incident details
  4. Summarize findings
  5. Return 'sleep' when done
  """
}

schedule {
  name: "heartbeat-monitor"
  agent: "heartbeat_monitor"
  cron: "0 */10 * * * *"  # 每 10 分钟

  heartbeat: {
    enabled: true
    context_mode: "ephemeral_with_summary"
    max_iterations: 50
  }

  delivery: ["log_file", "slack"]
  slack_channel: "#ops-alerts"
}
```

## 会话隔离

### 会话模式

```rust
pub enum HeartbeatContextMode {
    /// Ephemeral context with summary carryover (default)
    EphemeralWithSummary,

    /// Shared persistent context across all runs
    SharedPersistent,

    /// Fully ephemeral, no state carryover
    FullyEphemeral,
}
```

**配置：**

```symbiont
schedule {
  name: "data-pipeline"
  agent: "etl-agent"
  cron: "0 0 2 * * *"

  # 每次运行使用全新上下文，包含前次运行的摘要
  session_mode: "ephemeral_with_summary"
}
```

### 会话生命周期

每次调度执行的流程如下：

1. **执行前**：检查并发限制，应用抖动
2. **会话创建**：根据 `session_mode` 创建 `AgentContext`
3. **策略门控**：评估策略条件
4. **执行**：使用输入和上下文运行智能体
5. **交付**：将输出路由至配置的通道
6. **会话清理**：根据模式销毁或持久化上下文
7. **执行后**：更新运行记录，收集指标

## 交付路由

### 支持的通道

```rust
pub enum DeliveryChannel {
    Stdout,           // Print to console
    LogFile,          // Append to job-specific log file
    Webhook,          // HTTP POST to URL
    Slack,            // Slack webhook or API
    Email,            // SMTP email
    Custom(String),   // User-defined channel
}
```

### 配置示例

**单通道：**

```symbiont
schedule {
  name: "backup"
  agent: "backup-agent"
  cron: "0 0 3 * * *"
  delivery: ["log_file"]
}
```

**多通道：**

```symbiont
schedule {
  name: "security-scan"
  agent: "scanner"
  cron: "0 0 1 * * *"

  delivery: ["log_file", "slack", "email"]

  slack_channel: "#security"
  email_recipients: ["ops@example.com", "security@example.com"]
}
```

**Webhook 交付：**

```symbiont
schedule {
  name: "metrics-report"
  agent: "metrics-agent"
  cron: "0 */30 * * * *"

  delivery: ["webhook"]
  webhook_url: "https://metrics.example.com/ingest"
  webhook_headers: {
    "Authorization": "Bearer ${METRICS_API_KEY}"
    "Content-Type": "application/json"
  }
}
```

### DeliveryRouter Trait

自定义交付通道需要实现：

```rust
#[async_trait]
pub trait DeliveryRouter: Send + Sync {
    async fn route(
        &self,
        channel: &DeliveryChannel,
        job: &CronJobDefinition,
        run: &JobRunRecord,
        output: &str,
    ) -> Result<(), SchedulerError>;
}
```

## 策略执行

### PolicyGate

`PolicyGate` 在执行前评估调度专属策略：

```rust
pub struct PolicyGate {
    policy_engine: Arc<RealPolicyParser>,
}

impl PolicyGate {
    pub fn evaluate(
        &self,
        job: &CronJobDefinition,
        context: &AgentContext,
    ) -> Result<SchedulePolicyDecision, SchedulerError>;
}
```

### 策略条件

```symbiont
schedule {
  name: "production-deploy"
  agent: "deploy-agent"
  cron: "0 0 0 * * 0"  # 周日午夜

  policy {
    # 执行前需要人工审批
    require_approval: true

    # 强制终止前的最大运行时间
    max_runtime: "30m"

    # 要求具备特定能力
    require_capabilities: ["deployment", "production_write"]

    # 时间窗口限制（UTC）
    allowed_hours: {
      start: "00:00"
      end: "04:00"
    }

    # 环境限制
    allowed_environments: ["staging", "production"]

    # 需要 AgentPin 验证
    require_agent_pin: true
  }
}
```

### SchedulePolicyDecision

```rust
pub enum SchedulePolicyDecision {
    Allow,
    Deny { reason: String },
    RequiresApproval { approver: String, reason: String, policy_id: String },
}
```

## 生产加固

### 抖动

防止多个共享相同调度计划的任务同时启动导致的惊群效应：

```rust
pub struct CronSchedulerConfig {
    pub max_jitter_seconds: u64,  // Random delay 0-N seconds
    // ...
}
```

**示例：**

```toml
[scheduler]
max_jitter_seconds = 30  # 将任务启动分散在 30 秒的窗口内
```

### 单任务并发控制

限制单个任务的并发运行数，防止资源耗尽：

```symbiont
schedule {
  name: "data-sync"
  agent: "sync-agent"
  cron: "0 */5 * * * *"

  max_concurrent: 2  # 最多允许 2 个并发运行
}
```

如果任务已达到最大并发数，调度器将跳过本次调度周期。

### 死信队列

超过 `max_retries` 次重试的任务将转为 `DeadLetter` 状态，等待人工审查：

```symbiont
schedule {
  name: "flaky-job"
  agent: "unreliable-agent"
  cron: "0 0 * * * *"

  max_retries: 3  # 3 次失败后移入死信队列
}
```

**恢复操作：**

```bash
# 列出死信队列中的任务
symbi cron list --status dead_letter

# 查看失败原因
symbi cron history <job-id> --status failed

# 修复后将任务重置为活跃状态
symbi cron reset <job-id>
```

### AgentPin 验证

在执行前对智能体身份进行加密验证：

```symbiont
schedule {
  name: "secure-task"
  agent: "trusted-agent"
  cron: "0 0 * * * *"

  agent_pin_jwt: "${AGENT_PIN_JWT}"  # 来自 agentpin-cli 的 ES256 JWT

  policy {
    require_agent_pin: true
  }
}
```

调度器验证以下内容：
1. 使用 ES256（ECDSA P-256）验证 JWT 签名
2. 智能体 ID 与 `iss` 声明匹配
3. 域锚定与预期来源匹配
4. 过期时间（`exp`）有效

验证失败将触发 `SecurityEventType::AgentPinVerificationFailed` 审计事件。

## HTTP API 端点

### 调度管理

**POST /api/v1/schedule**
创建新的调度任务。

```bash
curl -X POST http://localhost:8080/api/v1/schedule \
  -H "Content-Type: application/json" \
  -d '{
    "name": "hourly-report",
    "agent_id": "reporter",
    "cron_expr": "0 0 * * * *",
    "session_mode": "ephemeral_with_summary",
    "delivery": ["stdout"]
  }'
```

**GET /api/v1/schedule**
列出所有任务（可按状态、智能体 ID 过滤）。

```bash
curl "http://localhost:8080/api/v1/schedule?status=active&agent_id=reporter"
```

**GET /api/v1/schedule/{job_id}**
获取任务详情。

```bash
curl http://localhost:8080/api/v1/schedule/job-123
```

**PUT /api/v1/schedule/{job_id}**
更新任务（cron 表达式、交付通道等）。

```bash
curl -X PUT http://localhost:8080/api/v1/schedule/job-123 \
  -H "Content-Type: application/json" \
  -d '{"cron_expr": "0 0 */2 * * *"}'
```

**DELETE /api/v1/schedule/{job_id}**
删除任务。

```bash
curl -X DELETE http://localhost:8080/api/v1/schedule/job-123
```

**POST /api/v1/schedule/{job_id}/pause**
暂停任务。

```bash
curl -X POST http://localhost:8080/api/v1/schedule/job-123/pause
```

**POST /api/v1/schedule/{job_id}/resume**
恢复已暂停的任务。

```bash
curl -X POST http://localhost:8080/api/v1/schedule/job-123/resume
```

**POST /api/v1/schedule/{job_id}/run**
触发立即执行。

```bash
curl -X POST http://localhost:8080/api/v1/schedule/job-123/run \
  -H "Content-Type: application/json" \
  -d '{"input": "Run with custom input"}'
```

**GET /api/v1/schedule/{job_id}/history**
获取运行历史。

```bash
curl "http://localhost:8080/api/v1/schedule/job-123/history?limit=20&status=failed"
```

**GET /api/v1/schedule/{job_id}/next_run**
获取下次调度运行时间。

```bash
curl http://localhost:8080/api/v1/schedule/job-123/next_run
```

### 健康监控

**GET /api/v1/health/scheduler**
调度器健康状态和指标。

```bash
curl http://localhost:8080/api/v1/health/scheduler
```

**响应：**

```json
{
  "status": "healthy",
  "active_jobs": 15,
  "paused_jobs": 3,
  "in_flight_jobs": 2,
  "metrics": {
    "runs_total": 1234,
    "runs_succeeded": 1180,
    "runs_failed": 54,
    "avg_execution_time_ms": 850
  }
}
```

## SDK 示例

### JavaScript SDK

```javascript
import { SymbiontClient } from '@symbiont/sdk-js';

const client = new SymbiontClient({
  baseUrl: 'http://localhost:8080',
  apiKey: process.env.SYMBI_API_KEY
});

// 创建调度任务
const job = await client.schedule.create({
  name: 'daily-backup',
  agentId: 'backup-agent',
  cronExpr: '0 0 2 * * *',
  sessionMode: 'ephemeral_with_summary',
  delivery: ['webhook'],
  webhookUrl: 'https://backup.example.com/notify'
});

console.log(`Created job: ${job.id}`);

// 列出活跃任务
const activeJobs = await client.schedule.list({ status: 'active' });
console.log(`Active jobs: ${activeJobs.length}`);

// 获取任务状态
const status = await client.schedule.getStatus(job.id);
console.log(`Next run: ${status.next_run}`);

// 触发立即运行
await client.schedule.runNow(job.id, { input: 'Backup database' });

// 暂停任务
await client.schedule.pause(job.id);

// 查看历史记录
const history = await client.schedule.getHistory(job.id, { limit: 10 });
history.forEach(run => {
  console.log(`Run ${run.id}: ${run.status} (${run.execution_time_ms}ms)`);
});

// 恢复任务
await client.schedule.resume(job.id);

// 删除任务
await client.schedule.delete(job.id);
```

### Python SDK

```python
from symbiont import SymbiontClient

client = SymbiontClient(
    base_url='http://localhost:8080',
    api_key=os.environ['SYMBI_API_KEY']
)

# 创建调度任务
job = client.schedule.create(
    name='hourly-metrics',
    agent_id='metrics-agent',
    cron_expr='0 0 * * * *',
    session_mode='ephemeral_with_summary',
    delivery=['slack', 'log_file'],
    slack_channel='#metrics'
)

print(f"Created job: {job.id}")

# 列出特定智能体的任务
jobs = client.schedule.list(agent_id='metrics-agent')
print(f"Found {len(jobs)} jobs for metrics-agent")

# 获取任务详情
details = client.schedule.get(job.id)
print(f"Cron: {details.cron_expr}")
print(f"Next run: {details.next_run}")

# 更新 cron 表达式
client.schedule.update(job.id, cron_expr='0 */30 * * * *')

# 触发立即运行
run = client.schedule.run_now(job.id, input='Generate metrics report')
print(f"Run ID: {run.id}")

# 维护期间暂停
client.schedule.pause(job.id)
print("Job paused for maintenance")

# 查看最近的失败记录
history = client.schedule.get_history(
    job.id,
    status='failed',
    limit=5
)
for run in history:
    print(f"Failed run {run.id}: {run.error_message}")

# 维护结束后恢复
client.schedule.resume(job.id)

# 检查调度器健康状态
health = client.schedule.health()
print(f"Scheduler status: {health.status}")
print(f"Active jobs: {health.active_jobs}")
print(f"In-flight jobs: {health.in_flight_jobs}")
```

## 配置

### CronSchedulerConfig

```rust
pub struct CronSchedulerConfig {
    /// Tick interval (default: 1 second)
    pub tick_interval: Duration,

    /// Global concurrency limit (default: 100)
    pub max_concurrent_cron_jobs: usize,

    /// Persistent job store path (default: None)
    pub job_store_path: Option<PathBuf>,

    /// Catch up missed runs on startup (default: true)
    pub enable_missed_run_catchup: bool,
}
```

### TOML 配置

```toml
[scheduler]
tick_interval_seconds = 1
max_jitter_seconds = 30
max_concurrent_jobs = 20
enable_metrics = true
default_max_retries = 3
shutdown_timeout_seconds = 60

[scheduler.delivery]
# Webhook 设置
webhook_timeout_seconds = 30
webhook_retry_attempts = 3

# Slack 设置
slack_api_token = "${SLACK_API_TOKEN}"
slack_default_channel = "#ops"

# 邮件设置
smtp_host = "smtp.example.com"
smtp_port = 587
smtp_username = "${SMTP_USER}"
smtp_password = "${SMTP_PASS}"
email_from = "symbiont@example.com"
```

### 环境变量

```bash
# 调度器设置
SYMBI_SCHEDULER_MAX_JITTER=30
SYMBI_SCHEDULER_MAX_CONCURRENT=20

# 交付设置
SYMBI_SLACK_TOKEN=xoxb-...
SYMBI_WEBHOOK_AUTH_HEADER="Bearer secret-token"

# AgentPin 验证
SYMBI_AGENTPIN_REQUIRED=true
SYMBI_AGENTPIN_DOMAIN=agent.example.com
```

## 可观测性

### 指标（兼容 Prometheus）

```
# 总运行次数
symbiont_cron_runs_total{job_name="daily-report",status="succeeded"} 450

# 失败运行次数
symbiont_cron_runs_total{job_name="daily-report",status="failed"} 5

# 执行时间直方图
symbiont_cron_execution_duration_seconds{job_name="daily-report"} 1.234

# 运行中任务计数
symbiont_cron_in_flight_jobs 3

# 死信队列任务
symbiont_cron_dead_letter_total{job_name="flaky-job"} 2
```

### 审计事件

所有调度器操作均会发出安全事件：

```rust
pub enum SecurityEventType {
    CronJobCreated,
    CronJobUpdated,
    CronJobDeleted,
    CronJobPaused,
    CronJobResumed,
    CronJobExecuted,
    CronJobFailed,
    CronJobDeadLettered,
    AgentPinVerificationFailed,
}
```

查询审计日志：

```bash
symbi audit query --type CronJobFailed --since "2026-02-01" --limit 50
```

## 最佳实践

1. **为共享调度计划使用抖动**：防止多个任务同时启动
2. **设置并发限制**：防止资源耗尽
3. **监控死信队列**：定期审查和修复失败的任务
4. **使用 EphemeralWithSummary**：防止长时间运行的心跳任务中内存无限增长
5. **启用 AgentPin 验证**：对智能体身份进行加密验证
6. **配置交付路由**：为不同类型的任务使用合适的通道
7. **设置策略门控**：执行时间窗口、审批和能力检查
8. **使用心跳模式进行监控**：持续的"评估-执行-休眠"循环
9. **在预发布环境中测试调度**：在上线生产前验证 cron 表达式和任务逻辑
10. **导出指标**：集成 Prometheus/Grafana 以获得运维可见性

## 故障排除

### 任务未运行

1. 检查任务状态：`symbi cron status <job-id>`
2. 验证 cron 表达式：使用 [crontab.guru](https://crontab.guru/)
3. 检查调度器健康状态：`curl http://localhost:8080/api/v1/health/scheduler`
4. 查看日志：`symbi logs --filter scheduler --level debug`

### 任务反复失败

1. 查看历史记录：`symbi cron history <job-id> --status failed`
2. 检查运行记录中的错误信息
3. 验证智能体配置和能力
4. 在调度器外部测试智能体：`symbi run <agent-id> --input "test"`
5. 检查策略门控：确保时间窗口和能力匹配

### 死信队列中的任务

1. 列出死信任务：`symbi cron list --status dead_letter`
2. 审查失败模式：`symbi cron history <job-id>`
3. 修复根本原因（智能体代码、权限、外部依赖）
4. 重置任务：`symbi cron reset <job-id>`

### 高内存使用

1. 检查会话模式：切换到 `ephemeral_with_summary` 或 `fully_ephemeral`
2. 减少心跳迭代次数：降低 `max_iterations`
3. 监控上下文大小：审查智能体输出的详细程度
4. 启用上下文归档：配置保留策略

## 从 v0.9.0 迁移

v1.0.0 版本新增了生产加固功能。请更新您的任务定义：

```diff
 schedule {
   name: "my-job"
   agent: "my-agent"
   cron: "0 0 * * * *"
+
+  # 添加并发限制
+  max_concurrent: 2
+
+  # 添加 AgentPin 进行身份验证
+  agent_pin_jwt: "${AGENT_PIN_JWT}"
+
+  policy {
+    require_agent_pin: true
+  }
 }
```

更新配置：

```diff
 [scheduler]
 tick_interval_seconds = 1
+ max_jitter_seconds = 30
+ default_max_retries = 3
+ shutdown_timeout_seconds = 60
```

没有破坏性的 API 变更。所有 v0.9.0 的任务将继续正常工作。
