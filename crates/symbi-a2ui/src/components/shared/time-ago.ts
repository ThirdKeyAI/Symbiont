import { LitElement, html, css } from 'lit';
import { customElement, property, state } from 'lit/decorators.js';

const UNITS: [string, number][] = [
  ['y', 31_536_000],
  ['mo', 2_592_000],
  ['d', 86_400],
  ['h', 3_600],
  ['m', 60],
  ['s', 1],
];

function relative(iso: string): string {
  const diff = (Date.now() - new Date(iso).getTime()) / 1000;
  if (diff < 0) return 'just now';
  for (const [unit, secs] of UNITS) {
    const val = Math.floor(diff / secs);
    if (val >= 1) return `${val}${unit} ago`;
  }
  return 'just now';
}

@customElement('time-ago')
export class TimeAgo extends LitElement {
  static styles = css`
    :host {
      display: inline;
      color: #64748b;
      font-size: inherit;
    }

    span {
      cursor: default;
    }
  `;

  @property() datetime = '';
  @state() private _text = '';
  private _interval?: ReturnType<typeof setInterval>;

  connectedCallback() {
    super.connectedCallback();
    this._update();
    this._interval = setInterval(() => this._update(), 15_000);
  }

  disconnectedCallback() {
    super.disconnectedCallback();
    if (this._interval) clearInterval(this._interval);
  }

  updated(changed: Map<string, unknown>) {
    if (changed.has('datetime')) this._update();
  }

  private _update() {
    this._text = this.datetime ? relative(this.datetime) : '—';
  }

  render() {
    const full = this.datetime ? new Date(this.datetime).toLocaleString() : '';
    return html`<span title=${full}>${this._text}</span>`;
  }
}
