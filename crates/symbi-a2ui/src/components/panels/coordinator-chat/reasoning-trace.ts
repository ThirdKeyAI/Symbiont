import { LitElement, html, css } from 'lit';
import { customElement, property } from 'lit/decorators.js';

export type ReasoningPhase = 'observe' | 'reason' | 'gate' | 'act' | 'idle';

@customElement('reasoning-trace')
export class ReasoningTrace extends LitElement {
  static styles = css`
    :host {
      display: block;
    }

    .phases {
      display: flex;
      gap: 0.25rem;
      padding: 0.5rem 0;
    }

    .pill {
      padding: 0.25rem 0.625rem;
      border-radius: 9999px;
      font-size: 0.6875rem;
      font-weight: 600;
      text-transform: uppercase;
      letter-spacing: 0.05em;
      background: rgba(255, 255, 255, 0.05);
      color: #64748b;
      transition: background 0.2s, color 0.2s;
    }

    .pill.active {
      background: rgba(45, 212, 191, 0.2);
      color: #2dd4bf;
    }
  `;

  @property() activePhase: ReasoningPhase = 'idle';

  private _phases: { id: ReasoningPhase; label: string }[] = [
    { id: 'observe', label: 'O' },
    { id: 'reason', label: 'R' },
    { id: 'gate', label: 'G' },
    { id: 'act', label: 'A' },
  ];

  render() {
    if (this.activePhase === 'idle') return html``;

    return html`
      <div class="phases">
        ${this._phases.map(
          (p) => html`
            <span class="pill ${p.id === this.activePhase ? 'active' : ''}">${p.label}</span>
          `,
        )}
      </div>
    `;
  }
}
