import { LitElement, html, css } from 'lit';
import { customElement, property, state } from 'lit/decorators.js';

export interface Column<T = Record<string, unknown>> {
  key: string;
  label: string;
  sortable?: boolean;
  render?: (row: T) => unknown;
}

@customElement('data-table')
export class DataTable extends LitElement {
  static styles = css`
    :host {
      display: block;
      overflow-x: auto;
    }

    table {
      width: 100%;
      border-collapse: collapse;
      font-size: 0.8125rem;
    }

    thead th {
      text-align: left;
      padding: 0.5rem 0.75rem;
      color: #64748b;
      font-weight: 500;
      border-bottom: 1px solid rgba(255, 255, 255, 0.1);
      white-space: nowrap;
      user-select: none;
    }

    thead th.sortable {
      cursor: pointer;
    }

    thead th.sortable:hover {
      color: #e2e8f0;
    }

    .sort-icon {
      margin-left: 0.25rem;
      font-size: 0.625rem;
    }

    tbody td {
      padding: 0.5rem 0.75rem;
      color: #e2e8f0;
      border-bottom: 1px solid rgba(255, 255, 255, 0.05);
    }

    tbody tr:hover {
      background: rgba(255, 255, 255, 0.03);
    }

    .no-data {
      text-align: center;
      padding: 2rem;
      color: #64748b;
    }
  `;

  @property({ attribute: false }) columns: Column[] = [];
  @property({ attribute: false }) rows: Record<string, unknown>[] = [];
  @property() filterText = '';

  @state() private _sortKey = '';
  @state() private _sortAsc = true;

  private _toggleSort(key: string) {
    if (this._sortKey === key) {
      this._sortAsc = !this._sortAsc;
    } else {
      this._sortKey = key;
      this._sortAsc = true;
    }
  }

  private _getFilteredSorted(): Record<string, unknown>[] {
    let data = [...this.rows];

    if (this.filterText) {
      const lower = this.filterText.toLowerCase();
      data = data.filter((row) =>
        Object.values(row).some((v) => String(v).toLowerCase().includes(lower)),
      );
    }

    if (this._sortKey) {
      data.sort((a, b) => {
        const av = a[this._sortKey];
        const bv = b[this._sortKey];
        const cmp = String(av ?? '').localeCompare(String(bv ?? ''), undefined, { numeric: true });
        return this._sortAsc ? cmp : -cmp;
      });
    }

    return data;
  }

  render() {
    const data = this._getFilteredSorted();

    return html`
      <table>
        <thead>
          <tr>
            ${this.columns.map(
              (col) => html`
                <th
                  class=${col.sortable ? 'sortable' : ''}
                  @click=${col.sortable ? () => this._toggleSort(col.key) : undefined}
                >
                  ${col.label}
                  ${col.sortable && this._sortKey === col.key
                    ? html`<span class="sort-icon">${this._sortAsc ? '▲' : '▼'}</span>`
                    : ''}
                </th>
              `,
            )}
          </tr>
        </thead>
        <tbody>
          ${data.length === 0
            ? html`<tr><td class="no-data" colspan=${this.columns.length}>No data</td></tr>`
            : data.map(
                (row) => html`
                  <tr>
                    ${this.columns.map((col) => {
                      const cell = col.render ? col.render(row) : row[col.key];
                      return html`<td>${cell ?? '—'}</td>`;
                    })}
                  </tr>
                `,
              )}
        </tbody>
      </table>
    `;
  }
}
