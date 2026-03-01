import { LitElement, html, css } from 'lit';
import { customElement, property, state } from 'lit/decorators.js';
import { unsafeHTML } from 'lit/directives/unsafe-html.js';
import { marked } from 'marked';

export interface ToolTrace {
  call_id: string;
  tool_name: string;
  arguments: string;
  result?: string;
  is_error?: boolean;
}

export interface PolicyTrace {
  action: string;
  decision: string;
  reason: string;
}

export interface ChatMessageData {
  id: string;
  role: 'user' | 'assistant';
  content: string;
  toolTraces?: ToolTrace[];
  policyTraces?: PolicyTrace[];
  tokens?: number;
}

@customElement('chat-message')
export class ChatMessage extends LitElement {
  static styles = css`
    :host {
      display: block;
      margin-bottom: 1rem;
    }

    .bubble {
      max-width: 85%;
      padding: 0.75rem 1rem;
      border-radius: 0.75rem;
      font-size: 0.875rem;
      line-height: 1.5;
      word-break: break-word;
    }

    .bubble.plain {
      white-space: pre-wrap;
    }

    /* Markdown content styles */
    .bubble h1, .bubble h2, .bubble h3, .bubble h4 {
      margin: 0.75rem 0 0.375rem 0;
      color: #f1f5f9;
    }

    .bubble h1 { font-size: 1.25rem; }
    .bubble h2 { font-size: 1.1rem; }
    .bubble h3 { font-size: 1rem; }

    .bubble h1:first-child, .bubble h2:first-child, .bubble h3:first-child {
      margin-top: 0;
    }

    .bubble p {
      margin: 0.375rem 0;
    }

    .bubble p:first-child { margin-top: 0; }
    .bubble p:last-child { margin-bottom: 0; }

    .bubble ul, .bubble ol {
      margin: 0.375rem 0;
      padding-left: 1.5rem;
    }

    .bubble li {
      margin: 0.125rem 0;
    }

    .bubble strong {
      color: #f1f5f9;
      font-weight: 600;
    }

    .bubble code {
      background: rgba(255, 255, 255, 0.08);
      padding: 0.125rem 0.375rem;
      border-radius: 0.25rem;
      font-family: 'Roboto Mono', monospace;
      font-size: 0.8125rem;
    }

    .bubble pre {
      background: rgba(0, 0, 0, 0.3);
      border: 1px solid rgba(255, 255, 255, 0.08);
      border-radius: 0.5rem;
      padding: 0.75rem;
      overflow-x: auto;
      margin: 0.5rem 0;
    }

    .bubble pre code {
      background: none;
      padding: 0;
      font-size: 0.8125rem;
    }

    .bubble hr {
      border: none;
      border-top: 1px solid rgba(255, 255, 255, 0.1);
      margin: 0.75rem 0;
    }

    .bubble a {
      color: #38bdf8;
      text-decoration: none;
    }

    .bubble a:hover {
      text-decoration: underline;
    }

    .bubble blockquote {
      border-left: 3px solid rgba(45, 212, 191, 0.4);
      margin: 0.5rem 0;
      padding: 0.25rem 0.75rem;
      color: #94a3b8;
    }

    .user-row {
      display: flex;
      justify-content: flex-end;
    }

    .user-row .bubble {
      background: rgba(45, 212, 191, 0.15);
      color: #e2e8f0;
      border: 1px solid rgba(45, 212, 191, 0.2);
    }

    .assistant-row {
      display: flex;
      justify-content: flex-start;
      flex-direction: column;
      gap: 0.5rem;
    }

    .assistant-row .bubble {
      background: #111827;
      color: #e2e8f0;
      border: 1px solid rgba(255, 255, 255, 0.1);
    }

    .tool-trace {
      background: rgba(255, 255, 255, 0.03);
      border: 1px solid rgba(255, 255, 255, 0.08);
      border-radius: 0.5rem;
      margin-top: 0.25rem;
      font-size: 0.8125rem;
      overflow: hidden;
    }

    .tool-header {
      display: flex;
      align-items: center;
      gap: 0.5rem;
      padding: 0.375rem 0.75rem;
      cursor: pointer;
      color: #64748b;
      user-select: none;
    }

    .tool-header:hover {
      color: #e2e8f0;
    }

    .tool-name {
      font-family: 'Roboto Mono', monospace;
      font-size: 0.75rem;
      color: #38bdf8;
    }

    .tool-body {
      padding: 0.5rem 0.75rem;
      border-top: 1px solid rgba(255, 255, 255, 0.05);
      font-family: 'Roboto Mono', monospace;
      font-size: 0.75rem;
      color: #cbd5e1;
      max-height: 12rem;
      overflow-y: auto;
      white-space: pre-wrap;
      word-break: break-all;
    }

    .tool-error {
      color: #fca5a5;
    }

    .policy-badge {
      display: inline-flex;
      align-items: center;
      gap: 0.25rem;
      padding: 0.125rem 0.5rem;
      border-radius: 9999px;
      font-size: 0.6875rem;
      font-weight: 600;
    }

    .policy-allow {
      background: rgba(34, 197, 94, 0.15);
      color: #22c55e;
    }

    .policy-deny {
      background: rgba(239, 68, 68, 0.15);
      color: #ef4444;
    }

    .meta-footer {
      font-size: 0.6875rem;
      color: #475569;
      padding-top: 0.25rem;
    }

    .chevron {
      font-size: 0.625rem;
      transition: transform 0.15s;
    }

    .chevron.open {
      transform: rotate(90deg);
    }
  `;

  @property({ attribute: false }) data!: ChatMessageData;

  @state() private _expandedTools = new Set<string>();

  private _toggleTool(callId: string) {
    if (this._expandedTools.has(callId)) {
      this._expandedTools.delete(callId);
    } else {
      this._expandedTools.add(callId);
    }
    this.requestUpdate();
  }

  private _renderMarkdown(text: string): string {
    return marked.parse(text, { async: false }) as string;
  }

  render() {
    if (this.data.role === 'user') {
      return html`
        <div class="user-row">
          <div class="bubble plain">${this.data.content}</div>
        </div>
      `;
    }

    return html`
      <div class="assistant-row">
        ${this.data.policyTraces?.map(
          (p) => html`
            <span class="policy-badge ${p.decision === 'allow' ? 'policy-allow' : 'policy-deny'}">
              ${p.decision} — ${p.reason}
            </span>
          `,
        )}
        ${this.data.toolTraces?.map((t) => this._renderToolTrace(t))}
        <div class="bubble">${unsafeHTML(this._renderMarkdown(this.data.content))}</div>
        ${this.data.tokens
          ? html`<div class="meta-footer">${this.data.tokens} tokens</div>`
          : ''}
      </div>
    `;
  }

  private _renderToolTrace(t: ToolTrace) {
    const expanded = this._expandedTools.has(t.call_id);
    return html`
      <div class="tool-trace">
        <div class="tool-header" @click=${() => this._toggleTool(t.call_id)}>
          <span class="chevron ${expanded ? 'open' : ''}">&#9654;</span>
          <span class="tool-name">${t.tool_name}</span>
          ${t.is_error ? html`<span style="color:#ef4444">error</span>` : ''}
        </div>
        ${expanded
          ? html`
              <div class="tool-body">
                <div style="color:#64748b">args:</div>
                ${t.arguments}
                ${t.result != null
                  ? html`
                      <div style="margin-top:0.5rem;color:#64748b">result:</div>
                      <div class="${t.is_error ? 'tool-error' : ''}">${t.result}</div>
                    `
                  : ''}
              </div>
            `
          : ''}
      </div>
    `;
  }
}
