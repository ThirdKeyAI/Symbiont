import { LitElement, html, css } from 'lit';
import { customElement, property } from 'lit/decorators.js';
import type { PolicyStatus } from './compliance-panel.js';

@customElement('policy-summary')
export class PolicySummaryComponent extends LitElement {
  static styles = css`
    :host { display: block; }

    .card {
      background: #111827;
      border: 1px solid rgba(255, 255, 255, 0.1);
      border-radius: 0.75rem;
      padding: 1.25rem;
    }

    .title {
      font-size: 0.8125rem;
      font-weight: 500;
      color: #64748b;
      text-transform: uppercase;
      letter-spacing: 0.05em;
      margin: 0 0 0.75rem;
    }

    .list {
      display: flex;
      flex-direction: column;
      gap: 0.5rem;
    }

    .item {
      display: flex;
      align-items: center;
      gap: 0.75rem;
      padding: 0.625rem 0.75rem;
      border-radius: 0.375rem;
      background: rgba(255, 255, 255, 0.02);
      border: 1px solid rgba(255, 255, 255, 0.05);
    }

    .status-indicator {
      width: 1.5rem;
      height: 1.5rem;
      border-radius: 0.375rem;
      display: flex;
      align-items: center;
      justify-content: center;
      font-size: 0.75rem;
      flex-shrink: 0;
    }

    .status-indicator.pass {
      background: rgba(34, 197, 94, 0.15);
      color: #22c55e;
    }

    .status-indicator.warning {
      background: rgba(234, 179, 8, 0.15);
      color: #eab308;
    }

    .status-indicator.fail {
      background: rgba(239, 68, 68, 0.15);
      color: #ef4444;
    }

    .info {
      flex: 1;
      min-width: 0;
    }

    .policy-name {
      font-size: 0.8125rem;
      font-weight: 500;
      color: #e2e8f0;
    }

    .policy-desc {
      font-size: 0.6875rem;
      color: #64748b;
      margin-top: 0.125rem;
    }

    .score {
      font-size: 0.8125rem;
      font-family: 'Roboto Mono', monospace;
      font-weight: 600;
      flex-shrink: 0;
    }

    .score.pass { color: #22c55e; }
    .score.warning { color: #eab308; }
    .score.fail { color: #ef4444; }
  `;

  @property({ attribute: false }) policies: PolicyStatus[] = [];

  private _statusIcon(status: string): string {
    switch (status) {
      case 'pass': return '✓';
      case 'warning': return '!';
      case 'fail': return '✕';
      default: return '?';
    }
  }

  render() {
    return html`
      <div class="card">
        <p class="title">Policy Status</p>
        ${this.policies.length === 0
          ? html`<empty-state icon="◉" title="No policies" description="No compliance policies configured"></empty-state>`
          : html`
            <div class="list">
              ${this.policies.map(
                (p) => html`
                  <div class="item">
                    <div class="status-indicator ${p.status}">${this._statusIcon(p.status)}</div>
                    <div class="info">
                      <div class="policy-name">${p.name}</div>
                      <div class="policy-desc">${p.description}</div>
                    </div>
                    <span class="score ${p.status}">${p.score}%</span>
                  </div>
                `,
              )}
            </div>
          `}
      </div>
    `;
  }
}
