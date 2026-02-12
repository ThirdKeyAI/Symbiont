import { LitElement, html, css } from 'lit';
import { customElement, property } from 'lit/decorators.js';

export type BadgeVariant = 'healthy' | 'warning' | 'error' | 'neutral';

@customElement('status-badge')
export class StatusBadge extends LitElement {
  static styles = css`
    :host {
      display: inline-flex;
    }

    .badge {
      display: inline-flex;
      align-items: center;
      gap: 0.375rem;
      padding: 0.125rem 0.5rem;
      border-radius: 9999px;
      font-size: 0.75rem;
      font-weight: 500;
      line-height: 1.5;
    }

    .dot {
      width: 0.375rem;
      height: 0.375rem;
      border-radius: 50%;
      flex-shrink: 0;
    }

    .healthy {
      background: rgba(34, 197, 94, 0.15);
      color: #22c55e;
    }
    .healthy .dot { background: #22c55e; }

    .warning {
      background: rgba(234, 179, 8, 0.15);
      color: #eab308;
    }
    .warning .dot { background: #eab308; }

    .error {
      background: rgba(239, 68, 68, 0.15);
      color: #ef4444;
    }
    .error .dot { background: #ef4444; }

    .neutral {
      background: rgba(100, 116, 139, 0.15);
      color: #64748b;
    }
    .neutral .dot { background: #64748b; }
  `;

  @property() variant: BadgeVariant = 'neutral';
  @property() label = '';

  render() {
    return html`
      <span class="badge ${this.variant}">
        <span class="dot"></span>
        ${this.label}
      </span>
    `;
  }
}
