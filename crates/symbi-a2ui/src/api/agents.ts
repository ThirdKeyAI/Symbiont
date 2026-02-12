import { get } from './client.js';
import type { AgentStatusResponse, GetAgentHistoryResponse } from './types.js';

export function listAgents(): Promise<string[]> {
  return get<string[]>('/api/v1/agents');
}

export function getAgentStatus(id: string): Promise<AgentStatusResponse> {
  return get<AgentStatusResponse>(`/api/v1/agents/${encodeURIComponent(id)}/status`);
}

export function getAgentHistory(id: string): Promise<GetAgentHistoryResponse> {
  return get<GetAgentHistoryResponse>(`/api/v1/agents/${encodeURIComponent(id)}/history`);
}
