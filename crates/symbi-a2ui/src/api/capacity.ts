import { get } from './client.js';

export interface Resources { memory_bytes: number; cpu_nanos: number }
export interface Totals { workers: number; memory_bytes: number | null; cpu_nanos: number | null }
export interface Worker { lease: string; backend: string; phase: string; resources: Resources | null }
export interface Capacity {
  observed_at_unix_ms: number; state_dir: string;
  limits: Resources & { max_workers: number };
  reserved: Totals; available: Totals; unknown_resource_leases: number;
  admission_blocked: boolean; workers: Worker[];
}
export interface Measurement {
  lease: string; observed_at_unix_ms: number; source: string;
  cpu_percent: string | null; memory_usage: string | null;
  cpu_time_micros: number | null; memory_bytes: number | null;
}
export const inspectCapacity = (): Promise<Capacity> => get('/api/v1/sandbox/capacity');
export const measureWorker = (lease: string): Promise<Measurement> => get(`/api/v1/sandbox/workers/${encodeURIComponent(lease)}/usage`);
