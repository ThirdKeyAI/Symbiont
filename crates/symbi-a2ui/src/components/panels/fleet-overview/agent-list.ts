import { LitElement, html, css } from 'lit';
import { customElement, property } from 'lit/decorators.js';
import type { AgentStatusResponse } from '../../../api/types.js';

@customElement('agent-list')
export class AgentList extends LitElement {
  static styles = css`
    :host { display: block; }

    .wrapper {
      background: #111827;
      border: 1px solid rgba(255, 255, 255, 0.1);
      border-radius: 0.75rem;
      padding: 0.75rem;
    }

    .search-row {
      margin-bottom: 0.75rem;
    }

    .cards {
      display: flex;
      flex-direction: column;
      gap: 0.5rem;
      max-height: 32rem;
      overflow-y: auto;
    }
  `;

  @property({ attribute: false }) agents: AgentStatusResponse[] = [];
  @property() searchText = '';

  private _filtered(): AgentStatusResponse[] {
    if (!this.searchText) return this.agents;
    const q = this.searchText.toLowerCase();
    return this.agents.filter(
      (a) =>
        a.agent_id.toLowerCase().includes(q) ||
        a.state.toLowerCase().includes(q),
    );
  }

  private _onSearch(e: CustomEvent<string>) {
    this.dispatchEvent(new CustomEvent('search', { detail: e.detail, bubbles: true, composed: true }));
  }

  render() {
    const filtered = this._filtered();

    return html`
      <div class="wrapper">
        <div class="search-row">
          <search-input
            placeholder="Filter agents..."
            .value=${this.searchText}
            @search=${this._onSearch}
          ></search-input>
        </div>
        ${filtered.length === 0
          ? html`<empty-state
              icon="⬡"
              title="No agents found"
              description=${this.searchText ? 'Try a different search term' : 'No agents registered yet'}
            ></empty-state>`
          : html`
            <div class="cards">
              ${filtered.map((a) => html`<agent-card .agent=${a}></agent-card>`)}
            </div>
          `}
      </div>
    `;
  }
}
