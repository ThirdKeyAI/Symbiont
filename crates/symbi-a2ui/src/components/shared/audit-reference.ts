import { LitElement, html, css } from 'lit';
import { customElement, property } from 'lit/decorators.js';
import { auditIdentity } from '../../api/run-audit.js';

@customElement('audit-reference')
export class AuditReference extends LitElement {
  static styles = css`
    button { background:#112d35; border:1px solid #2b6470; color:#5eead4; padding:.4rem .65rem;
      border-radius:.4rem; cursor:pointer; font:inherit; font-size:.8rem; margin:.3rem 0; }
    button:focus-visible { outline:2px solid #7dd3fc; outline-offset:2px; }
    .invalid { color:#fbbf24; font-size:.8rem; }
  `;
  @property({ attribute: false }) reference: unknown;
  @property() label = 'Inspect run';
  render() {
    try { auditIdentity(this.reference); }
    catch { return html`<span class="invalid">Audit reference unavailable or invalid</span>`; }
    return html`<button @click=${() => this.dispatchEvent(new CustomEvent('inspect-run', {
      detail: this.reference, bubbles: true, composed: true,
    }))}>${this.label}</button>`;
  }
}
