import { LitElement, html, css } from 'lit';
import { customElement, property } from 'lit/decorators.js';
import type { ChannelHealthResponse } from '../../../api/types.js';

@customElement('channel-compliance')
export class ChannelCompliance extends LitElement {
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

    .conn-indicator {
      width: 0.5rem;
      height: 0.5rem;
      border-radius: 50%;
      flex-shrink: 0;
    }

    .conn-indicator.connected { background: #22c55e; }
    .conn-indicator.disconnected { background: #ef4444; }

    .info {
      flex: 1;
      min-width: 0;
    }

    .channel-id {
      font-size: 0.8125rem;
      font-weight: 500;
      color: #e2e8f0;
      font-family: 'Roboto Mono', monospace;
      overflow: hidden;
      text-overflow: ellipsis;
      white-space: nowrap;
    }

    .channel-meta {
      font-size: 0.6875rem;
      color: #64748b;
      display: flex;
      gap: 0.75rem;
      margin-top: 0.125rem;
    }

    .right {
      display: flex;
      flex-direction: column;
      align-items: flex-end;
      gap: 0.25rem;
      flex-shrink: 0;
    }

    .uptime {
      font-size: 0.6875rem;
      color: #64748b;
      font-family: 'Roboto Mono', monospace;
    }
  `;

  @property({ attribute: false }) channels: ChannelHealthResponse[] = [];

  private _formatUptime(secs: number): string {
    const d = Math.floor(secs / 86400);
    const h = Math.floor((secs % 86400) / 3600);
    const m = Math.floor((secs % 3600) / 60);
    if (d > 0) return `${d}d ${h}h`;
    if (h > 0) return `${h}h ${m}m`;
    return `${m}m`;
  }

  render() {
    return html`
      <div class="card">
        <p class="title">Channel Health</p>
        ${this.channels.length === 0
          ? html`<empty-state icon="◆" title="No channels" description="No channel adapters registered"></empty-state>`
          : html`
            <div class="list">
              ${this.channels.map(
                (c) => html`
                  <div class="item">
                    <div class="conn-indicator ${c.connected ? 'connected' : 'disconnected'}"></div>
                    <div class="info">
                      <div class="channel-id">${c.id}</div>
                      <div class="channel-meta">
                        <span>${c.platform}</span>
                        ${c.workspace_name ? html`<span>${c.workspace_name}</span>` : ''}
                        <span>${c.channels_active} active</span>
                      </div>
                    </div>
                    <div class="right">
                      <status-badge
                        .variant=${c.connected ? 'healthy' : 'error'}
                        .label=${c.connected ? 'connected' : 'disconnected'}
                      ></status-badge>
                      <span class="uptime">↑ ${this._formatUptime(c.uptime_secs)}</span>
                      ${c.last_message_at ? html`<time-ago .datetime=${c.last_message_at}></time-ago>` : ''}
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
