// ── Agent types ──────────────────────────────────────────────────────

export type AgentState = 'idle' | 'running' | 'error' | 'stopped';

export interface ResourceUsage {
  memory_bytes: number;
  cpu_percent: number;
  active_tasks: number;
}

export interface AgentStatusResponse {
  agent_id: string;
  state: AgentState;
  last_activity: string;
  resource_usage: ResourceUsage;
}

export interface AgentExecutionRecord {
  execution_id: string;
  status: string;
  timestamp: string;
}

export interface GetAgentHistoryResponse {
  history: AgentExecutionRecord[];
}

// ── Health types ─────────────────────────────────────────────────────

export interface HealthResponse {
  status: string;
  uptime_seconds: number;
  timestamp: string;
  version: string;
}

export interface SchedulerHealthResponse {
  is_running: boolean;
  store_accessible: boolean;
  jobs_total: number;
  jobs_active: number;
  jobs_paused: number;
  jobs_dead_letter: number;
  global_active_runs: number;
  max_concurrent: number;
  runs_total: number;
  runs_succeeded: number;
  runs_failed: number;
  average_execution_time_ms: number;
  longest_run_ms: number;
}

// ── Schedule types ───────────────────────────────────────────────────

export interface ScheduleSummary {
  job_id: string;
  name: string;
  cron_expression: string;
  timezone: string;
  status: string;
  enabled: boolean;
  next_run: string | null;
  run_count: number;
}

export interface ScheduleDetail {
  job_id: string;
  name: string;
  cron_expression: string;
  timezone: string;
  status: string;
  enabled: boolean;
  one_shot: boolean;
  next_run: string | null;
  last_run: string | null;
  run_count: number;
  failure_count: number;
  created_at: string;
  updated_at: string;
}

export interface ScheduleRunEntry {
  run_id: string;
  started_at: string;
  completed_at: string | null;
  status: string;
  error: string | null;
  execution_time_ms: number | null;
}

export interface ScheduleHistoryResponse {
  job_id: string;
  history: ScheduleRunEntry[];
}

// ── Channel types ────────────────────────────────────────────────────

export interface ChannelSummary {
  id: string;
  name: string;
  platform: string;
  status: string;
}

export interface ChannelDetail {
  id: string;
  name: string;
  platform: string;
  status: string;
  config: Record<string, unknown>;
  created_at: string;
  updated_at: string;
}

export interface ChannelHealthResponse {
  id: string;
  connected: boolean;
  platform: string;
  workspace_name: string | null;
  channels_active: number;
  last_message_at: string | null;
  uptime_secs: number;
}

export interface ChannelAuditEntry {
  timestamp: string;
  event_type: string;
  user_id: string | null;
  channel_id: string | null;
  agent: string | null;
  details: Record<string, unknown>;
}

export interface ChannelAuditResponse {
  channel_id: string;
  entries: ChannelAuditEntry[];
}

// ── Error type ───────────────────────────────────────────────────────

export interface ErrorResponse {
  error: string;
  code: string;
  details?: Record<string, unknown>;
}

// ── Unified audit entry (client-side aggregation) ────────────────────

export type AuditSource = 'agent' | 'schedule' | 'channel';

export interface UnifiedAuditEntry {
  id: string;
  timestamp: string;
  source: AuditSource;
  sourceId: string;
  sourceName: string;
  eventType: string;
  status: string;
  details: Record<string, unknown>;
}
