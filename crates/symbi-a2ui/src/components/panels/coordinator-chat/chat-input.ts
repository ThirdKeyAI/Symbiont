import { LitElement, html, css } from 'lit';
import { customElement, property, state } from 'lit/decorators.js';

@customElement('chat-input')
export class ChatInput extends LitElement {
  static styles = css`
    :host {
      display: block;
    }

    .bar {
      display: flex;
      gap: 0.5rem;
      padding: 0.75rem 1rem;
      background: #111827;
      border-top: 1px solid rgba(255, 255, 255, 0.1);
    }

    textarea {
      flex: 1;
      resize: none;
      min-height: 2.5rem;
      max-height: 8rem;
      padding: 0.5rem 0.75rem;
      background: #0a0a0a;
      border: 1px solid #374151;
      border-radius: 0.5rem;
      color: #e2e8f0;
      font-family: 'Inter', ui-sans-serif, system-ui, sans-serif;
      font-size: 0.875rem;
      line-height: 1.4;
      outline: none;
      overflow-y: auto;
    }

    textarea:focus {
      border-color: #2dd4bf;
    }

    textarea:disabled {
      opacity: 0.5;
      cursor: not-allowed;
    }

    button {
      align-self: flex-end;
      padding: 0.5rem 1rem;
      background: linear-gradient(to right, #2dd4bf, #38bdf8);
      color: #0a0a0a;
      font-weight: 600;
      font-size: 0.875rem;
      border: none;
      border-radius: 0.5rem;
      cursor: pointer;
      white-space: nowrap;
    }

    button:hover {
      opacity: 0.9;
    }

    button:disabled {
      opacity: 0.4;
      cursor: not-allowed;
    }
  `;

  @property({ type: Boolean }) disabled = false;

  @state() private _text = '';

  private _onInput(e: InputEvent) {
    this._text = (e.target as HTMLTextAreaElement).value;
  }

  private _onKeydown(e: KeyboardEvent) {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      this._submit();
    }
  }

  private _submit() {
    const text = this._text.trim();
    if (!text || this.disabled) return;

    this.dispatchEvent(
      new CustomEvent('chat-submit', {
        detail: text,
        bubbles: true,
        composed: true,
      }),
    );
    this._text = '';
  }

  render() {
    return html`
      <div class="bar">
        <textarea
          rows="1"
          placeholder="Ask the coordinator..."
          .value=${this._text}
          @input=${this._onInput}
          @keydown=${this._onKeydown}
          ?disabled=${this.disabled}
        ></textarea>
        <button @click=${this._submit} ?disabled=${this.disabled || !this._text.trim()}>
          Send
        </button>
      </div>
    `;
  }
}
