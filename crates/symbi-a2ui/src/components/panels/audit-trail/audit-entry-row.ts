import { LitElement, html, css } from 'lit';
import { customElement, property } from 'lit/decorators.js';
import type { UnifiedAuditEntry } from '../../../api/types.js';
import type { BadgeVariant } from '../../shared/status-badge.js';

function statusToVariant(status: string): BadgeVariant {
  const s = status.toLowerCase();
  if (s === 'success' || s === 'completed' || s === 'succeeded') return 'healthy';
  if (s === 'failed' || s === 'error') return 'error';
  if (s === 'running' || s === 'in_progress' || s === 'pending') return 'warning';
  return 'neutral';
}

function sourceIcon(source: string): string {
  switch (source) {
    case 'agent': return '⬡';
    case 'schedule': return '◷';
    case 'channel': return '◆';
    default: return '○';
  }
}

@customElement('audit-entry-row')
export class AuditEntryRow extends LitElement {
  static styles = css`
    :host { display: block; }

    .row {
      display: flex;
      align-items: flex-start;
      gap: 0.75rem;
      padding: 0.625rem 0.75rem;
      border-radius: 0.375rem;
      border: 1px solid rgba(255, 255, 255, 0.05);
      background: rgba(255, 255, 255, 0.02);
      transition: background 0.15s;
    }

    .row:hover {
      background: rgba(255, 255, 255, 0.04);
    }

    .icon {
      flex-shrink: 0;
      width: 1.5rem;
      height: 1.5rem;
      display: flex;
      align-items: center;
      justify-content: center;
      border-radius: 50%;
      background: rgba(45, 212, 191, 0.1);
      color: #2dd4bf;
      font-size: 0.75rem;
    }

    .icon.agent { background: rgba(45, 212, 191, 0.1); color: #2dd4bf; }
    .icon.schedule { background: rgba(56, 189, 248, 0.1); color: #38bdf8; }
    .icon.channel { background: rgba(199, 146, 234, 0.1); color: #c792ea; }

    .body {
      flex: 1;
      min-width: 0;
    }

    .top-line {
      display: flex;
      align-items: center;
      gap: 0.5rem;
      margin-bottom: 0.25rem;
    }

    .source-name {
      font-size: 0.8125rem;
      font-weight: 500;
      color: #e2e8f0;
      overflow: hidden;
      text-overflow: ellipsis;
      white-space: nowrap;
    }

    .event-type {
      font-size: 0.6875rem;
      color: #64748b;
      font-family: 'Roboto Mono', monospace;
    }

    .details {
      font-size: 0.75rem;
      color: #64748b;
      margin-top: 0.25rem;
    }

    .details code {
      background: rgba(255, 255, 255, 0.05);
      padding: 0.125rem 0.25rem;
      border-radius: 0.25rem;
      font-family: 'Roboto Mono', monospace;
      font-size: 0.6875rem;
    }

    .right {
      display: flex;
      flex-direction: column;
      align-items: flex-end;
      gap: 0.25rem;
      flex-shrink: 0;
    }
  `;

  @property({ attribute: false }) entry!: UnifiedAuditEntry;

  render() {
    const e = this.entry;
    const variant = statusToVariant(e.status);
    const icon = sourceIcon(e.source);
    const hasDetails = Object.keys(e.details).length > 0;

    return html`
      <div class="row">
        <div class="icon ${e.source}">${icon}</div>
        <div class="body">
          <div class="top-line">
            <span class="source-name">${e.sourceName}</span>
            <span class="event-type">${e.eventType}</span>
          </div>
          ${hasDetails ? html`
            <div class="details">
              ${Object.entries(e.details).map(([k, v]) =>
                html`<span>${k}: <code>${String(v)}</code> </span>`,
              )}
            </div>
          ` : ''}
        </div>
        <div class="right">
          <status-badge .variant=${variant} .label=${e.status}></status-badge>
          <time-ago .datetime=${e.timestamp}></time-ago>
        </div>
      </div>
    `;
  }
}
