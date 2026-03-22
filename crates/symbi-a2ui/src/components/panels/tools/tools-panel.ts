import { LitElement, html, css } from 'lit';
import { customElement, state } from 'lit/decorators.js';
import { listTools, type ToolSummary } from '../../../api/tools.js';

@customElement('tools-panel')
export class ToolsPanel extends LitElement {
  @state() tools: ToolSummary[] = [];
  @state() loading = true;

  static styles = css`
    :host {
      display: block;
      padding: 1.5rem;
    }

    .tools-table {
      width: 100%;
      border-collapse: collapse;
    }

    .tools-table th {
      text-align: left;
      padding: 0.75rem;
      color: #94a3b8;
      font-size: 0.75rem;
      text-transform: uppercase;
      border-bottom: 1px solid #1e293b;
    }

    .tools-table td {
      padding: 0.75rem;
      border-bottom: 1px solid #1e293b;
    }

    .badge {
      display: inline-block;
      padding: 0.125rem 0.5rem;
      border-radius: 9999px;
      font-size: 0.75rem;
      font-weight: 500;
    }

    .badge-low {
      background: rgba(34, 197, 94, 0.1);
      color: #22c55e;
    }

    .badge-medium {
      background: rgba(234, 179, 8, 0.1);
      color: #eab308;
    }

    .badge-high {
      background: rgba(239, 68, 68, 0.1);
      color: #ef4444;
    }

    .badge-oneshot {
      background: rgba(59, 130, 246, 0.1);
      color: #3b82f6;
    }

    .badge-session {
      background: rgba(168, 85, 247, 0.1);
      color: #a855f7;
    }

    .badge-browser {
      background: rgba(20, 184, 166, 0.1);
      color: #14b8a6;
    }

    .tool-name {
      font-weight: 600;
      color: #e2e8f0;
    }

    .tool-binary {
      color: #94a3b8;
      font-family: monospace;
      font-size: 0.875rem;
    }

    .empty-state {
      text-align: center;
      padding: 3rem;
      color: #64748b;
    }

    h2 {
      color: #e2e8f0;
      margin-bottom: 1rem;
      font-size: 1.25rem;
    }

    .subtitle {
      color: #94a3b8;
      font-size: 0.875rem;
      margin-bottom: 1.5rem;
    }
  `;

  connectedCallback() {
    super.connectedCallback();
    this.loadTools();
  }

  async loadTools() {
    try {
      this.tools = await listTools();
    } catch (e) {
      console.error('Failed to load tools:', e);
    }
    this.loading = false;
  }

  render() {
    return html`
      <h2>ToolClad Manifests</h2>
      <p class="subtitle">
        Declarative tool interface contracts loaded from tools/
      </p>
      ${this.loading
        ? html`<loading-spinner></loading-spinner>`
        : this.tools.length === 0
          ? html`
              <div class="empty-state">
                <p>No ToolClad manifests loaded.</p>
                <p style="font-size: 0.75rem; margin-top: 0.5rem;">
                  Place .clad.toml files in the tools/ directory and restart the
                  runtime.
                </p>
              </div>
            `
          : html`
              <table class="tools-table">
                <thead>
                  <tr>
                    <th>Tool</th>
                    <th>Mode</th>
                    <th>Binary</th>
                    <th>Risk</th>
                    <th>Cedar Resource</th>
                  </tr>
                </thead>
                <tbody>
                  ${this.tools.map(
                    (tool) => html`
                      <tr>
                        <td>
                          <div class="tool-name">${tool.name}</div>
                          <div style="color: #64748b; font-size: 0.75rem;">
                            ${tool.description}
                          </div>
                        </td>
                        <td>
                          <span class="badge badge-${tool.mode}"
                            >${tool.mode}</span
                          >
                        </td>
                        <td>
                          <span class="tool-binary"
                            >${tool.binary || '\u2014'}</span
                          >
                        </td>
                        <td>
                          <span class="badge badge-${tool.risk_tier}"
                            >${tool.risk_tier}</span
                          >
                        </td>
                        <td style="color: #94a3b8; font-size: 0.875rem;">
                          ${tool.cedar_resource || '\u2014'}
                        </td>
                      </tr>
                    `,
                  )}
                </tbody>
              </table>
            `}
    `;
  }
}
