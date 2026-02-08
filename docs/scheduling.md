---
layout: default
title: Scheduling Guide
nav_order: 7
description: "Production-grade cron-based task scheduling for Symbiont AI agents"
---

# Scheduling Guide

## 🌐 Other Languages
{: .no_toc}

**English** | [中文简体](scheduling.zh-cn.md) | [Español](scheduling.es.md) | [Português](scheduling.pt.md) | [日本語](scheduling.ja.md) | [Deutsch](scheduling.de.md)

---

## Overview

Symbiont's scheduling system provides production-grade cron-based task execution for AI agents. The system supports:

- **Cron schedules**: Traditional cron syntax for recurring tasks
- **One-shot jobs**: Run once at a specific time
- **Heartbeat pattern**: Continuous assessment-action-sleep cycles for monitoring agents
- **Session isolation**: Ephemeral, shared, or fully isolated agent contexts
- **Delivery routing**: Multiple output channels (Stdout, LogFile, Webhook, Slack, Email, Custom)
- **Policy enforcement**: Security and compliance checks before execution
- **Production hardening**: Jitter, concurrency limits, dead-letter queues, and AgentPin verification

## Architecture

The scheduling system is built on three core components:

```
┌─────────────────────┐
│   CronScheduler     │  Background tick loop (1-second intervals)
│   (Tick Loop)       │  Job selection and execution orchestration
└──────────┬──────────┘
           │
           ▼
┌─────────────────────┐
│   SqliteJobStore    │  Persistent job storage
│   (Job Storage)     │  Transaction support, state management
└──────────┬──────────┘
           │
           ▼
┌─────────────────────┐
│DefaultAgentScheduler│  Agent execution runtime
│ (Execution Engine)  │  AgentContext lifecycle management
└─────────────────────┘
```

### CronScheduler

The `CronScheduler` is the primary entry point. It manages:

- Background tick loop running at 1-second intervals
- Job selection based on next run time
- Concurrency control and jitter injection
- Metrics collection and health monitoring
- Graceful shutdown with in-flight job tracking

### SqliteJobStore

The `SqliteJobStore` provides durable job persistence with:

- ACID transactions for job state updates
- Job lifecycle tracking (Active, Paused, Completed, Failed, DeadLetter)
- Run history with audit trail
- Query capabilities for filtering by status, agent ID, etc.

### DefaultAgentScheduler

The `DefaultAgentScheduler` executes scheduled agents:

- Creates isolated or shared `AgentContext` instances
- Manages session lifecycle (create, execute, destroy)
- Routes delivery to configured channels
- Enforces policy gates before execution

## DSL Syntax

### Schedule Block Structure

Schedule blocks are defined in Symbiont DSL files:

```symbiont
schedule {
  name: "daily-report"
  agent: "reporter-agent"
  cron: "0 9 * * *"

  session_mode: "ephemeral_with_summary"
  delivery: ["stdout", "log_file"]

  policy {
    require_approval: false
    max_runtime: "5m"
  }
}
```

### Cron Syntax

Standard cron syntax with five fields:

```
┌───────────── minute (0-59)
│ ┌─────────── hour (0-23)
│ │ ┌───────── day of month (1-31)
│ │ │ ┌─────── month (1-12)
│ │ │ │ ┌───── day of week (0-6, Sunday = 0)
│ │ │ │ │
* * * * *
```

**Examples:**

```symbiont
# Every day at 9 AM
cron: "0 9 * * *"

# Every Monday at 6 PM
cron: "0 18 * * 1"

# Every 15 minutes
cron: "*/15 * * * *"

# First day of every month at midnight
cron: "0 0 1 * *"
```

### One-Shot Jobs (At Syntax)

For jobs that run once at a specific time:

```symbiont
schedule {
  name: "deployment-check"
  agent: "health-checker"
  at: "2026-02-15T14:30:00Z"  # ISO 8601 timestamp

  delivery: ["webhook"]
  webhook_url: "https://ops.example.com/hooks/deployment"
}
```

### Heartbeat Pattern

For continuous monitoring agents that assess → act → sleep:

```symbiont
schedule {
  name: "system-monitor"
  agent: "heartbeat-agent"
  cron: "*/5 * * * *"  # Wake every 5 minutes

  heartbeat: {
    enabled: true
    context_mode: "ephemeral_with_summary"
    max_iterations: 100  # Safety limit
  }
}
```

The heartbeat agent follows this cycle:

1. **Assessment**: Evaluate system state (e.g., check metrics, logs)
2. **Action**: Take corrective action if needed (e.g., restart service, alert ops)
3. **Sleep**: Wait until next scheduled tick

## CLI Commands

The `symbi cron` command provides full lifecycle management:

### List Jobs

```bash
# List all jobs
symbi cron list

# Filter by status
symbi cron list --status active
symbi cron list --status paused

# Filter by agent
symbi cron list --agent "reporter-agent"

# JSON output
symbi cron list --format json
```

### Add Job

```bash
# From DSL file
symbi cron add --file agent.symbi --schedule "daily-report"

# Inline definition (JSON)
symbi cron add --json '{
  "name": "quick-task",
  "agent_id": "agent-123",
  "cron_expr": "0 * * * *"
}'
```

### Remove Job

```bash
# By job ID
symbi cron remove <job-id>

# By name
symbi cron remove --name "daily-report"

# Force remove (skip confirmation)
symbi cron remove <job-id> --force
```

### Pause/Resume

```bash
# Pause job (stops scheduling, preserves state)
symbi cron pause <job-id>

# Resume paused job
symbi cron resume <job-id>
```

### Status

```bash
# Job details with next run time
symbi cron status <job-id>

# Include last 10 run records
symbi cron status <job-id> --history 10

# Watch mode (auto-refresh every 5s)
symbi cron status <job-id> --watch
```

### Run Now

```bash
# Trigger immediate execution (bypasses schedule)
symbi cron run <job-id>

# With custom input
symbi cron run <job-id> --input "Check production database"
```

### History

```bash
# View run history for a job
symbi cron history <job-id>

# Last 20 runs
symbi cron history <job-id> --limit 20

# Filter by status
symbi cron history <job-id> --status failed

# Export to CSV
symbi cron history <job-id> --format csv > runs.csv
```

## Heartbeat Pattern

### HeartbeatContextMode

Controls how context persists across heartbeat iterations:

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

**EphemeralWithSummary (default)**:
- New `AgentContext` per iteration
- Summary of previous iteration appended to context
- Prevents unbounded memory growth
- Maintains continuity for related actions

**SharedPersistent**:
- Single `AgentContext` reused across all iterations
- Full conversation history preserved
- Higher memory usage
- Best for agents needing deep context (e.g., debugging sessions)

**FullyEphemeral**:
- New `AgentContext` per iteration, no carryover
- Lowest memory footprint
- Best for independent checks (e.g., API health probes)

### Heartbeat Agent Example

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
  cron: "*/10 * * * *"  # Every 10 minutes

  heartbeat: {
    enabled: true
    context_mode: "ephemeral_with_summary"
    max_iterations: 50
  }

  delivery: ["log_file", "slack"]
  slack_channel: "#ops-alerts"
}
```

## Session Isolation

### Session Modes

```rust
pub enum SessionIsolationMode {
    /// Ephemeral context with summary carryover (default)
    EphemeralWithSummary,

    /// Shared persistent context across all runs
    SharedPersistent,

    /// Fully ephemeral, no state carryover
    FullyEphemeral,
}
```

**Configuration:**

```symbiont
schedule {
  name: "data-pipeline"
  agent: "etl-agent"
  cron: "0 2 * * *"

  # Fresh context per run, summary of previous run included
  session_mode: "ephemeral_with_summary"
}
```

### Session Lifecycle

For each scheduled execution:

1. **Pre-execution**: Check concurrency limits, apply jitter
2. **Session creation**: Create `AgentContext` based on `session_mode`
3. **Policy gate**: Evaluate policy conditions
4. **Execution**: Run agent with input and context
5. **Delivery**: Route output to configured channels
6. **Session cleanup**: Destroy or persist context based on mode
7. **Post-execution**: Update run record, collect metrics

## Delivery Routing

### Supported Channels

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

### Configuration Examples

**Single channel:**

```symbiont
schedule {
  name: "backup"
  agent: "backup-agent"
  cron: "0 3 * * *"
  delivery: ["log_file"]
}
```

**Multiple channels:**

```symbiont
schedule {
  name: "security-scan"
  agent: "scanner"
  cron: "0 1 * * *"

  delivery: ["log_file", "slack", "email"]

  slack_channel: "#security"
  email_recipients: ["ops@example.com", "security@example.com"]
}
```

**Webhook delivery:**

```symbiont
schedule {
  name: "metrics-report"
  agent: "metrics-agent"
  cron: "*/30 * * * *"

  delivery: ["webhook"]
  webhook_url: "https://metrics.example.com/ingest"
  webhook_headers: {
    "Authorization": "Bearer ${METRICS_API_KEY}"
    "Content-Type": "application/json"
  }
}
```

### DeliveryRouter Trait

Custom delivery channels implement:

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

## Policy Enforcement

### PolicyGate

The `PolicyGate` evaluates schedule-specific policies before execution:

```rust
pub struct PolicyGate {
    policy_engine: Arc<RealPolicyParser>,
}

impl PolicyGate {
    pub async fn evaluate(
        &self,
        job: &CronJobDefinition,
        context: &AgentContext,
    ) -> Result<SchedulePolicyDecision, SchedulerError>;
}
```

### Policy Conditions

```symbiont
schedule {
  name: "production-deploy"
  agent: "deploy-agent"
  cron: "0 0 * * 0"  # Sunday midnight

  policy {
    # Require human approval before execution
    require_approval: true

    # Maximum runtime before forced termination
    max_runtime: "30m"

    # Require specific capabilities
    require_capabilities: ["deployment", "production_write"]

    # Time window enforcement (UTC)
    allowed_hours: {
      start: "00:00"
      end: "04:00"
    }

    # Environment restrictions
    allowed_environments: ["staging", "production"]

    # AgentPin verification required
    require_agent_pin: true
  }
}
```

### SchedulePolicyDecision

```rust
pub enum SchedulePolicyDecision {
    Allow,
    Deny { reason: String },
    RequireApproval { approvers: Vec<String> },
}
```

## Production Hardening

### Jitter

Prevents thundering herd when multiple jobs share a schedule:

```rust
pub struct CronSchedulerConfig {
    pub max_jitter_seconds: u64,  // Random delay 0-N seconds
    // ...
}
```

**Example:**

```toml
[scheduler]
max_jitter_seconds = 30  # Spread job starts across 30-second window
```

### Per-Job Concurrency

Limit concurrent runs per job to prevent resource exhaustion:

```symbiont
schedule {
  name: "data-sync"
  agent: "sync-agent"
  cron: "*/5 * * * *"

  max_concurrent: 2  # Allow max 2 concurrent runs
}
```

If a job is already running at max concurrency, the scheduler skips the tick.

### Dead-Letter Queue

Jobs exceeding `max_retries` move to `DeadLetter` status for manual review:

```symbiont
schedule {
  name: "flaky-job"
  agent: "unreliable-agent"
  cron: "0 * * * *"

  max_retries: 3  # After 3 failures, move to dead-letter
}
```

**Recovery:**

```bash
# List dead-lettered jobs
symbi cron list --status dead_letter

# Review failure reasons
symbi cron history <job-id> --status failed

# Reset job to active after fixing
symbi cron reset <job-id>
```

### AgentPin Verification

Cryptographically verify agent identity before execution:

```symbiont
schedule {
  name: "secure-task"
  agent: "trusted-agent"
  cron: "0 * * * *"

  agent_pin_jwt: "${AGENT_PIN_JWT}"  # ES256 JWT from agentpin-cli

  policy {
    require_agent_pin: true
  }
}
```

The scheduler verifies:
1. JWT signature using ES256 (ECDSA P-256)
2. Agent ID matches `iss` claim
3. Domain anchor matches expected origin
4. Expiry (`exp`) is valid

Failures trigger `SecurityEventType::AgentPinVerificationFailed` audit event.

## HTTP API Endpoints

### Schedule Management

**POST /api/v1/schedule**
Create a new scheduled job.

```bash
curl -X POST http://localhost:8080/api/v1/schedule \
  -H "Content-Type: application/json" \
  -d '{
    "name": "hourly-report",
    "agent_id": "reporter",
    "cron_expr": "0 * * * *",
    "session_mode": "ephemeral_with_summary",
    "delivery": ["stdout"]
  }'
```

**GET /api/v1/schedule**
List all jobs (filterable by status, agent ID).

```bash
curl "http://localhost:8080/api/v1/schedule?status=active&agent_id=reporter"
```

**GET /api/v1/schedule/{job_id}**
Get job details.

```bash
curl http://localhost:8080/api/v1/schedule/job-123
```

**PUT /api/v1/schedule/{job_id}**
Update job (cron expression, delivery, etc.).

```bash
curl -X PUT http://localhost:8080/api/v1/schedule/job-123 \
  -H "Content-Type: application/json" \
  -d '{"cron_expr": "0 */2 * * *"}'
```

**DELETE /api/v1/schedule/{job_id}**
Delete job.

```bash
curl -X DELETE http://localhost:8080/api/v1/schedule/job-123
```

**POST /api/v1/schedule/{job_id}/pause**
Pause job.

```bash
curl -X POST http://localhost:8080/api/v1/schedule/job-123/pause
```

**POST /api/v1/schedule/{job_id}/resume**
Resume paused job.

```bash
curl -X POST http://localhost:8080/api/v1/schedule/job-123/resume
```

**POST /api/v1/schedule/{job_id}/run**
Trigger immediate execution.

```bash
curl -X POST http://localhost:8080/api/v1/schedule/job-123/run \
  -H "Content-Type: application/json" \
  -d '{"input": "Run with custom input"}'
```

**GET /api/v1/schedule/{job_id}/history**
Get run history.

```bash
curl "http://localhost:8080/api/v1/schedule/job-123/history?limit=20&status=failed"
```

**GET /api/v1/schedule/{job_id}/next_run**
Get next scheduled run time.

```bash
curl http://localhost:8080/api/v1/schedule/job-123/next_run
```

### Health Monitoring

**GET /api/v1/health/scheduler**
Scheduler health and metrics.

```bash
curl http://localhost:8080/api/v1/health/scheduler
```

**Response:**

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

## SDK Examples

### JavaScript SDK

```javascript
import { SymbiontClient } from '@symbiont/sdk-js';

const client = new SymbiontClient({
  baseUrl: 'http://localhost:8080',
  apiKey: process.env.SYMBI_API_KEY
});

// Create scheduled job
const job = await client.schedule.create({
  name: 'daily-backup',
  agentId: 'backup-agent',
  cronExpr: '0 2 * * *',
  sessionMode: 'ephemeral_with_summary',
  delivery: ['webhook'],
  webhookUrl: 'https://backup.example.com/notify'
});

console.log(`Created job: ${job.id}`);

// List active jobs
const activeJobs = await client.schedule.list({ status: 'active' });
console.log(`Active jobs: ${activeJobs.length}`);

// Get job status
const status = await client.schedule.getStatus(job.id);
console.log(`Next run: ${status.next_run}`);

// Trigger immediate run
await client.schedule.runNow(job.id, { input: 'Backup database' });

// Pause job
await client.schedule.pause(job.id);

// View history
const history = await client.schedule.getHistory(job.id, { limit: 10 });
history.forEach(run => {
  console.log(`Run ${run.id}: ${run.status} (${run.execution_time_ms}ms)`);
});

// Resume job
await client.schedule.resume(job.id);

// Delete job
await client.schedule.delete(job.id);
```

### Python SDK

```python
from symbiont import SymbiontClient

client = SymbiontClient(
    base_url='http://localhost:8080',
    api_key=os.environ['SYMBI_API_KEY']
)

# Create scheduled job
job = client.schedule.create(
    name='hourly-metrics',
    agent_id='metrics-agent',
    cron_expr='0 * * * *',
    session_mode='ephemeral_with_summary',
    delivery=['slack', 'log_file'],
    slack_channel='#metrics'
)

print(f"Created job: {job.id}")

# List jobs for specific agent
jobs = client.schedule.list(agent_id='metrics-agent')
print(f"Found {len(jobs)} jobs for metrics-agent")

# Get job details
details = client.schedule.get(job.id)
print(f"Cron: {details.cron_expr}")
print(f"Next run: {details.next_run}")

# Update cron expression
client.schedule.update(job.id, cron_expr='*/30 * * * *')

# Trigger immediate run
run = client.schedule.run_now(job.id, input='Generate metrics report')
print(f"Run ID: {run.id}")

# Pause during maintenance
client.schedule.pause(job.id)
print("Job paused for maintenance")

# View recent failures
history = client.schedule.get_history(
    job.id,
    status='failed',
    limit=5
)
for run in history:
    print(f"Failed run {run.id}: {run.error_message}")

# Resume after maintenance
client.schedule.resume(job.id)

# Check scheduler health
health = client.schedule.health()
print(f"Scheduler status: {health.status}")
print(f"Active jobs: {health.active_jobs}")
print(f"In-flight jobs: {health.in_flight_jobs}")
```

## Configuration

### CronSchedulerConfig

```rust
pub struct CronSchedulerConfig {
    /// Tick interval in seconds (default: 1)
    pub tick_interval_seconds: u64,

    /// Maximum jitter to prevent thundering herd (default: 0)
    pub max_jitter_seconds: u64,

    /// Global concurrency limit (default: 10)
    pub max_concurrent_jobs: usize,

    /// Enable metrics collection (default: true)
    pub enable_metrics: bool,

    /// Dead-letter retry threshold (default: 3)
    pub default_max_retries: u32,

    /// Graceful shutdown timeout (default: 30s)
    pub shutdown_timeout_seconds: u64,
}
```

### TOML Configuration

```toml
[scheduler]
tick_interval_seconds = 1
max_jitter_seconds = 30
max_concurrent_jobs = 20
enable_metrics = true
default_max_retries = 3
shutdown_timeout_seconds = 60

[scheduler.delivery]
# Webhook settings
webhook_timeout_seconds = 30
webhook_retry_attempts = 3

# Slack settings
slack_api_token = "${SLACK_API_TOKEN}"
slack_default_channel = "#ops"

# Email settings
smtp_host = "smtp.example.com"
smtp_port = 587
smtp_username = "${SMTP_USER}"
smtp_password = "${SMTP_PASS}"
email_from = "symbiont@example.com"
```

### Environment Variables

```bash
# Scheduler settings
SYMBI_SCHEDULER_MAX_JITTER=30
SYMBI_SCHEDULER_MAX_CONCURRENT=20

# Delivery settings
SYMBI_SLACK_TOKEN=xoxb-...
SYMBI_WEBHOOK_AUTH_HEADER="Bearer secret-token"

# AgentPin verification
SYMBI_AGENTPIN_REQUIRED=true
SYMBI_AGENTPIN_DOMAIN=agent.example.com
```

## Observability

### Metrics (Prometheus-compatible)

```
# Total runs
symbiont_cron_runs_total{job_name="daily-report",status="succeeded"} 450

# Failed runs
symbiont_cron_runs_total{job_name="daily-report",status="failed"} 5

# Execution time histogram
symbiont_cron_execution_duration_seconds{job_name="daily-report"} 1.234

# In-flight jobs gauge
symbiont_cron_in_flight_jobs 3

# Dead-lettered jobs
symbiont_cron_dead_letter_total{job_name="flaky-job"} 2
```

### Audit Events

All scheduler actions emit security events:

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

Query audit log:

```bash
symbi audit query --type CronJobFailed --since "2026-02-01" --limit 50
```

## Best Practices

1. **Use jitter for shared schedules**: Prevent multiple jobs from starting simultaneously
2. **Set concurrency limits**: Protect against resource exhaustion
3. **Monitor dead-letter queue**: Review and fix failing jobs regularly
4. **Use EphemeralWithSummary**: Prevents unbounded memory growth in long-running heartbeats
5. **Enable AgentPin verification**: Cryptographically verify agent identity
6. **Configure delivery routing**: Use appropriate channels for different job types
7. **Set policy gates**: Enforce time windows, approvals, and capability checks
8. **Use heartbeat pattern for monitoring**: Continuous assessment-action-sleep cycles
9. **Test schedules in staging**: Validate cron expressions and job logic before production
10. **Export metrics**: Integrate with Prometheus/Grafana for operational visibility

## Troubleshooting

### Job Not Running

1. Check job status: `symbi cron status <job-id>`
2. Verify cron expression: Use [crontab.guru](https://crontab.guru/)
3. Check scheduler health: `curl http://localhost:8080/api/v1/health/scheduler`
4. Review logs: `symbi logs --filter scheduler --level debug`

### Job Failing Repeatedly

1. View history: `symbi cron history <job-id> --status failed`
2. Check error messages in run records
3. Verify agent configuration and capabilities
4. Test agent outside scheduler: `symbi run <agent-id> --input "test"`
5. Check policy gates: Ensure time windows and capabilities match

### Dead-Lettered Job

1. List dead-letter jobs: `symbi cron list --status dead_letter`
2. Review failure pattern: `symbi cron history <job-id>`
3. Fix root cause (agent code, permissions, external dependencies)
4. Reset job: `symbi cron reset <job-id>`

### High Memory Usage

1. Check session mode: Switch to `ephemeral_with_summary` or `fully_ephemeral`
2. Reduce heartbeat iterations: Lower `max_iterations`
3. Monitor context size: Review agent output verbosity
4. Enable context archiving: Configure retention policies

## Migration from v0.9.0

The v1.0.0 release adds production hardening features. Update your job definitions:

```diff
 schedule {
   name: "my-job"
   agent: "my-agent"
   cron: "0 * * * *"
+
+  # Add concurrency limit
+  max_concurrent: 2
+
+  # Add AgentPin for identity verification
+  agent_pin_jwt: "${AGENT_PIN_JWT}"
+
+  policy {
+    require_agent_pin: true
+  }
 }
```

Update configuration:

```diff
 [scheduler]
 tick_interval_seconds = 1
+ max_jitter_seconds = 30
+ default_max_retries = 3
+ shutdown_timeout_seconds = 60
```

No breaking API changes. All v0.9.0 jobs continue to work.
