import { LitElement, html, css } from 'lit';
import { customElement, property } from 'lit/decorators.js';

@customElement('audit-filters')
export class AuditFilters extends LitElement {
  static styles = css`
    :host { display: block; }

    .filters {
      display: flex;
      gap: 0.75rem;
      flex-wrap: wrap;
      background: #111827;
      border: 1px solid rgba(255, 255, 255, 0.1);
      border-radius: 0.75rem;
      padding: 0.75rem 1rem;
    }

    .filter-group {
      display: flex;
      flex-direction: column;
      gap: 0.25rem;
    }

    label {
      font-size: 0.6875rem;
      color: #64748b;
      text-transform: uppercase;
      letter-spacing: 0.05em;
    }

    select, input {
      padding: 0.375rem 0.5rem;
      background: #0a0a0a;
      border: 1px solid #374151;
      border-radius: 0.375rem;
      color: #e2e8f0;
      font-size: 0.8125rem;
      outline: none;
      min-width: 8rem;
    }

    select:focus, input:focus {
      border-color: #2dd4bf;
    }

    option {
      background: #111827;
      color: #e2e8f0;
    }

    .count {
      display: flex;
      align-items: flex-end;
      font-size: 0.75rem;
      color: #64748b;
      padding-bottom: 0.375rem;
    }
  `;

  @property({ attribute: false }) agents: string[] = [];
  @property() filterAgent = '';
  @property() filterType = '';
  @property() filterTimeRange = '';

  private _emit(key: string, value: string) {
    this.dispatchEvent(
      new CustomEvent('filter-change', {
        detail: { [key]: value },
        bubbles: true,
        composed: true,
      }),
    );
  }

  render() {
    return html`
      <div class="filters">
        <div class="filter-group">
          <label>Source</label>
          <input
            type="text"
            placeholder="Filter by agent..."
            .value=${this.filterAgent}
            @input=${(e: InputEvent) => this._emit('agent', (e.target as HTMLInputElement).value)}
          />
        </div>
        <div class="filter-group">
          <label>Type</label>
          <select
            .value=${this.filterType}
            @change=${(e: Event) => this._emit('type', (e.target as HTMLSelectElement).value)}
          >
            <option value="">All types</option>
            <option value="agent">Agent</option>
            <option value="schedule">Schedule</option>
            <option value="channel">Channel</option>
          </select>
        </div>
        <div class="filter-group">
          <label>Time range</label>
          <select
            .value=${this.filterTimeRange}
            @change=${(e: Event) => this._emit('timeRange', (e.target as HTMLSelectElement).value)}
          >
            <option value="">All time</option>
            <option value="1h">Last hour</option>
            <option value="6h">Last 6 hours</option>
            <option value="24h">Last 24 hours</option>
            <option value="7d">Last 7 days</option>
          </select>
        </div>
      </div>
    `;
  }
}
