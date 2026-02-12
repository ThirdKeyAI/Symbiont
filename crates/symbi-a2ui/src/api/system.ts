import { get } from './client.js';
import type { HealthResponse, SchedulerHealthResponse } from './types.js';

export function getHealth(): Promise<HealthResponse> {
  return get<HealthResponse>('/api/v1/health');
}

export function getSchedulerHealth(): Promise<SchedulerHealthResponse> {
  return get<SchedulerHealthResponse>('/api/v1/health/scheduler');
}

export function getMetrics(): Promise<Record<string, unknown>> {
  return get<Record<string, unknown>>('/api/v1/metrics');
}
