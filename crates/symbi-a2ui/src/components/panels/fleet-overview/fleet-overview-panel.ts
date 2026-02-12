import { LitElement, html, css } from 'lit';
import { customElement, state } from 'lit/decorators.js';
import { listAgents, getAgentStatus } from '../../../api/agents.js';
import { listSchedules } from '../../../api/schedules.js';
import { getHealth, getSchedulerHealth, getMetrics } from '../../../api/system.js';
import type {
  AgentStatusResponse,
  ScheduleSummary,
  HealthResponse,
  SchedulerHealthResponse,
} from '../../../api/types.js';

@customElement('fleet-overview-panel')
export class FleetOverviewPanel extends LitElement {
  static styles = css`
    :host { display: block; }

    h2 {
      font-size: 1.25rem;
      font-weight: 600;
      color: #e2e8f0;
      margin: 0 0 1rem;
    }

    .grid {
      display: grid;
      gap: 1rem;
    }

    .top-row {
      display: grid;
      grid-template-columns: 1fr;
      gap: 1rem;
    }

    .bottom-row {
      display: grid;
      grid-template-columns: 1fr 1fr;
      gap: 1rem;
    }

    @media (max-width: 768px) {
      .bottom-row {
        grid-template-columns: 1fr;
      }
    }

    .section-title {
      font-size: 0.8125rem;
      font-weight: 500;
      color: #64748b;
      margin: 0 0 0.5rem;
      text-transform: uppercase;
      letter-spacing: 0.05em;
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
  `;

  @state() private _agents: AgentStatusResponse[] = [];
  @state() private _schedules: ScheduleSummary[] = [];
  @state() private _health: HealthResponse | null = null;
  @state() private _schedulerHealth: SchedulerHealthResponse | null = null;
  @state() private _metrics: Record<string, unknown> | null = null;
  @state() private _loading = true;
  @state() private _error = '';
  @state() private _searchText = '';

  private _interval?: ReturnType<typeof setInterval>;

  connectedCallback() {
    super.connectedCallback();
    this._fetchAll();
    this._interval = setInterval(() => this._fetchAll(), 10_000);
  }

  disconnectedCallback() {
    super.disconnectedCallback();
    if (this._interval) clearInterval(this._interval);
  }

  private async _fetchAll() {
    try {
      const [agentIds, schedules, health, schedulerHealth, metrics] = await Promise.all([
        listAgents(),
        listSchedules(),
        getHealth(),
        getSchedulerHealth(),
        getMetrics(),
      ]);

      const agentStatuses = await Promise.all(
        agentIds.map((id) => getAgentStatus(id).catch(() => null)),
      );

      this._agents = agentStatuses.filter((a): a is AgentStatusResponse => a !== null);
      this._schedules = schedules;
      this._health = health;
      this._schedulerHealth = schedulerHealth;
      this._metrics = metrics;
      this._loading = false;
      this._error = '';
    } catch (e) {
      this._error = e instanceof Error ? e.message : 'Failed to fetch fleet data';
      this._loading = false;
    }
  }

  private _onSearch(e: CustomEvent<string>) {
    this._searchText = e.detail;
  }

  render() {
    if (this._loading) {
      return html`<loading-spinner label="Loading fleet data..."></loading-spinner>`;
    }

    return html`
      <h2>Agent Fleet Overview</h2>
      ${this._error ? html`<div class="error-banner">${this._error}</div>` : ''}

      <div class="grid">
        <div class="top-row">
          <system-health-bar
            .health=${this._health}
            .schedulerHealth=${this._schedulerHealth}
            .metrics=${this._metrics}
            .agentCount=${this._agents.length}
            .scheduleCount=${this._schedules.length}
          ></system-health-bar>
        </div>

        <div class="bottom-row">
          <div>
            <p class="section-title">Agents</p>
            <agent-list
              .agents=${this._agents}
              .searchText=${this._searchText}
              @search=${this._onSearch}
            ></agent-list>
          </div>
          <div>
            <p class="section-title">Schedules</p>
            <schedule-summary .schedules=${this._schedules}></schedule-summary>
          </div>
        </div>
      </div>
    `;
  }
}
