/**
 * WebSocket message types for the Coordinator Chat protocol.
 *
 * Mirrors the Rust `ws_types.rs` enums. The `type` field is the discriminant.
 */

// ---------------------------------------------------------------------------
// Client → Server
// ---------------------------------------------------------------------------

export interface ChatSend {
  type: 'ChatSend';
  /** Client-generated message id. */
  id: string;
  /** Natural-language content. */
  content: string;
}

export interface Ping {
  type: 'Ping';
}

export type ClientMessage = ChatSend | Ping;

// ---------------------------------------------------------------------------
// Server → Client
// ---------------------------------------------------------------------------

export interface ChatChunk {
  type: 'ChatChunk';
  request_id: string;
  content: string;
  done: boolean;
}

export interface ToolCallStarted {
  type: 'ToolCallStarted';
  request_id: string;
  call_id: string;
  tool_name: string;
  arguments: string;
}

export interface ToolCallResult {
  type: 'ToolCallResult';
  request_id: string;
  call_id: string;
  result: string;
  is_error: boolean;
}

export interface PolicyDecision {
  type: 'PolicyDecision';
  request_id: string;
  action: string;
  decision: string;
  reason: string;
}

export interface WsError {
  type: 'Error';
  request_id: string | null;
  code: string;
  message: string;
}

export interface Pong {
  type: 'Pong';
}

export type ServerMessage =
  | ChatChunk
  | ToolCallStarted
  | ToolCallResult
  | PolicyDecision
  | WsError
  | Pong;
