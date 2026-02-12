import { LitElement, html, css } from 'lit';
import { customElement, property, state } from 'lit/decorators.js';
import { getHealth } from '../../api/system.js';

@customElement('header-bar')
export class HeaderBar extends LitElement {
  static styles = css`
    :host {
      display: block;
    }

    header {
      display: flex;
      align-items: center;
      justify-content: space-between;
      height: 3rem;
      padding: 0 1rem;
      background: #111827;
      border-bottom: 1px solid rgba(255, 255, 255, 0.1);
    }

    .left {
      display: flex;
      align-items: center;
      gap: 0.75rem;
    }

    .toggle-btn {
      background: none;
      border: none;
      color: #64748b;
      cursor: pointer;
      font-size: 1.125rem;
      padding: 0.25rem;
      line-height: 1;
    }

    .toggle-btn:hover {
      color: #e2e8f0;
    }

    .brand {
      font-size: 0.9375rem;
      font-weight: 700;
      background: linear-gradient(to right, #2dd4bf, #38bdf8);
      -webkit-background-clip: text;
      -webkit-text-fill-color: transparent;
      background-clip: text;
    }

    .right {
      display: flex;
      align-items: center;
      gap: 1rem;
    }

    .status {
      display: flex;
      align-items: center;
      gap: 0.375rem;
      font-size: 0.75rem;
      color: #64748b;
    }

    .dot {
      width: 0.5rem;
      height: 0.5rem;
      border-radius: 50%;
      background: #374151;
    }

    .dot.connected {
      background: #22c55e;
    }

    .dot.error {
      background: #ef4444;
    }

    .version {
      font-family: 'Roboto Mono', monospace;
      font-size: 0.6875rem;
      color: #64748b;
    }

    .logout-btn {
      background: none;
      border: 1px solid #374151;
      color: #64748b;
      padding: 0.25rem 0.625rem;
      border-radius: 0.375rem;
      font-size: 0.75rem;
      cursor: pointer;
    }

    .logout-btn:hover {
      color: #e2e8f0;
      border-color: #4b5563;
    }
  `;

  @property() panel = '';
  @property({ type: Boolean }) sidebarCollapsed = false;
  @state() private _connected = false;
  @state() private _version = '';

  private _interval?: ReturnType<typeof setInterval>;

  connectedCallback() {
    super.connectedCallback();
    this._checkConnection();
    this._interval = setInterval(() => this._checkConnection(), 15_000);
  }

  disconnectedCallback() {
    super.disconnectedCallback();
    if (this._interval) clearInterval(this._interval);
  }

  private async _checkConnection() {
    try {
      const h = await getHealth();
      this._connected = h.status === 'healthy';
      this._version = h.version;
    } catch {
      this._connected = false;
    }
  }

  render() {
    return html`
      <header>
        <div class="left">
          <button class="toggle-btn" @click=${this._toggle} title="Toggle sidebar">☰</button>
          <span class="brand">Symbiont</span>
        </div>
        <div class="right">
          <div class="status">
            <span class="dot ${this._connected ? 'connected' : 'error'}"></span>
            ${this._connected ? 'Connected' : 'Disconnected'}
          </div>
          ${this._version ? html`<span class="version">v${this._version}</span>` : ''}
          <button class="logout-btn" @click=${this._logout}>Disconnect</button>
        </div>
      </header>
    `;
  }

  private _toggle() {
    this.dispatchEvent(new CustomEvent('toggle-sidebar', { bubbles: true, composed: true }));
  }

  private _logout() {
    this.dispatchEvent(new CustomEvent('logout', { bubbles: true, composed: true }));
  }
}
