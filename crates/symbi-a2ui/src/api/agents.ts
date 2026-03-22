import { get } from './client.js';
import type { AgentSummary, AgentStatusResponse, GetAgentHistoryResponse } from './types.js';

export function listAgents(): Promise<AgentSummary[]> {
  return get<AgentSummary[]>('/api/v1/agents');
}

export function getAgentStatus(id: string): Promise<AgentStatusResponse> {
  return get<AgentStatusResponse>(`/api/v1/agents/${encodeURIComponent(id)}/status`);
}

export function getAgentHistory(id: string): Promise<GetAgentHistoryResponse> {
  return get<GetAgentHistoryResponse>(`/api/v1/agents/${encodeURIComponent(id)}/history`);
}
