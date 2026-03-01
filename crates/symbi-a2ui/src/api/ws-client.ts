/**
 * WebSocket client for the Coordinator Chat.
 *
 * Singleton that auto-reconnects with exponential backoff and dispatches
 * typed CustomEvents for each ServerMessage variant.
 */

import { getToken } from './client.js';
import type { ClientMessage, ServerMessage } from './ws-types.js';

export type ConnectionState = 'connecting' | 'connected' | 'disconnected' | 'error';

export class WsClient extends EventTarget {
  private _ws: WebSocket | null = null;
  private _state: ConnectionState = 'disconnected';
  private _reconnectDelay = 1000;
  private _reconnectTimer: ReturnType<typeof setTimeout> | null = null;
  private _shouldReconnect = false;

  private static _instance: WsClient | null = null;

  static instance(): WsClient {
    if (!WsClient._instance) {
      WsClient._instance = new WsClient();
    }
    return WsClient._instance;
  }

  get state(): ConnectionState {
    return this._state;
  }

  connect(): void {
    if (this._ws && (this._ws.readyState === WebSocket.OPEN || this._ws.readyState === WebSocket.CONNECTING)) {
      return;
    }

    this._shouldReconnect = true;
    this._doConnect();
  }

  disconnect(): void {
    this._shouldReconnect = false;
    if (this._reconnectTimer) {
      clearTimeout(this._reconnectTimer);
      this._reconnectTimer = null;
    }
    if (this._ws) {
      this._ws.close(1000, 'Client disconnect');
      this._ws = null;
    }
    this._setState('disconnected');
  }

  send(msg: ClientMessage): void {
    if (this._ws && this._ws.readyState === WebSocket.OPEN) {
      this._ws.send(JSON.stringify(msg));
    }
  }

  private _doConnect(): void {
    const token = getToken();
    if (!token) {
      this._setState('error');
      return;
    }

    const proto = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
    const url = `${proto}//${window.location.host}/ws/chat?token=${encodeURIComponent(token)}`;

    this._setState('connecting');
    const ws = new WebSocket(url);

    ws.onopen = () => {
      this._setState('connected');
      this._reconnectDelay = 1000; // reset backoff
    };

    ws.onmessage = (event: MessageEvent) => {
      try {
        const msg: ServerMessage = JSON.parse(event.data);
        this.dispatchEvent(new CustomEvent('server-message', { detail: msg }));
      } catch {
        // ignore malformed frames
      }
    };

    ws.onerror = () => {
      this._setState('error');
    };

    ws.onclose = () => {
      this._ws = null;
      this._setState('disconnected');
      this._scheduleReconnect();
    };

    this._ws = ws;
  }

  private _scheduleReconnect(): void {
    if (!this._shouldReconnect) return;

    this._reconnectTimer = setTimeout(() => {
      this._reconnectTimer = null;
      this._doConnect();
    }, this._reconnectDelay);

    // Exponential backoff: 1s → 2s → 4s → 8s → ... → max 30s
    this._reconnectDelay = Math.min(this._reconnectDelay * 2, 30_000);
  }

  private _setState(s: ConnectionState): void {
    if (this._state === s) return;
    this._state = s;
    this.dispatchEvent(new CustomEvent('state-change', { detail: s }));
  }
}
