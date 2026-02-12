import { LitElement, html, css } from 'lit';
import { customElement, property } from 'lit/decorators.js';
import type { Violation } from './compliance-panel.js';

@customElement('violation-list')
export class ViolationList extends LitElement {
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

    .summary {
      display: flex;
      gap: 1rem;
      margin-bottom: 0.75rem;
    }

    .count-badge {
      display: flex;
      align-items: center;
      gap: 0.25rem;
      font-size: 0.75rem;
      font-weight: 600;
    }

    .count-badge.critical { color: #ef4444; }
    .count-badge.warning { color: #eab308; }
    .count-badge.info { color: #38bdf8; }

    .list {
      display: flex;
      flex-direction: column;
      gap: 0.375rem;
      max-height: 20rem;
      overflow-y: auto;
    }

    .item {
      display: flex;
      align-items: flex-start;
      gap: 0.5rem;
      padding: 0.5rem 0.625rem;
      border-radius: 0.375rem;
      background: rgba(255, 255, 255, 0.02);
      border: 1px solid rgba(255, 255, 255, 0.05);
    }

    .severity-dot {
      width: 0.375rem;
      height: 0.375rem;
      border-radius: 50%;
      flex-shrink: 0;
      margin-top: 0.375rem;
    }

    .severity-dot.critical { background: #ef4444; }
    .severity-dot.warning { background: #eab308; }
    .severity-dot.info { background: #38bdf8; }

    .item-body {
      flex: 1;
      min-width: 0;
    }

    .item-source {
      font-size: 0.6875rem;
      color: #64748b;
      margin-bottom: 0.125rem;
    }

    .item-message {
      font-size: 0.8125rem;
      color: #e2e8f0;
      word-break: break-word;
    }

    .item-time {
      flex-shrink: 0;
    }
  `;

  @property({ attribute: false }) violations: Violation[] = [];

  render() {
    const critical = this.violations.filter((v) => v.severity === 'critical').length;
    const warning = this.violations.filter((v) => v.severity === 'warning').length;
    const info = this.violations.filter((v) => v.severity === 'info').length;

    return html`
      <div class="card">
        <p class="title">Violations</p>
        <div class="summary">
          <span class="count-badge critical">${critical} critical</span>
          <span class="count-badge warning">${warning} warnings</span>
          <span class="count-badge info">${info} info</span>
        </div>
        ${this.violations.length === 0
          ? html`<empty-state
              icon="✓"
              title="No violations"
              description="All systems operating within compliance"
            ></empty-state>`
          : html`
            <div class="list">
              ${this.violations.map(
                (v) => html`
                  <div class="item">
                    <div class="severity-dot ${v.severity}"></div>
                    <div class="item-body">
                      <div class="item-source">${v.source}</div>
                      <div class="item-message">${v.message}</div>
                    </div>
                    <div class="item-time">
                      <time-ago .datetime=${v.timestamp}></time-ago>
                    </div>
                  </div>
                `,
              )}
            </div>
          `}
      </div>
    `;
  }
}
