import { LitElement, html, css } from 'lit';
import { customElement, property } from 'lit/decorators.js';
import type { UnifiedAuditEntry } from '../../../api/types.js';

@customElement('audit-timeline')
export class AuditTimeline extends LitElement {
  static styles = css`
    :host { display: block; }

    .timeline {
      display: flex;
      flex-direction: column;
      gap: 0.375rem;
      position: relative;
      padding-left: 1.5rem;
    }

    .timeline::before {
      content: '';
      position: absolute;
      left: 0.6875rem;
      top: 0;
      bottom: 0;
      width: 1px;
      background: rgba(255, 255, 255, 0.1);
    }

    .timeline-item {
      position: relative;
    }

    .timeline-item::before {
      content: '';
      position: absolute;
      left: -1.125rem;
      top: 0.875rem;
      width: 0.375rem;
      height: 0.375rem;
      border-radius: 50%;
      background: #374151;
    }

    .timeline-item.success::before { background: #22c55e; }
    .timeline-item.error::before { background: #ef4444; }
    .timeline-item.warning::before { background: #eab308; }

    .date-group {
      font-size: 0.6875rem;
      font-weight: 500;
      color: #64748b;
      text-transform: uppercase;
      letter-spacing: 0.05em;
      padding: 0.5rem 0 0.25rem;
      margin-left: -1.5rem;
    }

    .count-label {
      text-align: center;
      padding: 0.75rem;
      color: #64748b;
      font-size: 0.8125rem;
    }
  `;

  @property({ attribute: false }) entries: UnifiedAuditEntry[] = [];

  private _getDotClass(status: string): string {
    const s = status.toLowerCase();
    if (s === 'success' || s === 'completed' || s === 'succeeded') return 'success';
    if (s === 'failed' || s === 'error') return 'error';
    if (s === 'running' || s === 'pending') return 'warning';
    return '';
  }

  private _groupByDate(entries: UnifiedAuditEntry[]): Map<string, UnifiedAuditEntry[]> {
    const groups = new Map<string, UnifiedAuditEntry[]>();
    for (const entry of entries) {
      const date = new Date(entry.timestamp).toLocaleDateString(undefined, {
        weekday: 'short',
        month: 'short',
        day: 'numeric',
      });
      const group = groups.get(date) ?? [];
      group.push(entry);
      groups.set(date, group);
    }
    return groups;
  }

  render() {
    const MAX_VISIBLE = 200;
    const capped = this.entries.slice(0, MAX_VISIBLE);
    const groups = this._groupByDate(capped);

    return html`
      <div class="timeline">
        ${[...groups.entries()].map(
          ([date, entries]) => html`
            <div class="date-group">${date}</div>
            ${entries.map(
              (e) => html`
                <div class="timeline-item ${this._getDotClass(e.status)}">
                  <audit-entry-row .entry=${e}></audit-entry-row>
                </div>
              `,
            )}
          `,
        )}
      </div>
      ${this.entries.length > MAX_VISIBLE
        ? html`<p class="count-label">Showing ${MAX_VISIBLE} of ${this.entries.length} entries</p>`
        : ''}
    `;
  }
}
