import { LitElement, html, css } from 'lit';
import { customElement, state } from 'lit/decorators.js';
import { hasToken, setToken, clearToken } from '../../api/client.js';

export type PanelId = 'fleet' | 'audit' | 'compliance';

@customElement('app-shell')
export class AppShell extends LitElement {
  static styles = css`
    :host {
      display: flex;
      flex-direction: column;
      height: 100vh;
      overflow: hidden;
    }

    .main {
      display: flex;
      flex: 1;
      overflow: hidden;
    }

    .content {
      flex: 1;
      overflow-y: auto;
      padding: 1.5rem;
    }

    /* Token overlay */
    .overlay {
      position: fixed;
      inset: 0;
      z-index: 100;
      display: flex;
      align-items: center;
      justify-content: center;
      background: rgba(0, 0, 0, 0.8);
      backdrop-filter: blur(8px);
    }

    .token-card {
      background: #111827;
      border: 1px solid rgba(255, 255, 255, 0.1);
      border-radius: 1rem;
      padding: 2rem;
      width: 100%;
      max-width: 28rem;
    }

    .token-card h2 {
      font-size: 1.25rem;
      font-weight: 600;
      color: #e2e8f0;
      margin: 0 0 0.5rem;
    }

    .token-card p {
      color: #64748b;
      font-size: 0.875rem;
      margin: 0 0 1.5rem;
    }

    .token-card input {
      width: 100%;
      padding: 0.625rem 0.75rem;
      background: #0a0a0a;
      border: 1px solid #374151;
      border-radius: 0.5rem;
      color: #e2e8f0;
      font-family: 'Roboto Mono', monospace;
      font-size: 0.875rem;
      outline: none;
      box-sizing: border-box;
    }

    .token-card input:focus {
      border-color: #2dd4bf;
    }

    .token-card button {
      margin-top: 1rem;
      width: 100%;
      padding: 0.625rem;
      background: linear-gradient(to right, #2dd4bf, #38bdf8);
      color: #0a0a0a;
      font-weight: 600;
      border: none;
      border-radius: 0.5rem;
      cursor: pointer;
      font-size: 0.875rem;
    }

    .token-card button:hover {
      opacity: 0.9;
    }

    .logo-row {
      display: flex;
      align-items: center;
      gap: 0.5rem;
      margin-bottom: 1.5rem;
    }

    .logo-text {
      font-size: 1.125rem;
      font-weight: 700;
      background: linear-gradient(to right, #2dd4bf, #38bdf8);
      -webkit-background-clip: text;
      -webkit-text-fill-color: transparent;
      background-clip: text;
    }
  `;

  @state() private _panel: PanelId = 'fleet';
  @state() private _authenticated = hasToken();
  @state() private _tokenInput = '';
  @state() private _sidebarCollapsed = false;

  private _onNavigate(e: CustomEvent<PanelId>) {
    this._panel = e.detail;
  }

  private _onToggleSidebar() {
    this._sidebarCollapsed = !this._sidebarCollapsed;
  }

  private _onLogout() {
    clearToken();
    this._authenticated = false;
    this._tokenInput = '';
  }

  private _submitToken() {
    const v = this._tokenInput.trim();
    if (v) {
      setToken(v);
      this._authenticated = true;
    }
  }

  private _onKeydown(e: KeyboardEvent) {
    if (e.key === 'Enter') this._submitToken();
  }

  render() {
    if (!this._authenticated) {
      return html`
        <div class="overlay">
          <div class="token-card">
            <div class="logo-row">
              <span class="logo-text">Symbiont</span>
            </div>
            <h2>Connect to Runtime</h2>
            <p>Enter your SYMBI_AUTH_TOKEN to access the operations console.</p>
            <input
              type="password"
              placeholder="Bearer token"
              .value=${this._tokenInput}
              @input=${(e: InputEvent) => (this._tokenInput = (e.target as HTMLInputElement).value)}
              @keydown=${this._onKeydown}
            />
            <button @click=${this._submitToken}>Connect</button>
          </div>
        </div>
      `;
    }

    return html`
      <header-bar
        .panel=${this._panel}
        .sidebarCollapsed=${this._sidebarCollapsed}
        @toggle-sidebar=${this._onToggleSidebar}
        @logout=${this._onLogout}
      ></header-bar>
      <div class="main">
        <nav-sidebar
          .activePanel=${this._panel}
          .collapsed=${this._sidebarCollapsed}
          @navigate=${this._onNavigate}
        ></nav-sidebar>
        <main class="content">
          ${this._renderPanel()}
        </main>
      </div>
    `;
  }

  private _renderPanel() {
    switch (this._panel) {
      case 'fleet':
        return html`<fleet-overview-panel></fleet-overview-panel>`;
      case 'audit':
        return html`<audit-trail-panel></audit-trail-panel>`;
      case 'compliance':
        return html`<compliance-panel></compliance-panel>`;
    }
  }
}
