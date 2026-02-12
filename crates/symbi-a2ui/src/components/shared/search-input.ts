import { LitElement, html, css } from 'lit';
import { customElement, property } from 'lit/decorators.js';

@customElement('search-input')
export class SearchInput extends LitElement {
  static styles = css`
    :host {
      display: block;
    }

    .wrapper {
      position: relative;
    }

    input {
      width: 100%;
      padding: 0.5rem 0.75rem 0.5rem 2rem;
      background: #0a0a0a;
      border: 1px solid #374151;
      border-radius: 0.5rem;
      color: #e2e8f0;
      font-size: 0.8125rem;
      outline: none;
      box-sizing: border-box;
    }

    input:focus {
      border-color: #2dd4bf;
    }

    input::placeholder {
      color: #64748b;
    }

    .icon {
      position: absolute;
      left: 0.625rem;
      top: 50%;
      transform: translateY(-50%);
      color: #64748b;
      font-size: 0.875rem;
      pointer-events: none;
    }
  `;

  @property() placeholder = 'Search...';
  @property() value = '';
  @property({ type: Number }) debounce = 300;

  private _timeout?: ReturnType<typeof setTimeout>;

  private _onInput(e: InputEvent) {
    const val = (e.target as HTMLInputElement).value;
    this.value = val;
    if (this._timeout) clearTimeout(this._timeout);
    this._timeout = setTimeout(() => {
      this.dispatchEvent(
        new CustomEvent('search', { detail: val, bubbles: true, composed: true }),
      );
    }, this.debounce);
  }

  render() {
    return html`
      <div class="wrapper">
        <span class="icon">⌕</span>
        <input
          type="text"
          .value=${this.value}
          placeholder=${this.placeholder}
          @input=${this._onInput}
        />
      </div>
    `;
  }
}
