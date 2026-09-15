import { LitElement, html, css } from 'lit';
import { customElement, state } from 'lit/decorators.js';
import { inspectCapacity, measureWorker, type Capacity, type Measurement, type Worker } from '../../../api/capacity.js';

const memory = (value: number | null): string => value === null ? 'Unknown' : `${(value / 1048576).toLocaleString(undefined, { maximumFractionDigits: 2 })} MiB`;
const cpus = (value: number | null): string => value === null ? 'Unknown' : `${(value / 1e9).toLocaleString(undefined, { maximumFractionDigits: 3 })} CPUs`;
const time = (value: number): string => new Date(value).toLocaleString();
interface Sample { loading?: boolean; value?: Measurement; error?: string }

@customElement('capacity-panel')
export class CapacityPanel extends LitElement {
  static styles = css`
    :host { display:block; padding:1.5rem; color:#e2e8f0; font:14px/1.5 system-ui,sans-serif; }
    header { display:flex; justify-content:space-between; align-items:center; gap:1rem; flex-wrap:wrap; }
    h2,h3 { margin:0 0 .5rem; } h3 { font-size:1rem; } p { color:#b6c2d3; }
    section { margin:1.2rem 0; padding:1rem; border:1px solid #334155; border-radius:.5rem; background:#111827; }
    .metrics { display:grid; grid-template-columns:repeat(auto-fit,minmax(12rem,1fr)); gap:1rem; }
    .metric { padding:1rem; border-radius:.4rem; background:#0b1324; }
    .metric strong { display:block; font-size:1.25rem; margin:.2rem 0; }
    .metric span { color:#a4b3c7; font-size:.85rem; }
    .attention,.error { padding:.8rem; background:#fbbf2410; border-left:3px solid #fbbf24; color:#fde68a; }
    .error { border-color:#fb7185; color:#fda4af; }
    button { background:#12343b; color:#99f6e4; border:1px solid #39717a; border-radius:.4rem; padding:.5rem .8rem; cursor:pointer; font:inherit; }
    button:disabled { opacity:.5; cursor:wait; } button:focus-visible { outline:2px solid #7dd3fc; outline-offset:2px; }
    code { font-size:.8rem; overflow-wrap:anywhere; }
    .table-wrap { overflow-x:auto; } table { width:100%; border-collapse:collapse; }
    th,td { padding:.7rem; text-align:left; vertical-align:top; border-bottom:1px solid #334155; }
    td p { margin:.3rem 0; } .sample { min-width:15rem; } .phase { text-transform:capitalize; }
  `;
  @state() private _snapshot: Capacity | null = null;
  @state() private _loading = false;
  @state() private _error = '';
  @state() private _samples: Record<string, Sample> = {};
  private _generation = 0;

  connectedCallback() { super.connectedCallback(); void this._refresh(); }
  disconnectedCallback() { super.disconnectedCallback(); ++this._generation; }

  private async _refresh() {
    const generation = ++this._generation;
    this._snapshot = null; this._samples = {}; this._error = ''; this._loading = true;
    try {
      const snapshot = await inspectCapacity();
      if (generation === this._generation) this._snapshot = snapshot;
    } catch (error) {
      if (generation === this._generation) this._error = error instanceof Error ? error.message : 'Capacity unavailable';
    } finally { if (generation === this._generation) this._loading = false; }
  }

  private async _measure(worker: Worker) {
    const generation = this._generation;
    this._samples = { ...this._samples, [worker.lease]: { loading:true } };
    try {
      const value = await measureWorker(worker.lease);
      if (generation === this._generation) this._samples = { ...this._samples, [worker.lease]: { value } };
    } catch (error) {
      if (generation === this._generation) this._samples = { ...this._samples, [worker.lease]: {
        error: error instanceof Error ? error.message : 'Measurement unavailable',
      } };
    }
  }

  private _measurement(worker: Worker) {
    const sample = this._samples[worker.lease];
    const value = sample?.value;
    return html`<div class="sample">
      <button ?disabled=${sample?.loading || worker.phase !== 'created'} @click=${() => this._measure(worker)}>
        ${sample?.loading ? 'Sampling…' : 'Measure usage'}</button>
      ${sample?.error ? html`<p class="error" role="alert">Usage unavailable: ${sample.error}</p>` : ''}
      ${value ? html`<p>Sampled ${time(value.observed_at_unix_ms)}</p>
        ${value.source === 'docker_cli' ? html`<p>CPU: ${value.cpu_percent ?? 'Unknown'}</p><p>Memory / limit: ${value.memory_usage ?? 'Unknown'}</p>
          <p>Memory follows Docker's cache-adjusted display.</p>`
          : html`<p>${value.source === 'vmm_cgroup' ? 'Cgroup memory' : 'Resident memory'}: ${memory(value.memory_bytes)}</p>
            <p>CPU time: ${value.cpu_time_micros === null ? 'Unknown' : `${(value.cpu_time_micros / 1e6).toLocaleString()} seconds`}</p>
            <p>${value.source === 'vmm_cgroup' ? 'VMM cgroup counters' : 'VMM process counters'}; guest application usage is separate.</p>`}
      ` : !sample?.loading && !sample?.error ? html`<p>${worker.phase === 'created' ? 'Not sampled.' : 'Creation or cleanup is uncertain; usage is unknown.'}</p>` : ''}
    </div>`;
  }

  render() {
    const s = this._snapshot;
    return html`<header><h2>Worker capacity</h2><button ?disabled=${this._loading} @click=${this._refresh}>
      ${this._loading ? 'Refreshing…' : 'Refresh capacity'}</button></header>
      <p>Shared worker reservations across execution routes using this supervisor. Administrative access is required.</p>
      ${this._error ? html`<p class="error" role="alert">Capacity unavailable: ${this._error}. No empty pool or zero usage is assumed.</p>` : ''}
      <div aria-live="polite">${this._loading ? html`<p>Reading the running supervisor…</p>` : ''}</div>
      ${s ? html`<section aria-label="Shared capacity snapshot">
        <h3>Reserved capacity</h3><p>Observed ${time(s.observed_at_unix_ms)}. Refresh to obtain a new snapshot.</p>
        <div class="metrics">
          <div class="metric"><span>Worker slots reserved</span><strong>${s.reserved.workers} / ${s.limits.max_workers}</strong><span>${s.available.workers} available</span></div>
          <div class="metric"><span>Memory reserved</span><strong>${memory(s.reserved.memory_bytes)}</strong><span>${memory(s.available.memory_bytes)} available of ${memory(s.limits.memory_bytes)}</span></div>
          <div class="metric"><span>CPU reserved</span><strong>${cpus(s.reserved.cpu_nanos)}</strong><span>${cpus(s.available.cpu_nanos)} available of ${cpus(s.limits.cpu_nanos)}</span></div>
        </div>
        ${s.admission_blocked ? html`<p class="attention">Shared capacity currently blocks new workers. Existing route limits can also refuse admission.</p>` : ''}
        ${s.unknown_resource_leases ? html`<p class="attention">${s.unknown_resource_leases} retained leases lack resource metadata. Their memory and CPU charges are unknown and continue to block admission.</p>` : ''}
        <p>Reservations remain charged until removal is confirmed. Low measured usage does not release a reservation.</p>
        <p>Pool: <code>${s.state_dir}</code>. Separate supervisor directories are separate pools.</p>
      </section>
      <section><h3>Retained workers</h3><p>Lease state describes retained supervision records, not proof that a process is currently running. Usage is sampled separately on request.</p>
        ${s.workers.length ? html`<div class="table-wrap"><table><thead><tr><th>Lease / backend</th><th>Lease state</th><th>Reserved resources</th><th>Measured usage</th></tr></thead>
          <tbody>${s.workers.map(worker => html`<tr><td><code>${worker.lease}</code><p>${worker.backend}</p></td>
            <td class="phase">${worker.phase}</td><td>${memory(worker.resources?.memory_bytes ?? null)}<br>${cpus(worker.resources?.cpu_nanos ?? null)}</td>
            <td>${this._measurement(worker)}</td></tr>`)}</tbody></table></div>`
          : html`<p>No retained workers in this snapshot.</p>`}
      </section>` : ''}`;
  }
}
