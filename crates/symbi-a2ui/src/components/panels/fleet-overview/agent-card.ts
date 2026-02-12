import { LitElement, html, css } from 'lit';
import { customElement, property } from 'lit/decorators.js';
import type { AgentStatusResponse } from '../../../api/types.js';
import type { BadgeVariant } from '../../shared/status-badge.js';

function stateToVariant(state: string): BadgeVariant {
  switch (state) {
    case 'idle': return 'healthy';
    case 'running': return 'healthy';
    case 'error': return 'error';
    case 'stopped': return 'warning';
    default: return 'neutral';
  }
}

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes}B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)}KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)}MB`;
}

@customElement('agent-card')
export class AgentCard extends LitElement {
  static styles = css`
    :host { display: block; }

    .card {
      background: #111827;
      border: 1px solid rgba(255, 255, 255, 0.1);
      border-radius: 0.5rem;
      padding: 0.75rem 1rem;
      transition: border-color 0.15s;
    }

    .card:hover {
      border-color: rgba(45, 212, 191, 0.3);
    }

    .header {
      display: flex;
      align-items: center;
      justify-content: space-between;
      margin-bottom: 0.5rem;
    }

    .agent-id {
      font-size: 0.875rem;
      font-weight: 600;
      color: #e2e8f0;
      font-family: 'Roboto Mono', monospace;
      overflow: hidden;
      text-overflow: ellipsis;
      white-space: nowrap;
    }

    .resources {
      display: flex;
      gap: 1rem;
      font-size: 0.75rem;
      color: #64748b;
    }

    .resource {
      display: flex;
      align-items: center;
      gap: 0.25rem;
    }

    .resource-label {
      color: #4b5563;
    }

    .footer {
      display: flex;
      align-items: center;
      justify-content: space-between;
      margin-top: 0.5rem;
      font-size: 0.75rem;
    }
  `;

  @property({ attribute: false }) agent!: AgentStatusResponse;

  render() {
    const a = this.agent;
    const variant = stateToVariant(a.state);

    return html`
      <div class="card">
        <div class="header">
          <span class="agent-id">${a.agent_id}</span>
          <status-badge .variant=${variant} .label=${a.state}></status-badge>
        </div>
        <div class="resources">
          <span class="resource">
            <span class="resource-label">MEM</span>
            ${formatBytes(a.resource_usage.memory_bytes)}
          </span>
          <span class="resource">
            <span class="resource-label">CPU</span>
            ${a.resource_usage.cpu_percent.toFixed(1)}%
          </span>
          <span class="resource">
            <span class="resource-label">Tasks</span>
            ${a.resource_usage.active_tasks}
          </span>
        </div>
        <div class="footer">
          <time-ago .datetime=${a.last_activity}></time-ago>
        </div>
      </div>
    `;
  }
}
