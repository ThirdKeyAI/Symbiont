import { LitElement, html, css } from 'lit';
import { customElement, property } from 'lit/decorators.js';
import type { UnifiedAuditEntry } from '../../../api/types.js';
import type { BadgeVariant } from '../../shared/status-badge.js';

function statusToVariant(status: string): BadgeVariant {
  const s = status.toLowerCase();
  if (s === 'success' || s === 'completed' || s === 'succeeded') return 'healthy';
  if (s === 'failed' || s === 'error') return 'error';
  if (s === 'running' || s === 'in_progress' || s === 'pending') return 'warning';
  return 'neutral';
}

function sourceIcon(source: string): string {
  switch (source) {
    case 'agent': return '⬡';
    case 'schedule': return '◷';
    case 'channel': return '◆';
    case 'inter_agent': return '⇄';
    default: return '○';
  }
}

@customElement('audit-entry-row')
export class AuditEntryRow extends LitElement {
  static styles = css`
    :host { display: block; }

    .row {
      display: flex;
      align-items: flex-start;
      gap: 0.75rem;
      padding: 0.625rem 0.75rem;
      border-radius: 0.375rem;
      border: 1px solid rgba(255, 255, 255, 0.05);
      background: rgba(255, 255, 255, 0.02);
      transition: background 0.15s;
    }

    .row:hover {
      background: rgba(255, 255, 255, 0.04);
    }

    .icon {
      flex-shrink: 0;
      width: 1.5rem;
      height: 1.5rem;
      display: flex;
      align-items: center;
      justify-content: center;
      border-radius: 50%;
      background: rgba(45, 212, 191, 0.1);
      color: #2dd4bf;
      font-size: 0.75rem;
    }

    .icon.agent { background: rgba(45, 212, 191, 0.1); color: #2dd4bf; }
    .icon.schedule { background: rgba(56, 189, 248, 0.1); color: #38bdf8; }
    .icon.channel { background: rgba(199, 146, 234, 0.1); color: #c792ea; }
    .icon.inter_agent { background: rgba(251, 191, 36, 0.1); color: #fbbf24; }

    .inter-agent-flow {
      display: flex;
      align-items: center;
      gap: 0.375rem;
      font-size: 0.8125rem;
      color: #e2e8f0;
    }

    .inter-agent-flow .arrow {
      color: #64748b;
    }

    .inter-agent-flow .policy-decision {
      font-size: 0.6875rem;
      font-family: 'Roboto Mono', monospace;
      padding: 0.125rem 0.375rem;
      border-radius: 0.25rem;
      margin-left: 0.5rem;
    }

    .inter-agent-flow .policy-decision.allow {
      background: rgba(34, 197, 94, 0.15);
      color: #22c55e;
    }

    .inter-agent-flow .policy-decision.deny {
      background: rgba(239, 68, 68, 0.15);
      color: #ef4444;
    }

    .body {
      flex: 1;
      min-width: 0;
    }

    .top-line {
      display: flex;
      align-items: center;
      gap: 0.5rem;
      margin-bottom: 0.25rem;
    }

    .source-name {
      font-size: 0.8125rem;
      font-weight: 500;
      color: #e2e8f0;
      overflow: hidden;
      text-overflow: ellipsis;
      white-space: nowrap;
    }

    .event-type {
      font-size: 0.6875rem;
      color: #64748b;
      font-family: 'Roboto Mono', monospace;
    }

    .details {
      font-size: 0.75rem;
      color: #64748b;
      margin-top: 0.25rem;
    }

    .details code {
      background: rgba(255, 255, 255, 0.05);
      padding: 0.125rem 0.25rem;
      border-radius: 0.25rem;
      font-family: 'Roboto Mono', monospace;
      font-size: 0.6875rem;
    }

    .right {
      display: flex;
      flex-direction: column;
      align-items: flex-end;
      gap: 0.25rem;
      flex-shrink: 0;
    }
  `;

  @property({ attribute: false }) entry!: UnifiedAuditEntry;

  private _isInterAgentComm(e: UnifiedAuditEntry): boolean {
    return e.eventType === 'inter_agent_comm';
  }

  private _renderInterAgentBody(e: UnifiedAuditEntry) {
    const sender = String(e.details['sender'] ?? e.sourceName);
    const recipient = String(e.details['recipient'] ?? 'unknown');
    const policyDecision = String(e.details['policy_decision'] ?? '');
    const msgType = String(e.details['message_type'] ?? '');
    const toolName = e.details['tool_name'] ? String(e.details['tool_name']) : null;
    const decisionClass = policyDecision.toLowerCase() === 'allow' ? 'allow' : 'deny';

    return html`
      <div class="inter-agent-flow">
        <span>${sender}</span>
        <span class="arrow">&rarr;</span>
        <span>${recipient}</span>
        <span class="policy-decision ${decisionClass}">${policyDecision}</span>
      </div>
      <div class="details">
        <span>type: <code>${msgType}</code> </span>
        ${toolName ? html`<span>tool: <code>${toolName}</code> </span>` : ''}
      </div>
    `;
  }

  render() {
    const e = this.entry;
    const variant = statusToVariant(e.status);
    const isInterAgent = this._isInterAgentComm(e);
    const icon = isInterAgent ? sourceIcon('inter_agent') : sourceIcon(e.source);
    const iconClass = isInterAgent ? 'inter_agent' : e.source;
    const hasDetails = Object.keys(e.details).length > 0;

    return html`
      <div class="row">
        <div class="icon ${iconClass}">${icon}</div>
        <div class="body">
          ${isInterAgent
            ? this._renderInterAgentBody(e)
            : html`
              <div class="top-line">
                <span class="source-name">${e.sourceName}</span>
                <span class="event-type">${e.eventType}</span>
              </div>
              ${hasDetails ? html`
                <div class="details">
                  ${Object.entries(e.details).map(([k, v]) =>
                    html`<span>${k}: <code>${String(v)}</code> </span>`,
                  )}
                </div>
              ` : ''}
            `}
        </div>
        <div class="right">
          <status-badge .variant=${variant} .label=${e.status}></status-badge>
          <time-ago .datetime=${e.timestamp}></time-ago>
        </div>
      </div>
    `;
  }
}
