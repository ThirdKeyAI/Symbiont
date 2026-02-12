import { LitElement, html, css } from 'lit';
import { customElement, state } from 'lit/decorators.js';
import { listAgents, getAgentStatus, getAgentHistory } from '../../../api/agents.js';
import { listSchedules, getScheduleHistory } from '../../../api/schedules.js';
import { listChannels, getChannelHealth } from '../../../api/channels.js';
import { getSchedulerHealth } from '../../../api/system.js';
import type {
  AgentStatusResponse,
  ScheduleSummary,
  ChannelHealthResponse,
  SchedulerHealthResponse,
} from '../../../api/types.js';

export interface ComplianceScores {
  overall: number;
  agentHealth: number;
  scheduleCompliance: number;
  channelHealth: number;
}

export interface Violation {
  id: string;
  severity: 'critical' | 'warning' | 'info';
  source: string;
  message: string;
  timestamp: string;
}

export interface PolicyStatus {
  name: string;
  status: 'pass' | 'fail' | 'warning';
  description: string;
  score: number;
}

@customElement('compliance-panel')
export class CompliancePanel extends LitElement {
  static styles = css`
    :host { display: block; }

    h2 {
      font-size: 1.25rem;
      font-weight: 600;
      color: #e2e8f0;
      margin: 0 0 1rem;
    }

    .error-banner {
      background: rgba(239, 68, 68, 0.1);
      border: 1px solid rgba(239, 68, 68, 0.3);
      border-radius: 0.5rem;
      padding: 0.75rem 1rem;
      color: #fca5a5;
      font-size: 0.8125rem;
      margin-bottom: 1rem;
    }

    .grid {
      display: grid;
      gap: 1rem;
    }

    .top-row {
      display: grid;
      grid-template-columns: 1fr 1fr;
      gap: 1rem;
    }

    .bottom-row {
      display: grid;
      grid-template-columns: 1fr 1fr;
      gap: 1rem;
    }

    @media (max-width: 768px) {
      .top-row, .bottom-row {
        grid-template-columns: 1fr;
      }
    }
  `;

  @state() private _scores: ComplianceScores = { overall: 0, agentHealth: 0, scheduleCompliance: 0, channelHealth: 0 };
  @state() private _violations: Violation[] = [];
  @state() private _policies: PolicyStatus[] = [];
  @state() private _channelHealths: ChannelHealthResponse[] = [];
  @state() private _loading = true;
  @state() private _error = '';

  private _interval?: ReturnType<typeof setInterval>;

  connectedCallback() {
    super.connectedCallback();
    this._fetchAll();
    this._interval = setInterval(() => this._fetchAll(), 30_000);
  }

  disconnectedCallback() {
    super.disconnectedCallback();
    if (this._interval) clearInterval(this._interval);
  }

  private async _fetchAll() {
    try {
      const [agentIds, schedules, channels, schedulerHealth] = await Promise.all([
        listAgents(),
        listSchedules(),
        listChannels(),
        getSchedulerHealth(),
      ]);

      // Fetch agent statuses
      const agentStatuses = await Promise.all(
        agentIds.map((id) => getAgentStatus(id).catch(() => null)),
      );
      const validAgents = agentStatuses.filter((a): a is AgentStatusResponse => a !== null);

      // Fetch channel healths
      const channelHealths = await Promise.all(
        channels.map((c) => getChannelHealth(c.id).catch(() => null)),
      );
      this._channelHealths = channelHealths.filter((h): h is ChannelHealthResponse => h !== null);

      // Fetch schedule histories for compliance
      const schedHistories = await Promise.all(
        schedules.map((s) => getScheduleHistory(s.job_id).catch(() => null)),
      );

      // Fetch agent histories for violations
      const agentHistories = await Promise.all(
        agentIds.map((id) => getAgentHistory(id).catch(() => null)),
      );

      // Compute scores
      const scores = this._computeScores(validAgents, schedules, schedulerHealth, this._channelHealths);
      this._scores = scores;

      // Compute violations
      this._violations = this._computeViolations(validAgents, schedules, this._channelHealths, schedHistories, agentHistories, agentIds);

      // Compute policy statuses
      this._policies = this._computePolicies(scores, validAgents, schedules, this._channelHealths, schedulerHealth);

      this._loading = false;
      this._error = '';
    } catch (e) {
      this._error = e instanceof Error ? e.message : 'Failed to fetch compliance data';
      this._loading = false;
    }
  }

  private _computeScores(
    agents: AgentStatusResponse[],
    _schedules: ScheduleSummary[],
    schedulerHealth: SchedulerHealthResponse,
    channelHealths: ChannelHealthResponse[],
  ): ComplianceScores {
    // Agent health: % of agents in healthy (idle/running) state
    const agentHealth = agents.length > 0
      ? (agents.filter((a) => a.state === 'idle' || a.state === 'running').length / agents.length) * 100
      : 100;

    // Schedule compliance: successful runs / total runs
    const scheduleCompliance = schedulerHealth.runs_total > 0
      ? (schedulerHealth.runs_succeeded / schedulerHealth.runs_total) * 100
      : 100;

    // Channel health: % of channels connected
    const channelHealth = channelHealths.length > 0
      ? (channelHealths.filter((c) => c.connected).length / channelHealths.length) * 100
      : 100;

    // Overall: weighted average
    const overall = (agentHealth * 0.35 + scheduleCompliance * 0.35 + channelHealth * 0.3);

    return {
      overall: Math.round(overall * 10) / 10,
      agentHealth: Math.round(agentHealth * 10) / 10,
      scheduleCompliance: Math.round(scheduleCompliance * 10) / 10,
      channelHealth: Math.round(channelHealth * 10) / 10,
    };
  }

  private _computeViolations(
    agents: AgentStatusResponse[],
    schedules: ScheduleSummary[],
    channelHealths: ChannelHealthResponse[],
    schedHistories: (import('../../../api/types.js').ScheduleHistoryResponse | null)[],
    agentHistories: (import('../../../api/types.js').GetAgentHistoryResponse | null)[],
    agentIds: string[],
  ): Violation[] {
    const violations: Violation[] = [];

    // Agent violations
    agents.forEach((a) => {
      if (a.state === 'error') {
        violations.push({
          id: `agent-error-${a.agent_id}`,
          severity: 'critical',
          source: `Agent: ${a.agent_id}`,
          message: `Agent is in error state`,
          timestamp: a.last_activity,
        });
      }
      if (a.state === 'stopped') {
        violations.push({
          id: `agent-stopped-${a.agent_id}`,
          severity: 'warning',
          source: `Agent: ${a.agent_id}`,
          message: `Agent is stopped`,
          timestamp: a.last_activity,
        });
      }
    });

    // Channel violations
    channelHealths.forEach((c) => {
      if (!c.connected) {
        violations.push({
          id: `channel-disconnected-${c.id}`,
          severity: 'critical',
          source: `Channel: ${c.id}`,
          message: `Channel is disconnected (${c.platform})`,
          timestamp: c.last_message_at ?? new Date().toISOString(),
        });
      }
    });

    // Schedule violations - failed runs from history
    schedHistories.forEach((hist, idx) => {
      if (!hist) return;
      const schedule = schedules[idx];
      const recentFails = hist.history.filter((h) => h.status === 'failed').slice(0, 5);
      recentFails.forEach((f) => {
        violations.push({
          id: `sched-fail-${f.run_id}`,
          severity: 'warning',
          source: `Schedule: ${schedule?.name ?? hist.job_id}`,
          message: f.error ?? 'Scheduled run failed',
          timestamp: f.started_at,
        });
      });
    });

    // Agent execution violations
    agentHistories.forEach((hist, idx) => {
      if (!hist) return;
      const agentId = agentIds[idx];
      const recentFails = hist.history.filter((h) => h.status === 'failed').slice(0, 5);
      recentFails.forEach((f) => {
        violations.push({
          id: `agent-exec-fail-${f.execution_id}`,
          severity: 'warning',
          source: `Agent: ${agentId}`,
          message: `Execution ${f.execution_id} failed`,
          timestamp: f.timestamp,
        });
      });
    });

    // Sort by timestamp descending
    violations.sort((a, b) => new Date(b.timestamp).getTime() - new Date(a.timestamp).getTime());

    return violations;
  }

  private _computePolicies(
    scores: ComplianceScores,
    agents: AgentStatusResponse[],
    _schedules: ScheduleSummary[],
    channelHealths: ChannelHealthResponse[],
    schedulerHealth: SchedulerHealthResponse,
  ): PolicyStatus[] {
    return [
      {
        name: 'Agent Health',
        status: scores.agentHealth >= 95 ? 'pass' : scores.agentHealth >= 80 ? 'warning' : 'fail',
        description: `${agents.filter((a) => a.state === 'idle' || a.state === 'running').length}/${agents.length} agents healthy`,
        score: scores.agentHealth,
      },
      {
        name: 'Schedule Compliance',
        status: scores.scheduleCompliance >= 95 ? 'pass' : scores.scheduleCompliance >= 80 ? 'warning' : 'fail',
        description: `${schedulerHealth.runs_succeeded}/${schedulerHealth.runs_total} runs succeeded`,
        score: scores.scheduleCompliance,
      },
      {
        name: 'Channel Connectivity',
        status: scores.channelHealth >= 95 ? 'pass' : scores.channelHealth >= 80 ? 'warning' : 'fail',
        description: `${channelHealths.filter((c) => c.connected).length}/${channelHealths.length} channels connected`,
        score: scores.channelHealth,
      },
      {
        name: 'Scheduler Running',
        status: schedulerHealth.is_running ? 'pass' : 'fail',
        description: schedulerHealth.is_running ? 'Scheduler is active' : 'Scheduler is stopped',
        score: schedulerHealth.is_running ? 100 : 0,
      },
      {
        name: 'Store Accessibility',
        status: schedulerHealth.store_accessible ? 'pass' : 'fail',
        description: schedulerHealth.store_accessible ? 'Data store is accessible' : 'Data store is unreachable',
        score: schedulerHealth.store_accessible ? 100 : 0,
      },
      {
        name: 'Dead Letter Queue',
        status: schedulerHealth.jobs_dead_letter === 0 ? 'pass' : schedulerHealth.jobs_dead_letter <= 3 ? 'warning' : 'fail',
        description: `${schedulerHealth.jobs_dead_letter} jobs in dead letter queue`,
        score: schedulerHealth.jobs_dead_letter === 0 ? 100 : Math.max(0, 100 - schedulerHealth.jobs_dead_letter * 20),
      },
    ];
  }

  render() {
    if (this._loading) {
      return html`<loading-spinner label="Computing compliance scores..."></loading-spinner>`;
    }

    return html`
      <h2>Compliance Dashboard</h2>
      ${this._error ? html`<div class="error-banner">${this._error}</div>` : ''}
      <div class="grid">
        <div class="top-row">
          <compliance-score .scores=${this._scores}></compliance-score>
          <violation-list .violations=${this._violations}></violation-list>
        </div>
        <div class="bottom-row">
          <policy-summary .policies=${this._policies}></policy-summary>
          <channel-compliance .channels=${this._channelHealths}></channel-compliance>
        </div>
      </div>
    `;
  }
}
