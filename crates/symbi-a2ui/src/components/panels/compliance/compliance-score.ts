import { LitElement, html, css } from 'lit';
import { customElement, property } from 'lit/decorators.js';
import type { ComplianceScores } from './compliance-panel.js';

@customElement('compliance-score')
export class ComplianceScore extends LitElement {
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
      margin: 0 0 1rem;
    }

    .gauge-container {
      display: flex;
      align-items: center;
      justify-content: center;
      margin-bottom: 1.25rem;
    }

    svg {
      width: 10rem;
      height: 10rem;
    }

    .gauge-bg {
      fill: none;
      stroke: #1f2937;
      stroke-width: 8;
    }

    .gauge-fill {
      fill: none;
      stroke-width: 8;
      stroke-linecap: round;
      transition: stroke-dashoffset 0.6s ease;
    }

    .gauge-text {
      font-family: 'Roboto Mono', monospace;
      font-weight: 700;
      font-size: 1.75rem;
      fill: #e2e8f0;
      text-anchor: middle;
      dominant-baseline: central;
    }

    .gauge-label {
      font-size: 0.625rem;
      fill: #64748b;
      text-anchor: middle;
    }

    .breakdown {
      display: flex;
      flex-direction: column;
      gap: 0.5rem;
    }

    .metric-row {
      display: flex;
      align-items: center;
      justify-content: space-between;
    }

    .metric-name {
      font-size: 0.8125rem;
      color: #cbd5e1;
    }

    .metric-bar {
      flex: 1;
      height: 0.25rem;
      background: #1f2937;
      border-radius: 0.125rem;
      margin: 0 0.75rem;
      overflow: hidden;
    }

    .metric-bar-fill {
      height: 100%;
      border-radius: 0.125rem;
      transition: width 0.4s ease;
    }

    .metric-value {
      font-size: 0.8125rem;
      font-family: 'Roboto Mono', monospace;
      font-weight: 600;
      min-width: 3rem;
      text-align: right;
    }

    .healthy { color: #22c55e; }
    .warning { color: #eab308; }
    .error { color: #ef4444; }
  `;

  @property({ attribute: false }) scores: ComplianceScores = {
    overall: 0, agentHealth: 0, scheduleCompliance: 0, channelHealth: 0,
  };

  private _scoreColor(score: number): string {
    if (score >= 90) return '#22c55e';
    if (score >= 70) return '#eab308';
    return '#ef4444';
  }

  private _scoreClass(score: number): string {
    if (score >= 90) return 'healthy';
    if (score >= 70) return 'warning';
    return 'error';
  }

  render() {
    const { overall, agentHealth, scheduleCompliance, channelHealth } = this.scores;
    const r = 60;
    const c = 2 * Math.PI * r;
    const offset = c - (overall / 100) * c;
    const color = this._scoreColor(overall);

    const breakdowns = [
      { name: 'Agent Health', value: agentHealth },
      { name: 'Schedule Compliance', value: scheduleCompliance },
      { name: 'Channel Health', value: channelHealth },
    ];

    return html`
      <div class="card">
        <p class="title">Overall Score</p>
        <div class="gauge-container">
          <svg viewBox="0 0 160 160">
            <circle class="gauge-bg" cx="80" cy="80" r=${r}></circle>
            <circle
              class="gauge-fill"
              cx="80" cy="80" r=${r}
              stroke=${color}
              stroke-dasharray=${c}
              stroke-dashoffset=${offset}
              transform="rotate(-90 80 80)"
            ></circle>
            <text class="gauge-text" x="80" y="75">${overall}%</text>
            <text class="gauge-label" x="80" y="100">compliance</text>
          </svg>
        </div>
        <div class="breakdown">
          ${breakdowns.map((m) => html`
            <div class="metric-row">
              <span class="metric-name">${m.name}</span>
              <div class="metric-bar">
                <div
                  class="metric-bar-fill"
                  style="width: ${m.value}%; background: ${this._scoreColor(m.value)}"
                ></div>
              </div>
              <span class="metric-value ${this._scoreClass(m.value)}">${m.value}%</span>
            </div>
          `)}
        </div>
      </div>
    `;
  }
}
