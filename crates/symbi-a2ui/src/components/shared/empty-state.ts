import { LitElement, html, css } from 'lit';
import { customElement, property } from 'lit/decorators.js';

@customElement('empty-state')
export class EmptyState extends LitElement {
  static styles = css`
    :host {
      display: flex;
      align-items: center;
      justify-content: center;
      padding: 3rem 1rem;
    }

    .container {
      text-align: center;
      max-width: 20rem;
    }

    .icon {
      font-size: 2rem;
      margin-bottom: 0.75rem;
      opacity: 0.4;
    }

    .title {
      color: #e2e8f0;
      font-size: 0.9375rem;
      font-weight: 500;
      margin: 0 0 0.375rem;
    }

    .description {
      color: #64748b;
      font-size: 0.8125rem;
      margin: 0;
      line-height: 1.5;
    }
  `;

  @property() icon = '○';
  @property() title = 'No data';
  @property() description = '';

  render() {
    return html`
      <div class="container">
        <div class="icon">${this.icon}</div>
        <p class="title">${this.title}</p>
        ${this.description ? html`<p class="description">${this.description}</p>` : ''}
      </div>
    `;
  }
}
