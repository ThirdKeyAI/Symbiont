import { LitElement, html, css } from 'lit';
import { customElement, property } from 'lit/decorators.js';
import type { ScheduleSummary as ScheduleSummaryType } from '../../../api/types.js';
import type { BadgeVariant } from '../../shared/status-badge.js';

function scheduleVariant(s: ScheduleSummaryType): BadgeVariant {
  if (!s.enabled) return 'neutral';
  if (s.status === 'active' || s.status === 'running') return 'healthy';
  if (s.status === 'paused') return 'warning';
  return 'error';
}

@customElement('schedule-summary')
export class ScheduleSummaryComponent extends LitElement {
  static styles = css`
    :host { display: block; }

    .wrapper {
      background: #111827;
      border: 1px solid rgba(255, 255, 255, 0.1);
      border-radius: 0.75rem;
      padding: 0.75rem;
    }

    .list {
      display: flex;
      flex-direction: column;
      gap: 0.375rem;
      max-height: 32rem;
      overflow-y: auto;
    }

    .item {
      display: flex;
      align-items: center;
      justify-content: space-between;
      padding: 0.5rem 0.75rem;
      border-radius: 0.375rem;
      background: rgba(255, 255, 255, 0.02);
      border: 1px solid rgba(255, 255, 255, 0.05);
    }

    .item:hover {
      background: rgba(255, 255, 255, 0.04);
    }

    .info {
      display: flex;
      flex-direction: column;
      gap: 0.125rem;
      min-width: 0;
    }

    .name {
      font-size: 0.8125rem;
      font-weight: 500;
      color: #e2e8f0;
      overflow: hidden;
      text-overflow: ellipsis;
      white-space: nowrap;
    }

    .cron {
      font-size: 0.6875rem;
      color: #64748b;
      font-family: 'Roboto Mono', monospace;
    }

    .right {
      display: flex;
      flex-direction: column;
      align-items: flex-end;
      gap: 0.25rem;
      flex-shrink: 0;
    }

    .next-run {
      font-size: 0.6875rem;
      color: #64748b;
    }

    .run-count {
      font-size: 0.6875rem;
      color: #4b5563;
    }
  `;

  @property({ attribute: false }) schedules: ScheduleSummaryType[] = [];

  render() {
    if (this.schedules.length === 0) {
      return html`
        <div class="wrapper">
          <empty-state
            icon="◷"
            title="No schedules"
            description="No cron schedules configured yet"
          ></empty-state>
        </div>
      `;
    }

    return html`
      <div class="wrapper">
        <div class="list">
          ${this.schedules.map((s) => {
            const variant = scheduleVariant(s);
            return html`
              <div class="item">
                <div class="info">
                  <span class="name">${s.name}</span>
                  <span class="cron">${s.cron_expression} (${s.timezone})</span>
                </div>
                <div class="right">
                  <status-badge .variant=${variant} .label=${s.enabled ? s.status : 'disabled'}></status-badge>
                  ${s.next_run ? html`<span class="next-run">Next: <time-ago .datetime=${s.next_run}></time-ago></span>` : ''}
                  <span class="run-count">${s.run_count} runs</span>
                </div>
              </div>
            `;
          })}
        </div>
      </div>
    `;
  }
}
