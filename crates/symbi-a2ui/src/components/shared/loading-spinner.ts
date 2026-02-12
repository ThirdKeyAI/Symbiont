import { LitElement, html, css } from 'lit';
import { customElement, property } from 'lit/decorators.js';

@customElement('loading-spinner')
export class LoadingSpinner extends LitElement {
  static styles = css`
    :host {
      display: flex;
      align-items: center;
      justify-content: center;
      padding: 2rem;
    }

    .container {
      display: flex;
      flex-direction: column;
      align-items: center;
      gap: 0.75rem;
    }

    .spinner {
      width: 2rem;
      height: 2rem;
      border: 2px solid #374151;
      border-top-color: #2dd4bf;
      border-radius: 50%;
      animation: spin 0.8s linear infinite;
    }

    .label {
      color: #64748b;
      font-size: 0.8125rem;
    }

    @keyframes spin {
      to { transform: rotate(360deg); }
    }
  `;

  @property() label = 'Loading...';

  render() {
    return html`
      <div class="container">
        <div class="spinner"></div>
        <span class="label">${this.label}</span>
      </div>
    `;
  }
}
