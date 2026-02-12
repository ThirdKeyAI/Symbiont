import { LitElement, html, css } from 'lit';
import { customElement, state } from 'lit/decorators.js';
import { listAgents, getAgentHistory } from '../../../api/agents.js';
import { listSchedules, getScheduleHistory } from '../../../api/schedules.js';
import { listChannels, getChannelAudit } from '../../../api/channels.js';
import type { UnifiedAuditEntry } from '../../../api/types.js';

@customElement('audit-trail-panel')
export class AuditTrailPanel extends LitElement {
  static styles = css`
    :host { display: block; }

    h2 {
      font-size: 1.25rem;
      font-weight: 600;
      color: #e2e8f0;
      margin: 0 0 1rem;
    }

    .error-banner {
      background: rgba(239, 68, 68, 0.1);
      border: 1px solid rgba(239, 68, 68, 0.3);
      border-radius: 0.5rem;
      padding: 0.75rem 1rem;
      color: #fca5a5;
      font-size: 0.8125rem;
      margin-bottom: 1rem;
    }

    .layout {
      display: flex;
      flex-direction: column;
      gap: 1rem;
    }
  `;

  @state() private _entries: UnifiedAuditEntry[] = [];
  @state() private _loading = true;
  @state() private _error = '';
  @state() private _filterAgent = '';
  @state() private _filterType = '';
  @state() private _filterTimeRange = '';

  private _interval?: ReturnType<typeof setInterval>;

  connectedCallback() {
    super.connectedCallback();
    this._fetchAll();
    this._interval = setInterval(() => this._fetchAll(), 15_000);
  }

  disconnectedCallback() {
    super.disconnectedCallback();
    if (this._interval) clearInterval(this._interval);
  }

  private async _fetchAll() {
    try {
      const [agentIds, schedules, channels] = await Promise.all([
        listAgents(),
        listSchedules(),
        listChannels(),
      ]);

      const entries: UnifiedAuditEntry[] = [];

      // Fetch agent histories
      const agentHistories = await Promise.all(
        agentIds.map((id) =>
          getAgentHistory(id)
            .then((resp) =>
              resp.history.map((h) => ({
                id: h.execution_id,
                timestamp: h.timestamp,
                source: 'agent' as const,
                sourceId: id,
                sourceName: id,
                eventType: 'execution',
                status: h.status,
                details: {} as Record<string, unknown>,
              })),
            )
            .catch(() => []),
        ),
      );
      entries.push(...agentHistories.flat());

      // Fetch schedule histories
      const schedHistories = await Promise.all(
        schedules.map((s) =>
          getScheduleHistory(s.job_id)
            .then((resp) =>
              resp.history.map((h) => ({
                id: h.run_id,
                timestamp: h.started_at,
                source: 'schedule' as const,
                sourceId: s.job_id,
                sourceName: s.name,
                eventType: 'scheduled_run',
                status: h.status,
                details: {
                  ...(h.error ? { error: h.error } : {}),
                  ...(h.execution_time_ms != null ? { execution_time_ms: h.execution_time_ms } : {}),
                } as Record<string, unknown>,
              })),
            )
            .catch(() => []),
        ),
      );
      entries.push(...schedHistories.flat());

      // Fetch channel audits
      const channelAudits = await Promise.all(
        channels.map((c) =>
          getChannelAudit(c.id)
            .then((resp) =>
              resp.entries.map((e, i) => ({
                id: `${c.id}-${i}`,
                timestamp: e.timestamp,
                source: 'channel' as const,
                sourceId: c.id,
                sourceName: c.name,
                eventType: e.event_type,
                status: e.event_type,
                details: e.details,
              })),
            )
            .catch(() => []),
        ),
      );
      entries.push(...channelAudits.flat());

      // Sort by timestamp descending
      entries.sort((a, b) => new Date(b.timestamp).getTime() - new Date(a.timestamp).getTime());

      this._entries = entries;
      this._loading = false;
      this._error = '';
    } catch (e) {
      this._error = e instanceof Error ? e.message : 'Failed to fetch audit data';
      this._loading = false;
    }
  }

  private _filteredEntries(): UnifiedAuditEntry[] {
    let data = this._entries;

    if (this._filterAgent) {
      const q = this._filterAgent.toLowerCase();
      data = data.filter(
        (e) =>
          e.sourceName.toLowerCase().includes(q) ||
          e.sourceId.toLowerCase().includes(q),
      );
    }

    if (this._filterType) {
      data = data.filter((e) => e.source === this._filterType);
    }

    if (this._filterTimeRange) {
      const now = Date.now();
      let cutoff = 0;
      switch (this._filterTimeRange) {
        case '1h': cutoff = now - 3_600_000; break;
        case '6h': cutoff = now - 21_600_000; break;
        case '24h': cutoff = now - 86_400_000; break;
        case '7d': cutoff = now - 604_800_000; break;
      }
      if (cutoff) {
        data = data.filter((e) => new Date(e.timestamp).getTime() >= cutoff);
      }
    }

    return data;
  }

  private _onFilterChange(e: CustomEvent) {
    const detail = e.detail as { agent?: string; type?: string; timeRange?: string };
    if (detail.agent !== undefined) this._filterAgent = detail.agent;
    if (detail.type !== undefined) this._filterType = detail.type;
    if (detail.timeRange !== undefined) this._filterTimeRange = detail.timeRange;
  }

  private _getUniqueAgents(): string[] {
    const set = new Set(this._entries.map((e) => e.sourceName));
    return [...set].sort();
  }

  render() {
    if (this._loading) {
      return html`<loading-spinner label="Loading audit trail..."></loading-spinner>`;
    }

    const filtered = this._filteredEntries();

    return html`
      <h2>Audit Trail Explorer</h2>
      ${this._error ? html`<div class="error-banner">${this._error}</div>` : ''}
      <div class="layout">
        <audit-filters
          .agents=${this._getUniqueAgents()}
          .filterAgent=${this._filterAgent}
          .filterType=${this._filterType}
          .filterTimeRange=${this._filterTimeRange}
          @filter-change=${this._onFilterChange}
        ></audit-filters>
        ${filtered.length === 0
          ? html`<empty-state
              icon="◈"
              title="No audit entries"
              description=${this._entries.length > 0 ? 'No entries match the current filters' : 'No execution history available yet'}
            ></empty-state>`
          : html`<audit-timeline .entries=${filtered}></audit-timeline>`}
      </div>
    `;
  }
}
