import { get } from './client.js';

export interface Resources { memory_bytes: number; cpu_nanos: number }
export interface Totals { workers: number; memory_bytes: number | null; cpu_nanos: number | null }
export interface WorkerOrigin {
  agent_id: string; run_id: string; public_key: string; dispatch_id: string;
  call_fingerprint: string; tool_name: string; iteration: number;
}
export interface Worker { lease: string; backend: string; phase: string; resources: Resources | null; origin: WorkerOrigin | null }
export interface StagingCapacity {
  initialized: boolean;
  limits: { max_snapshots: number; reserved_bytes: number };
  reserved: { snapshots: number; reserved_bytes: number };
  available: { snapshots: number; reserved_bytes: number };
  admission_blocked: boolean;
  reservations: { id: string; reserved_bytes: number; active_hold: boolean; worker_leases: string[] }[];
}
export interface Capacity {
  observed_at_unix_ms: number; state_dir: string;
  limits: Resources & { max_workers: number };
  reserved: Totals; available: Totals; unknown_resource_leases: number;
  admission_blocked: boolean; workers: Worker[];
  staging: StagingCapacity | null; staging_error: string | null;
}
export interface Measurement {
  lease: string; observed_at_unix_ms: number; source: string;
  cpu_percent: string | null; memory_usage: string | null;
  cpu_time_micros: number | null; memory_bytes: number | null;
}
export const inspectCapacity = (): Promise<Capacity> => get('/api/v1/sandbox/capacity');
export const measureWorker = (lease: string): Promise<Measurement> => get(`/api/v1/sandbox/workers/${encodeURIComponent(lease)}/usage`);
