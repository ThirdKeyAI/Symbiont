import { LitElement, html, css } from 'lit';
import { customElement, state } from 'lit/decorators.js';
import { WsClient, type ConnectionState } from '../../../api/ws-client.js';
import type {
  ServerMessage,
  AuditOpened,
  ChatChunk,
  ToolCallStarted,
  ToolCallResult as ToolCallResultMsg,
  PolicyDecision,
} from '../../../api/ws-types.js';
import type { ChatMessageData, ToolTrace, PolicyTrace } from './chat-message.js';
import type { ReasoningPhase } from './reasoning-trace.js';

let _msgCounter = 0;
const PENDING_KEY = 'symbi.chat.pending.v1';
interface PendingRequest { id: string; content: string }
function restoreRequest(): PendingRequest | null {
  try {
    const saved = JSON.parse(sessionStorage.getItem(PENDING_KEY) ?? 'null');
    if (saved && typeof saved.id === 'string' && /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(saved.id)
        && typeof saved.content === 'string' && new TextEncoder().encode(saved.content).length <= 65536) return saved;
  } catch { /* No transport is initiated by restoring browser state. */ }
  return null;
}

@customElement('coordinator-chat-panel')
export class CoordinatorChatPanel extends LitElement {
  static styles = css`
    :host {
      display: flex;
      flex-direction: column;
      height: 100%;
    }

    .header {
      display: flex;
      align-items: center;
      justify-content: space-between;
      padding: 0 0 1rem;
    }

    h2 {
      margin: 0;
      font-size: 1.125rem;
      font-weight: 600;
      color: #e2e8f0;
    }

    .connection-dot {
      width: 0.5rem;
      height: 0.5rem;
      border-radius: 50%;
      display: inline-block;
      margin-right: 0.375rem;
    }

    .connection-dot.connected {
      background: #22c55e;
    }

    .connection-dot.connecting {
      background: #eab308;
    }

    .connection-dot.disconnected,
    .connection-dot.error {
      background: #ef4444;
    }

    .connection-label {
      font-size: 0.75rem;
      color: #64748b;
      display: flex;
      align-items: center;
    }

    .messages {
      flex: 1;
      overflow-y: auto;
      padding: 0.5rem 0;
    }

    .empty-state {
      display: flex;
      flex-direction: column;
      align-items: center;
      justify-content: center;
      height: 100%;
      color: #475569;
      font-size: 0.875rem;
      gap: 0.5rem;
    }

    .empty-icon {
      font-size: 2rem;
      opacity: 0.3;
    }

    .thinking {
      display: flex;
      align-items: center;
      gap: 0.5rem;
      padding: 0.75rem 1rem;
      max-width: 85%;
      border-radius: 0.75rem;
      background: #111827;
      border: 1px solid rgba(255, 255, 255, 0.1);
      color: #94a3b8;
      font-size: 0.875rem;
      margin-bottom: 1rem;
    }

    .thinking-dots {
      display: flex;
      gap: 0.25rem;
    }

    .thinking-dots span {
      width: 0.375rem;
      height: 0.375rem;
      border-radius: 50%;
      background: #2dd4bf;
      animation: pulse 1.4s infinite ease-in-out;
    }

    .thinking-dots span:nth-child(2) {
      animation-delay: 0.2s;
    }

    .thinking-dots span:nth-child(3) {
      animation-delay: 0.4s;
    }

    @keyframes pulse {
      0%, 80%, 100% {
        opacity: 0.2;
        transform: scale(0.8);
      }
      40% {
        opacity: 1;
        transform: scale(1);
      }
    }
  `;

  @state() private _messages: ChatMessageData[] = [];
  @state() private _isProcessing = false;
  @state() private _connectionState: ConnectionState = 'disconnected';
  @state() private _activePhase: ReasoningPhase = 'idle';

  @state() private _pendingAudit: AuditOpened | null = null;
  @state() private _pendingRequest: PendingRequest | null = restoreRequest();
  @state() private _notAdmitted = false;
  @state() private _checkingOutcome = false;
  @state() private _storageError = '';


  private _ws = WsClient.instance();
  // Tool traces indexed by call_id
  private _pendingToolTraces = new Map<string, ToolTrace>();
  private _pendingPolicyTraces: PolicyTrace[] = [];

  private _onServerMessage = (e: Event) => {
    const msg = (e as CustomEvent<ServerMessage>).detail;
    this._handleServerMessage(msg);
  };

  private _onStateChange = (e: Event) => {
    this._connectionState = (e as CustomEvent<ConnectionState>).detail;
    if (this._isProcessing && ['error', 'disconnected'].includes(this._connectionState)) {
      this._onError({message:'Connection lost; the run outcome is unknown. Inspect its audit before retrying.', request_id:this._pendingRequest?.id ?? null});
    }
  };

  connectedCallback() {
    super.connectedCallback();
    this._ws.addEventListener('server-message', this._onServerMessage);
    this._ws.addEventListener('state-change', this._onStateChange);
    this._connectionState = this._ws.state;
    this._ws.connect();
  }

  disconnectedCallback() {
    super.disconnectedCallback();
    this._ws.removeEventListener('server-message', this._onServerMessage);
    this._ws.removeEventListener('state-change', this._onStateChange);
  }

  private _handleServerMessage(msg: ServerMessage) {
    if ('request_id' in msg && msg.request_id && msg.request_id !== this._pendingRequest?.id) return;
    switch (msg.type) {
      case 'AuditOpened':
        this._pendingAudit = msg;
        break;
      case 'ChatChunk':
        this._onChatChunk(msg);
        break;
      case 'ToolCallStarted':
        this._onToolCallStarted(msg);
        break;
      case 'ToolCallResult':
        this._onToolCallResult(msg);
        break;
      case 'PolicyDecision':
        this._onPolicyDecision(msg);
        break;
      case 'Error':
        this._onError(msg);
        break;
    }
  }

  private _onChatChunk(msg: ChatChunk) {
    if (msg.done) {
      // Finalize assistant message
      const toolTraces = Array.from(this._pendingToolTraces.values());
      const policyTraces = [...this._pendingPolicyTraces];
      this._pendingToolTraces.clear();
      this._pendingPolicyTraces = [];

      this._messages = [
        ...this._messages,
        {
          id: `msg-${++_msgCounter}`,
          role: 'assistant',
          content: msg.replayed ? `Saved response (no work repeated; earlier conversation context is not restored):\n\n${msg.content}` : msg.content,
          audit: this._pendingAudit?.request_id === msg.request_id ? this._pendingAudit.audit : undefined,
          toolTraces: toolTraces.length > 0 ? toolTraces : undefined,
          policyTraces: policyTraces.length > 0 ? policyTraces : undefined,
        },
      ];
      this._clearPending();
      this._pendingAudit = null;
      this._isProcessing = false;
      this._activePhase = 'idle';
      this._scrollToBottom();
    }
  }

  private _onToolCallStarted(msg: ToolCallStarted) {
    this._activePhase = 'act';
    this._pendingToolTraces.set(msg.call_id, {
      call_id: msg.call_id,
      tool_name: msg.tool_name,
      arguments: msg.arguments,
    });
  }

  private _onToolCallResult(msg: ToolCallResultMsg) {
    this._activePhase = 'observe';
    const trace = this._pendingToolTraces.get(msg.call_id);
    if (trace) {
      trace.result = msg.result;
      trace.is_error = msg.is_error;
    }
  }

  private _onPolicyDecision(msg: PolicyDecision) {
    this._activePhase = 'gate';
    this._pendingPolicyTraces.push({
      action: msg.action,
      decision: msg.decision,
      reason: msg.reason,
    });
  }

  private _onError(msg: { message: string; request_id: string | null; code?: string }) {
    this._checkingOutcome = false;
    this._notAdmitted = msg.code === 'INVOCATION_NOT_FOUND' || msg.code === 'SESSION_BUSY';
    if (msg.code === 'INVOCATION_RECONCILED') this._clearPending();
    this._messages = [
      ...this._messages,
      {
        id: `msg-${++_msgCounter}`,
        role: 'assistant',
        content: `Error: ${msg.message}`,
        audit: this._pendingAudit?.request_id === msg.request_id ? this._pendingAudit?.audit : undefined,
      },
    ];
    this._pendingAudit = null;
    this._pendingToolTraces.clear();
    this._pendingPolicyTraces = [];
    this._isProcessing = false;
    this._activePhase = 'idle';
  }

  private _clearPending() {
    try { sessionStorage.removeItem(PENDING_KEY); } catch { /* Retrying a retained ID remains safe. */ }
    this._pendingRequest = null;
    this._notAdmitted = false;
    this._checkingOutcome = false;
  }

  private _checkOutcome() {
    if (!this._pendingRequest) return;
    this._isProcessing = true;
    this._checkingOutcome = true;
    if (!this._ws.send({ type: 'ChatInspect', ...this._pendingRequest })) {
      this._isProcessing = false;
    }
  }

  private _retryUnadmitted() {
    if (!this._pendingRequest || !this._notAdmitted) return;
    this._isProcessing = true;
    this._notAdmitted = false;
    this._activePhase = 'reason';
    if (!this._ws.send({ type: 'ChatSend', ...this._pendingRequest })) this._isProcessing = false;
  }

  private _leaveUnresolved() {
    this._messages = [...this._messages, { id: `msg-${++_msgCounter}`, role: 'assistant',
      content: `Request ${this._pendingRequest?.id} remains in runtime history. Dismissing this notice does not resolve or retry it.`,
      audit: this._pendingAudit?.audit }];
    this._clearPending();
  }

  private _onChatSubmit(e: CustomEvent<string>) {
    if (this._isProcessing || this._pendingRequest || this._connectionState !== 'connected') return;
    const content = e.detail;
    const id = crypto.randomUUID();
    const request = { id, content };
    try { sessionStorage.setItem(PENDING_KEY, JSON.stringify(request)); }
    catch {
      this._storageError = 'Browser session storage is unavailable. The request was not sent because its retry identity could not be retained.';
      return;
    }
    this._storageError = '';
    this._pendingRequest = request;
    this._pendingAudit = null;
    this._notAdmitted = false;
    this._messages = [...this._messages, { id, role: 'user', content }];
    this._isProcessing = true;
    this._activePhase = 'reason';
    if (!this._ws.send({ type: 'ChatSend', ...request })) this._isProcessing = false;
    this._scrollToBottom();
  }

  private _thinkingLabel(): string {
    if (this._checkingOutcome) return 'Checking saved outcome…';
    switch (this._activePhase) {
      case 'reason':
        return 'Thinking…';
      case 'gate':
        return 'Evaluating policy…';
      case 'act':
        return 'Calling tools…';
      case 'observe':
        return 'Processing results…';
      default:
        return 'Thinking…';
    }
  }

  private _scrollToBottom() {
    requestAnimationFrame(() => {
      const el = this.shadowRoot?.querySelector('.messages');
      if (el) el.scrollTop = el.scrollHeight;
    });
  }

  render() {
    return html`
      <div class="header">
        <h2>Coordinator</h2>
        <span class="connection-label">
          <span class="connection-dot ${this._connectionState}"></span>
          ${this._connectionState}
        </span>
      </div>

      <div class="messages">
        ${this._messages.length === 0 && !this._isProcessing
          ? html`
              <div class="empty-state">
                <span class="empty-icon">&#9671;</span>
                <span>Ask the coordinator about your agent fleet</span>
              </div>
            `
          : this._messages.map(
              (m) => html`<chat-message .data=${m}></chat-message>`,
            )}
        ${this._isProcessing
          ? html`
              <div class="thinking">
                <div class="thinking-dots">
                  <span></span><span></span><span></span>
                </div>
                ${this._thinkingLabel()}
                ${this._pendingAudit ? html`<audit-reference .reference=${this._pendingAudit.audit}></audit-reference>` : ''}
              </div>
            `
          : ''}
      </div>

      ${this._isProcessing && !this._checkingOutcome
        ? html`<reasoning-trace .activePhase=${this._activePhase}></reasoning-trace>`
        : ''}

      ${this._storageError ? html`<p role="alert">${this._storageError}</p>` : ''}
      ${this._pendingRequest && !this._isProcessing ? html`<section aria-label="Pending chat request">
        <p>Retained request <code>${this._pendingRequest.id}</code>. Check its saved outcome before sending new work.</p>
        <p>Reconnecting starts fresh conversation context; outcome checks do not restore the earlier conversation.</p>
        <button @click=${this._checkOutcome} ?disabled=${this._connectionState !== 'connected'}>Check outcome</button>
        ${this._notAdmitted ? html`<p>No admission was found. Retrying the same request may start work.</p>
          <button @click=${this._retryUnadmitted} ?disabled=${this._connectionState !== 'connected'}>Retry same request</button>` : ''}
        <button @click=${this._leaveUnresolved}>Dismiss retained request</button>
      </section>` : ''}
      <chat-input
        .disabled=${this._isProcessing || !!this._pendingRequest || this._connectionState !== 'connected'}
        @chat-submit=${this._onChatSubmit}
      ></chat-input>
    `;
  }
}
