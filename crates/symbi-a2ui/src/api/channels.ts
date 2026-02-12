import { get } from './client.js';
import type {
  ChannelSummary,
  ChannelHealthResponse,
  ChannelAuditResponse,
} from './types.js';

export function listChannels(): Promise<ChannelSummary[]> {
  return get<ChannelSummary[]>('/api/v1/channels');
}

export function getChannelHealth(id: string): Promise<ChannelHealthResponse> {
  return get<ChannelHealthResponse>(`/api/v1/channels/${encodeURIComponent(id)}/health`);
}

export function getChannelAudit(id: string): Promise<ChannelAuditResponse> {
  return get<ChannelAuditResponse>(`/api/v1/channels/${encodeURIComponent(id)}/audit`);
}
