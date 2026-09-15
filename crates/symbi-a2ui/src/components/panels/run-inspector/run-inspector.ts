import { LitElement, html, css, type PropertyValues } from 'lit';
import { customElement, property, state } from 'lit/decorators.js';
import { auditIdentity, inspectRun, type RunAuditReference, type RunView } from '../../../api/run-audit.js';

const readable = (value: unknown): string => typeof value === 'string' ? value : JSON.stringify(value, null, 2) ?? 'Not recorded';

@customElement('run-inspector')
export class RunInspector extends LitElement {
  static styles = css`
    dialog { width:min(64rem, calc(100vw - 3rem)); max-height:85vh; box-sizing:border-box;
      background:#111827; color:#e2e8f0; border:1px solid #475569; border-radius:.8rem;
      padding:1.5rem; font:14px/1.5 system-ui,sans-serif; }
    dialog::backdrop { background:#020617b8; }
    h2,h3 { margin:0 0 .6rem; } h3 { font-size:1rem; }
    header,.actions,.links { display:flex; gap:.7rem; align-items:center; flex-wrap:wrap; }
    header { justify-content:space-between; margin-bottom:1rem; }
    p { color:#b6c2d3; margin:.5rem 0; }
    section { margin:1.2rem 0; padding:1rem; border:1px solid #334155; border-radius:.5rem; }
    button { border:1px solid #39717a; border-radius:.4rem; padding:.5rem .9rem;
      background:#12343b; color:#99f6e4; cursor:pointer; font:inherit; }
    button:disabled { opacity:.5; cursor:wait; }
    button:focus-visible,textarea:focus-visible { outline:2px solid #7dd3fc; outline-offset:2px; }
    textarea { width:100%; min-height:5rem; box-sizing:border-box; background:#020617; color:#e2e8f0;
      border:1px solid #475569; border-radius:.4rem; padding:.7rem; margin:.4rem 0; font:12px/1.5 monospace; }
    .error,.attention { border-left:3px solid #fbbf24; padding:.7rem; background:#fbbf2410; color:#fde68a; }
    .error { border-color:#fb7185; color:#fda4af; }
    .status { display:inline-block; padding:.2rem .6rem; background:#334155; border-radius:.3rem; }
    .completed { color:#86efac; } .incomplete,.unknown_effects { color:#fde68a; }
    .metrics { display:grid; grid-template-columns:repeat(auto-fit,minmax(8rem,1fr)); gap:.8rem; }
    .metric { background:#0b1324; padding:.7rem; border-radius:.4rem; }
    .metric strong { display:block; color:#e2e8f0; font-size:1.35rem; }
    .metric span { color:#a4b3c7; font-size:.8rem; }
    pre,code { white-space:pre-wrap; overflow-wrap:anywhere; font-size:12px; }
    pre { max-height:24rem; overflow:auto; background:#020617; padding:.7rem; border-radius:.4rem; }
    details { margin:.6rem 0; } summary { cursor:pointer; color:#cbd5e1; }
    .table-wrap { overflow-x:auto; } table { width:100%; border-collapse:collapse; }
    th,td { text-align:left; border-bottom:1px solid #334155; padding:.5rem; }
  `;
  @property({ attribute: false }) reference: RunAuditReference | null = null;
  @state() private _input = '';
  @state() private _view: RunView | null = null;
  @state() private _error = '';
  @state() private _loading = false;
  private _generation = 0;

  firstUpdated() { this.shadowRoot?.querySelector('dialog')?.showModal(); }
  protected updated(changes: PropertyValues) {
    if (changes.has('reference') && this.reference) {
      this._input = JSON.stringify(this.reference, null, 2);
      void this._inspect();
    }
  }
  disconnectedCallback() { super.disconnectedCallback(); ++this._generation; }
  private _close() { this.dispatchEvent(new CustomEvent('close-inspector', { bubbles:true, composed:true })); }
  private async _inspect() {
    const generation = ++this._generation;
    this._view = null; this._error = ''; this._loading = true;
    try {
      const { reference } = auditIdentity(JSON.parse(this._input));
      const view = await inspectRun(reference);
      if (generation === this._generation) this._view = view;
    } catch (error) {
      if (generation === this._generation) this._error = error instanceof Error ? error.message : 'Inspection unavailable';
    } finally { if (generation === this._generation) this._loading = false; }
  }
  private _details(label: string, value: unknown) {
    return html`<details><summary>${label}</summary><pre>${readable(value)}</pre></details>`;
  }
  private _budget(view: RunView) {
    const b = view.budget?.snapshot;
    const recovered = view.recovery.recovered_budget;
    const root = recovered?.scopes.find(scope => scope.scope === 0);
    return html`<section><h3>Shared token budget</h3>
      ${b ? html`
        <p>Latest recorded snapshot: ${view.budget!.recorded_at}. This is not a live balance.
          Parent scopes include descendant usage. Available tokens also respect ancestor balances.</p>
        <div class="metrics">
          ${[['Limit',b.limit],['Recorded usage',b.usage.total_tokens],['Reserved',b.reserved_tokens],
            ['Uncertain charge',b.uncertain_tokens],['Available',b.available_tokens]].map(([label,value]) => html`
              <div class="metric"><strong>${value}</strong><span>${label}</span></div>`)}
        </div>
        ${b.exceeded ? html`<p class="attention">Budget exceeded; further allowance is closed.</p>` : ''}
        <p>Shared ledger <code>${b.root_id}</code> · scope ${b.scope}</p>
        <p>Uncertain charges remain spent until their outcome is established.</p>
      ` : html`<p>${root ? 'No final budget snapshot; reconstructed accounting is available below.' : 'Shared budget snapshot not recorded. Usage and remaining allowance are unknown.'}</p>`}
      ${view.budget_root ? html`<audit-reference .label=${'Inspect family accounting'} .reference=${view.budget_root.audit}></audit-reference>` : ''}
      ${root ? html`<h3>Reconstructed family accounting</h3>
        <p>Rebuilt from verified reservation and settlement records in this snapshot. Parent totals include descendants.
          A request without a settlement may still be active or interrupted; its full reservation stays charged.</p>
        <div class="metrics">
          ${[['Family limit',root.limit],['Recorded usage',root.usage.total_tokens],['Uncertain charge',root.uncertain_tokens],
            ['Remaining after charges',root.available_tokens]].map(([label,value]) => html`<div class="metric"><strong>${value}</strong><span>${label}</span></div>`)}
        </div>
        ${root.exceeded ? html`<p class="attention">Recorded usage violated its reservation. The family allowance is closed.</p>` : ''}
        <p>These recovered figures do not authorize resuming or replaying work.</p>
        ${this._details('Inference reservation history',recovered!.reservations)}
      ` : ''}
      ${this._details('Recorded run limits',view.limits)}
    </section>`;
  }
  private _result(view: RunView) {
    return html`
      <section aria-label="Verified run summary">
        <h3>Run <code>${view.audit.run_id}</code></h3>
        <span class="status ${view.status}">${view.status.replaceAll('_',' ')}</span>
        <p>Signature and chain verified against the supplied key. Retain that key independently;
          a key supplied alongside a journal is not independent proof of its origin.</p>
        <p>Agent <code>${view.agent_id}</code> · ${view.recovery.verified_records} verified records</p>
        <p>Last record: ${view.last_record_at}</p>
        ${view.recovery.requires_reconciliation ? html`<p class="attention">Reconciliation required for the original run.
          Missing or error outcomes do not establish that no effects occurred. Do not replay on this evidence alone.</p>` : ''}
        ${view.recovery.unverified_tail_bytes ? html`<p class="attention">${view.recovery.unverified_tail_bytes} trailing bytes are unverified.</p>` : ''}
        ${view.recovery.warnings.map(w => html`<p class="attention">${w}</p>`)}
        ${this._details('Snapshot and retained reference',{snapshot_sha256:view.snapshot_sha256,audit:view.audit})}
        ${this._details('Original terminal result',view.terminal)}
        ${this._details('Recorded source identity',view.source)}
      </section>
      ${view.invocation ? html`<section><h3>Invocation assessment</h3><p>Status: ${view.invocation.status}</p>
        ${view.invocation.error ? html`<p class="attention">${view.invocation.error}</p>` : ''}
        ${view.invocation.resolution ? html`<p>An operator recorded a separate assessment. The original journal and unknown outcomes remain unchanged.
          This receipt does not authorize replay.</p>${this._details('Signed operator assessment',view.invocation.resolution)}` : html`<p>No operator resolution returned.</p>`}
      </section>` : ''}
      <section><h3>Parent and delegated runs</h3>
        ${view.parent ? html`<audit-reference label="Inspect parent" .reference=${view.parent.audit}></audit-reference>`
          : html`<p>${view.parent_context ? 'Parent context was recorded without a navigable audit reference.' : 'No parent reference recorded.'}</p>`}
        ${view.children.length ? html`<p>Child outcomes below were recorded by this parent. Inspect each child to verify its own journal.</p>
          ${view.children.map(child => html`<div class="links"><audit-reference
            .label=${`Inspect child ${child.target ?? child.agent_id}`} .reference=${child.audit}></audit-reference>
            <span>Parent recorded: ${child.parent_recorded_outcome ? readable(child.parent_recorded_outcome) : 'No finish recorded'}</span></div>`)}`
          : html`<p>No delegated child references recorded.</p>`}
      </section>
      ${this._budget(view)}
      <section><h3>Effective permissions and resource ceilings</h3>
        <p>These are the access decisions recorded for individual actions. CPU, memory and time values are configured ceilings;
          measured CPU/memory usage and live shared admission occupancy are not available in this view.</p>
        ${view.permissions.length ? view.permissions.map(p => html`<details><summary>
          #${p.sequence} · ${p.decision} · ${p.contract?.name ?? p.tool_name ?? (p.target ? `${p.action_type} ${p.target}` : p.action_type) ?? 'action'}
          </summary>${p.reason ? html`<p>${p.reason}</p>` : ''}
          ${Object.keys(p.grants).length ? html`<pre>${readable(p.grants)}</pre>` : html`<p>No effective grant details recorded for this decision.</p>`}
          ${this._details('Decision contract',p)}</details>`)
          : html`<p>No per-action permission decisions recorded. This does not imply unrestricted access.</p>`}
      </section>
      <section><h3>Tracked effects</h3>
        <p>A recorded result is not independent confirmation of an external service's state.</p>
        ${view.recovery.effects.length ? html`<div class="table-wrap"><table>
          <thead><tr><th>Kind</th><th>Start record</th><th>Finish record</th><th>Outcome</th></tr></thead>
          <tbody>${view.recovery.effects.map(e => html`<tr><td>${e.identity.kind}</td><td>${e.start_sequence}</td>
            <td>${e.finish_sequence ?? 'Missing'}</td><td>${e.outcome.replaceAll('_',' ')}</td></tr>`)}</tbody>
        </table></div>` : html`<p>No effects tracked in this journal.</p>`}
      </section>`;
  }
  render() {
    return html`<dialog aria-labelledby="inspector-title" @cancel=${this._close}>
      <header><h2 id="inspector-title">Run Inspector</h2><button @click=${this._close}>Close inspector</button></header>
      <p>Inspect signed run evidence from this runtime's project. Administrative access is required.</p>
      <label for="audit-reference">Audit reference (JSON)</label>
      <textarea id="audit-reference" spellcheck="false" .value=${this._input}
        @input=${(e: InputEvent) => { this._input = (e.target as HTMLTextAreaElement).value; ++this._generation; this._view = null; this._error = ''; this._loading = false; }}></textarea>
      <div class="actions"><button ?disabled=${this._loading} @click=${this._inspect}>
        ${this._loading ? 'Verifying…' : 'Inspect / refresh'}</button></div>
      ${this._error ? html`<p class="error" role="alert">Evidence unavailable: ${this._error}. No verified result is displayed.</p>` : ''}
      <div aria-live="polite">${this._loading ? html`<p>Verifying journal snapshot…</p>` : ''}</div>
      ${this._view ? this._result(this._view) : ''}
    </dialog>`;
  }
}
