import { LitElement, html, css } from 'lit';
import { customElement, property } from 'lit/decorators.js';
import type { HealthResponse, SchedulerHealthResponse } from '../../../api/types.js';

function formatUptime(secs: number): string {
  const d = Math.floor(secs / 86400);
  const h = Math.floor((secs % 86400) / 3600);
  const m = Math.floor((secs % 3600) / 60);
  if (d > 0) return `${d}d ${h}h`;
  if (h > 0) return `${h}h ${m}m`;
  return `${m}m`;
}

@customElement('system-health-bar')
export class SystemHealthBar extends LitElement {
  static styles = css`
    :host { display: block; }

    .bar {
      display: flex;
      gap: 1rem;
      flex-wrap: wrap;
      background: #111827;
      border: 1px solid rgba(255, 255, 255, 0.1);
      border-radius: 0.75rem;
      padding: 1rem 1.25rem;
    }

    .metric {
      display: flex;
      flex-direction: column;
      min-width: 7rem;
    }

    .metric-label {
      font-size: 0.6875rem;
      color: #64748b;
      text-transform: uppercase;
      letter-spacing: 0.05em;
      margin-bottom: 0.25rem;
    }

    .metric-value {
      font-size: 1.5rem;
      font-weight: 700;
      color: #e2e8f0;
      font-family: 'Roboto Mono', monospace;
    }

    .metric-value.healthy { color: #22c55e; }
    .metric-value.warning { color: #eab308; }
    .metric-value.error { color: #ef4444; }

    .divider {
      width: 1px;
      background: rgba(255, 255, 255, 0.1);
      align-self: stretch;
    }

    .success-rate {
      font-size: 0.75rem;
      color: #64748b;
      margin-top: 0.125rem;
    }
  `;

  @property({ attribute: false }) health: HealthResponse | null = null;
  @property({ attribute: false }) schedulerHealth: SchedulerHealthResponse | null = null;
  @property({ attribute: false }) metrics: Record<string, unknown> | null = null;
  @property({ type: Number }) agentCount = 0;
  @property({ type: Number }) scheduleCount = 0;

  render() {
    const sh = this.schedulerHealth;
    const successRate = sh && sh.runs_total > 0
      ? ((sh.runs_succeeded / sh.runs_total) * 100).toFixed(1)
      : '—';
    const successClass = sh && sh.runs_total > 0
      ? (sh.runs_succeeded / sh.runs_total >= 0.95 ? 'healthy' : sh.runs_succeeded / sh.runs_total >= 0.8 ? 'warning' : 'error')
      : '';

    return html`
      <div class="bar">
        <div class="metric">
          <span class="metric-label">Status</span>
          <span class="metric-value ${this.health?.status === 'healthy' ? 'healthy' : 'error'}">
            ${this.health?.status ?? '—'}
          </span>
        </div>
        <div class="divider"></div>
        <div class="metric">
          <span class="metric-label">Uptime</span>
          <span class="metric-value">
            ${this.health ? formatUptime(this.health.uptime_seconds) : '—'}
          </span>
        </div>
        <div class="divider"></div>
        <div class="metric">
          <span class="metric-label">Agents</span>
          <span class="metric-value">${this.agentCount}</span>
        </div>
        <div class="divider"></div>
        <div class="metric">
          <span class="metric-label">Schedules</span>
          <span class="metric-value">${this.scheduleCount}</span>
        </div>
        <div class="divider"></div>
        <div class="metric">
          <span class="metric-label">Active Jobs</span>
          <span class="metric-value">${sh?.jobs_active ?? '—'}</span>
        </div>
        <div class="divider"></div>
        <div class="metric">
          <span class="metric-label">Success Rate</span>
          <span class="metric-value ${successClass}">
            ${successRate}${successRate !== '—' ? '%' : ''}
          </span>
          ${sh ? html`<span class="success-rate">${sh.runs_succeeded}/${sh.runs_total} runs</span>` : ''}
        </div>
        <div class="divider"></div>
        <div class="metric">
          <span class="metric-label">Avg Exec Time</span>
          <span class="metric-value">${sh ? `${sh.average_execution_time_ms.toFixed(0)}ms` : '—'}</span>
        </div>
        <div class="divider"></div>
        <div class="metric">
          <span class="metric-label">Scheduler</span>
          <span class="metric-value ${sh?.is_running ? 'healthy' : 'error'}">
            ${sh?.is_running ? 'Running' : 'Stopped'}
          </span>
        </div>
      </div>
    `;
  }
}
