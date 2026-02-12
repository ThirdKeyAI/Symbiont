import { get } from './client.js';
import type { ScheduleSummary, ScheduleHistoryResponse } from './types.js';

export function listSchedules(): Promise<ScheduleSummary[]> {
  return get<ScheduleSummary[]>('/api/v1/schedules');
}

export function getScheduleHistory(id: string): Promise<ScheduleHistoryResponse> {
  return get<ScheduleHistoryResponse>(`/api/v1/schedules/${encodeURIComponent(id)}/history`);
}
