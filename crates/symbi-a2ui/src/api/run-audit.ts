import { get } from './client.js';

export interface RunAuditReference { run_id: string; path: string; public_key: string }
export interface RunLink { agent_id: string; audit: RunAuditReference; target?: string; parent_recorded_outcome?: unknown }
export interface BudgetSnapshot {
  root_id: string; scope: number; limit: number; usage: { total_tokens: number };
  reserved_tokens: number; uncertain_tokens: number; available_tokens: number; exceeded: boolean;
}
export interface RunView {
  audit: RunAuditReference; agent_id: string; snapshot_sha256: string; status: string;
  first_record_at: string; last_record_at: string;
  parent: RunLink | null; parent_context: unknown; children: RunLink[];
  budget: { recorded_at: string; sequence: number; snapshot: BudgetSnapshot } | null;
  budget_root?: RunLink | null;
  permissions: { sequence: number; decision: string; tool_name: string | null; action_type: string | null; target: string | null;
    contract: { name?: string } | null; reason: string | null; grants: Record<string, unknown>;
  }[];
  recovery: { journal_complete: boolean; requires_reconciliation: boolean; verified_records: number;
    unverified_tail_bytes: number; warnings: string[];
    recovered_budget?: { root_id: string; scopes: BudgetSnapshot[]; reservations: unknown[] } | null;
    effects: { identity: { kind: string }; outcome: string; start_sequence: number; finish_sequence: number | null }[];
  };
  terminal: unknown; limits: unknown; source: unknown;
  invocation: { status: string; error?: string; resolution?: unknown } | null;
}
const uuid = '[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}';
const identity = new RegExp(`^(${uuid})\\.(${uuid})\\.jsonl$`, 'i');

export function auditIdentity(value: unknown): { agent: string; reference: RunAuditReference } {
  if (!value || typeof value !== 'object') throw new Error('Enter an audit reference object.');
  const r = value as Partial<RunAuditReference>;
  const match = typeof r.path === 'string' ? identity.exec(r.path.split('/').pop() ?? '') : null;
  if (!match || match[2] !== r.run_id || typeof r.public_key !== 'string' || !/^[0-9a-f]{64}$/i.test(r.public_key)) {
    throw new Error('Reference requires a matching agent.run.jsonl filename, run_id and 64-character public_key.');
  }
  return { agent: match[1], reference: r as RunAuditReference };
}

export function inspectRun(reference: RunAuditReference): Promise<RunView> {
  const { agent } = auditIdentity(reference);
  return get(`/api/v1/audit/runs/${agent}/${reference.run_id}?public_key=${reference.public_key}`);
}
