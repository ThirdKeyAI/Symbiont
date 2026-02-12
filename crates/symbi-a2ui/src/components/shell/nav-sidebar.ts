import { LitElement, html, css } from 'lit';
import { customElement, property } from 'lit/decorators.js';
import type { PanelId } from './app-shell.js';

interface NavItem {
  id: PanelId;
  label: string;
  icon: string;
}

const NAV_ITEMS: NavItem[] = [
  { id: 'fleet', label: 'Fleet Overview', icon: '⬡' },
  { id: 'audit', label: 'Audit Trail', icon: '◈' },
  { id: 'compliance', label: 'Compliance', icon: '◉' },
];

@customElement('nav-sidebar')
export class NavSidebar extends LitElement {
  static styles = css`
    :host {
      display: block;
      flex-shrink: 0;
    }

    nav {
      height: 100%;
      background: #111827;
      border-right: 1px solid rgba(255, 255, 255, 0.1);
      display: flex;
      flex-direction: column;
      padding: 0.75rem 0;
      transition: width 0.2s ease;
    }

    nav.expanded {
      width: 14rem;
    }

    nav.collapsed {
      width: 3.5rem;
    }

    button {
      display: flex;
      align-items: center;
      gap: 0.75rem;
      width: 100%;
      padding: 0.625rem 1rem;
      background: none;
      border: none;
      color: #64748b;
      font-size: 0.875rem;
      cursor: pointer;
      text-align: left;
      transition: color 0.15s, background 0.15s;
      white-space: nowrap;
    }

    button:hover {
      color: #e2e8f0;
      background: rgba(255, 255, 255, 0.05);
    }

    button.active {
      color: #2dd4bf;
      background: rgba(45, 212, 191, 0.1);
      border-right: 2px solid #2dd4bf;
    }

    .icon {
      font-size: 1.125rem;
      flex-shrink: 0;
      width: 1.5rem;
      text-align: center;
    }

    .label {
      overflow: hidden;
    }

    nav.collapsed .label {
      display: none;
    }
  `;

  @property() activePanel: PanelId = 'fleet';
  @property({ type: Boolean }) collapsed = false;

  private _navigate(id: PanelId) {
    this.dispatchEvent(new CustomEvent('navigate', { detail: id, bubbles: true, composed: true }));
  }

  render() {
    return html`
      <nav class=${this.collapsed ? 'collapsed' : 'expanded'}>
        ${NAV_ITEMS.map(
          (item) => html`
            <button
              class=${item.id === this.activePanel ? 'active' : ''}
              @click=${() => this._navigate(item.id)}
              title=${item.label}
            >
              <span class="icon">${item.icon}</span>
              <span class="label">${item.label}</span>
            </button>
          `,
        )}
      </nav>
    `;
  }
}
